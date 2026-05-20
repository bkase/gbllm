//! S4 promotion gate surface.

use std::error::Error;
use std::fmt;
use std::path::Path;

use gbf_artifact::{GutenbergManifest, GutenbergManifestError};
use gbf_foundation::{CanonicalJson, DomainHash, Hash256};
use serde::{Deserialize, Serialize};

/// Schema id for the D8 promotion-gate artifact.
pub const S4_PROMOTION_GATE_SCHEMA: &str = "s4_promotion_gate.v1";

/// Canonical S4 path for the promotion-gate product.
pub const S4_PROMOTION_GATE_PATH: &str = "experiments/S4/promotion_gate/promotion_gate.json";

/// Maximum TinyStories ternary-vs-full precision bpc gap inherited from S3.
pub const S4_PROMOTION_MAX_TERNARY_GAP_TS: f64 = 0.5;

/// Maximum Gutenberg corpus unmappable rate from D5/D8 P-6.
pub const S4_PROMOTION_MAX_GUTENBERG_UNMAPPABLE_RATE: f64 = 0.005;

/// Promotion-gate evaluation start event.
pub const S4_PROMOTION_GATE_STARTED_EVENT: &str = "s4_promotion_gate_started";

/// Promotion-gate predicate check event.
pub const S4_PROMOTION_GATE_CHECK_EVENT: &str = "s4_promotion_gate_check";

/// Promotion-gate final outcome event.
pub const S4_PROMOTION_GATE_OUTCOME_EVENT: &str = "s4_promotion_gate_outcome";

/// Promotion-gate tracing target.
pub const S4_PROMOTION_GATE_LOG_TARGET: &str = "gbf_experiments::s4::promote";

const PRODUCT_SCHEMA_VERSION: &str = "1";
const S3_CHECKPOINT_PROMOTION_SCHEMA: &str = "s3_checkpoint_promotion.v1";
const S3_V0_SUCCESS_PROMOTION_SCHEMA: &str = "s3_v0_success.v1";
const S3_ORACLE_AGREEMENT_PROMOTION_SCHEMA: &str = "s3_oracle_agreement.v1";
const S4_CONTAMINATION_PROMOTION_SCHEMA: &str = "s4_contamination_report.v1";
const S4_BASELINE_GUTENBERG_PROMOTION_SCHEMA: &str = "s4_baseline_gutenberg.v1";
const S3_REPETITION_COLLAPSE_PROMOTION_SCHEMA: &str = "s3_repetition_collapse_check.v1";
const PHASE_D_BUILD_KINDS: &[&str] = &[
    "phase_d",
    "PhaseD",
    "phase_d_resumable",
    "phase_d_resumeable",
    "s3_phase_d",
];
const PROMOTION_PREDICATES: &[&str] = &[
    "P-1", "P-2", "P-3", "P-4", "P-5", "P-6", "P-7", "P-8", "P-9",
];
const PROMOTION_GATE_DOMAIN: DomainHash<'static> = DomainHash::new(
    "gbf-experiments",
    "PromotionGateProduct",
    S4_PROMOTION_GATE_SCHEMA,
    PRODUCT_SCHEMA_VERSION,
);
const CHECKPOINT_DOMAIN: DomainHash<'static> = DomainHash::new(
    "gbf-experiments",
    "S3CheckpointPromotionArtifact",
    S3_CHECKPOINT_PROMOTION_SCHEMA,
    PRODUCT_SCHEMA_VERSION,
);
const V0_SUCCESS_DOMAIN: DomainHash<'static> = DomainHash::new(
    "gbf-experiments",
    "S3V0SuccessPromotionArtifact",
    S3_V0_SUCCESS_PROMOTION_SCHEMA,
    PRODUCT_SCHEMA_VERSION,
);
const ORACLE_DOMAIN: DomainHash<'static> = DomainHash::new(
    "gbf-experiments",
    "S3OracleAgreementPromotionArtifact",
    S3_ORACLE_AGREEMENT_PROMOTION_SCHEMA,
    PRODUCT_SCHEMA_VERSION,
);
const CONTAMINATION_DOMAIN: DomainHash<'static> = DomainHash::new(
    "gbf-experiments",
    "S4ContaminationPromotionArtifact",
    S4_CONTAMINATION_PROMOTION_SCHEMA,
    PRODUCT_SCHEMA_VERSION,
);
const BASELINE_DOMAIN: DomainHash<'static> = DomainHash::new(
    "gbf-experiments",
    "S4BaselineGutenbergPromotionArtifact",
    S4_BASELINE_GUTENBERG_PROMOTION_SCHEMA,
    PRODUCT_SCHEMA_VERSION,
);
const REPETITION_DOMAIN: DomainHash<'static> = DomainHash::new(
    "gbf-experiments",
    "S3RepetitionCollapsePromotionArtifact",
    S3_REPETITION_COLLAPSE_PROMOTION_SCHEMA,
    PRODUCT_SCHEMA_VERSION,
);

/// Explicit artifact path plus self-hash consumed by the promotion gate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromotionGateArtifactRef {
    /// Explicit artifact path supplied by the caller.
    pub artifact_path: String,
    /// Caller-supplied self-hash for the artifact at `artifact_path`.
    pub artifact_self_hash: Hash256,
}

impl PromotionGateArtifactRef {
    /// Construct an artifact reference.
    #[must_use]
    pub fn new(artifact_path: impl Into<String>, artifact_self_hash: Hash256) -> Self {
        Self {
            artifact_path: artifact_path.into(),
            artifact_self_hash,
        }
    }

    fn uses_latest_selector(&self) -> bool {
        let trimmed = self.artifact_path.trim();
        if trimmed.is_empty() {
            return true;
        }
        Path::new(trimmed).components().any(|component| {
            component
                .as_os_str()
                .to_str()
                .is_some_and(|part| part.eq_ignore_ascii_case("latest"))
        })
    }
}

/// A parsed artifact together with the explicit path/self-hash reference used to load it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromotionGateBoundArtifact<T> {
    /// Explicit artifact path plus expected self-hash.
    pub artifact_ref: PromotionGateArtifactRef,
    /// Parsed artifact payload.
    pub artifact: T,
}

impl<T> PromotionGateBoundArtifact<T> {
    /// Construct a bound promotion-gate artifact.
    #[must_use]
    pub fn new(artifact_ref: PromotionGateArtifactRef, artifact: T) -> Self {
        Self {
            artifact_ref,
            artifact,
        }
    }
}

/// Minimal checkpoint facts consumed by D8 P-1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S3CheckpointPromotionArtifact {
    /// Schema id for this promotion-side checkpoint summary.
    pub schema: String,
    /// Canonical self-hash for the S3 checkpoint.
    pub checkpoint_self_hash: Hash256,
    /// S3 checkpoint build/phase tag.
    pub build_kind: String,
    /// Whether the checkpoint carries deployed ternary weights.
    pub contains_deployed_ternary_weights: bool,
    /// Whether QAT shadow-weight payloads are present for Phase-D resume.
    pub contains_qat_shadow_weights: bool,
}

impl S3CheckpointPromotionArtifact {
    /// Construct a checkpoint summary with a computed self-hash.
    pub fn new(
        build_kind: impl Into<String>,
        contains_deployed_ternary_weights: bool,
        contains_qat_shadow_weights: bool,
    ) -> Result<Self, PromotionGateError> {
        let mut artifact = Self {
            schema: S3_CHECKPOINT_PROMOTION_SCHEMA.to_owned(),
            checkpoint_self_hash: Hash256::ZERO,
            build_kind: build_kind.into(),
            contains_deployed_ternary_weights,
            contains_qat_shadow_weights,
        };
        artifact.checkpoint_self_hash = artifact.compute_self_hash()?;
        Ok(artifact)
    }

    /// Compute the checkpoint self-hash.
    pub fn compute_self_hash(&self) -> Result<Hash256, PromotionGateError> {
        compute_self_hash(self, "checkpoint_self_hash", CHECKPOINT_DOMAIN)
    }

    fn phase_d_resumable(&self) -> bool {
        PHASE_D_BUILD_KINDS.contains(&self.build_kind.as_str())
            && self.contains_deployed_ternary_weights
            && self.contains_qat_shadow_weights
    }
}

/// S3 v0_success acceptance bits consumed by D8 P-1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct V0SuccessAcceptanceBits {
    /// S3 Q1 acceptance bit.
    pub q1_holds: bool,
    /// S3 Q2 acceptance bit.
    pub q2_holds: bool,
    /// S3 Q3 acceptance bit.
    pub q3_holds: bool,
    /// S3 Q4 acceptance bit.
    pub q4_holds: bool,
    /// S3 Q5 acceptance bit.
    pub q5_holds: bool,
    /// S3 Q6 acceptance bit.
    pub q6_holds: bool,
}

impl V0SuccessAcceptanceBits {
    /// All v0_success acceptance bits set.
    #[must_use]
    pub const fn all_pass() -> Self {
        Self {
            q1_holds: true,
            q2_holds: true,
            q3_holds: true,
            q4_holds: true,
            q5_holds: true,
            q6_holds: true,
        }
    }

    fn all_set(self) -> bool {
        self.q1_holds
            && self.q2_holds
            && self.q3_holds
            && self.q4_holds
            && self.q5_holds
            && self.q6_holds
    }
}

/// Overall S3 v0_success outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum V0SuccessGateOutcome {
    /// v0_success passed.
    Pass,
    /// v0_success failed.
    Fail,
}

/// Minimal S3 v0_success artifact facts consumed by D8 P-1/P-3.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S3V0SuccessPromotionArtifact {
    /// Schema id, always `s3_v0_success.v1`.
    pub schema: String,
    /// Self-hash of the checkpoint evaluated by v0_success.
    pub checkpoint_self_hash: Hash256,
    /// Self-hash of the TinyStories manifest evaluated by v0_success.
    pub tinystories_manifest_self_hash: Hash256,
    /// Per-quality-gate acceptance bits.
    pub acceptance_bits: V0SuccessAcceptanceBits,
    /// Overall v0_success outcome.
    pub outcome: V0SuccessGateOutcome,
    /// TinyStories ternary-vs-full precision bpc gap.
    pub ternary_gap_ts: f64,
    /// Canonical self-hash for the v0_success artifact.
    pub v0_success_self_hash: Hash256,
}

impl S3V0SuccessPromotionArtifact {
    /// Construct a v0_success summary with a computed self-hash.
    pub fn new(
        checkpoint_self_hash: Hash256,
        tinystories_manifest_self_hash: Hash256,
        acceptance_bits: V0SuccessAcceptanceBits,
        outcome: V0SuccessGateOutcome,
        ternary_gap_ts: f64,
    ) -> Result<Self, PromotionGateError> {
        let mut artifact = Self {
            schema: S3_V0_SUCCESS_PROMOTION_SCHEMA.to_owned(),
            checkpoint_self_hash,
            tinystories_manifest_self_hash,
            acceptance_bits,
            outcome,
            ternary_gap_ts,
            v0_success_self_hash: Hash256::ZERO,
        };
        artifact.v0_success_self_hash = artifact.compute_self_hash()?;
        Ok(artifact)
    }

    /// Compute the v0_success self-hash.
    pub fn compute_self_hash(&self) -> Result<Hash256, PromotionGateError> {
        compute_self_hash(self, "v0_success_self_hash", V0_SUCCESS_DOMAIN)
    }
}

/// Overall S3 oracle-agreement outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum S3OracleAgreementOutcome {
    /// The agreement product passed.
    Agree,
    /// The agreement product disagreed.
    Disagree,
}

/// Minimal S3 oracle-agreement facts consumed by D8 P-2.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S3OracleAgreementPromotionArtifact {
    /// Schema id, always `s3_oracle_agreement.v1`.
    pub schema: String,
    /// Self-hash of the checkpoint evaluated by oracle agreement.
    pub checkpoint_self_hash: Hash256,
    /// Self-hash of the TinyStories manifest evaluated by oracle agreement.
    pub tinystories_manifest_self_hash: Hash256,
    /// Agreement outcome.
    pub outcome: S3OracleAgreementOutcome,
    /// Whether all per-token bpc gaps were inside the S3-pinned tolerance.
    pub per_token_bpc_gaps_within_tolerance: bool,
    /// Canonical self-hash for the oracle-agreement artifact.
    pub oracle_agreement_self_hash: Hash256,
}

impl S3OracleAgreementPromotionArtifact {
    /// Construct an oracle-agreement summary with a computed self-hash.
    pub fn new(
        checkpoint_self_hash: Hash256,
        tinystories_manifest_self_hash: Hash256,
        outcome: S3OracleAgreementOutcome,
        per_token_bpc_gaps_within_tolerance: bool,
    ) -> Result<Self, PromotionGateError> {
        let mut artifact = Self {
            schema: S3_ORACLE_AGREEMENT_PROMOTION_SCHEMA.to_owned(),
            checkpoint_self_hash,
            tinystories_manifest_self_hash,
            outcome,
            per_token_bpc_gaps_within_tolerance,
            oracle_agreement_self_hash: Hash256::ZERO,
        };
        artifact.oracle_agreement_self_hash = artifact.compute_self_hash()?;
        Ok(artifact)
    }

    /// Compute the oracle-agreement self-hash.
    pub fn compute_self_hash(&self) -> Result<Hash256, PromotionGateError> {
        compute_self_hash(self, "oracle_agreement_self_hash", ORACLE_DOMAIN)
    }
}

/// S4 cross-corpus contamination outcome consumed by D8 P-5.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "PascalCase", deny_unknown_fields)]
pub enum S4ContaminationOutcome {
    /// Cross-corpus contamination check was clean.
    Clean,
    /// Contamination check found non-gating warnings.
    Warn {
        /// Warning labels.
        findings: Vec<String>,
    },
    /// Contamination check hard-failed.
    HardFail {
        /// Gating failure labels.
        failures: Vec<String>,
        /// Non-gating warning labels observed alongside failures.
        warnings: Vec<String>,
    },
}

/// Minimal S4 contamination-report facts consumed by D8 P-5.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S4ContaminationPromotionArtifact {
    /// Schema id, always `s4_contamination_report.v1`.
    pub schema: String,
    /// Self-hash of the TinyStories manifest compared by contamination.
    pub tinystories_manifest_self_hash: Hash256,
    /// Self-hash of the Gutenberg manifest compared by contamination.
    pub gutenberg_manifest_self_hash: Hash256,
    /// Contamination outcome.
    pub outcome: S4ContaminationOutcome,
    /// Canonical self-hash for the contamination report.
    pub contamination_self_hash: Hash256,
}

impl S4ContaminationPromotionArtifact {
    /// Construct a contamination summary with a computed self-hash.
    pub fn new(
        tinystories_manifest_self_hash: Hash256,
        gutenberg_manifest_self_hash: Hash256,
        outcome: S4ContaminationOutcome,
    ) -> Result<Self, PromotionGateError> {
        let mut artifact = Self {
            schema: S4_CONTAMINATION_PROMOTION_SCHEMA.to_owned(),
            tinystories_manifest_self_hash,
            gutenberg_manifest_self_hash,
            outcome,
            contamination_self_hash: Hash256::ZERO,
        };
        artifact.contamination_self_hash = artifact.compute_self_hash()?;
        Ok(artifact)
    }

    /// Compute the contamination report self-hash.
    pub fn compute_self_hash(&self) -> Result<Hash256, PromotionGateError> {
        compute_self_hash(self, "contamination_self_hash", CONTAMINATION_DOMAIN)
    }

    fn promotes(&self) -> bool {
        matches!(
            self.outcome,
            S4ContaminationOutcome::Clean | S4ContaminationOutcome::Warn { .. }
        )
    }
}

/// Minimal Gutenberg KN-5 baseline facts consumed by D8 P-7.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S4BaselineGutenbergPromotionArtifact {
    /// Schema id, always `s4_baseline_gutenberg.v1`.
    pub schema: String,
    /// Self-hash of the Gutenberg manifest used by the baseline.
    pub gutenberg_manifest_self_hash: Hash256,
    /// SHA-256 of the Gutenberg train corpus bytes.
    pub corpus_train_sha: Hash256,
    /// SHA-256 of the Gutenberg validation corpus bytes.
    pub corpus_val_sha: Hash256,
    /// KN-5 validation bpc.
    pub bpc_kn5: f64,
    /// Canonical self-hash for the baseline artifact.
    pub baseline_self_hash: Hash256,
}

impl S4BaselineGutenbergPromotionArtifact {
    /// Construct a Gutenberg baseline summary with a computed self-hash.
    pub fn new(
        gutenberg_manifest_self_hash: Hash256,
        corpus_train_sha: Hash256,
        corpus_val_sha: Hash256,
        bpc_kn5: f64,
    ) -> Result<Self, PromotionGateError> {
        let mut artifact = Self {
            schema: S4_BASELINE_GUTENBERG_PROMOTION_SCHEMA.to_owned(),
            gutenberg_manifest_self_hash,
            corpus_train_sha,
            corpus_val_sha,
            bpc_kn5,
            baseline_self_hash: Hash256::ZERO,
        };
        artifact.baseline_self_hash = artifact.compute_self_hash()?;
        Ok(artifact)
    }

    /// Compute the Gutenberg baseline self-hash.
    pub fn compute_self_hash(&self) -> Result<Hash256, PromotionGateError> {
        compute_self_hash(self, "baseline_self_hash", BASELINE_DOMAIN)
    }
}

/// Repetition-collapse check outcome consumed by D8 P-8.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum S3RepetitionCollapseOutcome {
    /// No repetition collapse signature was detected.
    Pass,
    /// A repetition collapse signature was detected.
    Fail,
}

/// Minimal repetition-collapse facts consumed by D8 P-8.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S3RepetitionCollapsePromotionArtifact {
    /// Schema id for the repetition-collapse check.
    pub schema: String,
    /// Self-hash of the checkpoint evaluated by the repetition check.
    pub checkpoint_self_hash: Hash256,
    /// Self-hash of the TinyStories manifest evaluated by the repetition check.
    pub tinystories_manifest_self_hash: Hash256,
    /// Repetition-collapse outcome.
    pub outcome: S3RepetitionCollapseOutcome,
    /// Canonical self-hash for the repetition-collapse artifact.
    pub repetition_self_hash: Hash256,
}

impl S3RepetitionCollapsePromotionArtifact {
    /// Construct a repetition-collapse summary with a computed self-hash.
    pub fn new(
        checkpoint_self_hash: Hash256,
        tinystories_manifest_self_hash: Hash256,
        outcome: S3RepetitionCollapseOutcome,
    ) -> Result<Self, PromotionGateError> {
        let mut artifact = Self {
            schema: S3_REPETITION_COLLAPSE_PROMOTION_SCHEMA.to_owned(),
            checkpoint_self_hash,
            tinystories_manifest_self_hash,
            outcome,
            repetition_self_hash: Hash256::ZERO,
        };
        artifact.repetition_self_hash = artifact.compute_self_hash()?;
        Ok(artifact)
    }

    /// Compute the repetition-collapse check self-hash.
    pub fn compute_self_hash(&self) -> Result<Hash256, PromotionGateError> {
        compute_self_hash(self, "repetition_self_hash", REPETITION_DOMAIN)
    }
}

/// Promotion-gate input bundle.
#[derive(Debug, Clone, PartialEq)]
pub struct PromotionGateInputs {
    /// Self-hash of the TinyStories manifest expected across S3 artifacts.
    pub tinystories_manifest_self_hash: Hash256,
    /// S3 checkpoint artifact.
    pub c_ts: PromotionGateBoundArtifact<S3CheckpointPromotionArtifact>,
    /// S3 v0_success artifact.
    pub c_ts_v0success: Option<PromotionGateBoundArtifact<S3V0SuccessPromotionArtifact>>,
    /// S3 oracle-agreement artifact.
    pub c_ts_oracle_agreement:
        Option<PromotionGateBoundArtifact<S3OracleAgreementPromotionArtifact>>,
    /// Gutenberg manifest artifact.
    pub gb_manifest: PromotionGateBoundArtifact<GutenbergManifest>,
    /// S4 cross-corpus contamination report.
    pub contamination_report: PromotionGateBoundArtifact<S4ContaminationPromotionArtifact>,
    /// S4 Gutenberg KN-5 baseline artifact.
    pub baseline_gutenberg:
        Option<PromotionGateBoundArtifact<S4BaselineGutenbergPromotionArtifact>>,
    /// S3 repetition-collapse check artifact.
    pub repetition_collapse_check:
        PromotionGateBoundArtifact<S3RepetitionCollapsePromotionArtifact>,
}

/// Artifact references bound into `s4_promotion_gate.v1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromotionGateInputBindings {
    /// S3 checkpoint artifact reference.
    #[serde(rename = "c_TS")]
    pub c_ts: PromotionGateArtifactRef,
    /// S3 v0_success artifact reference.
    #[serde(rename = "c_TS_v0success")]
    pub c_ts_v0success: Option<PromotionGateArtifactRef>,
    /// S3 oracle-agreement artifact reference.
    #[serde(rename = "c_TS_oracle_agreement")]
    pub c_ts_oracle_agreement: Option<PromotionGateArtifactRef>,
    /// Gutenberg manifest artifact reference.
    pub gb_manifest: PromotionGateArtifactRef,
    /// S4 contamination report reference.
    pub contamination_report: PromotionGateArtifactRef,
    /// S4 Gutenberg baseline reference.
    pub baseline_gutenberg: Option<PromotionGateArtifactRef>,
    /// S3 repetition-collapse check reference.
    pub repetition_collapse_check: PromotionGateArtifactRef,
}

impl PromotionGateInputBindings {
    fn from_inputs(inputs: &PromotionGateInputs) -> Self {
        Self {
            c_ts: inputs.c_ts.artifact_ref.clone(),
            c_ts_v0success: inputs
                .c_ts_v0success
                .as_ref()
                .map(|artifact| artifact.artifact_ref.clone()),
            c_ts_oracle_agreement: inputs
                .c_ts_oracle_agreement
                .as_ref()
                .map(|artifact| artifact.artifact_ref.clone()),
            gb_manifest: inputs.gb_manifest.artifact_ref.clone(),
            contamination_report: inputs.contamination_report.artifact_ref.clone(),
            baseline_gutenberg: inputs
                .baseline_gutenberg
                .as_ref()
                .map(|artifact| artifact.artifact_ref.clone()),
            repetition_collapse_check: inputs.repetition_collapse_check.artifact_ref.clone(),
        }
    }

    fn refs(&self) -> impl Iterator<Item = &PromotionGateArtifactRef> {
        [
            Some(&self.c_ts),
            self.c_ts_v0success.as_ref(),
            self.c_ts_oracle_agreement.as_ref(),
            Some(&self.gb_manifest),
            Some(&self.contamination_report),
            self.baseline_gutenberg.as_ref(),
            Some(&self.repetition_collapse_check),
        ]
        .into_iter()
        .flatten()
    }
}

/// Outcome recorded in `s4_promotion_gate.v1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "PascalCase", deny_unknown_fields)]
pub enum PromotionGateOutcome {
    /// All promotion predicates held.
    Promoted {
        /// Promoted TinyStories checkpoint self-hash.
        #[serde(rename = "c_TS_checkpoint_sha")]
        c_ts_checkpoint_sha: Hash256,
        /// Promoted Gutenberg manifest self-hash.
        gutenberg_manifest_sha: Hash256,
    },
    /// At least one promotion predicate failed.
    Rejected {
        /// Complete list of failed evaluable predicates in canonical order.
        reasons: Vec<PromotionGateRejectionReason>,
    },
}

/// D8 promotion-gate rejection reasons in canonical emission order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum PromotionGateRejectionReason {
    /// P-1: the v0_success artifact was absent.
    #[serde(rename = "P1_v0success_missing")]
    P1V0SuccessMissing,
    /// P-1: the checkpoint self-hash did not round-trip.
    #[serde(rename = "P1_checkpoint_self_hash_invalid")]
    P1CheckpointSelfHashInvalid,
    /// P-1: the v0_success self-hash did not round-trip.
    #[serde(rename = "P1_v0success_self_hash_invalid")]
    P1V0SuccessSelfHashInvalid,
    /// P-1: v0_success was bound to a different checkpoint.
    #[serde(rename = "P1_v0success_checkpoint_mismatch")]
    P1V0SuccessCheckpointMismatch,
    /// P-1: v0_success was bound to a different TinyStories manifest.
    #[serde(rename = "P1_v0success_manifest_mismatch")]
    P1V0SuccessManifestMismatch,
    /// P-1: v0_success was present but did not pass every acceptance bit.
    #[serde(rename = "P1_v0success_not_passing")]
    P1V0SuccessNotPassing,
    /// P-1: the checkpoint cannot resume Phase D with QAT shadow weights.
    #[serde(
        rename = "P1_checkpoint_not_phase_d_resumable",
        alias = "P1_checkpoint_not_phase_d_resumeable"
    )]
    P1CheckpointNotPhaseDResumable,
    /// P-2: the oracle-agreement artifact was absent.
    #[serde(rename = "P2_oracle_missing")]
    P2OracleMissing,
    /// P-2: the oracle-agreement self-hash did not round-trip.
    #[serde(rename = "P2_oracle_self_hash_invalid")]
    P2OracleSelfHashInvalid,
    /// P-2: oracle agreement was bound to a different checkpoint.
    #[serde(rename = "P2_oracle_checkpoint_mismatch")]
    P2OracleCheckpointMismatch,
    /// P-2: oracle agreement was bound to a different TinyStories manifest.
    #[serde(rename = "P2_oracle_manifest_mismatch")]
    P2OracleManifestMismatch,
    /// P-2: oracle agreement was not Agree or its gaps exceeded tolerance.
    #[serde(rename = "P2_oracle_disagreement")]
    P2OracleDisagreement,
    /// P-3: TinyStories ternary gap was too large or non-finite.
    #[serde(rename = "P3_ternary_gap_too_large")]
    P3TernaryGapTooLarge,
    /// P-4: Gutenberg manifest validation failed.
    #[serde(rename = "P4_gutenberg_manifest_invalid")]
    P4GutenbergManifestInvalid,
    /// P-5: contamination hard-failed.
    #[serde(rename = "P5_contamination_dirty")]
    P5ContaminationDirty,
    /// P-5: contamination self-hash did not round-trip.
    #[serde(rename = "P5_contamination_self_hash_invalid")]
    P5ContaminationSelfHashInvalid,
    /// P-5: contamination was bound to different manifests.
    #[serde(rename = "P5_contamination_manifest_mismatch")]
    P5ContaminationManifestMismatch,
    /// P-6: Gutenberg unmappable rate exceeded D5.
    #[serde(rename = "P6_unmappable_rate_too_high")]
    P6UnmappableRateTooHigh,
    /// P-7: the Gutenberg baseline artifact was absent.
    #[serde(rename = "P7_baseline_missing")]
    P7BaselineMissing,
    /// P-7: the Gutenberg baseline self-hash did not round-trip.
    #[serde(rename = "P7_baseline_self_hash_invalid")]
    P7BaselineSelfHashInvalid,
    /// P-7: the baseline was bound to different corpus artifacts.
    #[serde(rename = "P7_baseline_manifest_mismatch")]
    P7BaselineManifestMismatch,
    /// P-7: baseline bpc was non-finite.
    #[serde(rename = "P7_baseline_nonfinite")]
    P7BaselineNonfinite,
    /// P-8: repetition collapse was observed.
    #[serde(rename = "P8_repetition_collapse")]
    P8RepetitionCollapse,
    /// P-8: repetition-collapse self-hash did not round-trip.
    #[serde(rename = "P8_repetition_self_hash_invalid")]
    P8RepetitionSelfHashInvalid,
    /// P-8: repetition check was bound to a different checkpoint.
    #[serde(rename = "P8_repetition_checkpoint_mismatch")]
    P8RepetitionCheckpointMismatch,
    /// P-8: repetition check was bound to a different TinyStories manifest.
    #[serde(rename = "P8_repetition_manifest_mismatch")]
    P8RepetitionManifestMismatch,
    /// P-9: one or more artifact paths used `latest` or omitted an explicit path.
    #[serde(rename = "P9_non_explicit_artifact_selector")]
    P9NonExplicitArtifactSelector,
}

/// `s4_promotion_gate.v1` product.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromotionGateProduct {
    /// Schema id, always `s4_promotion_gate.v1`.
    pub schema: String,
    /// Explicit artifact references consumed by the gate.
    pub input_artifacts: PromotionGateInputBindings,
    /// Self-hash of the TinyStories manifest.
    pub tinystories_manifest_self_hash: Hash256,
    /// Self-hash of the Gutenberg manifest.
    pub gutenberg_manifest_self_hash: Hash256,
    /// Self-hash of the TinyStories checkpoint.
    #[serde(rename = "c_TS_checkpoint_self_hash")]
    pub c_ts_checkpoint_self_hash: Hash256,
    /// Self-hash of the S3 v0_success artifact, if supplied.
    #[serde(rename = "c_TS_v0success_self_hash")]
    pub c_ts_v0success_self_hash: Option<Hash256>,
    /// Self-hash of the S3 oracle-agreement artifact, if supplied.
    #[serde(rename = "c_TS_oracle_agreement_self_hash")]
    pub c_ts_oracle_agreement_self_hash: Option<Hash256>,
    /// Self-hash of the S4 contamination report.
    pub contamination_self_hash: Hash256,
    /// Self-hash of the S4 Gutenberg baseline, if supplied.
    pub baseline_gutenberg_self_hash: Option<Hash256>,
    /// Self-hash of the repetition-collapse check.
    pub repetition_collapse_check_self_hash: Hash256,
    /// Optional self-hash of the sibling S4 corpus progression artifact.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corpus_progression_self_hash: Option<Hash256>,
    /// Promotion gate outcome.
    pub outcome: PromotionGateOutcome,
    /// Self-hash over canonical JSON with this field omitted.
    pub promotion_gate_self_hash: Hash256,
}

impl PromotionGateProduct {
    /// Evaluate D8 and return a self-hashed promotion-gate product.
    pub fn evaluate(inputs: PromotionGateInputs) -> Result<Self, PromotionGateError> {
        emit_promotion_gate_started(&inputs);
        let input_artifacts = PromotionGateInputBindings::from_inputs(&inputs);
        let reasons = evaluate_rejection_reasons(&inputs, &input_artifacts);
        let rejection_count = reasons.len();
        emit_promotion_gate_checks(&reasons);
        let outcome = if reasons.is_empty() {
            PromotionGateOutcome::Promoted {
                c_ts_checkpoint_sha: inputs.c_ts.artifact_ref.artifact_self_hash,
                gutenberg_manifest_sha: inputs.gb_manifest.artifact_ref.artifact_self_hash,
            }
        } else {
            PromotionGateOutcome::Rejected { reasons }
        };
        let mut product = Self {
            schema: S4_PROMOTION_GATE_SCHEMA.to_owned(),
            input_artifacts,
            tinystories_manifest_self_hash: inputs.tinystories_manifest_self_hash,
            gutenberg_manifest_self_hash: inputs.gb_manifest.artifact_ref.artifact_self_hash,
            c_ts_checkpoint_self_hash: inputs.c_ts.artifact_ref.artifact_self_hash,
            c_ts_v0success_self_hash: inputs
                .c_ts_v0success
                .as_ref()
                .map(|artifact| artifact.artifact_ref.artifact_self_hash),
            c_ts_oracle_agreement_self_hash: inputs
                .c_ts_oracle_agreement
                .as_ref()
                .map(|artifact| artifact.artifact_ref.artifact_self_hash),
            contamination_self_hash: inputs.contamination_report.artifact_ref.artifact_self_hash,
            baseline_gutenberg_self_hash: inputs
                .baseline_gutenberg
                .as_ref()
                .map(|artifact| artifact.artifact_ref.artifact_self_hash),
            repetition_collapse_check_self_hash: inputs
                .repetition_collapse_check
                .artifact_ref
                .artifact_self_hash,
            corpus_progression_self_hash: None,
            outcome,
            promotion_gate_self_hash: Hash256::ZERO,
        };
        product.promotion_gate_self_hash = product.compute_self_hash()?;
        emit_promotion_gate_outcome(&product, rejection_count);
        Ok(product)
    }

    /// Compute the promotion-gate self-hash.
    pub fn compute_self_hash(&self) -> Result<Hash256, PromotionGateError> {
        compute_self_hash(self, "promotion_gate_self_hash", PROMOTION_GATE_DOMAIN)
    }

    /// Validate structure and self-hash.
    pub fn validate_canonical_write(&self) -> Result<(), PromotionGateError> {
        if self.schema != S4_PROMOTION_GATE_SCHEMA {
            return Err(PromotionGateError::InvalidSchema {
                observed: self.schema.clone(),
            });
        }
        let recomputed = self.compute_self_hash()?;
        if recomputed != self.promotion_gate_self_hash {
            return Err(PromotionGateError::SelfHashMismatch {
                expected: recomputed,
                observed: self.promotion_gate_self_hash,
            });
        }
        Ok(())
    }

    /// Canonical JSON bytes including `promotion_gate_self_hash`.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, PromotionGateError> {
        self.validate_canonical_write()?;
        CanonicalJson::to_vec(self).map_err(PromotionGateError::CanonicalJson)
    }

    /// Bind this product to its sibling `s4_corpus_progression.v1` artifact.
    pub fn with_corpus_progression_self_hash(
        mut self,
        corpus_progression_self_hash: Hash256,
    ) -> Result<Self, PromotionGateError> {
        if corpus_progression_self_hash == Hash256::ZERO {
            return Err(PromotionGateError::InvalidCorpusProgressionSelfHash);
        }
        self.corpus_progression_self_hash = Some(corpus_progression_self_hash);
        self.promotion_gate_self_hash = Hash256::ZERO;
        self.promotion_gate_self_hash = self.compute_self_hash()?;
        Ok(self)
    }
}

/// Evaluate D8 and return a self-hashed promotion-gate product.
pub fn promotion_gate(
    inputs: PromotionGateInputs,
) -> Result<PromotionGateProduct, PromotionGateError> {
    PromotionGateProduct::evaluate(inputs)
}

/// Errors from promotion-gate product construction.
#[derive(Debug)]
pub enum PromotionGateError {
    /// Product schema did not match `s4_promotion_gate.v1`.
    InvalidSchema {
        /// Observed schema id.
        observed: String,
    },
    /// Stored product self-hash differed from recomputation.
    SelfHashMismatch {
        /// Expected recomputed self-hash.
        expected: Hash256,
        /// Observed stored self-hash.
        observed: Hash256,
    },
    /// JSON serialization failed.
    Json(serde_json::Error),
    /// Canonical JSON serialization failed.
    CanonicalJson(gbf_foundation::CanonicalJsonError),
    /// Self-hash computation expected a top-level object.
    ExpectedObjectForSelfHash,
    /// Corpus progression binding attempted to use the zero sentinel hash.
    InvalidCorpusProgressionSelfHash,
}

impl fmt::Display for PromotionGateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSchema { observed } => {
                write!(f, "expected s4_promotion_gate.v1 schema, got {observed:?}")
            }
            Self::SelfHashMismatch { expected, observed } => write!(
                f,
                "s4_promotion_gate.v1 self-hash mismatch: expected {expected}, observed {observed}"
            ),
            Self::Json(error) => write!(f, "{error}"),
            Self::CanonicalJson(error) => write!(f, "{error}"),
            Self::ExpectedObjectForSelfHash => {
                f.write_str("promotion-gate self-hash requires a top-level object")
            }
            Self::InvalidCorpusProgressionSelfHash => {
                f.write_str("corpus_progression_self_hash must not be zero")
            }
        }
    }
}

impl Error for PromotionGateError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Json(error) => Some(error),
            Self::CanonicalJson(error) => Some(error),
            _ => None,
        }
    }
}

impl From<serde_json::Error> for PromotionGateError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

impl From<gbf_foundation::CanonicalJsonError> for PromotionGateError {
    fn from(error: gbf_foundation::CanonicalJsonError) -> Self {
        Self::CanonicalJson(error)
    }
}

fn evaluate_rejection_reasons(
    inputs: &PromotionGateInputs,
    input_artifacts: &PromotionGateInputBindings,
) -> Vec<PromotionGateRejectionReason> {
    let mut reasons = Vec::new();
    let checkpoint_self_hash_valid = self_hash_valid(
        inputs.c_ts.artifact.checkpoint_self_hash,
        inputs.c_ts.artifact.compute_self_hash(),
        inputs.c_ts.artifact_ref.artifact_self_hash,
    ) && inputs.c_ts.artifact.schema
        == S3_CHECKPOINT_PROMOTION_SCHEMA;
    if !checkpoint_self_hash_valid {
        push_reason(
            &mut reasons,
            PromotionGateRejectionReason::P1CheckpointSelfHashInvalid,
        );
    }
    if checkpoint_self_hash_valid && !inputs.c_ts.artifact.phase_d_resumable() {
        push_reason(
            &mut reasons,
            PromotionGateRejectionReason::P1CheckpointNotPhaseDResumable,
        );
    }

    let v0_success_valid = evaluate_p1_v0_success(inputs, &mut reasons);
    evaluate_p2_oracle(inputs, &mut reasons);
    evaluate_p3_ternary_gap(inputs, v0_success_valid, &mut reasons);
    let manifest_status = evaluate_p4_manifest(inputs, &mut reasons);
    evaluate_p5_contamination(inputs, &mut reasons);
    evaluate_p6_unmappable(inputs, manifest_status.hash_valid, &mut reasons);
    evaluate_p7_baseline(inputs, manifest_status.p4_valid, &mut reasons);
    evaluate_p8_repetition(inputs, &mut reasons);
    if input_artifacts
        .refs()
        .any(PromotionGateArtifactRef::uses_latest_selector)
    {
        push_reason(
            &mut reasons,
            PromotionGateRejectionReason::P9NonExplicitArtifactSelector,
        );
    }
    reasons.sort_unstable();
    reasons
}

fn evaluate_p1_v0_success(
    inputs: &PromotionGateInputs,
    reasons: &mut Vec<PromotionGateRejectionReason>,
) -> bool {
    let Some(v0_success) = &inputs.c_ts_v0success else {
        push_reason(reasons, PromotionGateRejectionReason::P1V0SuccessMissing);
        return false;
    };
    let artifact = &v0_success.artifact;
    let v0_success_valid = self_hash_valid(
        artifact.v0_success_self_hash,
        artifact.compute_self_hash(),
        v0_success.artifact_ref.artifact_self_hash,
    ) && artifact.schema == S3_V0_SUCCESS_PROMOTION_SCHEMA;
    if !v0_success_valid {
        push_reason(
            reasons,
            PromotionGateRejectionReason::P1V0SuccessSelfHashInvalid,
        );
        return false;
    }
    if artifact.checkpoint_self_hash != inputs.c_ts.artifact_ref.artifact_self_hash {
        push_reason(
            reasons,
            PromotionGateRejectionReason::P1V0SuccessCheckpointMismatch,
        );
    }
    if artifact.tinystories_manifest_self_hash != inputs.tinystories_manifest_self_hash {
        push_reason(
            reasons,
            PromotionGateRejectionReason::P1V0SuccessManifestMismatch,
        );
    }
    if !artifact.acceptance_bits.all_set() || artifact.outcome != V0SuccessGateOutcome::Pass {
        push_reason(reasons, PromotionGateRejectionReason::P1V0SuccessNotPassing);
    }
    true
}

fn evaluate_p2_oracle(
    inputs: &PromotionGateInputs,
    reasons: &mut Vec<PromotionGateRejectionReason>,
) {
    let Some(oracle) = &inputs.c_ts_oracle_agreement else {
        push_reason(reasons, PromotionGateRejectionReason::P2OracleMissing);
        return;
    };
    let artifact = &oracle.artifact;
    let oracle_valid = self_hash_valid(
        artifact.oracle_agreement_self_hash,
        artifact.compute_self_hash(),
        oracle.artifact_ref.artifact_self_hash,
    ) && artifact.schema == S3_ORACLE_AGREEMENT_PROMOTION_SCHEMA;
    if !oracle_valid {
        push_reason(
            reasons,
            PromotionGateRejectionReason::P2OracleSelfHashInvalid,
        );
        return;
    }
    if artifact.checkpoint_self_hash != inputs.c_ts.artifact_ref.artifact_self_hash {
        push_reason(
            reasons,
            PromotionGateRejectionReason::P2OracleCheckpointMismatch,
        );
    }
    if artifact.tinystories_manifest_self_hash != inputs.tinystories_manifest_self_hash {
        push_reason(
            reasons,
            PromotionGateRejectionReason::P2OracleManifestMismatch,
        );
    }
    if artifact.outcome != S3OracleAgreementOutcome::Agree
        || !artifact.per_token_bpc_gaps_within_tolerance
    {
        push_reason(reasons, PromotionGateRejectionReason::P2OracleDisagreement);
    }
}

fn evaluate_p3_ternary_gap(
    inputs: &PromotionGateInputs,
    v0_success_valid: bool,
    reasons: &mut Vec<PromotionGateRejectionReason>,
) {
    if !v0_success_valid {
        return;
    }
    let artifact = &inputs
        .c_ts_v0success
        .as_ref()
        .expect("valid v0_success implies present")
        .artifact;
    if !artifact.ternary_gap_ts.is_finite()
        || artifact.ternary_gap_ts > S4_PROMOTION_MAX_TERNARY_GAP_TS
    {
        push_reason(reasons, PromotionGateRejectionReason::P3TernaryGapTooLarge);
    }
}

#[derive(Debug, Clone, Copy)]
struct ManifestStatus {
    hash_valid: bool,
    p4_valid: bool,
}

fn evaluate_p4_manifest(
    inputs: &PromotionGateInputs,
    reasons: &mut Vec<PromotionGateRejectionReason>,
) -> ManifestStatus {
    let manifest = &inputs.gb_manifest.artifact;
    let hash_valid = self_hash_valid(
        manifest.manifest_self_hash,
        manifest.compute_self_hash().map_err(|error| match error {
            GutenbergManifestError::CanonicalJson(error) => {
                PromotionGateError::CanonicalJson(error)
            }
            GutenbergManifestError::Json(error) => PromotionGateError::Json(error),
            _ => PromotionGateError::ExpectedObjectForSelfHash,
        }),
        inputs.gb_manifest.artifact_ref.artifact_self_hash,
    );
    if !hash_valid {
        push_reason(
            reasons,
            PromotionGateRejectionReason::P4GutenbergManifestInvalid,
        );
        return ManifestStatus {
            hash_valid: false,
            p4_valid: false,
        };
    }
    match manifest.validate_canonical_write() {
        Ok(()) => ManifestStatus {
            hash_valid: true,
            p4_valid: true,
        },
        Err(GutenbergManifestError::InvalidUnmappableRateCorpus { .. }) => ManifestStatus {
            hash_valid: true,
            p4_valid: true,
        },
        Err(_) => {
            push_reason(
                reasons,
                PromotionGateRejectionReason::P4GutenbergManifestInvalid,
            );
            ManifestStatus {
                hash_valid: true,
                p4_valid: false,
            }
        }
    }
}

fn evaluate_p5_contamination(
    inputs: &PromotionGateInputs,
    reasons: &mut Vec<PromotionGateRejectionReason>,
) {
    let contamination = &inputs.contamination_report;
    let artifact = &contamination.artifact;
    let contamination_valid = self_hash_valid(
        artifact.contamination_self_hash,
        artifact.compute_self_hash(),
        contamination.artifact_ref.artifact_self_hash,
    ) && artifact.schema == S4_CONTAMINATION_PROMOTION_SCHEMA;
    if !contamination_valid {
        push_reason(
            reasons,
            PromotionGateRejectionReason::P5ContaminationSelfHashInvalid,
        );
        return;
    }
    if artifact.tinystories_manifest_self_hash != inputs.tinystories_manifest_self_hash
        || artifact.gutenberg_manifest_self_hash
            != inputs.gb_manifest.artifact_ref.artifact_self_hash
    {
        push_reason(
            reasons,
            PromotionGateRejectionReason::P5ContaminationManifestMismatch,
        );
    }
    if !artifact.promotes() {
        push_reason(reasons, PromotionGateRejectionReason::P5ContaminationDirty);
    }
}

fn evaluate_p6_unmappable(
    inputs: &PromotionGateInputs,
    manifest_hash_valid: bool,
    reasons: &mut Vec<PromotionGateRejectionReason>,
) {
    if !manifest_hash_valid {
        return;
    }
    let unmappable_rate = inputs.gb_manifest.artifact.unmappable_rate_corpus;
    if !unmappable_rate.is_finite()
        || !(0.0..=S4_PROMOTION_MAX_GUTENBERG_UNMAPPABLE_RATE).contains(&unmappable_rate)
    {
        push_reason(
            reasons,
            PromotionGateRejectionReason::P6UnmappableRateTooHigh,
        );
    }
}

fn evaluate_p7_baseline(
    inputs: &PromotionGateInputs,
    manifest_p4_valid: bool,
    reasons: &mut Vec<PromotionGateRejectionReason>,
) {
    let Some(baseline) = &inputs.baseline_gutenberg else {
        push_reason(reasons, PromotionGateRejectionReason::P7BaselineMissing);
        return;
    };
    let artifact = &baseline.artifact;
    let baseline_valid = self_hash_valid(
        artifact.baseline_self_hash,
        artifact.compute_self_hash(),
        baseline.artifact_ref.artifact_self_hash,
    ) && artifact.schema == S4_BASELINE_GUTENBERG_PROMOTION_SCHEMA;
    if !baseline_valid {
        push_reason(
            reasons,
            PromotionGateRejectionReason::P7BaselineSelfHashInvalid,
        );
        return;
    }
    let bpc_finite = artifact.bpc_kn5.is_finite();
    if !bpc_finite {
        push_reason(reasons, PromotionGateRejectionReason::P7BaselineNonfinite);
    }
    let corpus_mismatch = manifest_p4_valid
        && (artifact.corpus_train_sha != inputs.gb_manifest.artifact.train_sha256
            || artifact.corpus_val_sha != inputs.gb_manifest.artifact.val_sha256);
    if artifact.gutenberg_manifest_self_hash != inputs.gb_manifest.artifact_ref.artifact_self_hash
        || corpus_mismatch
    {
        push_reason(
            reasons,
            PromotionGateRejectionReason::P7BaselineManifestMismatch,
        );
    }
}

fn evaluate_p8_repetition(
    inputs: &PromotionGateInputs,
    reasons: &mut Vec<PromotionGateRejectionReason>,
) {
    let repetition = &inputs.repetition_collapse_check;
    let artifact = &repetition.artifact;
    let repetition_valid = self_hash_valid(
        artifact.repetition_self_hash,
        artifact.compute_self_hash(),
        repetition.artifact_ref.artifact_self_hash,
    ) && artifact.schema == S3_REPETITION_COLLAPSE_PROMOTION_SCHEMA;
    if !repetition_valid {
        push_reason(
            reasons,
            PromotionGateRejectionReason::P8RepetitionSelfHashInvalid,
        );
        return;
    }
    if artifact.checkpoint_self_hash != inputs.c_ts.artifact_ref.artifact_self_hash {
        push_reason(
            reasons,
            PromotionGateRejectionReason::P8RepetitionCheckpointMismatch,
        );
    }
    if artifact.tinystories_manifest_self_hash != inputs.tinystories_manifest_self_hash {
        push_reason(
            reasons,
            PromotionGateRejectionReason::P8RepetitionManifestMismatch,
        );
    }
    if artifact.outcome != S3RepetitionCollapseOutcome::Pass {
        push_reason(reasons, PromotionGateRejectionReason::P8RepetitionCollapse);
    }
}

fn push_reason(
    reasons: &mut Vec<PromotionGateRejectionReason>,
    reason: PromotionGateRejectionReason,
) {
    if !reasons.contains(&reason) {
        reasons.push(reason);
    }
}

fn self_hash_valid(
    stored: Hash256,
    computed: Result<Hash256, PromotionGateError>,
    expected: Hash256,
) -> bool {
    computed.is_ok_and(|hash| hash == stored && stored == expected)
}

fn emit_promotion_gate_started(inputs: &PromotionGateInputs) {
    tracing::info!(
        target: S4_PROMOTION_GATE_LOG_TARGET,
        event_name = S4_PROMOTION_GATE_STARTED_EVENT,
        schema = S4_PROMOTION_GATE_SCHEMA,
        tinystories_manifest_self_hash = %inputs.tinystories_manifest_self_hash,
        c_TS_checkpoint_self_hash = %inputs.c_ts.artifact_ref.artifact_self_hash,
        gutenberg_manifest_self_hash = %inputs.gb_manifest.artifact_ref.artifact_self_hash,
        contamination_self_hash = %inputs.contamination_report.artifact_ref.artifact_self_hash,
        has_v0success = inputs.c_ts_v0success.is_some(),
        has_oracle_agreement = inputs.c_ts_oracle_agreement.is_some(),
        has_baseline_gutenberg = inputs.baseline_gutenberg.is_some(),
        "s4 promotion gate started"
    );
}

fn emit_promotion_gate_checks(reasons: &[PromotionGateRejectionReason]) {
    for predicate in PROMOTION_PREDICATES {
        let reason_count = reasons
            .iter()
            .filter(|reason| reason.predicate_label() == *predicate)
            .count() as u64;
        tracing::info!(
            target: S4_PROMOTION_GATE_LOG_TARGET,
            event_name = S4_PROMOTION_GATE_CHECK_EVENT,
            schema = S4_PROMOTION_GATE_SCHEMA,
            predicate,
            passed = reason_count == 0,
            reason_count,
            "s4 promotion gate predicate checked"
        );
    }
}

fn emit_promotion_gate_outcome(product: &PromotionGateProduct, reason_count: usize) {
    tracing::info!(
        target: S4_PROMOTION_GATE_LOG_TARGET,
        event_name = S4_PROMOTION_GATE_OUTCOME_EVENT,
        schema = S4_PROMOTION_GATE_SCHEMA,
        outcome = product.outcome.kind(),
        promoted = product.outcome.promoted(),
        reason_count = reason_count as u64,
        promotion_gate_self_hash = %product.promotion_gate_self_hash,
        c_TS_checkpoint_self_hash = %product.c_ts_checkpoint_self_hash,
        gutenberg_manifest_self_hash = %product.gutenberg_manifest_self_hash,
        contamination_self_hash = %product.contamination_self_hash,
        "s4 promotion gate outcome"
    );
}

impl PromotionGateOutcome {
    fn kind(&self) -> &'static str {
        match self {
            Self::Promoted { .. } => "Promoted",
            Self::Rejected { .. } => "Rejected",
        }
    }

    fn promoted(&self) -> bool {
        matches!(self, Self::Promoted { .. })
    }
}

impl PromotionGateRejectionReason {
    fn predicate_label(self) -> &'static str {
        match self {
            Self::P1V0SuccessMissing
            | Self::P1CheckpointSelfHashInvalid
            | Self::P1V0SuccessSelfHashInvalid
            | Self::P1V0SuccessCheckpointMismatch
            | Self::P1V0SuccessManifestMismatch
            | Self::P1V0SuccessNotPassing
            | Self::P1CheckpointNotPhaseDResumable => "P-1",
            Self::P2OracleMissing
            | Self::P2OracleSelfHashInvalid
            | Self::P2OracleCheckpointMismatch
            | Self::P2OracleManifestMismatch
            | Self::P2OracleDisagreement => "P-2",
            Self::P3TernaryGapTooLarge => "P-3",
            Self::P4GutenbergManifestInvalid => "P-4",
            Self::P5ContaminationDirty
            | Self::P5ContaminationSelfHashInvalid
            | Self::P5ContaminationManifestMismatch => "P-5",
            Self::P6UnmappableRateTooHigh => "P-6",
            Self::P7BaselineMissing
            | Self::P7BaselineSelfHashInvalid
            | Self::P7BaselineManifestMismatch
            | Self::P7BaselineNonfinite => "P-7",
            Self::P8RepetitionCollapse
            | Self::P8RepetitionSelfHashInvalid
            | Self::P8RepetitionCheckpointMismatch
            | Self::P8RepetitionManifestMismatch => "P-8",
            Self::P9NonExplicitArtifactSelector => "P-9",
        }
    }
}

fn compute_self_hash<T: Serialize>(
    payload: &T,
    self_hash_field: &'static str,
    domain: DomainHash<'static>,
) -> Result<Hash256, PromotionGateError> {
    let mut value = serde_json::to_value(payload).map_err(PromotionGateError::Json)?;
    value
        .as_object_mut()
        .ok_or(PromotionGateError::ExpectedObjectForSelfHash)?
        .remove(self_hash_field);
    let canonical =
        CanonicalJson::value_to_vec(&value).map_err(PromotionGateError::CanonicalJson)?;
    domain
        .hash_canonical_bytes(&canonical)
        .map_err(PromotionGateError::CanonicalJson)
}
