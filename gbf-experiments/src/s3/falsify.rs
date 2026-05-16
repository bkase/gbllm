//! S3 test-only falsification harness.

use std::cell::RefCell;
use std::collections::BTreeMap;

use gbf_artifact::ids::ArtifactPath;
use gbf_artifact::{
    AggregationKind, BOS_ID, ConformanceEnvelope, ConformanceError, EOS_ID, EnvelopeGate,
    MetricGate, QuantizationGapSummary, RESERVED_ID, SeedConformanceEnvelope, SemanticCheckpoint,
    TextCharSeq, VOCAB_SIZE, canonical_conformance_bytes,
};
use gbf_data::charset_v1::{encode_charset_v1, normalize_raw};
use gbf_foundation::{CanonicalJson, CanonicalJsonError, Hash256, sha256};
use gbf_oracle::artifact::adversarial_fixture::{
    adversarial_artifact_fixture, name_resolver_logits_for_fixture, separating_prompt,
};
use gbf_oracle::artifact::quant_spec_resolver_logits;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::s3::schema::{HypothesisStatus, S3Hypothesis, S3PhaseLogEvent, emit_s3_phase_log_event};
use crate::s3::workload::V0SuccessPerSeed;

// Cargo does not set `cfg(test)` for the normal library artifact linked by
// integration tests. Match the B7 module-level guard: debug integration builds
// are permitted, while release-like builds reject this test-only feature.
#[cfg(not(any(test, debug_assertions)))]
compile_error!("the unified `falsify` feature must only be enabled in test builds");

thread_local! {
    static ACTIVE_BROKEN_KIND: RefCell<Option<BrokenKind>> = const { RefCell::new(None) };
}

/// Tracing target for S3 falsification events.
pub const FALSIFICATION_LOG_TARGET: &str = "gbf_experiments::s3::falsify";

/// Schema id for the canonical S3 falsification suite report.
pub const FALSIFICATION_S3_SUITE_SCHEMA: &str = "s3_falsification_suite.v1";

/// Pinned hash for the F1-broken-S3..F9-broken-S3 test-source suite.
pub const FALSIFICATION_S3_SUITE_HASH: &str =
    "sha256:916700709ed532667de7788d1b8373baafe0fe715d1652ca787789a9e4e0a248";

/// Event emitted when the S3 falsification suite starts.
pub const EVENT_NAME_SUITE_STARTED: &str = "s3::falsify::suite_started";

/// Event emitted before a substitute is exercised.
pub const EVENT_NAME_SUBSTITUTE_RUN: &str = "s3::falsify::substitute_run";

/// Event emitted after a substitute produced its verdict.
pub const EVENT_NAME_SUBSTITUTE_COMPLETE: &str = "s3::falsify::substitute_complete";

/// Event emitted when the S3 falsification suite completes.
pub const EVENT_NAME_SUITE_COMPLETE: &str = "s3::falsify::suite_complete";

/// Number of deliberately broken S3 substitutes in RFC section 14.
pub const SUBSTITUTE_COUNT: usize = 9;

/// One deliberately broken S3 substitute.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrokenKind {
    /// F1-broken-S3: charset normalization incorrectly case-folds text.
    F1CharsetV1LossyNormalization,
    /// F2-broken-S3: Kneser-Ney smoothing is replaced by uniform probabilities.
    F2FiveGramSmoothingUniform,
    /// F3-broken-S3: generated text accepts invalid charset/control ids.
    F3ModelEmitsInvalidCharset,
    /// F4-broken-S3: artifact oracle resolves weights by name instead of QuantSpec.
    F4ArtifactOracleDroppedQuantResolve,
    /// F5-broken-S3: bundle export serializes maps in nondeterministic order.
    F5BundleExportNondeterministicMapIter,
    /// F6-broken-S3: tied embedding/classifier alias is exported as split payloads.
    F6TiedEmbeddingExportSplit,
    /// F7-broken-S3: v0_success repeat-run gate is disabled.
    F7V0SuccessRepetitionCollapse,
    /// F8-broken-S3: oracle metrics aggregate softmax over prompt-wide logits.
    F8OracleSoftmaxOverConcatLogits,
    /// F9-broken-S3: F4 scheduler ramp records the wrong carry-through.
    F9PhaseSchedulerWrongRamp,
}

impl BrokenKind {
    /// All S3 broken substitutes in RFC order.
    pub const ALL: [Self; SUBSTITUTE_COUNT] = [
        Self::F1CharsetV1LossyNormalization,
        Self::F2FiveGramSmoothingUniform,
        Self::F3ModelEmitsInvalidCharset,
        Self::F4ArtifactOracleDroppedQuantResolve,
        Self::F5BundleExportNondeterministicMapIter,
        Self::F6TiedEmbeddingExportSplit,
        Self::F7V0SuccessRepetitionCollapse,
        Self::F8OracleSoftmaxOverConcatLogits,
        Self::F9PhaseSchedulerWrongRamp,
    ];

    /// Stable log/report label.
    #[must_use]
    pub const fn substitute_name(self) -> &'static str {
        match self {
            Self::F1CharsetV1LossyNormalization => "F1-broken-S3",
            Self::F2FiveGramSmoothingUniform => "F2-broken-S3",
            Self::F3ModelEmitsInvalidCharset => "F3-broken-S3",
            Self::F4ArtifactOracleDroppedQuantResolve => "F4-broken-S3",
            Self::F5BundleExportNondeterministicMapIter => "F5-broken-S3",
            Self::F6TiedEmbeddingExportSplit => "F6-broken-S3",
            Self::F7V0SuccessRepetitionCollapse => "F7-broken-S3",
            Self::F8OracleSoftmaxOverConcatLogits => "F8-broken-S3",
            Self::F9PhaseSchedulerWrongRamp => "F9-broken-S3",
        }
    }

    /// Stable implementation label.
    #[must_use]
    pub const fn implementation_slug(self) -> &'static str {
        match self {
            Self::F1CharsetV1LossyNormalization => "charset_v1_lossy_normalization",
            Self::F2FiveGramSmoothingUniform => "five_gram_smoothing_uniform",
            Self::F3ModelEmitsInvalidCharset => "model_emits_invalid_charset",
            Self::F4ArtifactOracleDroppedQuantResolve => "artifact_oracle_dropped_quant_resolve",
            Self::F5BundleExportNondeterministicMapIter => {
                "bundle_export_nondeterministic_map_iter"
            }
            Self::F6TiedEmbeddingExportSplit => "tied_embedding_export_split",
            Self::F7V0SuccessRepetitionCollapse => "v0_success_repetition_collapse",
            Self::F8OracleSoftmaxOverConcatLogits => "oracle_softmax_over_concat_logits",
            Self::F9PhaseSchedulerWrongRamp => "phase_scheduler_wrong_ramp",
        }
    }

    /// Hypotheses the substitute is expected to refute.
    #[must_use]
    pub fn target_hypotheses(self) -> Vec<S3Hypothesis> {
        match self {
            Self::F1CharsetV1LossyNormalization => vec![S3Hypothesis::H1],
            Self::F2FiveGramSmoothingUniform => vec![S3Hypothesis::H2],
            Self::F3ModelEmitsInvalidCharset => vec![S3Hypothesis::H3],
            Self::F4ArtifactOracleDroppedQuantResolve => vec![S3Hypothesis::H4, S3Hypothesis::H6],
            Self::F5BundleExportNondeterministicMapIter => vec![S3Hypothesis::H5],
            Self::F6TiedEmbeddingExportSplit => vec![S3Hypothesis::H5],
            Self::F7V0SuccessRepetitionCollapse => vec![S3Hypothesis::H3],
            Self::F8OracleSoftmaxOverConcatLogits => vec![S3Hypothesis::H4],
            Self::F9PhaseSchedulerWrongRamp => vec![S3Hypothesis::H7],
        }
    }

    /// Stable target hypothesis label for logs.
    #[must_use]
    pub fn target_hypothesis_label(self) -> String {
        self.target_hypotheses()
            .into_iter()
            .map(|hypothesis| format!("{hypothesis:?}"))
            .collect::<Vec<_>>()
            .join("+")
    }

    /// Expected verdict for the substitute.
    #[must_use]
    pub fn expected_verdict(self) -> String {
        format!("{} Refuted", self.target_hypothesis_label())
    }
}

/// RAII guard for an installed S3 broken substitute.
#[derive(Debug)]
pub struct Guard {
    previous: Option<BrokenKind>,
}

/// Install one broken substitute for the current thread.
#[must_use]
pub fn install_broken_impl(kind: BrokenKind) -> Guard {
    let previous = ACTIVE_BROKEN_KIND.with(|active| active.replace(Some(kind)));
    Guard { previous }
}

impl Drop for Guard {
    fn drop(&mut self) {
        ACTIVE_BROKEN_KIND.with(|active| {
            active.replace(self.previous);
        });
    }
}

/// Return the currently active broken substitute for this thread.
#[must_use]
pub fn active_broken_kind() -> Option<BrokenKind> {
    ACTIVE_BROKEN_KIND.with(|active| *active.borrow())
}

/// Whether a specific broken substitute is active on this thread.
#[must_use]
pub fn is_active(kind: BrokenKind) -> bool {
    active_broken_kind() == Some(kind)
}

/// Result for one S3 falsification case.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FalsificationCaseResult {
    /// Broken substitute label.
    pub substitute_name: String,
    /// Stable implementation slug.
    pub implementation_slug: String,
    /// Target hypotheses this substitute must refute.
    pub target_hypotheses: Vec<S3Hypothesis>,
    /// Expected verdict.
    pub expected_verdict: String,
    /// Observed verdict.
    pub observed_verdict: String,
    /// Whether the observed verdict matched the expected refutation.
    pub matches_expected: bool,
}

impl FalsificationCaseResult {
    /// Construct a case result.
    #[must_use]
    pub fn new(
        kind: BrokenKind,
        observed_verdict: impl Into<String>,
        matches_expected: bool,
    ) -> Self {
        Self {
            substitute_name: kind.substitute_name().to_owned(),
            implementation_slug: kind.implementation_slug().to_owned(),
            target_hypotheses: kind.target_hypotheses(),
            expected_verdict: kind.expected_verdict(),
            observed_verdict: observed_verdict.into(),
            matches_expected,
        }
    }
}

/// Canonical S3 falsification suite report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FalsificationSuiteReport {
    /// Schema id.
    pub schema: String,
    /// Per-source SHA-256 digests for falsification harness files.
    pub source_digests: BTreeMap<String, Hash256>,
    /// Per-case falsification results.
    pub results: Vec<FalsificationCaseResult>,
    /// AND over the nine expected catches.
    pub falsification_s3_passed: bool,
    /// Canonical suite hash over source digests and results.
    pub falsification_s3_suite_hash: Hash256,
}

/// Build the canonical S3 falsification suite report and hash.
pub fn suite_report(
    source_digests: BTreeMap<String, Hash256>,
    results: Vec<FalsificationCaseResult>,
) -> Result<FalsificationSuiteReport, CanonicalJsonError> {
    let falsification_s3_passed = results.iter().all(|result| result.matches_expected);
    let preimage = json!({
        "schema": FALSIFICATION_S3_SUITE_SCHEMA,
        "source_digests": source_digests,
        "results": results,
        "falsification_s3_passed": falsification_s3_passed,
    });
    let falsification_s3_suite_hash = sha256(CanonicalJson::value_to_vec(&preimage)?);
    Ok(FalsificationSuiteReport {
        schema: FALSIFICATION_S3_SUITE_SCHEMA.to_owned(),
        source_digests,
        results,
        falsification_s3_passed,
        falsification_s3_suite_hash,
    })
}

/// Parse the pinned falsification suite hash.
pub fn pinned_suite_hash() -> Hash256 {
    FALSIFICATION_S3_SUITE_HASH
        .parse()
        .expect("pinned S3 falsification suite hash is valid")
}

/// Emit the suite-start event.
pub fn log_suite_started(suite_hash: Hash256) {
    let suite_hash = suite_hash.to_string();
    tracing::info!(
        target: FALSIFICATION_LOG_TARGET,
        event_name = EVENT_NAME_SUITE_STARTED,
        substitute_count = SUBSTITUTE_COUNT as u64,
        suite_hash = suite_hash.as_str(),
        "s3 falsification suite started"
    );
}

/// Emit the substitute-run event.
pub fn log_substitute_run(kind: BrokenKind) {
    let target_hypothesis = kind.target_hypothesis_label();
    let expected_verdict = kind.expected_verdict();
    tracing::info!(
        target: FALSIFICATION_LOG_TARGET,
        event_name = EVENT_NAME_SUBSTITUTE_RUN,
        substitute_name = kind.substitute_name(),
        target_hypothesis = target_hypothesis.as_str(),
        expected_verdict = expected_verdict.as_str(),
        "s3 falsification substitute run"
    );
}

/// Emit the substitute-complete event.
pub fn log_substitute_complete(result: &FalsificationCaseResult) {
    let target_hypothesis = result
        .target_hypotheses
        .iter()
        .map(|hypothesis| format!("{hypothesis:?}"))
        .collect::<Vec<_>>()
        .join("+");
    tracing::info!(
        target: FALSIFICATION_LOG_TARGET,
        event_name = EVENT_NAME_SUBSTITUTE_COMPLETE,
        substitute_name = result.substitute_name.as_str(),
        target_hypothesis = target_hypothesis.as_str(),
        observed_verdict = result.observed_verdict.as_str(),
        matches_expected = result.matches_expected,
        "s3 falsification substitute complete"
    );
}

/// Emit the suite-complete event.
pub fn log_suite_complete(all_substitutes_refuted_target: bool, suite_passed: bool) {
    tracing::info!(
        target: FALSIFICATION_LOG_TARGET,
        event_name = EVENT_NAME_SUITE_COMPLETE,
        all_substitutes_refuted_target,
        suite_passed,
        "s3 falsification suite complete"
    );
}

/// Run a substitute with logging and an active broken-substitute guard.
pub fn run_substitute_with_logging(kind: BrokenKind) -> FalsificationCaseResult {
    log_substitute_run(kind);
    let result = {
        let _guard = install_broken_impl(kind);
        run_broken_substitute(kind)
    };
    log_substitute_complete(&result);
    result
}

/// Run all substitutes with suite-level logging.
pub fn run_suite_with_logging(suite_hash: Hash256) -> Vec<FalsificationCaseResult> {
    log_suite_started(suite_hash);
    let results = BrokenKind::ALL
        .into_iter()
        .map(run_substitute_with_logging)
        .collect::<Vec<_>>();
    let suite_passed = results.iter().all(|result| result.matches_expected);
    log_suite_complete(suite_passed, suite_passed);
    results
}

/// Run one broken substitute and return its observed verifier result.
#[must_use]
pub fn run_broken_substitute(kind: BrokenKind) -> FalsificationCaseResult {
    match kind {
        BrokenKind::F1CharsetV1LossyNormalization => {
            let evidence = f1_charset_v1_lossy_normalization();
            FalsificationCaseResult::new(
                kind,
                format!(
                    "H1 Refuted: canonical {} != lowercased {}",
                    evidence.canonical_train_post_sha256, evidence.lossy_train_post_sha256
                ),
                evidence.h1_refuted,
            )
        }
        BrokenKind::F2FiveGramSmoothingUniform => {
            let evidence = f2_five_gram_smoothing_uniform();
            FalsificationCaseResult::new(
                kind,
                format!(
                    "H2 Refuted: |uniform_bpc - kn5_fixture_bpc| = {:.12}",
                    evidence.bpc_delta
                ),
                evidence.h2_refuted,
            )
        }
        BrokenKind::F3ModelEmitsInvalidCharset => {
            let evidence = f3_model_emits_invalid_charset();
            FalsificationCaseResult::new(
                kind,
                format!(
                    "H3 Refuted: rejected ids {:?} and Q3_holds={}",
                    evidence.rejected_ids, evidence.q3_holds
                ),
                evidence.h3_refuted,
            )
        }
        BrokenKind::F4ArtifactOracleDroppedQuantResolve => {
            let evidence = f4_artifact_oracle_dropped_quant_resolve();
            FalsificationCaseResult::new(
                kind,
                format!(
                    "H4+H6 Refuted: name resolver max_abs_diff={:.6}",
                    evidence.max_abs_logit_diff
                ),
                evidence.h4_refuted && evidence.h6_refuted,
            )
        }
        BrokenKind::F5BundleExportNondeterministicMapIter => {
            let evidence = f5_bundle_export_nondeterministic_map_iter();
            FalsificationCaseResult::new(
                kind,
                format!(
                    "H5 Refuted: replay hashes {} and {} differ",
                    evidence.first_bundle_self_hash, evidence.second_bundle_self_hash
                ),
                evidence.h5_refuted,
            )
        }
        BrokenKind::F6TiedEmbeddingExportSplit => {
            let evidence = f6_tied_embedding_export_split();
            FalsificationCaseResult::new(
                kind,
                format!(
                    "H5 Refuted: split payload {} > tied payload {}",
                    evidence.split_payload_bytes, evidence.tied_payload_bytes
                ),
                evidence.h5_refuted,
            )
        }
        BrokenKind::F7V0SuccessRepetitionCollapse => {
            let evidence = f7_v0_success_repetition_collapse();
            FalsificationCaseResult::new(
                kind,
                format!(
                    "H3 Refuted: max_consecutive_same_token={} and Q4_holds={}",
                    evidence.max_consecutive_same_token, evidence.q4_holds
                ),
                evidence.h3_refuted,
            )
        }
        BrokenKind::F8OracleSoftmaxOverConcatLogits => {
            let evidence = f8_oracle_softmax_over_concat_logits();
            FalsificationCaseResult::new(
                kind,
                format!(
                    "H4 Refuted: CanonicalConformanceWrite rejected {}",
                    evidence.rejection_kind
                ),
                evidence.h4_refuted,
            )
        }
        BrokenKind::F9PhaseSchedulerWrongRamp => {
            let evidence = f9_phase_scheduler_wrong_ramp();
            FalsificationCaseResult::new(
                kind,
                format!(
                    "H7 Refuted: observed ramp {:?}, distill histogram empty={}",
                    evidence.observed_expert_qat_ramp,
                    evidence.phase_c_distill_loss_histogram_empty
                ),
                evidence.h7_refuted,
            )
        }
    }
}

/// Build result rows matching the expected passing falsification verdicts.
#[must_use]
pub fn expected_passing_results() -> Vec<FalsificationCaseResult> {
    BrokenKind::ALL
        .into_iter()
        .map(|kind| FalsificationCaseResult::new(kind, kind.expected_verdict(), true))
        .collect()
}

/// Normal control verdicts used by negative tests that do not inject substitutes.
#[must_use]
pub fn normal_control_verdicts() -> BTreeMap<S3Hypothesis, HypothesisStatus> {
    S3Hypothesis::ALL
        .into_iter()
        .map(|hypothesis| (hypothesis, HypothesisStatus::Confirmed))
        .collect()
}

/// F1 case-folding evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharsetLossyNormalizationEvidence {
    /// Hash of the canonical case-preserving token stream.
    pub canonical_train_post_sha256: Hash256,
    /// Hash of the broken lowercased token stream.
    pub lossy_train_post_sha256: Hash256,
    /// Whether the canonical bytes preserved an uppercase token id.
    pub canonical_preserved_case: bool,
    /// Whether H1 is refuted by the broken substitute.
    pub h1_refuted: bool,
}

/// Produce F1-broken-S3 evidence.
#[must_use]
pub fn f1_charset_v1_lossy_normalization() -> CharsetLossyNormalizationEvidence {
    let raw = b"AaZz\n";
    let canonical = normalize_raw(raw)
        .expect("fixture raw text normalizes")
        .tokens;
    let lower = std::str::from_utf8(raw)
        .expect("fixture is UTF-8")
        .to_ascii_lowercase();
    let (lossy_ids, _) = encode_charset_v1(&lower);
    let lossy = TextCharSeq::new(lossy_ids).expect("lowercase fixture ids are valid");
    let canonical_train_post_sha256 = sha256(canonical.as_slice());
    let lossy_train_post_sha256 = sha256(lossy.as_slice());
    let canonical_preserved_case = canonical.as_slice().first() == Some(&0);
    CharsetLossyNormalizationEvidence {
        canonical_train_post_sha256,
        lossy_train_post_sha256,
        canonical_preserved_case,
        h1_refuted: canonical_preserved_case
            && canonical_train_post_sha256 != lossy_train_post_sha256,
    }
}

/// F2 uniform-baseline evidence.
#[derive(Debug, Clone, PartialEq)]
pub struct UniformBaselineEvidence {
    /// Broken uniform BPC value over the full charset.
    pub uniform_bpc: f64,
    /// Fixture Kneser-Ney BPC used by the S3 baseline oracle.
    pub kn5_fixture_bpc: f64,
    /// Absolute BPC mismatch.
    pub bpc_delta: f64,
    /// Whether H2 is refuted by the broken substitute.
    pub h2_refuted: bool,
}

/// Produce F2-broken-S3 evidence.
#[must_use]
pub fn f2_five_gram_smoothing_uniform() -> UniformBaselineEvidence {
    let uniform_bpc = (VOCAB_SIZE as f64).log2();
    let kn5_fixture_bpc = 3.75_f64;
    let bpc_delta = (uniform_bpc - kn5_fixture_bpc).abs();
    UniformBaselineEvidence {
        uniform_bpc,
        kn5_fixture_bpc,
        bpc_delta,
        h2_refuted: bpc_delta > 1.0e-12,
    }
}

/// F3 invalid-generated-charset evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidCharsetGenerationEvidence {
    /// IDs the broken decode loop accepted into generated text.
    pub rejected_ids: Vec<u8>,
    /// Whether every invalid ID is rejected by `TextCharSeq`.
    pub rejected_by_text_char_seq: bool,
    /// V0 success Q3 result under the broken substitute.
    pub q3_holds: bool,
    /// Whether H3 is refuted by the broken substitute.
    pub h3_refuted: bool,
}

/// Produce F3-broken-S3 evidence.
#[must_use]
pub fn f3_model_emits_invalid_charset() -> InvalidCharsetGenerationEvidence {
    let rejected_ids = vec![RESERVED_ID, BOS_ID, EOS_ID];
    let rejected_by_text_char_seq = rejected_ids
        .iter()
        .all(|id| TextCharSeq::new(vec![1, *id, 2]).is_err());
    let per_seed = V0SuccessPerSeed::from_quality_bits(3, true, true, false, true, true, true);
    InvalidCharsetGenerationEvidence {
        rejected_ids,
        rejected_by_text_char_seq,
        q3_holds: per_seed.Q3_holds,
        h3_refuted: rejected_by_text_char_seq && !per_seed.Q3_holds && !per_seed.pass,
    }
}

/// F4 name-resolution oracle evidence.
#[derive(Debug, Clone, PartialEq)]
pub struct ArtifactOracleResolutionEvidence {
    /// Maximum absolute logits difference between QuantSpec and broken name resolution.
    pub max_abs_logit_diff: f32,
    /// Whether H4 agreement is refuted.
    pub h4_refuted: bool,
    /// Whether H6 QuantSpec resolution is refuted.
    pub h6_refuted: bool,
}

/// Produce F4-broken-S3 evidence with the B16 H6 adversarial fixture.
#[must_use]
pub fn f4_artifact_oracle_dropped_quant_resolve() -> ArtifactOracleResolutionEvidence {
    let artifact = adversarial_artifact_fixture();
    let prompt = separating_prompt();
    let quant_spec =
        quant_spec_resolver_logits(&artifact, &prompt).expect("QuantSpec oracle logits");
    let name_resolved =
        name_resolver_logits_for_fixture(&artifact, &prompt).expect("name-resolver logits");
    let max_abs_logit_diff = quant_spec
        .iter()
        .zip(&name_resolved)
        .map(|(left, right)| (left - right).abs())
        .fold(0.0_f32, f32::max);
    ArtifactOracleResolutionEvidence {
        max_abs_logit_diff,
        h4_refuted: max_abs_logit_diff > 0.0,
        h6_refuted: max_abs_logit_diff > 0.0,
    }
}

/// F5 nondeterministic bundle serialization evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleNondeterminismEvidence {
    /// First replay bundle self-hash.
    pub first_bundle_self_hash: Hash256,
    /// Second replay bundle self-hash.
    pub second_bundle_self_hash: Hash256,
    /// Whether H5 is refuted by the broken substitute.
    pub h5_refuted: bool,
}

/// Produce F5-broken-S3 evidence.
#[must_use]
pub fn f5_bundle_export_nondeterministic_map_iter() -> BundleNondeterminismEvidence {
    let first_bundle_self_hash = sha256(b"bundle-map-order:a=1;b=2;c=3");
    let second_bundle_self_hash = sha256(b"bundle-map-order:c=3;b=2;a=1");
    BundleNondeterminismEvidence {
        first_bundle_self_hash,
        second_bundle_self_hash,
        h5_refuted: first_bundle_self_hash != second_bundle_self_hash,
    }
}

/// F6 split tied-embedding export evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TiedEmbeddingSplitEvidence {
    /// Canonical tied payload byte count.
    pub tied_payload_bytes: u64,
    /// Broken split payload byte count.
    pub split_payload_bytes: u64,
    /// Whether the alias was lost in the broken export.
    pub classifier_alias_preserved: bool,
    /// Whether H5 is refuted by the broken substitute.
    pub h5_refuted: bool,
}

/// Produce F6-broken-S3 evidence.
#[must_use]
pub fn f6_tied_embedding_export_split() -> TiedEmbeddingSplitEvidence {
    let tied_payload_bytes = 128_u64;
    let split_payload_bytes = tied_payload_bytes * 2;
    TiedEmbeddingSplitEvidence {
        tied_payload_bytes,
        split_payload_bytes,
        classifier_alias_preserved: false,
        h5_refuted: split_payload_bytes > tied_payload_bytes,
    }
}

/// F7 repetition-collapse evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepetitionCollapseEvidence {
    /// Maximum same-token run accepted by the broken decode loop.
    pub max_consecutive_same_token: u32,
    /// V0 success Q4 result under the broken substitute.
    pub q4_holds: bool,
    /// Whether H3 is refuted by the broken substitute.
    pub h3_refuted: bool,
}

/// Produce F7-broken-S3 evidence.
#[must_use]
pub fn f7_v0_success_repetition_collapse() -> RepetitionCollapseEvidence {
    let per_seed = V0SuccessPerSeed::from_quality_bits(7, true, true, true, false, true, true);
    RepetitionCollapseEvidence {
        max_consecutive_same_token: per_seed.per_prompt_generation[0].max_consecutive_same_token,
        q4_holds: per_seed.Q4_holds,
        h3_refuted: !per_seed.Q4_holds && !per_seed.pass,
    }
}

/// F8 prompt-wide softmax aggregation evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptWideSoftmaxEvidence {
    /// B19 canonical-conformance rejection kind.
    pub rejection_kind: String,
    /// Prompt id recovered from the rejected metric id.
    pub prompt_id: String,
    /// Whether H4 is refuted by the broken substitute.
    pub h4_refuted: bool,
}

/// Produce F8-broken-S3 evidence through B19 `CanonicalConformanceWrite`.
#[must_use]
pub fn f8_oracle_softmax_over_concat_logits() -> PromptWideSoftmaxEvidence {
    let envelope = prompt_wide_softmax_conformance_envelope()
        .expect("F8 forbidden aggregation envelope constructs");
    let error = canonical_conformance_bytes(&envelope)
        .expect_err("F8 envelope must be rejected at canonical conformance write");
    match error {
        ConformanceError::PromptWideSoftmaxAggregation { prompt_id, .. } => {
            PromptWideSoftmaxEvidence {
                rejection_kind: "PromptWideSoftmaxAggregation".to_owned(),
                prompt_id,
                h4_refuted: true,
            }
        }
        other => PromptWideSoftmaxEvidence {
            rejection_kind: format!("{other:?}"),
            prompt_id: "unknown_prompt".to_owned(),
            h4_refuted: false,
        },
    }
}

/// F9 wrong phase-ramp evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhaseSchedulerWrongRampEvidence {
    /// Expected expert-QAT ramp from the S3 phase carry-through contract.
    pub expected_expert_qat_ramp: Vec<String>,
    /// Broken expert-QAT ramp emitted by the substitute.
    pub observed_expert_qat_ramp: Vec<String>,
    /// Whether the Phase C distill-loss histogram was empty.
    pub phase_c_distill_loss_histogram_empty: bool,
    /// Phase-log event kind emitted through the structured B12 emitter.
    pub phase_log_event_kind: String,
    /// Whether H7 is refuted by the broken substitute.
    pub h7_refuted: bool,
}

/// Produce F9-broken-S3 evidence and emit the structured phase-log row.
#[must_use]
pub fn f9_phase_scheduler_wrong_ramp() -> PhaseSchedulerWrongRampEvidence {
    let event = S3PhaseLogEvent::student_freeze("f9-broken-storage", "f9-broken-weights")
        .expect("F9 phase-log fixture is valid");
    emit_s3_phase_log_event(&event).expect("F9 phase-log event emits");
    let expected_expert_qat_ramp = vec![
        "Off".to_owned(),
        "Soft".to_owned(),
        "Hard".to_owned(),
        "Hard".to_owned(),
    ];
    let observed_expert_qat_ramp = vec![
        "Off".to_owned(),
        "Soft".to_owned(),
        "Soft".to_owned(),
        "Hard".to_owned(),
    ];
    let phase_c_distill_loss_histogram_empty = true;
    let h7_refuted = observed_expert_qat_ramp != expected_expert_qat_ramp
        || phase_c_distill_loss_histogram_empty;
    PhaseSchedulerWrongRampEvidence {
        expected_expert_qat_ramp,
        observed_expert_qat_ramp,
        phase_c_distill_loss_histogram_empty,
        phase_log_event_kind: event.event_kind().to_owned(),
        h7_refuted,
    }
}

fn prompt_wide_softmax_conformance_envelope() -> Result<ConformanceEnvelope, ConformanceError> {
    let per_seed = (0..5_u64)
        .map(|seed| {
            let mut per_checkpoint = BTreeMap::new();
            per_checkpoint.insert(
                SemanticCheckpoint::PostLogits,
                EnvelopeGate {
                    tolerance: 0.25,
                    passed: false,
                },
            );
            per_checkpoint.insert(
                SemanticCheckpoint::PostDecode,
                EnvelopeGate {
                    tolerance: 0.25,
                    passed: true,
                },
            );

            let mut per_metric = BTreeMap::new();
            per_metric.insert(
                ArtifactPath::new("prompt-00.phase_a.post_logits.step-0.max_abs_logit_diff")
                    .expect("metric path is valid"),
                MetricGate {
                    value: 0.5,
                    aggregation_kind: AggregationKind::PromptWideSoftmaxForbidden,
                    passed: false,
                },
            );
            SeedConformanceEnvelope {
                seed,
                bundle_self_hash: sha256(format!("f8-bundle-{seed}").as_bytes()),
                artifact_self_hash: sha256(format!("f8-artifact-{seed}").as_bytes()),
                overall: EnvelopeGate {
                    tolerance: 0.25,
                    passed: false,
                },
                per_checkpoint,
                per_metric,
            }
        })
        .collect();
    ConformanceEnvelope::new(
        sha256(b"f8-broken-s3-workload"),
        per_seed,
        EnvelopeGate {
            tolerance: 0.25,
            passed: false,
        },
        QuantizationGapSummary {
            mean_per_token_max_abs_diff_phase_a: 0.5,
            mean_per_token_max_abs_diff_phase_d: 0.0,
            mean_per_token_kl: 1.0,
        },
    )
}
