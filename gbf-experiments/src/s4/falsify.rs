//! S4 test-only falsification surface.

use gbf_artifact::GutenbergDropReason;
use gbf_data::UNMAPPABLE_EXAMPLE_DROP_THRESHOLD;
use gbf_foundation::Hash256;

use crate::s4::contamination::S4_CONTAMINATION_NGRAM_N;
use crate::s4::corpus_oracle::{
    S4_CORPUS_ORACLE_FIXTURE_FALLBACK, S4CorpusOracleCheckId, S4CorpusOracleInputs,
    S4CorpusOracleSuiteReport, run_fixture_local_corpus_oracle,
};
use crate::s4::schema::{HypothesisStatus, S4Hypothesis};

/// S4 falsification tracing target.
pub const S4_FALSIFY_LOG_TARGET: &str = "gbf_experiments::s4::falsify";

/// Structured event emitted before one broken S4 variant is run.
pub const S4_FALSIFY_VARIANT_RUN_EVENT_NAME: &str = "s4_falsify_variant_run";

/// Structured event emitted after one broken S4 variant has produced an outcome.
pub const S4_FALSIFY_OUTCOME_EVENT_NAME: &str = "s4_falsify_outcome";

/// Marker proving the S4 falsification module is feature gated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct S4FalsificationSurface;

/// F-S4 O5 falsification cases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum S4FalsificationCase {
    /// F1-broken: lossy Gutenberg decompression.
    LossyGutenbergDecompression,
    /// F2-broken: contamination n-gram width is too small.
    ContaminationWindowTooSmall,
    /// F3-broken: promotion gate silently skips P-2 oracle agreement.
    PromotionGateSkipsOracleAgreement,
    /// F4-broken: Gutenberg training initializes from random weights.
    TrainRandomInit,
    /// F5-broken: ArtifactOracle silently uses TinyStories validation normalization.
    OracleDriftUnderCorpusSwitch,
    /// F6-broken: high unmappable-density document is retained.
    UnmappableRateSilentlyDropped,
}

impl S4FalsificationCase {
    /// All O5 falsification cases in RFC order.
    pub const ALL: [Self; 6] = [
        Self::LossyGutenbergDecompression,
        Self::ContaminationWindowTooSmall,
        Self::PromotionGateSkipsOracleAgreement,
        Self::TrainRandomInit,
        Self::OracleDriftUnderCorpusSwitch,
        Self::UnmappableRateSilentlyDropped,
    ];

    /// Stable O5 case id.
    #[must_use]
    pub const fn case_id(self) -> &'static str {
        match self {
            Self::LossyGutenbergDecompression => "F1-broken-S4",
            Self::ContaminationWindowTooSmall => "F2-broken-S4",
            Self::PromotionGateSkipsOracleAgreement => "F3-broken-S4",
            Self::TrainRandomInit => "F4-broken-S4",
            Self::OracleDriftUnderCorpusSwitch => "F5-broken-S4",
            Self::UnmappableRateSilentlyDropped => "F6-broken-S4",
        }
    }

    /// Stable broken-substitute label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LossyGutenbergDecompression => "gutenberg_manifest_lossy_decompression",
            Self::ContaminationWindowTooSmall => "contamination_check_window_too_small",
            Self::PromotionGateSkipsOracleAgreement => "promotion_gate_skips_oracle_agreement",
            Self::TrainRandomInit => "gutenberg_train_resets_to_random_weights_silently",
            Self::OracleDriftUnderCorpusSwitch => "oracle_drift_under_corpus_switch",
            Self::UnmappableRateSilentlyDropped => "unmappable_rate_silently_dropped",
        }
    }

    /// Hypothesis expected to be refuted by this broken substitute.
    #[must_use]
    pub const fn expected_refuted_hypothesis(self) -> S4Hypothesis {
        match self {
            Self::LossyGutenbergDecompression | Self::UnmappableRateSilentlyDropped => {
                S4Hypothesis::H1
            }
            Self::ContaminationWindowTooSmall => S4Hypothesis::H2,
            Self::PromotionGateSkipsOracleAgreement => S4Hypothesis::H3,
            Self::TrainRandomInit => S4Hypothesis::H6,
            Self::OracleDriftUnderCorpusSwitch => S4Hypothesis::H5,
        }
    }

    fn corpus_case(self) -> Option<S4CorpusOracleFalsificationCase> {
        match self {
            Self::LossyGutenbergDecompression => {
                Some(S4CorpusOracleFalsificationCase::LossyGutenbergDecompression)
            }
            Self::ContaminationWindowTooSmall => {
                Some(S4CorpusOracleFalsificationCase::ContaminationWindowTooSmall)
            }
            Self::UnmappableRateSilentlyDropped => {
                Some(S4CorpusOracleFalsificationCase::UnmappableRateSilentlyDropped)
            }
            Self::PromotionGateSkipsOracleAgreement
            | Self::TrainRandomInit
            | Self::OracleDriftUnderCorpusSwitch => None,
        }
    }
}

/// Corpus-side broken substitute cases owned by F-S4.16.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum S4CorpusOracleFalsificationCase {
    /// F1-broken: lossy Gutenberg decompression changes D3 body bytes.
    LossyGutenbergDecompression,
    /// F2-broken: contamination checker uses a window smaller than D6's n=13.
    ContaminationWindowTooSmall,
    /// F6-broken: per-document unmappable density breach is computed but retained.
    UnmappableRateSilentlyDropped,
}

impl S4CorpusOracleFalsificationCase {
    /// Stable broken-substitute label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LossyGutenbergDecompression => "gutenberg_manifest_lossy_decompression",
            Self::ContaminationWindowTooSmall => "contamination_check_window_too_small",
            Self::UnmappableRateSilentlyDropped => "unmappable_rate_silently_dropped",
        }
    }

    /// Hypothesis expected to be refuted by this broken substitute.
    #[must_use]
    pub const fn expected_refuted_hypothesis(self) -> S4Hypothesis {
        match self {
            Self::ContaminationWindowTooSmall => S4Hypothesis::H2,
            Self::LossyGutenbergDecompression | Self::UnmappableRateSilentlyDropped => {
                S4Hypothesis::H1
            }
        }
    }

    fn expected_failed_check(self) -> S4CorpusOracleCheckId {
        match self {
            Self::LossyGutenbergDecompression => S4CorpusOracleCheckId::StripperIdempotence,
            Self::ContaminationWindowTooSmall => S4CorpusOracleCheckId::ContaminationOverlapMath,
            Self::UnmappableRateSilentlyDropped => S4CorpusOracleCheckId::UnmappableAccounting,
        }
    }
}

impl From<S4CorpusOracleFalsificationCase> for S4FalsificationCase {
    fn from(value: S4CorpusOracleFalsificationCase) -> Self {
        match value {
            S4CorpusOracleFalsificationCase::LossyGutenbergDecompression => {
                Self::LossyGutenbergDecompression
            }
            S4CorpusOracleFalsificationCase::ContaminationWindowTooSmall => {
                Self::ContaminationWindowTooSmall
            }
            S4CorpusOracleFalsificationCase::UnmappableRateSilentlyDropped => {
                Self::UnmappableRateSilentlyDropped
            }
        }
    }
}

/// Promotion-gate P-2 falsification fixture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S4PromotionGateFalsificationFixture {
    /// Whether P-2 oracle agreement is required by the promotion contract.
    pub oracle_agreement_required: bool,
    /// Whether the hash-bound oracle-agreement artifact is present.
    pub oracle_agreement_artifact_present: bool,
    /// Whether the supplied oracle-agreement artifact says `Agree`.
    pub oracle_agreement_agrees: bool,
    /// Whether the broken gate would promote after skipping P-2.
    pub broken_gate_promotes_without_oracle: bool,
}

/// S4-Run-Ok-4 lineage falsification fixture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S4LineageFalsificationFixture {
    /// Expected c_TS tensor payload SHA from the promoted checkpoint.
    pub c_ts_checkpoint_payload_sha: Hash256,
    /// Payload SHA computed from actual in-memory initial model weights.
    pub actual_initial_checkpoint_payload_sha: Hash256,
    /// Payload SHA recorded by the broken runner, usually copied from config.
    pub recorded_initial_checkpoint_payload_sha: Hash256,
}

/// Oracle drift falsification fixture for H5.
#[derive(Debug, Clone, PartialEq)]
pub struct S4OracleDriftFalsificationFixture {
    /// Corpus normalization the ArtifactOracle must use.
    pub expected_artifact_oracle_corpus: &'static str,
    /// Corpus normalization actually used by the broken ArtifactOracle.
    pub observed_artifact_oracle_corpus: &'static str,
    /// Live-training scorer bpc.
    pub live_training_bpc: f64,
    /// ReferenceModelBundle scorer bpc.
    pub reference_bundle_bpc: f64,
    /// ArtifactOracle scorer bpc.
    pub artifact_oracle_bpc: f64,
    /// S3-pinned inter-oracle tolerance.
    pub tolerance: f64,
}

/// Inputs for the complete F-S4 O5 falsification suite.
#[derive(Debug, Clone, PartialEq)]
pub struct S4FalsificationSuiteInputs {
    /// Corpus-side COr fallback fixture used by F1/F2/F6.
    pub corpus_oracle_inputs: S4CorpusOracleInputs,
    /// Promotion-gate P-2 fixture used by F3.
    pub promotion_gate: S4PromotionGateFalsificationFixture,
    /// Lineage fixture used by F4.
    pub lineage: S4LineageFalsificationFixture,
    /// Oracle drift fixture used by F5.
    pub oracle_drift: S4OracleDriftFalsificationFixture,
}

/// Falsification result for one corpus-side broken substitute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S4CorpusOracleFalsificationResult {
    /// Broken substitute that was injected.
    pub case: S4CorpusOracleFalsificationCase,
    /// Explicit fallback evaluator used to detect the break.
    pub fallback_name: &'static str,
    /// Corpus-side COr suite report after injecting the break.
    pub report: S4CorpusOracleSuiteReport,
    /// Expected hypothesis refuted by the injected break.
    pub expected_refuted_hypothesis: S4Hypothesis,
    /// Status observed for the expected hypothesis.
    pub observed_status: HypothesisStatus,
}

impl S4CorpusOracleFalsificationResult {
    /// True when the broken substitute failed loudly through the named fallback.
    #[must_use]
    pub fn refuted_as_expected(&self) -> bool {
        self.fallback_name == S4_CORPUS_ORACLE_FIXTURE_FALLBACK
            && self.observed_status == HypothesisStatus::Refuted
            && self
                .report
                .failed_checks()
                .contains(&self.case.expected_failed_check())
    }
}

/// Falsification result for one O5 broken substitute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S4FalsificationCaseResult {
    /// Broken substitute case.
    pub case: S4FalsificationCase,
    /// Expected hypothesis refuted by the injected break.
    pub expected_refuted_hypothesis: S4Hypothesis,
    /// Observed status for the expected hypothesis.
    pub observed_status: HypothesisStatus,
    /// Stable diagnostic detail.
    pub detail: String,
}

impl S4FalsificationCaseResult {
    /// True when the broken substitute flipped the expected hypothesis.
    #[must_use]
    pub fn refuted_as_expected(&self) -> bool {
        self.expected_refuted_hypothesis == self.case.expected_refuted_hypothesis()
            && self.observed_status == HypothesisStatus::Refuted
    }
}

/// Complete O5 falsification-suite result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S4FalsificationSuiteReport {
    /// Results in O5/RFC order.
    pub results: Vec<S4FalsificationCaseResult>,
}

impl S4FalsificationSuiteReport {
    /// True when every O5 broken substitute refuted its target hypothesis.
    #[must_use]
    pub fn passed(&self) -> bool {
        self.results.len() == S4FalsificationCase::ALL.len()
            && self
                .results
                .iter()
                .map(|result| result.case)
                .eq(S4FalsificationCase::ALL)
            && self
                .results
                .iter()
                .all(S4FalsificationCaseResult::refuted_as_expected)
    }
}

/// Run the complete F-S4 O5 falsification suite.
#[must_use]
pub fn run_s4_falsification_suite(
    inputs: &S4FalsificationSuiteInputs,
) -> S4FalsificationSuiteReport {
    S4FalsificationSuiteReport {
        results: S4FalsificationCase::ALL
            .into_iter()
            .map(|case| run_s4_falsification_case(inputs, case))
            .collect(),
    }
}

/// Run one F-S4 O5 falsification case.
#[must_use]
pub fn run_s4_falsification_case(
    inputs: &S4FalsificationSuiteInputs,
    case: S4FalsificationCase,
) -> S4FalsificationCaseResult {
    tracing::info!(
        target: S4_FALSIFY_LOG_TARGET,
        event_name = S4_FALSIFY_VARIANT_RUN_EVENT_NAME,
        case_id = case.case_id(),
        variant = case.as_str(),
        expected_refuted_hypothesis = hypothesis_label(case.expected_refuted_hypothesis()),
        "s4 falsification variant run"
    );

    let result = run_s4_falsification_case_inner(inputs, case);
    tracing::info!(
        target: S4_FALSIFY_LOG_TARGET,
        event_name = S4_FALSIFY_OUTCOME_EVENT_NAME,
        case_id = result.case.case_id(),
        variant = result.case.as_str(),
        expected_refuted_hypothesis = hypothesis_label(result.expected_refuted_hypothesis),
        observed_status = hypothesis_status_label(&result.observed_status),
        refuted_as_expected = result.refuted_as_expected(),
        detail = result.detail.as_str(),
        "s4 falsification outcome"
    );
    result
}

fn run_s4_falsification_case_inner(
    inputs: &S4FalsificationSuiteInputs,
    case: S4FalsificationCase,
) -> S4FalsificationCaseResult {
    if let Some(corpus_case) = case.corpus_case() {
        let result = run_corpus_oracle_falsification(&inputs.corpus_oracle_inputs, corpus_case);
        return S4FalsificationCaseResult {
            case,
            expected_refuted_hypothesis: result.expected_refuted_hypothesis,
            observed_status: result.observed_status,
            detail: format!(
                "{} detected by {}",
                corpus_case.as_str(),
                result.fallback_name
            ),
        };
    }

    match case {
        S4FalsificationCase::PromotionGateSkipsOracleAgreement => {
            run_promotion_gate_skip_oracle_case(&inputs.promotion_gate)
        }
        S4FalsificationCase::TrainRandomInit => run_train_random_init_case(&inputs.lineage),
        S4FalsificationCase::OracleDriftUnderCorpusSwitch => {
            run_oracle_drift_case(&inputs.oracle_drift)
        }
        S4FalsificationCase::LossyGutenbergDecompression
        | S4FalsificationCase::ContaminationWindowTooSmall
        | S4FalsificationCase::UnmappableRateSilentlyDropped => {
            unreachable!("corpus cases returned above")
        }
    }
}

const fn hypothesis_label(hypothesis: S4Hypothesis) -> &'static str {
    match hypothesis {
        S4Hypothesis::H1 => "H1",
        S4Hypothesis::H2 => "H2",
        S4Hypothesis::H3 => "H3",
        S4Hypothesis::H4 => "H4",
        S4Hypothesis::H5 => "H5",
        S4Hypothesis::H6 => "H6",
        S4Hypothesis::H7 => "H7",
    }
}

fn hypothesis_status_label(status: &HypothesisStatus) -> &'static str {
    match status {
        HypothesisStatus::Confirmed => "Confirmed",
        HypothesisStatus::Refuted => "Refuted",
        HypothesisStatus::NotEvaluatedDueToPriorGate { .. } => "NotEvaluatedDueToPriorGate",
    }
}

/// Inject one corpus-side broken substitute and require the COr fallback to catch it.
#[must_use]
pub fn run_corpus_oracle_falsification(
    clean_inputs: &S4CorpusOracleInputs,
    case: S4CorpusOracleFalsificationCase,
) -> S4CorpusOracleFalsificationResult {
    let mut broken = clean_inputs.clone();
    match case {
        S4CorpusOracleFalsificationCase::LossyGutenbergDecompression => {
            if let Some(first) = broken.stripper_cases.first_mut() {
                first.raw_utf8.retain(u8::is_ascii);
            }
        }
        S4CorpusOracleFalsificationCase::ContaminationWindowTooSmall => {
            broken.contamination_math.n = S4_CONTAMINATION_NGRAM_N - 10;
        }
        S4CorpusOracleFalsificationCase::UnmappableRateSilentlyDropped => {
            if let Some(source) = broken
                .unmappable_manifest
                .sources
                .iter_mut()
                .find(|source| source.drop_reason.is_none())
            {
                let body = 100_u64;
                let count = ((UNMAPPABLE_EXAMPLE_DROP_THRESHOLD * body as f64).floor() as u64) + 1;
                source.post_charset_token_length = Some(body);
                source.unmappable_count = Some(count);
                source.unmappable_density = Some(count as f64 / body as f64);
                source.drop_reason = None;
                source.duplicate_of_book_id = None;
            } else if let Some(source) = broken.unmappable_manifest.sources.first_mut() {
                source.drop_reason = Some(GutenbergDropReason::UnmappableDensityHigh);
            }
        }
    }

    let report = run_fixture_local_corpus_oracle(&broken);
    let expected_refuted_hypothesis = case.expected_refuted_hypothesis();
    let observed_status = report.hypothesis_status(expected_refuted_hypothesis);
    S4CorpusOracleFalsificationResult {
        case,
        fallback_name: report
            .fallback_name
            .expect("corpus-oracle falsification must use named fallback"),
        report,
        expected_refuted_hypothesis,
        observed_status,
    }
}

fn run_promotion_gate_skip_oracle_case(
    fixture: &S4PromotionGateFalsificationFixture,
) -> S4FalsificationCaseResult {
    let p2_should_reject = fixture.oracle_agreement_required
        && (!fixture.oracle_agreement_artifact_present || !fixture.oracle_agreement_agrees);
    let refuted = p2_should_reject && fixture.broken_gate_promotes_without_oracle;
    S4FalsificationCaseResult {
        case: S4FalsificationCase::PromotionGateSkipsOracleAgreement,
        expected_refuted_hypothesis: S4Hypothesis::H3,
        observed_status: if refuted {
            HypothesisStatus::Refuted
        } else {
            HypothesisStatus::Confirmed
        },
        detail: if refuted {
            "promotion gate promoted after skipping required P-2 oracle agreement".to_owned()
        } else {
            "promotion gate P-2 skip was not exposed by fixture".to_owned()
        },
    }
}

fn run_train_random_init_case(
    fixture: &S4LineageFalsificationFixture,
) -> S4FalsificationCaseResult {
    let actual_mismatched_cts =
        fixture.actual_initial_checkpoint_payload_sha != fixture.c_ts_checkpoint_payload_sha;
    let recorded_copied_from_config = fixture.recorded_initial_checkpoint_payload_sha
        == fixture.c_ts_checkpoint_payload_sha
        && fixture.recorded_initial_checkpoint_payload_sha
            != fixture.actual_initial_checkpoint_payload_sha;
    let refuted = actual_mismatched_cts && recorded_copied_from_config;
    S4FalsificationCaseResult {
        case: S4FalsificationCase::TrainRandomInit,
        expected_refuted_hypothesis: S4Hypothesis::H6,
        observed_status: if refuted {
            HypothesisStatus::Refuted
        } else {
            HypothesisStatus::Confirmed
        },
        detail: if refuted {
            "initial_checkpoint_payload_sha must be computed from actual in-memory weights before step 1, not copied from c_TS config"
                .to_owned()
        } else {
            "lineage fixture did not expose random-init payload mismatch".to_owned()
        },
    }
}

fn run_oracle_drift_case(fixture: &S4OracleDriftFalsificationFixture) -> S4FalsificationCaseResult {
    let wrong_corpus =
        fixture.observed_artifact_oracle_corpus != fixture.expected_artifact_oracle_corpus;
    let live_reference_gap = (fixture.live_training_bpc - fixture.reference_bundle_bpc).abs();
    let artifact_reference_gap = (fixture.artifact_oracle_bpc - fixture.reference_bundle_bpc).abs();
    let gap_failed =
        live_reference_gap > fixture.tolerance || artifact_reference_gap > fixture.tolerance;
    let refuted = wrong_corpus || gap_failed;
    S4FalsificationCaseResult {
        case: S4FalsificationCase::OracleDriftUnderCorpusSwitch,
        expected_refuted_hypothesis: S4Hypothesis::H5,
        observed_status: if refuted {
            HypothesisStatus::Refuted
        } else {
            HypothesisStatus::Confirmed
        },
        detail: if wrong_corpus {
            "ArtifactOracle used non-Gutenberg validation normalization under corpus switch"
                .to_owned()
        } else if gap_failed {
            "three-way oracle bpc gap exceeded S3-pinned tolerance".to_owned()
        } else {
            "oracle drift fixture did not expose H5 disagreement".to_owned()
        },
    }
}
