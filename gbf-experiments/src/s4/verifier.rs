//! H1-H7 verifier and S4 outcome-dispatch surface.

use std::collections::{BTreeMap, BTreeSet};

use gbf_foundation::Hash256;
use serde::{Deserialize, Serialize};

use crate::s4::schema::{
    HypothesisStatus, S4_CANONICAL_SEEDS, S4Completion, S4Decision, S4Hypothesis, S4Outcome,
    S4VerifierBundle,
};

/// Tracing event emitted before each H1-H7 verifier runs.
pub const S4_VERIFIER_STARTED_EVENT: &str = "s4_verifier_started";
/// Tracing event emitted after each H1-H7 verifier finishes.
pub const S4_VERIFIER_FINALIZED_EVENT: &str = "s4_verifier_finalized";

/// H1 hard cap on aggregate Gutenberg unmappable rate.
pub const S4_H1_MAX_UNMAPPABLE_RATE: f64 = 0.005;
/// H1 maximum marker-missing drop fraction.
pub const S4_H1_MAX_MARKER_MISSING_FRACTION: f64 = 0.05;
/// H1 maximum unmappable-density drop fraction.
pub const S4_H1_MAX_UNMAPPABLE_DENSITY_FRACTION: f64 = 0.02;
/// H1 minimum retained Gutenberg book count after all drops.
pub const S4_H1_MIN_RETAINED_BOOKS: u64 = 1_350;
/// H2 non-gating contamination warning threshold.
pub const S4_H2_CONTAMINATION_WARN_THRESHOLD: f64 = 0.0005;
/// H2 hard-fail contamination threshold.
pub const S4_H2_CONTAMINATION_HARD_FAIL_THRESHOLD: f64 = 0.0010;
/// H4 strict Gutenberg KN-5 margin.
pub const S4_H4_MIN_BPC_MARGIN_VS_KN5: f64 = 0.05;
/// H4 suspicious-low median bpc sentinel.
pub const S4_H4_SUSPICIOUS_LOW_BPC_THRESHOLD: f64 = 0.5;
/// H7 minimum TinyStories-only to Gutenberg-trained improvement.
pub const S4_H7_MIN_SHIFT_IMPROVEMENT: f64 = 0.10;
/// H7 minimum number of improved canonical seeds.
pub const S4_H7_MIN_SHIFTED_SEEDS: usize = 4;

/// A verifier result for one S4 hypothesis.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S4HypothesisVerifierOutput {
    /// Hypothesis evaluated by this verifier.
    pub hypothesis: S4Hypothesis,
    /// Closure status for the hypothesis.
    pub status: HypothesisStatus,
    /// RFC outcome if this hypothesis is the dispatching failure.
    pub outcome_if_refuted: Option<S4Outcome>,
    /// H2 warning bit. Non-H2 verifiers leave this false.
    pub contamination_warning: bool,
    /// H4 suspicious-low bpc bit. Non-H4 verifiers leave this false.
    pub suspicious_low_bpc: bool,
    /// Concrete observations that determined the status.
    pub observations: Vec<String>,
}

impl S4HypothesisVerifierOutput {
    fn confirmed(hypothesis: S4Hypothesis, observations: Vec<String>) -> Self {
        Self {
            hypothesis,
            status: HypothesisStatus::Confirmed,
            outcome_if_refuted: None,
            contamination_warning: false,
            suspicious_low_bpc: false,
            observations,
        }
    }

    fn refuted(
        hypothesis: S4Hypothesis,
        outcome_if_refuted: S4Outcome,
        observations: Vec<String>,
    ) -> Self {
        Self {
            hypothesis,
            status: HypothesisStatus::Refuted,
            outcome_if_refuted: Some(outcome_if_refuted),
            contamination_warning: false,
            suspicious_low_bpc: false,
            observations,
        }
    }

    /// True iff the verifier confirmed the hypothesis.
    #[must_use]
    pub const fn confirmed_status(&self) -> bool {
        matches!(self.status, HypothesisStatus::Confirmed)
    }
}

/// H1 verifier inputs extracted from `gutenberg_manifest.v1` and COr-1..COr-5.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S4H1CorpusIntegrityEvidence {
    /// Whether COr-1..COr-5 all passed.
    pub corpus_oracle_c_or_1_through_5_passed: bool,
    /// Length of the preselected Gutenberg `book_ids` list.
    pub book_id_count: u64,
    /// Retained train books.
    pub train_book_count: u64,
    /// Retained validation books.
    pub val_book_count: u64,
    /// Total dropped books, summed across all drop-count fields.
    pub drop_count_total: u64,
    /// Books dropped by marker checks.
    pub drop_count_marker_missing: u64,
    /// Books dropped by per-document unmappable density.
    pub drop_count_unmappable_density: u64,
    /// Aggregate Gutenberg unmappable rate after charset normalization.
    pub unmappable_rate_corpus_gutenberg: f64,
    /// Whether manifest self-hash recomputation matches the recorded value.
    pub manifest_self_hash_round_trips: bool,
    /// Recorded manifest self-hash, when available for diagnostics.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest_self_hash_recorded: Option<Hash256>,
}

/// H2 verifier inputs extracted from cross-corpus contamination and COr-6.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S4H2ContaminationEvidence {
    /// 13-token overlap fraction from TinyStories train to Gutenberg val.
    pub ts_train_to_gb_val_overlap: f64,
    /// 13-token overlap fraction from Gutenberg train to TinyStories val.
    pub gb_train_to_ts_val_overlap: f64,
    /// Whether COr-6 validated the contamination measurement oracle.
    pub corpus_oracle_c_or_6_passed: bool,
}

/// Promotion predicate families covered by H3 soundness fixtures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum S4PromotionPredicateFamily {
    /// D8 P-1.
    #[serde(rename = "P-1")]
    P1,
    /// D8 P-2.
    #[serde(rename = "P-2")]
    P2,
    /// D8 P-3.
    #[serde(rename = "P-3")]
    P3,
    /// D8 P-4.
    #[serde(rename = "P-4")]
    P4,
    /// D8 P-5.
    #[serde(rename = "P-5")]
    P5,
    /// D8 P-6.
    #[serde(rename = "P-6")]
    P6,
    /// D8 P-7.
    #[serde(rename = "P-7")]
    P7,
    /// D8 P-8.
    #[serde(rename = "P-8")]
    P8,
    /// D8 P-9.
    #[serde(rename = "P-9")]
    P9,
}

impl S4PromotionPredicateFamily {
    /// All D8 promotion predicate families in canonical order.
    pub const ALL: [Self; 9] = [
        Self::P1,
        Self::P2,
        Self::P3,
        Self::P4,
        Self::P5,
        Self::P6,
        Self::P7,
        Self::P8,
        Self::P9,
    ];
}

/// H3 fixture result for one deliberately broken promotion predicate family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S4PromotionPredicateFamilyEvidence {
    /// Predicate family under test.
    pub family: S4PromotionPredicateFamily,
    /// Whether the gate rejected the bundle with that family broken.
    pub rejected_when_broken: bool,
}

/// H3 verifier inputs extracted from promotion-gate soundness fixtures.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S4H3PromotionGateEvidence {
    /// Reference-positive bundle was promoted.
    pub reference_positive_bundle_promoted: bool,
    /// Every P-1..P-9 broken-family fixture result.
    pub broken_predicate_families: Vec<S4PromotionPredicateFamilyEvidence>,
    /// A bundle that does not satisfy D8 P-1..P-9 was rejected.
    pub invalid_bundle_rejected: bool,
    /// Repeated evaluation with the same inputs yielded identical bytes.
    pub referentially_transparent: bool,
    /// `promotion_gate_self_hash` recomputation matched the recorded value.
    pub promotion_gate_self_hash_round_trips: bool,
    /// Recorded promotion-gate self-hash, when available for diagnostics.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub promotion_gate_self_hash_recorded: Option<Hash256>,
}

/// Per-seed H4 Gutenberg score observation.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S4SeedQualityEvidence {
    /// Gutenberg continuation seed.
    pub seed: u64,
    /// Ternary checkpoint bpc on Gutenberg validation.
    pub bpc_ternary_gutenberg_val: f64,
    /// Whether inherited `v0_success` passed for this seed.
    pub v0_success_passed: bool,
}

/// H4 verifier inputs extracted from KN-5 baseline and per-seed scores.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S4H4GeneralizationEvidence {
    /// Gutenberg KN-5 baseline bpc on validation.
    pub bpc_kn5_gutenberg_val: f64,
    /// Per-canonical-seed score evidence.
    pub per_seed: Vec<S4SeedQualityEvidence>,
}

/// H5 verifier inputs extracted from `s4_oracle_agreement.v1`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S4H5OracleAgreementEvidence {
    /// Mandatory H5 seed. S4 v1 requires seed 0.
    pub seed: u64,
    /// Whether `s4_oracle_agreement.v1` recorded `Agree`.
    pub outcome_agree: bool,
    /// Maximum live-vs-denotational per-token bpc gap.
    pub gap_live_vs_denotational: f64,
    /// S3-pinned live-vs-denotational tolerance.
    pub tolerance_live_vs_denotational: f64,
    /// Maximum live-vs-artifact per-token bpc gap.
    pub gap_live_vs_artifact: f64,
    /// S3-pinned live-vs-artifact tolerance.
    pub tolerance_live_vs_artifact: f64,
    /// Maximum denotational-vs-artifact per-token bpc gap.
    pub gap_denotational_vs_artifact: f64,
    /// S3-pinned denotational-vs-artifact tolerance.
    pub tolerance_denotational_vs_artifact: f64,
}

/// Required artifact identity for H6 self-hash replay evidence.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum S4DeterministicArtifact {
    /// `gutenberg_manifest.v1`.
    GutenbergManifest,
    /// `s4_corpus_quality.v1`.
    CorpusQuality,
    /// `s4_contamination_report.v1`.
    ContaminationReport,
    /// `s4_corpus_progression.v1`.
    CorpusProgression,
    /// `s4_promotion_gate.v1`.
    PromotionGate,
    /// `s4_baseline_gutenberg.v1`.
    BaselineGutenberg,
    /// `s4_fp_reference.v1` for one seed.
    FpReference {
        /// Gutenberg continuation seed.
        seed: u64,
    },
    /// `s4_gutenberg_run_log.v1` for one seed.
    GutenbergRunLog {
        /// Gutenberg continuation seed.
        seed: u64,
    },
    /// `s4_gutenberg_checkpoint.v1` for one seed.
    GutenbergCheckpoint {
        /// Gutenberg continuation seed.
        seed: u64,
    },
    /// `s4_gutenberg_score.v1` for one seed.
    GutenbergScore {
        /// Gutenberg continuation seed.
        seed: u64,
    },
    /// `s4_oracle_agreement.v1`.
    OracleAgreement,
    /// `s4_report.v1`.
    Report,
}

/// Per-seed H6 tensor-payload replay evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S4SeedReplayEvidence {
    /// Gutenberg continuation seed.
    pub seed: u64,
    /// Original canonical tensor payload SHA.
    pub original_tensor_payload_sha: Hash256,
    /// Replay canonical tensor payload SHA.
    pub replay_tensor_payload_sha: Hash256,
}

/// Per-artifact H6 self-hash replay evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S4ArtifactReplayEvidence {
    /// Required S4 artifact.
    pub artifact: S4DeterministicArtifact,
    /// Original artifact self-hash.
    pub original_self_hash: Hash256,
    /// Replay artifact self-hash.
    pub replay_self_hash: Hash256,
}

/// H6 verifier inputs extracted from replay artifacts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S4H6DeterminismEvidence {
    /// Per-canonical-seed tensor replay checks.
    pub per_seed_tensor_payloads: Vec<S4SeedReplayEvidence>,
    /// Required artifact self-hash replay checks.
    pub artifact_self_hashes: Vec<S4ArtifactReplayEvidence>,
}

/// Per-seed H7 distribution-shift observation.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S4SeedShiftEvidence {
    /// Gutenberg continuation seed.
    pub seed: u64,
    /// Gutenberg-trained checkpoint bpc on Gutenberg validation.
    pub c_gb_bpc_gutenberg_val: f64,
}

/// H7 verifier inputs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S4H7DistributionShiftEvidence {
    /// TinyStories-only checkpoint bpc on Gutenberg validation.
    pub c_ts_bpc_gutenberg_val: f64,
    /// Per-canonical-seed Gutenberg-trained bpc on Gutenberg validation.
    pub per_seed: Vec<S4SeedShiftEvidence>,
}

/// Full H1-H7 verifier input set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S4VerifierEvidence {
    /// H1 evidence.
    pub h1: S4H1CorpusIntegrityEvidence,
    /// H2 evidence.
    pub h2: S4H2ContaminationEvidence,
    /// H3 evidence.
    pub h3: S4H3PromotionGateEvidence,
    /// H4 evidence.
    pub h4: S4H4GeneralizationEvidence,
    /// H5 evidence.
    pub h5: S4H5OracleAgreementEvidence,
    /// H6 evidence.
    pub h6: S4H6DeterminismEvidence,
    /// H7 evidence.
    pub h7: S4H7DistributionShiftEvidence,
    /// Whether canonical `c_TS_ref` was accepted by a sound promotion gate.
    pub promotion_gate_accepted_canonical: bool,
    /// Per-seed completion states across the five S4 runs.
    pub completions: Vec<S4Completion>,
}

/// Full verifier product for the report/outcome layer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S4VerifierReport {
    /// Per-hypothesis verifier outputs.
    pub hypothesis_outputs: BTreeMap<S4Hypothesis, S4HypothesisVerifierOutput>,
    /// Legacy/schema bundle consumed by the outcome dispatcher.
    pub bundle: S4VerifierBundle,
    /// RFC §11 outcome.
    pub outcome: S4Outcome,
    /// RFC §11 decision.
    pub decision: S4Decision,
}

/// Verify H1 corpus integrity.
#[must_use]
pub fn verify_h1_corpus_integrity(
    evidence: &S4H1CorpusIntegrityEvidence,
) -> S4HypothesisVerifierOutput {
    with_verifier_events("H1", || verify_h1_corpus_integrity_impl(evidence))
}

fn verify_h1_corpus_integrity_impl(
    evidence: &S4H1CorpusIntegrityEvidence,
) -> S4HypothesisVerifierOutput {
    let mut failures = Vec::new();
    if !evidence.corpus_oracle_c_or_1_through_5_passed {
        failures.push("COr-1..COr-5 did not all pass".to_owned());
    }
    if evidence.book_id_count == 0 {
        failures.push("book_id_count must be nonzero".to_owned());
    }
    if !is_finite_nonnegative(evidence.unmappable_rate_corpus_gutenberg) {
        failures.push(format!(
            "unmappable_rate_corpus_gutenberg must be finite and nonnegative, got {}",
            evidence.unmappable_rate_corpus_gutenberg
        ));
    } else if evidence.unmappable_rate_corpus_gutenberg > S4_H1_MAX_UNMAPPABLE_RATE {
        failures.push(format!(
            "unmappable_rate_corpus_gutenberg {} exceeds {}",
            evidence.unmappable_rate_corpus_gutenberg, S4_H1_MAX_UNMAPPABLE_RATE
        ));
    }
    if exceeds_ratio(
        evidence.drop_count_marker_missing,
        evidence.book_id_count,
        5,
        100,
    ) {
        failures.push(format!(
            "drop_count_marker_missing {} exceeds {} * book_id_count {}",
            evidence.drop_count_marker_missing,
            S4_H1_MAX_MARKER_MISSING_FRACTION,
            evidence.book_id_count
        ));
    }
    if exceeds_ratio(
        evidence.drop_count_unmappable_density,
        evidence.book_id_count,
        2,
        100,
    ) {
        failures.push(format!(
            "drop_count_unmappable_density {} exceeds {} * book_id_count {}",
            evidence.drop_count_unmappable_density,
            S4_H1_MAX_UNMAPPABLE_DENSITY_FRACTION,
            evidence.book_id_count
        ));
    }
    let retained = checked_add_or_failure(
        evidence.train_book_count,
        evidence.val_book_count,
        "train_book_count + val_book_count",
        &mut failures,
    );
    if let Some(retained) = retained {
        if retained < S4_H1_MIN_RETAINED_BOOKS {
            failures.push(format!(
                "retained train+val book count {retained} is below {S4_H1_MIN_RETAINED_BOOKS}"
            ));
        }
        if let Some(total) = checked_add_or_failure(
            retained,
            evidence.drop_count_total,
            "train_book_count + val_book_count + drop_count_total",
            &mut failures,
        ) && total != evidence.book_id_count
        {
            failures.push(format!(
                "train+val+drop_total {total} does not equal book_id_count {}",
                evidence.book_id_count
            ));
        }
    }
    if !evidence.manifest_self_hash_round_trips {
        failures.push(recorded_hash_failure(
            "manifest_self_hash did not round-trip",
            evidence.manifest_self_hash_recorded,
        ));
    }

    verdict_from_failures(
        S4Hypothesis::H1,
        S4Outcome::FailCorpusIntegrity,
        failures,
        vec![format!(
            "H1 checked {} selected books, {} retained, unmappable_rate={}",
            evidence.book_id_count,
            evidence
                .train_book_count
                .saturating_add(evidence.val_book_count),
            evidence.unmappable_rate_corpus_gutenberg
        )],
    )
}

/// Verify H2 cross-corpus contamination.
#[must_use]
pub fn verify_h2_contamination(evidence: S4H2ContaminationEvidence) -> S4HypothesisVerifierOutput {
    with_verifier_events("H2", || verify_h2_contamination_impl(evidence))
}

fn verify_h2_contamination_impl(evidence: S4H2ContaminationEvidence) -> S4HypothesisVerifierOutput {
    let mut failures = Vec::new();
    let mut warning = false;
    for (field, value) in [
        (
            "ts_train_to_gb_val_overlap",
            evidence.ts_train_to_gb_val_overlap,
        ),
        (
            "gb_train_to_ts_val_overlap",
            evidence.gb_train_to_ts_val_overlap,
        ),
    ] {
        if !is_finite_nonnegative(value) {
            failures.push(format!(
                "{field} must be finite and nonnegative, got {value}"
            ));
        } else if value > S4_H2_CONTAMINATION_HARD_FAIL_THRESHOLD {
            failures.push(format!(
                "{field} {value} exceeds hard fail threshold {}",
                S4_H2_CONTAMINATION_HARD_FAIL_THRESHOLD
            ));
        } else if value >= S4_H2_CONTAMINATION_WARN_THRESHOLD {
            warning = true;
        }
    }
    if !evidence.corpus_oracle_c_or_6_passed {
        failures.push("COr-6 did not pass".to_owned());
    }

    let mut output = verdict_from_failures(
        S4Hypothesis::H2,
        S4Outcome::FailContamination,
        failures,
        vec![format!(
            "H2 gated overlaps: TS_train->GB_val={}, GB_train->TS_val={}",
            evidence.ts_train_to_gb_val_overlap, evidence.gb_train_to_ts_val_overlap
        )],
    );
    output.contamination_warning = warning && output.confirmed_status();
    if output.contamination_warning {
        output
            .observations
            .push("contamination warning threshold reached without hard fail".to_owned());
    }
    output
}

/// Verify H3 promotion-gate implementation soundness.
#[must_use]
pub fn verify_h3_promotion_gate(
    evidence: &S4H3PromotionGateEvidence,
) -> S4HypothesisVerifierOutput {
    with_verifier_events("H3", || verify_h3_promotion_gate_impl(evidence))
}

fn verify_h3_promotion_gate_impl(
    evidence: &S4H3PromotionGateEvidence,
) -> S4HypothesisVerifierOutput {
    let mut failures = Vec::new();
    if !evidence.reference_positive_bundle_promoted {
        failures.push("reference-positive promotion bundle was not promoted".to_owned());
    }
    if !evidence.invalid_bundle_rejected {
        failures.push("promotion gate accepted a bundle with failed D8 predicates".to_owned());
    }
    if !evidence.referentially_transparent {
        failures.push("promotion gate was not referentially transparent".to_owned());
    }
    if !evidence.promotion_gate_self_hash_round_trips {
        failures.push(recorded_hash_failure(
            "promotion_gate_self_hash did not round-trip",
            evidence.promotion_gate_self_hash_recorded,
        ));
    }
    let expected = S4PromotionPredicateFamily::ALL
        .into_iter()
        .collect::<BTreeSet<_>>();
    let observed = evidence
        .broken_predicate_families
        .iter()
        .map(|entry| entry.family)
        .collect::<BTreeSet<_>>();
    if observed != expected || observed.len() != evidence.broken_predicate_families.len() {
        failures.push(format!(
            "broken predicate fixtures must cover exactly P-1..P-9 once, got {:?}",
            evidence
                .broken_predicate_families
                .iter()
                .map(|entry| entry.family)
                .collect::<Vec<_>>()
        ));
    }
    for entry in &evidence.broken_predicate_families {
        if !entry.rejected_when_broken {
            failures.push(format!(
                "promotion gate promoted a bundle with {:?} broken",
                entry.family
            ));
        }
    }

    verdict_from_failures(
        S4Hypothesis::H3,
        S4Outcome::FailPromotionGate,
        failures,
        vec![format!(
            "H3 covered {} broken predicate-family fixtures",
            evidence.broken_predicate_families.len()
        )],
    )
}

/// Verify H4 cross-corpus generalization.
#[must_use]
pub fn verify_h4_generalization(
    evidence: &S4H4GeneralizationEvidence,
) -> S4HypothesisVerifierOutput {
    with_verifier_events("H4", || verify_h4_generalization_impl(evidence))
}

fn verify_h4_generalization_impl(
    evidence: &S4H4GeneralizationEvidence,
) -> S4HypothesisVerifierOutput {
    let mut failures = Vec::new();
    let mut suspicious_low_bpc = false;
    if !is_finite_nonnegative(evidence.bpc_kn5_gutenberg_val) {
        failures.push(format!(
            "bpc_kn5_gutenberg_val must be finite and nonnegative, got {}",
            evidence.bpc_kn5_gutenberg_val
        ));
    }
    check_canonical_seed_values(
        evidence.per_seed.iter().map(|entry| entry.seed),
        "H4 per_seed",
        &mut failures,
    );
    let threshold = evidence.bpc_kn5_gutenberg_val - S4_H4_MIN_BPC_MARGIN_VS_KN5;
    let mut bpcs = Vec::with_capacity(evidence.per_seed.len());
    for entry in &evidence.per_seed {
        if !is_finite_nonnegative(entry.bpc_ternary_gutenberg_val) {
            failures.push(format!(
                "bpc_ternary_gutenberg_val for seed {} must be finite and nonnegative, got {}",
                entry.seed, entry.bpc_ternary_gutenberg_val
            ));
            continue;
        }
        bpcs.push(entry.bpc_ternary_gutenberg_val);
        if entry.bpc_ternary_gutenberg_val >= threshold {
            failures.push(format!(
                "seed {} bpc_ternary {} does not beat KN-5 threshold {}",
                entry.seed, entry.bpc_ternary_gutenberg_val, threshold
            ));
        }
        if !entry.v0_success_passed {
            failures.push(format!("seed {} v0_success did not pass", entry.seed));
        }
    }
    if bpcs.len() == S4_CANONICAL_SEEDS.len() {
        let median = median_f64(&mut bpcs);
        if median < S4_H4_SUSPICIOUS_LOW_BPC_THRESHOLD {
            suspicious_low_bpc = true;
            failures.push(format!(
                "median bpc_ternary_gutenberg {median} is below suspicious threshold {}",
                S4_H4_SUSPICIOUS_LOW_BPC_THRESHOLD
            ));
        }
    }

    let mut output = verdict_from_failures(
        S4Hypothesis::H4,
        S4Outcome::FailQualityOnGutenberg,
        failures,
        vec![format!(
            "H4 checked {} per-seed Gutenberg scores against KN-5 bpc {}",
            evidence.per_seed.len(),
            evidence.bpc_kn5_gutenberg_val
        )],
    );
    output.suspicious_low_bpc = suspicious_low_bpc;
    output
}

/// Verify H5 three-way oracle agreement.
#[must_use]
pub fn verify_h5_oracle_agreement(
    evidence: S4H5OracleAgreementEvidence,
) -> S4HypothesisVerifierOutput {
    with_verifier_events("H5", || verify_h5_oracle_agreement_impl(evidence))
}

fn verify_h5_oracle_agreement_impl(
    evidence: S4H5OracleAgreementEvidence,
) -> S4HypothesisVerifierOutput {
    let mut failures = Vec::new();
    if evidence.seed != 0 {
        failures.push(format!("H5 requires seed 0, got {}", evidence.seed));
    }
    if !evidence.outcome_agree {
        failures.push("oracle agreement outcome was not Agree".to_owned());
    }
    for (gap_field, gap, tolerance_field, tolerance) in [
        (
            "gap_live_vs_denotational",
            evidence.gap_live_vs_denotational,
            "tolerance_live_vs_denotational",
            evidence.tolerance_live_vs_denotational,
        ),
        (
            "gap_live_vs_artifact",
            evidence.gap_live_vs_artifact,
            "tolerance_live_vs_artifact",
            evidence.tolerance_live_vs_artifact,
        ),
        (
            "gap_denotational_vs_artifact",
            evidence.gap_denotational_vs_artifact,
            "tolerance_denotational_vs_artifact",
            evidence.tolerance_denotational_vs_artifact,
        ),
    ] {
        if !is_finite_nonnegative(gap) {
            failures.push(format!(
                "{gap_field} must be finite and nonnegative, got {gap}"
            ));
        }
        if !is_finite_nonnegative(tolerance) {
            failures.push(format!(
                "{tolerance_field} must be finite and nonnegative, got {tolerance}"
            ));
        }
        if is_finite_nonnegative(gap) && is_finite_nonnegative(tolerance) && gap > tolerance {
            failures.push(format!(
                "{gap_field} {gap} exceeds {tolerance_field} {tolerance}"
            ));
        }
    }

    verdict_from_failures(
        S4Hypothesis::H5,
        S4Outcome::FailOracleDisagreement,
        failures,
        vec![format!(
            "H5 checked seed {} oracle gaps ({}, {}, {})",
            evidence.seed,
            evidence.gap_live_vs_denotational,
            evidence.gap_live_vs_artifact,
            evidence.gap_denotational_vs_artifact
        )],
    )
}

/// Verify H6 determinism across corpus switching.
#[must_use]
pub fn verify_h6_determinism(evidence: &S4H6DeterminismEvidence) -> S4HypothesisVerifierOutput {
    with_verifier_events("H6", || verify_h6_determinism_impl(evidence))
}

fn verify_h6_determinism_impl(evidence: &S4H6DeterminismEvidence) -> S4HypothesisVerifierOutput {
    let mut failures = Vec::new();
    check_canonical_seed_values(
        evidence
            .per_seed_tensor_payloads
            .iter()
            .map(|entry| entry.seed),
        "H6 per_seed_tensor_payloads",
        &mut failures,
    );
    for entry in &evidence.per_seed_tensor_payloads {
        if entry.original_tensor_payload_sha == Hash256::ZERO {
            failures.push(format!(
                "seed {} original tensor payload SHA is zero",
                entry.seed
            ));
        }
        if entry.replay_tensor_payload_sha == Hash256::ZERO {
            failures.push(format!(
                "seed {} replay tensor payload SHA is zero",
                entry.seed
            ));
        }
        if entry.original_tensor_payload_sha != entry.replay_tensor_payload_sha {
            failures.push(format!(
                "seed {} tensor payload SHA mismatch: original {}, replay {}",
                entry.seed, entry.original_tensor_payload_sha, entry.replay_tensor_payload_sha
            ));
        }
    }

    let expected_artifacts = required_h6_artifacts().into_iter().collect::<BTreeSet<_>>();
    let observed_artifacts = evidence
        .artifact_self_hashes
        .iter()
        .map(|entry| entry.artifact.clone())
        .collect::<BTreeSet<_>>();
    if observed_artifacts != expected_artifacts
        || observed_artifacts.len() != evidence.artifact_self_hashes.len()
    {
        failures.push(format!(
            "H6 artifact replay evidence must cover exactly the required artifact set, got {:?}",
            evidence
                .artifact_self_hashes
                .iter()
                .map(|entry| entry.artifact.clone())
                .collect::<Vec<_>>()
        ));
    }
    for entry in &evidence.artifact_self_hashes {
        if let Some(seed) = entry.artifact.seed() {
            check_seed_value(seed, "H6 artifact seed", &mut failures);
        }
        if entry.original_self_hash == Hash256::ZERO {
            failures.push(format!("{:?} original self-hash is zero", entry.artifact));
        }
        if entry.replay_self_hash == Hash256::ZERO {
            failures.push(format!("{:?} replay self-hash is zero", entry.artifact));
        }
        if entry.original_self_hash != entry.replay_self_hash {
            failures.push(format!(
                "{:?} self-hash mismatch: original {}, replay {}",
                entry.artifact, entry.original_self_hash, entry.replay_self_hash
            ));
        }
    }

    verdict_from_failures(
        S4Hypothesis::H6,
        S4Outcome::FailSubstrate,
        failures,
        vec![format!(
            "H6 checked {} seed tensor payloads and {} artifact self-hashes",
            evidence.per_seed_tensor_payloads.len(),
            evidence.artifact_self_hashes.len()
        )],
    )
}

/// Verify H7 distribution-shift sanity.
#[must_use]
pub fn verify_h7_distribution_shift(
    evidence: &S4H7DistributionShiftEvidence,
) -> S4HypothesisVerifierOutput {
    with_verifier_events("H7", || verify_h7_distribution_shift_impl(evidence))
}

fn verify_h7_distribution_shift_impl(
    evidence: &S4H7DistributionShiftEvidence,
) -> S4HypothesisVerifierOutput {
    let mut failures = Vec::new();
    if !is_finite_nonnegative(evidence.c_ts_bpc_gutenberg_val) {
        failures.push(format!(
            "c_ts_bpc_gutenberg_val must be finite and nonnegative, got {}",
            evidence.c_ts_bpc_gutenberg_val
        ));
    }
    check_canonical_seed_values(
        evidence.per_seed.iter().map(|entry| entry.seed),
        "H7 per_seed",
        &mut failures,
    );
    let mut improved = 0_usize;
    for entry in &evidence.per_seed {
        if !is_finite_nonnegative(entry.c_gb_bpc_gutenberg_val) {
            failures.push(format!(
                "seed {} c_gb_bpc_gutenberg_val must be finite and nonnegative, got {}",
                entry.seed, entry.c_gb_bpc_gutenberg_val
            ));
            continue;
        }
        let improvement = evidence.c_ts_bpc_gutenberg_val - entry.c_gb_bpc_gutenberg_val;
        if improvement > S4_H7_MIN_SHIFT_IMPROVEMENT {
            improved += 1;
        }
    }
    if failures.is_empty() && improved < S4_H7_MIN_SHIFTED_SEEDS {
        failures.push(format!(
            "only {improved} seeds improved by more than {}, need at least {}",
            S4_H7_MIN_SHIFT_IMPROVEMENT, S4_H7_MIN_SHIFTED_SEEDS
        ));
    }

    let mut output = verdict_from_failures(
        S4Hypothesis::H7,
        S4Outcome::PassClean,
        failures,
        vec![format!(
            "H7 checked {} per-seed shift observations, improved seeds={improved}",
            evidence.per_seed.len()
        )],
    );
    output.outcome_if_refuted = None;
    output
}

/// Run all H1-H7 verifiers and dispatch the RFC §11 outcome.
#[must_use]
pub fn verify_s4(evidence: &S4VerifierEvidence) -> S4VerifierReport {
    let outputs = vec![
        verify_h1_corpus_integrity(&evidence.h1),
        verify_h2_contamination(evidence.h2),
        verify_h3_promotion_gate(&evidence.h3),
        verify_h4_generalization(&evidence.h4),
        verify_h5_oracle_agreement(evidence.h5),
        verify_h6_determinism(&evidence.h6),
        verify_h7_distribution_shift(&evidence.h7),
    ];
    let bundle = verifier_bundle_from_outputs(
        &outputs,
        evidence.promotion_gate_accepted_canonical,
        evidence.completions.clone(),
    );
    let outcome = dispatch_s4_outcome(&bundle);
    let decision = decision_for_s4_outcome(outcome);
    let hypothesis_outputs = outputs
        .into_iter()
        .map(|output| (output.hypothesis, output))
        .collect();
    S4VerifierReport {
        hypothesis_outputs,
        bundle,
        outcome,
        decision,
    }
}

/// Build the schema verifier bundle from per-hypothesis outputs.
#[must_use]
pub fn verifier_bundle_from_outputs(
    outputs: &[S4HypothesisVerifierOutput],
    promotion_gate_accepted_canonical: bool,
    completions: Vec<S4Completion>,
) -> S4VerifierBundle {
    let hypothesis_statuses = outputs
        .iter()
        .map(|output| (output.hypothesis, output.status.clone()))
        .collect::<BTreeMap<_, _>>();
    let status = |hypothesis| {
        hypothesis_statuses
            .get(&hypothesis)
            .cloned()
            .unwrap_or_else(|| HypothesisStatus::NotEvaluatedDueToPriorGate {
                reason: "missing verifier output".to_owned(),
            })
    };

    S4VerifierBundle {
        corpus_integrity_passed: matches!(status(S4Hypothesis::H1), HypothesisStatus::Confirmed),
        contamination_passed: matches!(status(S4Hypothesis::H2), HypothesisStatus::Confirmed),
        promotion_gate_sound: matches!(status(S4Hypothesis::H3), HypothesisStatus::Confirmed),
        promotion_gate_accepted_canonical,
        gutenberg_quality_passed: matches!(status(S4Hypothesis::H4), HypothesisStatus::Confirmed),
        oracle_agreement_passed: matches!(status(S4Hypothesis::H5), HypothesisStatus::Confirmed),
        substrate_passed: matches!(status(S4Hypothesis::H6), HypothesisStatus::Confirmed),
        suspicious_low_bpc: outputs.iter().any(|output| output.suspicious_low_bpc),
        contamination_warning: outputs.iter().any(|output| output.contamination_warning),
        completions,
        hypothesis_statuses,
    }
}

/// Dispatch an S4 verifier bundle to exactly one RFC §11 outcome.
#[must_use]
pub fn dispatch_s4_outcome(bundle: &S4VerifierBundle) -> S4Outcome {
    if is_refuted(bundle, S4Hypothesis::H1)
        || (is_confirmed(bundle, S4Hypothesis::H1) && !bundle.corpus_integrity_passed)
    {
        return S4Outcome::FailCorpusIntegrity;
    }
    if is_not_evaluated(bundle, S4Hypothesis::H1) {
        return S4Outcome::FailSubstrate;
    }
    if is_refuted(bundle, S4Hypothesis::H2)
        || (is_confirmed(bundle, S4Hypothesis::H2) && !bundle.contamination_passed)
    {
        return S4Outcome::FailContamination;
    }
    if is_not_evaluated(bundle, S4Hypothesis::H2) {
        return S4Outcome::FailSubstrate;
    }
    if is_refuted(bundle, S4Hypothesis::H3)
        || (is_confirmed(bundle, S4Hypothesis::H3) && !bundle.promotion_gate_sound)
    {
        return S4Outcome::FailPromotionGate;
    }
    if is_not_evaluated(bundle, S4Hypothesis::H3) {
        return S4Outcome::FailSubstrate;
    }
    if !bundle.promotion_gate_accepted_canonical {
        return S4Outcome::FailPromotionGateReadiness;
    }
    if bundle
        .completions
        .iter()
        .any(|completion| matches!(completion, S4Completion::DivergedAt { .. }))
    {
        return S4Outcome::FailSubstrate;
    }
    if bundle.suspicious_low_bpc {
        return S4Outcome::FailSuspicious;
    }
    if is_refuted(bundle, S4Hypothesis::H4)
        || (is_confirmed(bundle, S4Hypothesis::H4) && !bundle.gutenberg_quality_passed)
    {
        return S4Outcome::FailQualityOnGutenberg;
    }
    if is_not_evaluated(bundle, S4Hypothesis::H4) {
        return S4Outcome::FailSubstrate;
    }
    if is_refuted(bundle, S4Hypothesis::H5)
        || (is_confirmed(bundle, S4Hypothesis::H5) && !bundle.oracle_agreement_passed)
    {
        return S4Outcome::FailOracleDisagreement;
    }
    if is_not_evaluated(bundle, S4Hypothesis::H5) {
        return S4Outcome::FailSubstrate;
    }
    if is_refuted(bundle, S4Hypothesis::H6)
        || (is_confirmed(bundle, S4Hypothesis::H6) && !bundle.substrate_passed)
    {
        return S4Outcome::FailSubstrate;
    }
    if is_not_evaluated(bundle, S4Hypothesis::H6) {
        return S4Outcome::FailSubstrate;
    }
    if bundle.contamination_warning {
        S4Outcome::PassWithContaminationWarning
    } else {
        S4Outcome::PassClean
    }
}

/// Map an S4 outcome to the RFC §11 decision tag.
#[must_use]
pub fn decision_for_s4_outcome(outcome: S4Outcome) -> S4Decision {
    match outcome {
        S4Outcome::PassClean => S4Decision::ProceedToS5,
        S4Outcome::PassWithContaminationWarning => S4Decision::ProceedToS5WithContaminationWarning,
        S4Outcome::FailCorpusIntegrity => S4Decision::Halt {
            reason: "corpus-integrity-broken".to_owned(),
        },
        S4Outcome::FailContamination => S4Decision::Halt {
            reason: "contamination-dirty".to_owned(),
        },
        S4Outcome::FailPromotionGate => S4Decision::Halt {
            reason: "promotion-gate-unsound".to_owned(),
        },
        S4Outcome::FailPromotionGateReadiness => S4Decision::Halt {
            reason: "promotion-gate-rejected-canonical".to_owned(),
        },
        S4Outcome::FailQualityOnGutenberg => S4Decision::Investigate {
            reason: "propose-step-budget-or-Toy1".to_owned(),
        },
        S4Outcome::FailOracleDisagreement => S4Decision::Halt {
            reason: "oracle-disagrees-on-gutenberg".to_owned(),
        },
        S4Outcome::FailSubstrate => S4Decision::Investigate {
            reason: "burn-or-corpus-loader".to_owned(),
        },
        S4Outcome::FailSuspicious => S4Decision::Halt {
            reason: "audit-split-and-bpc".to_owned(),
        },
    }
}

/// Required H6 replay artifact identities.
#[must_use]
pub fn required_h6_artifacts() -> Vec<S4DeterministicArtifact> {
    let mut artifacts = vec![
        S4DeterministicArtifact::GutenbergManifest,
        S4DeterministicArtifact::CorpusQuality,
        S4DeterministicArtifact::ContaminationReport,
        S4DeterministicArtifact::CorpusProgression,
        S4DeterministicArtifact::PromotionGate,
        S4DeterministicArtifact::BaselineGutenberg,
        S4DeterministicArtifact::OracleAgreement,
        S4DeterministicArtifact::Report,
    ];
    for seed in S4_CANONICAL_SEEDS {
        artifacts.push(S4DeterministicArtifact::FpReference { seed });
        artifacts.push(S4DeterministicArtifact::GutenbergRunLog { seed });
        artifacts.push(S4DeterministicArtifact::GutenbergCheckpoint { seed });
        artifacts.push(S4DeterministicArtifact::GutenbergScore { seed });
    }
    artifacts
}

impl S4DeterministicArtifact {
    fn seed(&self) -> Option<u64> {
        match self {
            Self::FpReference { seed }
            | Self::GutenbergRunLog { seed }
            | Self::GutenbergCheckpoint { seed }
            | Self::GutenbergScore { seed } => Some(*seed),
            Self::GutenbergManifest
            | Self::CorpusQuality
            | Self::ContaminationReport
            | Self::CorpusProgression
            | Self::PromotionGate
            | Self::BaselineGutenberg
            | Self::OracleAgreement
            | Self::Report => None,
        }
    }
}

fn with_verifier_events(
    name: &'static str,
    f: impl FnOnce() -> S4HypothesisVerifierOutput,
) -> S4HypothesisVerifierOutput {
    tracing::info!(
        target: crate::S4_LOG_TARGET,
        event_name = S4_VERIFIER_STARTED_EVENT,
        name,
        "s4 verifier started"
    );
    let output = f();
    let outcome = hypothesis_status_label(&output.status);
    let reason = verifier_reason(&output);
    tracing::info!(
        target: crate::S4_LOG_TARGET,
        event_name = S4_VERIFIER_FINALIZED_EVENT,
        name,
        outcome,
        reason,
        "s4 verifier finalized"
    );
    output
}

fn verdict_from_failures(
    hypothesis: S4Hypothesis,
    outcome_if_refuted: S4Outcome,
    failures: Vec<String>,
    mut success_observations: Vec<String>,
) -> S4HypothesisVerifierOutput {
    if failures.is_empty() {
        success_observations.push("all falsification rules clear".to_owned());
        S4HypothesisVerifierOutput::confirmed(hypothesis, success_observations)
    } else {
        S4HypothesisVerifierOutput::refuted(hypothesis, outcome_if_refuted, failures)
    }
}

fn recorded_hash_failure(message: &'static str, recorded: Option<Hash256>) -> String {
    recorded.map_or_else(
        || message.to_owned(),
        |hash| format!("{message}: recorded {hash}"),
    )
}

fn verifier_reason(output: &S4HypothesisVerifierOutput) -> &str {
    output
        .observations
        .first()
        .map_or("no verifier observations recorded", String::as_str)
}

fn hypothesis_status_label(status: &HypothesisStatus) -> &'static str {
    match status {
        HypothesisStatus::Confirmed => "confirmed",
        HypothesisStatus::Refuted => "refuted",
        HypothesisStatus::NotEvaluatedDueToPriorGate { .. } => "not_evaluated_due_to_prior_gate",
    }
}

fn is_refuted(bundle: &S4VerifierBundle, hypothesis: S4Hypothesis) -> bool {
    matches!(bundle.status(hypothesis), HypothesisStatus::Refuted)
}

fn is_confirmed(bundle: &S4VerifierBundle, hypothesis: S4Hypothesis) -> bool {
    matches!(bundle.status(hypothesis), HypothesisStatus::Confirmed)
}

fn is_not_evaluated(bundle: &S4VerifierBundle, hypothesis: S4Hypothesis) -> bool {
    matches!(
        bundle.status(hypothesis),
        HypothesisStatus::NotEvaluatedDueToPriorGate { .. }
    )
}

fn is_finite_nonnegative(value: f64) -> bool {
    value.is_finite() && value >= 0.0
}

fn exceeds_ratio(count: u64, total: u64, numerator: u64, denominator: u64) -> bool {
    u128::from(count) * u128::from(denominator) > u128::from(total) * u128::from(numerator)
}

fn checked_add_or_failure(
    lhs: u64,
    rhs: u64,
    field: &'static str,
    failures: &mut Vec<String>,
) -> Option<u64> {
    lhs.checked_add(rhs).or_else(|| {
        failures.push(format!("{field} overflowed u64"));
        None
    })
}

fn check_canonical_seed_values(
    seeds: impl Iterator<Item = u64>,
    field: &'static str,
    failures: &mut Vec<String>,
) {
    let observed = seeds.collect::<Vec<_>>();
    for seed in &observed {
        check_seed_value(*seed, field, failures);
    }
    let mut sorted = observed.clone();
    sorted.sort_unstable();
    if sorted != S4_CANONICAL_SEEDS {
        failures.push(format!(
            "{field} must cover canonical seeds {:?}, got {observed:?}",
            S4_CANONICAL_SEEDS
        ));
    }
}

fn check_seed_value(seed: u64, field: &'static str, failures: &mut Vec<String>) {
    if !S4_CANONICAL_SEEDS.contains(&seed) {
        failures.push(format!("{field} contains non-canonical seed {seed}"));
    }
}

fn median_f64(values: &mut [f64]) -> f64 {
    values.sort_by(|lhs, rhs| lhs.total_cmp(rhs));
    values[values.len() / 2]
}
