//! Canonical S1 schema primitives.

use std::fmt;

use gbf_foundation::{Hash256, SemVer, sha256};
use serde::ser::{
    self, SerializeMap, SerializeSeq, SerializeStruct, SerializeStructVariant, SerializeTuple,
    SerializeTupleStruct, SerializeTupleVariant,
};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::S1_LOG_TARGET;

const CRATE_NAME: &str = "gbf-experiments";
const SCHEMA_VERSION: &str = "1";

/// Build identity recorded on S1 checkpoint-like artifacts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum S1BuildKind {
    /// Default Phase A binary, with QAT code present but configured off.
    PhaseA,
    /// Ablation binary, with QAT code paths compiled out.
    Ablation,
}

/// Completion state recorded in S1 run and report artifacts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum S1Completion {
    /// The run completed the requested optimizer steps.
    Completed,
    /// The run observed its first non-finite loss or gradient at `step`.
    DivergedAt {
        /// First diverged training step.
        step: u64,
    },
    /// A downstream artifact was not produced because an earlier gate stopped.
    NotReached,
}

/// Summary of final gradient norms for `s1_run_log.v1`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GradNormSummary {
    /// Final global L2 norm over all trainable gradients.
    #[serde(deserialize_with = "finite_nonnegative_f32")]
    pub global_l2: f32,
    /// Largest per-tensor L2 norm observed in the final step.
    #[serde(deserialize_with = "finite_nonnegative_f32")]
    pub max_l2: f32,
    /// Mean per-tensor L2 norm observed in the final step.
    #[serde(deserialize_with = "finite_nonnegative_f32")]
    pub mean_l2: f32,
}

/// First tensor-byte mismatch recorded by `s1_ablation.v1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TensorMismatch {
    /// Tensor name that first differed.
    pub tensor: String,
    /// Byte offset within the tensor payload.
    pub byte_offset: u64,
}

/// Pinned interpolation smoothing settings for S1 n-gram baselines.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SmoothingScheme {
    /// Add-alpha smoothing parameter.
    #[serde(deserialize_with = "finite_nonnegative_f64")]
    pub alpha: f64,
    /// Interpolation lambdas in D4 order: trigram, bigram, then unigram.
    #[serde(deserialize_with = "finite_nonnegative_f64_array_3")]
    pub lambdas: [f64; 3],
}

/// Count cardinalities emitted with `s1_baseline.v1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CountsSummary {
    /// Training bytes used to fit the baseline.
    pub train_bytes: u64,
    /// Number of distinct unigram entries.
    pub distinct_unigrams: u64,
    /// Number of distinct bigram entries.
    pub distinct_bigrams: u64,
    /// Number of distinct trigram entries.
    pub distinct_trigrams: u64,
}

/// `s1_oracle.v1` D7 measurement-oracle artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OracleReport {
    /// Schema id. Expected value: `s1_oracle.v1`.
    #[serde(deserialize_with = "schema_s1_oracle")]
    pub schema: String,
    /// O-metric-0 rejection-sampler adversarial result.
    pub o_metric_0: bool,
    /// O-metric-1 uniform-logits scorer result.
    pub o_metric_1: bool,
    /// O-metric-2 hand-counted n-gram fixture result.
    pub o_metric_2: bool,
    /// O-metric-3 reset-boundary spy result.
    pub o_metric_3: bool,
    /// O-metric-4 shuffle permutation and pin result.
    pub o_metric_4: bool,
    /// Aggregate H5 metric-oracle verdict.
    pub metric_oracle_passed: bool,
    /// Failed oracle ids in D7 order.
    pub failed_oracle_ids: Vec<String>,
    /// Self-hash over the oracle report with this field omitted.
    pub oracle_self_hash: Hash256,
}

impl OracleReport {
    /// Construct an oracle report after recomputing aggregate fields from the
    /// per-oracle booleans.
    pub fn from_oracle_bools(
        o_metric_0: bool,
        o_metric_1: bool,
        o_metric_2: bool,
        o_metric_3: bool,
        o_metric_4: bool,
    ) -> Result<Self, S1SchemaError> {
        let per_oracle = [
            ("O-metric-0", o_metric_0),
            ("O-metric-1", o_metric_1),
            ("O-metric-2", o_metric_2),
            ("O-metric-3", o_metric_3),
            ("O-metric-4", o_metric_4),
        ];
        let failed_oracle_ids = per_oracle
            .iter()
            .filter_map(|(id, passed)| (!*passed).then_some((*id).to_owned()))
            .collect::<Vec<_>>();
        Self {
            schema: "s1_oracle.v1".to_owned(),
            o_metric_0,
            o_metric_1,
            o_metric_2,
            o_metric_3,
            o_metric_4,
            metric_oracle_passed: failed_oracle_ids.is_empty(),
            failed_oracle_ids,
            oracle_self_hash: Hash256::ZERO,
        }
        .with_computed_self_hash()
    }

    /// Validate that aggregate fields agree with per-oracle booleans.
    pub fn validate_aggregate_consistency(&self) -> Result<(), S1SchemaError> {
        let expected_failed = [
            ("O-metric-0", self.o_metric_0),
            ("O-metric-1", self.o_metric_1),
            ("O-metric-2", self.o_metric_2),
            ("O-metric-3", self.o_metric_3),
            ("O-metric-4", self.o_metric_4),
        ]
        .into_iter()
        .filter_map(|(id, passed)| (!passed).then_some(id.to_owned()))
        .collect::<Vec<_>>();
        if self.metric_oracle_passed != expected_failed.is_empty() {
            return Err(S1SchemaError::Custom(
                "s1_oracle.v1 metric_oracle_passed disagrees with per-oracle booleans".to_owned(),
            ));
        }
        if self.failed_oracle_ids != expected_failed {
            return Err(S1SchemaError::Custom(
                "s1_oracle.v1 failed_oracle_ids disagree with per-oracle booleans".to_owned(),
            ));
        }
        Ok(())
    }
}

impl<'de> Deserialize<'de> for OracleReport {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct RawOracleReport {
            #[serde(deserialize_with = "schema_s1_oracle")]
            schema: String,
            o_metric_0: bool,
            o_metric_1: bool,
            o_metric_2: bool,
            o_metric_3: bool,
            o_metric_4: bool,
            metric_oracle_passed: bool,
            failed_oracle_ids: Vec<String>,
            oracle_self_hash: Hash256,
        }

        let raw = RawOracleReport::deserialize(deserializer)?;
        let report = Self {
            schema: raw.schema,
            o_metric_0: raw.o_metric_0,
            o_metric_1: raw.o_metric_1,
            o_metric_2: raw.o_metric_2,
            o_metric_3: raw.o_metric_3,
            o_metric_4: raw.o_metric_4,
            metric_oracle_passed: raw.metric_oracle_passed,
            failed_oracle_ids: raw.failed_oracle_ids,
            oracle_self_hash: raw.oracle_self_hash,
        };
        report
            .validate_aggregate_consistency()
            .map_err(serde::de::Error::custom)?;
        Ok(report)
    }
}

/// S1 outcome tag from RFC section 8.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum S1Outcome {
    /// H1, H2, H3, H4, and H5 confirmed.
    #[serde(rename = "Pass-clean")]
    PassClean,
    /// H3 refuted while the other closure hypotheses confirmed.
    #[serde(rename = "Pass-with-warning")]
    PassWithWarning,
    /// H1 refuted, or any seed diverged.
    #[serde(rename = "Fail-substrate")]
    FailSubstrate,
    /// H2 refuted without the suspicious-low-bpc condition.
    #[serde(rename = "Fail-capacity")]
    FailCapacity,
    /// Median validation bpc is suspiciously low.
    #[serde(rename = "Fail-suspicious")]
    FailSuspicious,
    /// H4 refuted.
    #[serde(rename = "Fail-phase")]
    FailPhase,
    /// H5 refuted.
    #[serde(rename = "Fail-metric")]
    FailMetric,
}

/// S1 decision tag from RFC section 8.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum S1Decision {
    /// Proceed to S2 without warning.
    #[serde(rename = "ProceedToS2")]
    ProceedToS2,
    /// Proceed to S2 with the T12.5 prerequisite.
    #[serde(rename = "ProceedToS2-with-T12.5-prereq")]
    ProceedToS2WithT125Prereq,
    /// Investigation is required before S1 can close.
    #[serde(rename = "Investigate")]
    Investigate {
        /// Investigation reason tag.
        reason: String,
    },
    /// Halt blocks closure.
    #[serde(rename = "Halt")]
    Halt {
        /// Halt reason tag.
        reason: String,
    },
}

/// Seed-local artifact hashes recorded in `s1_report.v1` front matter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PerSeedArtifacts {
    /// Seed id.
    pub seed: u64,
    /// Completion state for the seed.
    pub completion: S1Completion,
    /// Checkpoint metadata self-hash, if produced.
    pub checkpoint_self_hash: Option<Hash256>,
    /// Run-log self-hash, if produced.
    pub run_log_self_hash: Option<Hash256>,
    /// Score self-hash, if produced.
    pub score_self_hash: Option<Hash256>,
    /// Negative-test self-hash, if produced for seed 0.
    pub negative_self_hash: Option<Hash256>,
    /// Ablation self-hash, if produced for seed 0.
    pub ablation_self_hash: Option<Hash256>,
}

/// A 40-character lowercase hexadecimal Git commit id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct GitCommitId(String);

impl GitCommitId {
    /// Create a checked Git commit id.
    pub fn new(value: impl Into<String>) -> Result<Self, S1SchemaError> {
        let value = value.into();
        if value.len() == 40
            && value
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        {
            Ok(Self(value))
        } else {
            Err(S1SchemaError::InvalidGitCommitId(value))
        }
    }

    /// Borrow the commit id string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for GitCommitId {
    type Error = S1SchemaError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<GitCommitId> for String {
    fn from(value: GitCommitId) -> Self {
        value.0
    }
}

/// RFC revision reference recorded in `s1_report.v1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RfcRevisionRef {
    /// Git commit id for the RFC revision.
    GitCommitId(GitCommitId),
    /// SHA-256 digest when the RFC is materialized as a content blob.
    Hash256(Hash256),
}

/// `s1_checkpoint.v1` checkpoint metadata sidecar.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckpointMetadata {
    /// Schema id. Expected value: `s1_checkpoint.v1`.
    #[serde(deserialize_with = "schema_s1_checkpoint")]
    pub schema: String,
    /// S1 seed.
    pub seed: u64,
    /// Training corpus hash.
    pub corpus_train_sha: Hash256,
    /// Validation corpus hash.
    pub corpus_val_sha: Hash256,
    /// Model configuration hash.
    pub model_config_hash: Hash256,
    /// Training configuration hash.
    pub train_config_hash: Hash256,
    /// Feature-selected build kind.
    pub build_kind: S1BuildKind,
    /// Build configuration hash.
    pub build_config_hash: Hash256,
    /// Dependency lockfile hash.
    pub dependency_lockfile_sha: Hash256,
    /// Rust toolchain hash.
    pub rust_toolchain_hash: Hash256,
    /// Device profile hash.
    pub device_profile_hash: Hash256,
    /// RNG stream definition hash.
    pub rng_stream_def_hash: Hash256,
    /// Pass implementation version.
    pub pass_version: SemVer,
    /// Training budget profile used to produce the checkpoint.
    pub budget_profile: String,
    /// Final optimizer step.
    pub final_step: u64,
    /// Final finite train loss in nats per byte.
    #[serde(deserialize_with = "finite_nonnegative_f32")]
    pub final_train_loss: f32,
    /// Completion state.
    pub completion: S1Completion,
    /// SHA-256 of the metadata-free SafeTensors checkpoint bytes.
    pub checkpoint_safetensors_sha256: Hash256,
    /// Self-hash over the metadata with this field omitted.
    pub checkpoint_self_hash: Hash256,
}

/// `s1_run_log.v1` run-log artifact.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunLog {
    /// Schema id. Expected value: `s1_run_log.v1`.
    #[serde(deserialize_with = "schema_s1_run_log")]
    pub schema: String,
    /// S1 seed.
    pub seed: u64,
    /// Training configuration hash.
    pub train_config_hash: Hash256,
    /// One finite loss per optimizer step.
    #[serde(deserialize_with = "finite_loss_points")]
    pub losses: Vec<(u64, f32)>,
    /// Reset-context bpc evaluation points, including step 0.
    #[serde(deserialize_with = "finite_bpc_points")]
    pub eval_points: Vec<(u64, f64)>,
    /// Final gradient summary.
    pub final_grad_norms: GradNormSummary,
    /// Self-hash over the run log with this field omitted.
    pub run_log_self_hash: Hash256,
}

/// `s1_score.v1` validation score artifact.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScoreReport {
    /// Schema id. Expected value: `s1_score.v1`.
    #[serde(deserialize_with = "schema_s1_score")]
    pub schema: String,
    /// S1 seed.
    pub seed: u64,
    /// Checkpoint safetensors hash.
    pub checkpoint_sha: Hash256,
    /// Validation corpus hash.
    pub corpus_val_sha: Hash256,
    /// Reset-context chunk size.
    pub chunk_size: u64,
    /// Number of scored tokens.
    pub token_count: u64,
    /// Sum of base-2 negative log likelihoods.
    #[serde(deserialize_with = "finite_f64")]
    pub log2_sum: f64,
    /// Bits per character.
    #[serde(deserialize_with = "finite_nonnegative_f64")]
    pub bpc: f64,
    /// Self-hash over the score with this field omitted.
    pub score_self_hash: Hash256,
}

/// `s1_negative_test.v1` seed-0 shuffle sensitivity artifact.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NegativeTestReport {
    /// Schema id. Expected value: `s1_negative_test.v1`.
    #[serde(deserialize_with = "schema_s1_negative_test")]
    pub schema: String,
    /// S1 seed.
    ///
    /// The RFC-owned negative test uses seed 0, but that invariant belongs to
    /// the `bd-2tlx` emitter. The schema boundary records the seed so fixtures
    /// and negative-path tests can deserialize structurally valid artifacts.
    pub seed: u64,
    /// Checkpoint safetensors hash.
    pub checkpoint_sha: Hash256,
    /// Validation corpus hash.
    pub corpus_val_sha: Hash256,
    /// Fisher-Yates shuffle seed.
    pub shuffle_seed: u64,
    /// Original validation bpc.
    #[serde(deserialize_with = "finite_nonnegative_f64")]
    pub bpc_original: f64,
    /// Shuffled validation bpc.
    #[serde(deserialize_with = "finite_nonnegative_f64")]
    pub bpc_shuffled: f64,
    /// Hash of the shuffled validation bytes.
    pub shuffled_val_sha256: Hash256,
    /// Difference `bpc_shuffled - bpc_original`.
    #[serde(deserialize_with = "finite_f64")]
    pub delta: f64,
    /// Whether the model is sensitive to byte order under the H3 rule.
    pub sensitive: bool,
    /// Self-hash over the negative-test report with this field omitted.
    pub negative_self_hash: Hash256,
}

/// `s1_ablation.v1` seed-0 Phase A vs ablation comparison artifact.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AblationReport {
    /// Schema id. Expected value: `s1_ablation.v1`.
    #[serde(deserialize_with = "schema_s1_ablation")]
    pub schema: String,
    /// S1 seed.
    ///
    /// The RFC-owned ablation comparison uses seed 0, but that invariant
    /// belongs to the `bd-3b3l` emitter. The schema boundary records the seed
    /// without enforcing producer scheduling.
    pub seed: u64,
    /// Phase A checkpoint safetensors hash.
    pub phase_a_checkpoint_sha: Hash256,
    /// Ablation checkpoint safetensors hash.
    pub ablation_checkpoint_sha: Hash256,
    /// Phase A canonical tensor payload hash.
    pub phase_a_tensor_payload_sha: Hash256,
    /// Ablation canonical tensor payload hash.
    pub ablation_tensor_payload_sha: Hash256,
    /// Whether Phase A and ablation payloads are byte-identical.
    pub phase_a_eq_ablation: bool,
    /// First mismatch, or null when payloads match.
    pub first_mismatch: Option<TensorMismatch>,
    /// Self-hash over the ablation report with this field omitted.
    pub ablation_self_hash: Hash256,
}

/// `s1_baseline.v1` n-gram baseline report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BaselineReport {
    /// Schema id. Expected value: `s1_baseline.v1`.
    #[serde(deserialize_with = "schema_s1_baseline")]
    pub schema: String,
    /// Training corpus hash.
    pub corpus_train_sha: Hash256,
    /// Validation corpus hash.
    pub corpus_val_sha: Hash256,
    /// Pinned smoothing scheme.
    pub smoothing: SmoothingScheme,
    /// Trigram reset-context bpc.
    #[serde(deserialize_with = "finite_nonnegative_f64")]
    pub bpc_3gram: f64,
    /// Bigram reset-context bpc.
    #[serde(deserialize_with = "finite_nonnegative_f64")]
    pub bpc_2gram: f64,
    /// Unigram reset-context bpc.
    #[serde(deserialize_with = "finite_nonnegative_f64")]
    pub bpc_unigram: f64,
    /// Counts cardinality summary.
    pub counts_summary: CountsSummary,
    /// Hash of the baseline counts blob.
    pub counts_blob_sha256: Hash256,
    /// Self-hash over the baseline report with this field omitted.
    pub baseline_self_hash: Hash256,
}

/// `s1_report.v1` front matter. The markdown body is owned by the report emitter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportFrontMatter {
    /// Schema id. Expected value: `s1_report.v1`.
    #[serde(deserialize_with = "schema_s1_report")]
    pub schema: String,
    /// S1 outcome tag.
    pub s1_outcome: S1Outcome,
    /// S1 decision tag.
    pub decision: S1Decision,
    /// Baseline report self-hash.
    pub baseline_self_hash: Hash256,
    /// Per-seed artifact hash table.
    pub per_seed_artifacts: Vec<PerSeedArtifacts>,
    /// RFC3339 UTC generation time. Excluded from the report self-hash.
    pub generated_at: String,
    /// RFC revision used for the report.
    pub rfc_revision: RfcRevisionRef,
    /// Hash of the pre-registered predictions section.
    pub predictions_section_hash: Hash256,
    /// Commit introducing the predictions section.
    pub predictions_commit: GitCommitId,
    /// First commit that introduced S1 result artifacts.
    pub first_result_commit: GitCommitId,
    /// Self-hash over front matter with `generated_at` and this field omitted.
    pub report_self_hash: Hash256,
}

macro_rules! impl_s1_artifact {
    ($type:ty, $schema_id:literal, $type_name:literal, $self_hash_field:literal, $field:ident) => {
        impl $type {
            /// DomainHash context for this S1 artifact.
            #[must_use]
            pub const fn domain() -> DomainHash<'static> {
                DomainHash::new(CRATE_NAME, $type_name, $schema_id, SCHEMA_VERSION)
            }

            /// Canonical JSON bytes used for this artifact's self-hash.
            pub fn canonical_json_bytes(&self) -> Result<Vec<u8>, S1SchemaError> {
                canonical_json_bytes_omitting_fields(self, &[$self_hash_field])
            }

            /// Compute this artifact's self-hash from canonical JSON.
            pub fn computed_self_hash(&self) -> Result<Hash256, S1SchemaError> {
                tracing::debug!(
                    target: S1_LOG_TARGET,
                    event_name = "s1.schema.hash.start",
                    schema_id = $schema_id,
                    schema_version = SCHEMA_VERSION,
                    "s1.schema.hash.start"
                );
                let hash =
                    self_hash_omitting_fields(Self::domain(), self, $self_hash_field, &[], &[])?;
                tracing::debug!(
                    target: S1_LOG_TARGET,
                    event_name = "s1.schema.hash.complete",
                    schema_id = $schema_id,
                    schema_version = SCHEMA_VERSION,
                    self_hash = %hash,
                    "s1.schema.hash.complete"
                );
                Ok(hash)
            }

            /// Return a copy with its stored self-hash replaced by the computed one.
            pub fn with_computed_self_hash(mut self) -> Result<Self, S1SchemaError> {
                self.$field = self.computed_self_hash()?;
                Ok(self)
            }
        }
    };
}

impl_s1_artifact!(
    CheckpointMetadata,
    "s1_checkpoint.v1",
    "CheckpointMetadata",
    "checkpoint_self_hash",
    checkpoint_self_hash
);
impl_s1_artifact!(
    RunLog,
    "s1_run_log.v1",
    "RunLog",
    "run_log_self_hash",
    run_log_self_hash
);
impl_s1_artifact!(
    ScoreReport,
    "s1_score.v1",
    "ScoreReport",
    "score_self_hash",
    score_self_hash
);
impl_s1_artifact!(
    NegativeTestReport,
    "s1_negative_test.v1",
    "NegativeTestReport",
    "negative_self_hash",
    negative_self_hash
);
impl_s1_artifact!(
    AblationReport,
    "s1_ablation.v1",
    "AblationReport",
    "ablation_self_hash",
    ablation_self_hash
);
impl_s1_artifact!(
    BaselineReport,
    "s1_baseline.v1",
    "BaselineReport",
    "baseline_self_hash",
    baseline_self_hash
);

impl OracleReport {
    /// DomainHash context for this S1 artifact.
    #[must_use]
    pub const fn domain() -> DomainHash<'static> {
        DomainHash::new(CRATE_NAME, "OracleReport", "s1_oracle.v1", SCHEMA_VERSION)
    }

    /// Canonical JSON bytes used for this artifact's self-hash.
    pub fn canonical_json_bytes(&self) -> Result<Vec<u8>, S1SchemaError> {
        self.validate_aggregate_consistency()?;
        canonical_json_bytes_omitting_fields(self, &["oracle_self_hash"])
    }

    /// Compute this artifact's self-hash from canonical JSON.
    pub fn computed_self_hash(&self) -> Result<Hash256, S1SchemaError> {
        self.validate_aggregate_consistency()?;
        tracing::debug!(
            target: S1_LOG_TARGET,
            event_name = "s1.schema.hash.start",
            schema_id = "s1_oracle.v1",
            schema_version = SCHEMA_VERSION,
            "s1.schema.hash.start"
        );
        let hash = self_hash_omitting_fields(Self::domain(), self, "oracle_self_hash", &[], &[])?;
        tracing::debug!(
            target: S1_LOG_TARGET,
            event_name = "s1.schema.hash.complete",
            schema_id = "s1_oracle.v1",
            schema_version = SCHEMA_VERSION,
            self_hash = %hash,
            "s1.schema.hash.complete"
        );
        Ok(hash)
    }

    /// Return a copy with its stored self-hash replaced by the computed one.
    pub fn with_computed_self_hash(mut self) -> Result<Self, S1SchemaError> {
        self.validate_aggregate_consistency()?;
        self.oracle_self_hash = self.computed_self_hash()?;
        Ok(self)
    }
}

impl ReportFrontMatter {
    /// DomainHash context for `s1_report.v1` front matter.
    #[must_use]
    pub const fn domain() -> DomainHash<'static> {
        DomainHash::new(
            CRATE_NAME,
            "ReportFrontMatter",
            "s1_report.v1",
            SCHEMA_VERSION,
        )
    }

    /// Canonical JSON bytes for the front-matter portion of `s1_report.v1`.
    ///
    /// This is not the final report hash preimage by itself. The load-bearing
    /// report contract appends the exact markdown body bytes; use
    /// `crate::s1::report::report_self_hash` or the report emitter when
    /// validating or producing `report_self_hash`.
    pub fn canonical_json_bytes(&self) -> Result<Vec<u8>, S1SchemaError> {
        canonical_json_bytes_omitting_fields(self, &["generated_at", "report_self_hash"])
    }

    /// Compute the front-matter-only schema hash for canonical JSON tests.
    ///
    /// This helper deliberately omits the markdown body and therefore must not
    /// be used as the final `s1_report.v1` `report_self_hash`. Production report
    /// emission and validation should route through `crate::s1::report`, whose
    /// self-hash preimage is front matter plus the exact markdown body bytes.
    pub fn computed_self_hash(&self) -> Result<Hash256, S1SchemaError> {
        tracing::debug!(
            target: S1_LOG_TARGET,
            event_name = "s1.schema.hash.start",
            schema_id = "s1_report.v1",
            schema_version = SCHEMA_VERSION,
            "s1.schema.hash.start"
        );
        let hash = self_hash_omitting_fields(
            Self::domain(),
            self,
            "report_self_hash",
            &["generated_at"],
            &["baseline_self_hash"],
        )?;
        tracing::debug!(
            target: S1_LOG_TARGET,
            event_name = "s1.schema.hash.complete",
            schema_id = "s1_report.v1",
            schema_version = SCHEMA_VERSION,
            self_hash = %hash,
            "s1.schema.hash.complete"
        );
        Ok(hash)
    }

    /// Return a copy with `report_self_hash` replaced by the front-matter-only hash.
    ///
    /// Prefer the report emitter for real reports; this is retained for schema
    /// round-trip/property tests that exercise only front-matter canonical JSON.
    pub fn with_computed_self_hash(mut self) -> Result<Self, S1SchemaError> {
        self.report_self_hash = self.computed_self_hash()?;
        Ok(self)
    }
}

/// S1 canonical JSON encoder.
///
/// Object keys are sorted lexicographically by Unicode scalar value order. For
/// UTF-8 strings, Rust's bytewise `str` ordering preserves scalar order.
pub struct S1CanonicalJson;

impl S1CanonicalJson {
    /// Serialize `payload` to S1 canonical JSON bytes.
    pub fn to_vec<T>(payload: &T) -> Result<Vec<u8>, S1SchemaError>
    where
        T: Serialize,
    {
        let value = payload.serialize(CanonicalSerializer)?;
        let mut out = Vec::new();
        write_canonical_value(&value, &mut out)?;
        Ok(out)
    }

    /// Serialize an already materialized JSON value to S1 canonical JSON bytes.
    pub fn value_to_vec(value: &Value) -> Result<Vec<u8>, S1SchemaError> {
        let value = canonical_value_from_json(value)?;
        let mut out = Vec::new();
        write_canonical_value(&value, &mut out)?;
        Ok(out)
    }
}

/// Domain metadata for S1 RFC section 1 `DomainHash`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DomainHash<'a> {
    crate_name: &'a str,
    type_name: &'a str,
    schema_id: &'a str,
    schema_version: &'a str,
}

impl<'a> DomainHash<'a> {
    /// Create a domain hash context.
    #[must_use]
    pub const fn new(
        crate_name: &'a str,
        type_name: &'a str,
        schema_id: &'a str,
        schema_version: &'a str,
    ) -> Self {
        Self {
            crate_name,
            type_name,
            schema_id,
            schema_version,
        }
    }

    /// Hash a payload after canonical JSON serialization.
    ///
    /// Top-level `*_self_hash` fields are rejected here. Use
    /// [`self_hash_for_value`] when computing an artifact's own self-hash.
    pub fn hash<T>(&self, payload: &T) -> Result<Hash256, S1SchemaError>
    where
        T: Serialize,
    {
        let value = payload.serialize(CanonicalSerializer)?;
        reject_top_level_self_hash(&value)?;
        self.hash_value_without_self_hash_check(&value)
    }

    fn hash_value_without_self_hash_check(
        &self,
        value: &CanonicalValue,
    ) -> Result<Hash256, S1SchemaError> {
        self.validate_components()?;
        let mut canonical = Vec::new();
        write_canonical_value(value, &mut canonical)?;
        self.hash_canonical_bytes(&canonical)
    }

    fn hash_canonical_bytes(&self, canonical: &[u8]) -> Result<Hash256, S1SchemaError> {
        tracing::debug!(
            target: S1_LOG_TARGET,
            event_name = "s1.domain_hash",
            crate_name = self.crate_name,
            type_name = self.type_name,
            schema_id = self.schema_id,
            schema_version = self.schema_version,
            "s1 domain hash"
        );

        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"gbf:");
        bytes.extend_from_slice(self.crate_name.as_bytes());
        bytes.push(b':');
        bytes.extend_from_slice(self.type_name.as_bytes());
        bytes.push(b':');
        bytes.extend_from_slice(self.schema_id.as_bytes());
        bytes.push(b':');
        bytes.extend_from_slice(self.schema_version.as_bytes());
        bytes.push(0);
        bytes.extend_from_slice(canonical);
        Ok(sha256(bytes))
    }

    fn validate_components(&self) -> Result<(), S1SchemaError> {
        for (name, value) in [
            ("crate", self.crate_name),
            ("type", self.type_name),
            ("schema_id", self.schema_id),
            ("schema_version", self.schema_version),
        ] {
            if value
                .as_bytes()
                .iter()
                .any(|byte| *byte == b':' || *byte == 0)
            {
                return Err(S1SchemaError::InvalidDomainComponent {
                    component: name,
                    value: value.to_owned(),
                });
            }
        }
        Ok(())
    }
}

/// Compute an artifact self-hash after omitting its named top-level self-hash field.
///
/// The omission rule is top-level only. Nested fields whose names happen to end
/// in `_self_hash` remain part of the payload, while any other top-level
/// `*_self_hash` field is rejected so it cannot silently enter the hash
/// preimage.
pub fn self_hash_for_value(
    domain: DomainHash<'_>,
    value: &Value,
    self_hash_field: &str,
) -> Result<Hash256, S1SchemaError> {
    let stripped = value_without_self_hash(value, self_hash_field)?;
    let stripped = canonical_value_from_json(&stripped)?;
    reject_top_level_self_hash(&stripped)?;
    domain.hash_value_without_self_hash_check(&stripped)
}

/// Return a clone of `value` with the top-level `self_hash_field` set to its computed hash.
pub fn value_with_self_hash(
    domain: DomainHash<'_>,
    value: &Value,
    self_hash_field: &str,
) -> Result<Value, S1SchemaError> {
    let hash = self_hash_for_value(domain, value, self_hash_field)?;
    let mut with_hash = value_without_self_hash(value, self_hash_field)?;
    let object = with_hash
        .as_object_mut()
        .ok_or(S1SchemaError::ExpectedObjectForSelfHash)?;
    object.insert(self_hash_field.to_owned(), Value::String(hash.to_string()));
    Ok(with_hash)
}

/// Return a clone of `value` with the named top-level self-hash field omitted.
///
/// This helper does not inspect nested objects; S1 self-hash stripping applies
/// only to the artifact object's own self-hash field.
pub fn value_without_self_hash(
    value: &Value,
    self_hash_field: &str,
) -> Result<Value, S1SchemaError> {
    validate_self_hash_field_name(self_hash_field)?;
    value_without_fields(value, &[self_hash_field])
}

fn canonical_json_bytes_omitting_fields<T>(
    artifact: &T,
    omitted_fields: &[&str],
) -> Result<Vec<u8>, S1SchemaError>
where
    T: Serialize,
{
    let value = artifact.serialize(CanonicalSerializer)?;
    let stripped = canonical_value_without_fields(value, omitted_fields)?;
    let mut out = Vec::new();
    write_canonical_value(&stripped, &mut out)?;
    Ok(out)
}

fn self_hash_omitting_fields<T>(
    domain: DomainHash<'_>,
    artifact: &T,
    self_hash_field: &str,
    extra_omitted_fields: &[&str],
    allowed_self_hash_fields: &[&str],
) -> Result<Hash256, S1SchemaError>
where
    T: Serialize,
{
    validate_self_hash_field_name(self_hash_field)?;
    let value = artifact.serialize(CanonicalSerializer)?;
    let mut omitted_fields = Vec::with_capacity(extra_omitted_fields.len() + 1);
    omitted_fields.push(self_hash_field);
    omitted_fields.extend_from_slice(extra_omitted_fields);
    let stripped = canonical_value_without_fields(value, &omitted_fields)?;
    reject_top_level_self_hash_except(&stripped, allowed_self_hash_fields)?;
    domain.hash_value_without_self_hash_check(&stripped)
}

fn value_without_fields(value: &Value, omitted_fields: &[&str]) -> Result<Value, S1SchemaError> {
    let mut stripped = value.clone();
    let object = stripped
        .as_object_mut()
        .ok_or(S1SchemaError::ExpectedObjectForSelfHash)?;
    for field in omitted_fields {
        object.remove(*field);
    }
    Ok(stripped)
}

fn canonical_value_without_fields(
    value: CanonicalValue,
    omitted_fields: &[&str],
) -> Result<CanonicalValue, S1SchemaError> {
    let CanonicalValue::Object(object) = value else {
        return Err(S1SchemaError::ExpectedObjectForSelfHash);
    };
    Ok(CanonicalValue::Object(
        object
            .into_iter()
            .filter(|(key, _)| !omitted_fields.contains(&key.as_str()))
            .collect(),
    ))
}

macro_rules! schema_literal_deserializer {
    ($name:ident, $expected:literal) => {
        fn $name<'de, D>(deserializer: D) -> Result<String, D::Error>
        where
            D: Deserializer<'de>,
        {
            let value = String::deserialize(deserializer)?;
            if value == $expected {
                Ok(value)
            } else {
                Err(serde::de::Error::custom(format!(
                    "expected schema id {:?}, got {:?}",
                    $expected, value
                )))
            }
        }
    };
}

schema_literal_deserializer!(schema_s1_checkpoint, "s1_checkpoint.v1");
schema_literal_deserializer!(schema_s1_run_log, "s1_run_log.v1");
schema_literal_deserializer!(schema_s1_score, "s1_score.v1");
schema_literal_deserializer!(schema_s1_negative_test, "s1_negative_test.v1");
schema_literal_deserializer!(schema_s1_ablation, "s1_ablation.v1");
schema_literal_deserializer!(schema_s1_baseline, "s1_baseline.v1");
schema_literal_deserializer!(schema_s1_oracle, "s1_oracle.v1");
schema_literal_deserializer!(schema_s1_report, "s1_report.v1");

fn finite_f32<'de, D>(deserializer: D) -> Result<f32, D::Error>
where
    D: Deserializer<'de>,
{
    let value = f32::deserialize(deserializer)?;
    if value.is_finite() {
        Ok(value)
    } else {
        Err(serde::de::Error::custom(
            "non-finite f32 values are forbidden in S1 JSON",
        ))
    }
}

fn finite_nonnegative_f32<'de, D>(deserializer: D) -> Result<f32, D::Error>
where
    D: Deserializer<'de>,
{
    let value = finite_f32(deserializer)?;
    if value >= 0.0 {
        Ok(value)
    } else {
        Err(serde::de::Error::custom(
            "negative f32 values are forbidden for this S1 field",
        ))
    }
}

fn finite_f64<'de, D>(deserializer: D) -> Result<f64, D::Error>
where
    D: Deserializer<'de>,
{
    let value = f64::deserialize(deserializer)?;
    if value.is_finite() {
        Ok(value)
    } else {
        Err(serde::de::Error::custom(
            "non-finite f64 values are forbidden in S1 JSON",
        ))
    }
}

fn finite_nonnegative_f64<'de, D>(deserializer: D) -> Result<f64, D::Error>
where
    D: Deserializer<'de>,
{
    let value = finite_f64(deserializer)?;
    if value >= 0.0 {
        Ok(value)
    } else {
        Err(serde::de::Error::custom(
            "negative f64 values are forbidden for this S1 field",
        ))
    }
}

fn finite_nonnegative_f64_array_3<'de, D>(deserializer: D) -> Result<[f64; 3], D::Error>
where
    D: Deserializer<'de>,
{
    let values = <[f64; 3]>::deserialize(deserializer)?;
    for value in values {
        if !value.is_finite() || value < 0.0 {
            return Err(serde::de::Error::custom(
                "smoothing lambdas must be finite and non-negative",
            ));
        }
    }
    Ok(values)
}

fn finite_loss_points<'de, D>(deserializer: D) -> Result<Vec<(u64, f32)>, D::Error>
where
    D: Deserializer<'de>,
{
    let values = Vec::<(u64, f32)>::deserialize(deserializer)?;
    for (_, value) in &values {
        if !value.is_finite() || *value < 0.0 {
            return Err(serde::de::Error::custom(
                "loss points must be finite and non-negative",
            ));
        }
    }
    Ok(values)
}

fn finite_bpc_points<'de, D>(deserializer: D) -> Result<Vec<(u64, f64)>, D::Error>
where
    D: Deserializer<'de>,
{
    let values = Vec::<(u64, f64)>::deserialize(deserializer)?;
    for (_, value) in &values {
        if !value.is_finite() || *value < 0.0 {
            return Err(serde::de::Error::custom(
                "bpc points must be finite and non-negative",
            ));
        }
    }
    Ok(values)
}

/// Errors from S1 schema canonicalization and hashing.
#[derive(Debug)]
pub enum S1SchemaError {
    /// Serde reported a custom serialization error.
    Custom(String),
    /// JSON tree materialization or scalar escaping failed.
    Json(serde_json::Error),
    /// NaN and infinities are forbidden in hashed S1 payloads.
    NonFiniteFloat,
    /// JSON object keys must serialize to strings.
    NonStringObjectKey,
    /// Object keys must be unique after string serialization.
    DuplicateObjectKey(String),
    /// A domain component would make the RFC section 1 prefix ambiguous.
    InvalidDomainComponent {
        /// Component name.
        component: &'static str,
        /// Rejected component value.
        value: String,
    },
    /// Self-hash helpers require a top-level JSON object.
    ExpectedObjectForSelfHash,
    /// Self-hash fields must follow the RFC `*_self_hash` convention.
    InvalidSelfHashFieldName(String),
    /// Plain `DomainHash::hash` refuses to include an artifact's own hash.
    SelfHashFieldMustBeOmitted(String),
    /// Git commit ids must be full lowercase SHA-1 commit ids.
    InvalidGitCommitId(String),
}

impl fmt::Display for S1SchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Custom(message) => write!(f, "{message}"),
            Self::Json(error) => write!(f, "{error}"),
            Self::NonFiniteFloat => write!(f, "non-finite floats are forbidden in S1 JSON"),
            Self::NonStringObjectKey => write!(f, "JSON object keys must be strings"),
            Self::DuplicateObjectKey(key) => write!(f, "duplicate JSON object key {key:?}"),
            Self::InvalidDomainComponent { component, value } => {
                write!(f, "invalid domain component {component}={value:?}")
            }
            Self::ExpectedObjectForSelfHash => {
                write!(f, "self-hash helpers require a top-level object")
            }
            Self::InvalidSelfHashFieldName(field) => {
                write!(f, "self-hash field {field:?} must end with _self_hash")
            }
            Self::SelfHashFieldMustBeOmitted(field) => {
                write!(f, "field {field:?} must be omitted before domain hashing")
            }
            Self::InvalidGitCommitId(value) => {
                write!(f, "invalid Git commit id {value:?}")
            }
        }
    }
}

impl std::error::Error for S1SchemaError {}

impl ser::Error for S1SchemaError {
    fn custom<T>(msg: T) -> Self
    where
        T: fmt::Display,
    {
        Self::Custom(msg.to_string())
    }
}

#[derive(Debug, Clone, PartialEq)]
enum CanonicalValue {
    Null,
    Bool(bool),
    I64(i64),
    U64(u64),
    F64(f64),
    String(String),
    Array(Vec<CanonicalValue>),
    Object(Vec<(String, CanonicalValue)>),
}

struct CanonicalSerializer;

impl ser::Serializer for CanonicalSerializer {
    type Ok = CanonicalValue;
    type Error = S1SchemaError;
    type SerializeSeq = ArraySerializer;
    type SerializeTuple = ArraySerializer;
    type SerializeTupleStruct = ArraySerializer;
    type SerializeTupleVariant = TupleVariantSerializer;
    type SerializeMap = ObjectSerializer;
    type SerializeStruct = ObjectSerializer;
    type SerializeStructVariant = StructVariantSerializer;

    fn serialize_bool(self, v: bool) -> Result<Self::Ok, Self::Error> {
        Ok(CanonicalValue::Bool(v))
    }

    fn serialize_i8(self, v: i8) -> Result<Self::Ok, Self::Error> {
        Ok(CanonicalValue::I64(i64::from(v)))
    }

    fn serialize_i16(self, v: i16) -> Result<Self::Ok, Self::Error> {
        Ok(CanonicalValue::I64(i64::from(v)))
    }

    fn serialize_i32(self, v: i32) -> Result<Self::Ok, Self::Error> {
        Ok(CanonicalValue::I64(i64::from(v)))
    }

    fn serialize_i64(self, v: i64) -> Result<Self::Ok, Self::Error> {
        Ok(CanonicalValue::I64(v))
    }

    fn serialize_u8(self, v: u8) -> Result<Self::Ok, Self::Error> {
        Ok(CanonicalValue::U64(u64::from(v)))
    }

    fn serialize_u16(self, v: u16) -> Result<Self::Ok, Self::Error> {
        Ok(CanonicalValue::U64(u64::from(v)))
    }

    fn serialize_u32(self, v: u32) -> Result<Self::Ok, Self::Error> {
        Ok(CanonicalValue::U64(u64::from(v)))
    }

    fn serialize_u64(self, v: u64) -> Result<Self::Ok, Self::Error> {
        Ok(CanonicalValue::U64(v))
    }

    fn serialize_f32(self, v: f32) -> Result<Self::Ok, Self::Error> {
        self.serialize_f64(f64::from(v))
    }

    fn serialize_f64(self, v: f64) -> Result<Self::Ok, Self::Error> {
        if v.is_finite() {
            Ok(CanonicalValue::F64(v))
        } else {
            Err(S1SchemaError::NonFiniteFloat)
        }
    }

    fn serialize_char(self, v: char) -> Result<Self::Ok, Self::Error> {
        Ok(CanonicalValue::String(v.to_string()))
    }

    fn serialize_str(self, v: &str) -> Result<Self::Ok, Self::Error> {
        Ok(CanonicalValue::String(v.to_owned()))
    }

    fn serialize_bytes(self, v: &[u8]) -> Result<Self::Ok, Self::Error> {
        Ok(CanonicalValue::Array(
            v.iter()
                .copied()
                .map(|byte| CanonicalValue::U64(u64::from(byte)))
                .collect(),
        ))
    }

    fn serialize_none(self) -> Result<Self::Ok, Self::Error> {
        Ok(CanonicalValue::Null)
    }

    fn serialize_some<T>(self, value: &T) -> Result<Self::Ok, Self::Error>
    where
        T: ?Sized + Serialize,
    {
        value.serialize(self)
    }

    fn serialize_unit(self) -> Result<Self::Ok, Self::Error> {
        Ok(CanonicalValue::Null)
    }

    fn serialize_unit_struct(self, _name: &'static str) -> Result<Self::Ok, Self::Error> {
        Ok(CanonicalValue::Null)
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
    ) -> Result<Self::Ok, Self::Error> {
        Ok(CanonicalValue::String(variant.to_owned()))
    }

    fn serialize_newtype_struct<T>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error>
    where
        T: ?Sized + Serialize,
    {
        value.serialize(self)
    }

    fn serialize_newtype_variant<T>(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error>
    where
        T: ?Sized + Serialize,
    {
        Ok(CanonicalValue::Object(vec![(
            variant.to_owned(),
            value.serialize(CanonicalSerializer)?,
        )]))
    }

    fn serialize_seq(self, _len: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
        Ok(ArraySerializer { values: Vec::new() })
    }

    fn serialize_tuple(self, _len: usize) -> Result<Self::SerializeTuple, Self::Error> {
        Ok(ArraySerializer { values: Vec::new() })
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleStruct, Self::Error> {
        Ok(ArraySerializer { values: Vec::new() })
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleVariant, Self::Error> {
        Ok(TupleVariantSerializer {
            variant: variant.to_owned(),
            values: Vec::new(),
        })
    }

    fn serialize_map(self, _len: Option<usize>) -> Result<Self::SerializeMap, Self::Error> {
        Ok(ObjectSerializer {
            entries: Vec::new(),
            next_key: None,
        })
    }

    fn serialize_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStruct, Self::Error> {
        Ok(ObjectSerializer {
            entries: Vec::new(),
            next_key: None,
        })
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStructVariant, Self::Error> {
        Ok(StructVariantSerializer {
            variant: variant.to_owned(),
            entries: Vec::new(),
        })
    }
}

struct KeySerializer;

impl ser::Serializer for KeySerializer {
    type Ok = String;
    type Error = S1SchemaError;
    type SerializeSeq = ser::Impossible<String, S1SchemaError>;
    type SerializeTuple = ser::Impossible<String, S1SchemaError>;
    type SerializeTupleStruct = ser::Impossible<String, S1SchemaError>;
    type SerializeTupleVariant = ser::Impossible<String, S1SchemaError>;
    type SerializeMap = ser::Impossible<String, S1SchemaError>;
    type SerializeStruct = ser::Impossible<String, S1SchemaError>;
    type SerializeStructVariant = ser::Impossible<String, S1SchemaError>;

    fn serialize_str(self, v: &str) -> Result<Self::Ok, Self::Error> {
        Ok(v.to_owned())
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
    ) -> Result<Self::Ok, Self::Error> {
        Ok(variant.to_owned())
    }

    fn serialize_newtype_struct<T>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error>
    where
        T: ?Sized + Serialize,
    {
        value.serialize(self)
    }

    fn serialize_bool(self, _v: bool) -> Result<Self::Ok, Self::Error> {
        Err(S1SchemaError::NonStringObjectKey)
    }

    fn serialize_i8(self, _v: i8) -> Result<Self::Ok, Self::Error> {
        Err(S1SchemaError::NonStringObjectKey)
    }

    fn serialize_i16(self, _v: i16) -> Result<Self::Ok, Self::Error> {
        Err(S1SchemaError::NonStringObjectKey)
    }

    fn serialize_i32(self, _v: i32) -> Result<Self::Ok, Self::Error> {
        Err(S1SchemaError::NonStringObjectKey)
    }

    fn serialize_i64(self, _v: i64) -> Result<Self::Ok, Self::Error> {
        Err(S1SchemaError::NonStringObjectKey)
    }

    fn serialize_u8(self, _v: u8) -> Result<Self::Ok, Self::Error> {
        Err(S1SchemaError::NonStringObjectKey)
    }

    fn serialize_u16(self, _v: u16) -> Result<Self::Ok, Self::Error> {
        Err(S1SchemaError::NonStringObjectKey)
    }

    fn serialize_u32(self, _v: u32) -> Result<Self::Ok, Self::Error> {
        Err(S1SchemaError::NonStringObjectKey)
    }

    fn serialize_u64(self, _v: u64) -> Result<Self::Ok, Self::Error> {
        Err(S1SchemaError::NonStringObjectKey)
    }

    fn serialize_f32(self, _v: f32) -> Result<Self::Ok, Self::Error> {
        Err(S1SchemaError::NonStringObjectKey)
    }

    fn serialize_f64(self, _v: f64) -> Result<Self::Ok, Self::Error> {
        Err(S1SchemaError::NonStringObjectKey)
    }

    fn serialize_char(self, v: char) -> Result<Self::Ok, Self::Error> {
        Ok(v.to_string())
    }

    fn serialize_bytes(self, _v: &[u8]) -> Result<Self::Ok, Self::Error> {
        Err(S1SchemaError::NonStringObjectKey)
    }

    fn serialize_none(self) -> Result<Self::Ok, Self::Error> {
        Err(S1SchemaError::NonStringObjectKey)
    }

    fn serialize_some<T>(self, value: &T) -> Result<Self::Ok, Self::Error>
    where
        T: ?Sized + Serialize,
    {
        value.serialize(self)
    }

    fn serialize_unit(self) -> Result<Self::Ok, Self::Error> {
        Err(S1SchemaError::NonStringObjectKey)
    }

    fn serialize_unit_struct(self, _name: &'static str) -> Result<Self::Ok, Self::Error> {
        Err(S1SchemaError::NonStringObjectKey)
    }

    fn serialize_newtype_variant<T>(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _value: &T,
    ) -> Result<Self::Ok, Self::Error>
    where
        T: ?Sized + Serialize,
    {
        Err(S1SchemaError::NonStringObjectKey)
    }

    fn serialize_seq(self, _len: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
        Err(S1SchemaError::NonStringObjectKey)
    }

    fn serialize_tuple(self, _len: usize) -> Result<Self::SerializeTuple, Self::Error> {
        Err(S1SchemaError::NonStringObjectKey)
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleStruct, Self::Error> {
        Err(S1SchemaError::NonStringObjectKey)
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleVariant, Self::Error> {
        Err(S1SchemaError::NonStringObjectKey)
    }

    fn serialize_map(self, _len: Option<usize>) -> Result<Self::SerializeMap, Self::Error> {
        Err(S1SchemaError::NonStringObjectKey)
    }

    fn serialize_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStruct, Self::Error> {
        Err(S1SchemaError::NonStringObjectKey)
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStructVariant, Self::Error> {
        Err(S1SchemaError::NonStringObjectKey)
    }
}

struct ArraySerializer {
    values: Vec<CanonicalValue>,
}

impl SerializeSeq for ArraySerializer {
    type Ok = CanonicalValue;
    type Error = S1SchemaError;

    fn serialize_element<T>(&mut self, value: &T) -> Result<(), Self::Error>
    where
        T: ?Sized + Serialize,
    {
        self.values.push(value.serialize(CanonicalSerializer)?);
        Ok(())
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(CanonicalValue::Array(self.values))
    }
}

impl SerializeTuple for ArraySerializer {
    type Ok = CanonicalValue;
    type Error = S1SchemaError;

    fn serialize_element<T>(&mut self, value: &T) -> Result<(), Self::Error>
    where
        T: ?Sized + Serialize,
    {
        SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        SerializeSeq::end(self)
    }
}

impl SerializeTupleStruct for ArraySerializer {
    type Ok = CanonicalValue;
    type Error = S1SchemaError;

    fn serialize_field<T>(&mut self, value: &T) -> Result<(), Self::Error>
    where
        T: ?Sized + Serialize,
    {
        SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        SerializeSeq::end(self)
    }
}

struct TupleVariantSerializer {
    variant: String,
    values: Vec<CanonicalValue>,
}

impl SerializeTupleVariant for TupleVariantSerializer {
    type Ok = CanonicalValue;
    type Error = S1SchemaError;

    fn serialize_field<T>(&mut self, value: &T) -> Result<(), Self::Error>
    where
        T: ?Sized + Serialize,
    {
        self.values.push(value.serialize(CanonicalSerializer)?);
        Ok(())
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(CanonicalValue::Object(vec![(
            self.variant,
            CanonicalValue::Array(self.values),
        )]))
    }
}

struct ObjectSerializer {
    entries: Vec<(String, CanonicalValue)>,
    next_key: Option<String>,
}

impl SerializeMap for ObjectSerializer {
    type Ok = CanonicalValue;
    type Error = S1SchemaError;

    fn serialize_key<T>(&mut self, key: &T) -> Result<(), Self::Error>
    where
        T: ?Sized + Serialize,
    {
        self.next_key = Some(key.serialize(KeySerializer)?);
        Ok(())
    }

    fn serialize_value<T>(&mut self, value: &T) -> Result<(), Self::Error>
    where
        T: ?Sized + Serialize,
    {
        let key = self
            .next_key
            .take()
            .ok_or_else(|| S1SchemaError::Custom("missing object key".to_owned()))?;
        push_unique_entry(
            &mut self.entries,
            key,
            value.serialize(CanonicalSerializer)?,
        )
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(CanonicalValue::Object(self.entries))
    }
}

impl SerializeStruct for ObjectSerializer {
    type Ok = CanonicalValue;
    type Error = S1SchemaError;

    fn serialize_field<T>(&mut self, key: &'static str, value: &T) -> Result<(), Self::Error>
    where
        T: ?Sized + Serialize,
    {
        push_unique_entry(
            &mut self.entries,
            key.to_owned(),
            value.serialize(CanonicalSerializer)?,
        )
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(CanonicalValue::Object(self.entries))
    }
}

struct StructVariantSerializer {
    variant: String,
    entries: Vec<(String, CanonicalValue)>,
}

impl SerializeStructVariant for StructVariantSerializer {
    type Ok = CanonicalValue;
    type Error = S1SchemaError;

    fn serialize_field<T>(&mut self, key: &'static str, value: &T) -> Result<(), Self::Error>
    where
        T: ?Sized + Serialize,
    {
        push_unique_entry(
            &mut self.entries,
            key.to_owned(),
            value.serialize(CanonicalSerializer)?,
        )
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(CanonicalValue::Object(vec![(
            self.variant,
            CanonicalValue::Object(self.entries),
        )]))
    }
}

fn push_unique_entry(
    entries: &mut Vec<(String, CanonicalValue)>,
    key: String,
    value: CanonicalValue,
) -> Result<(), S1SchemaError> {
    if entries.iter().any(|(existing, _)| existing == &key) {
        return Err(S1SchemaError::DuplicateObjectKey(key));
    }
    entries.push((key, value));
    Ok(())
}

fn canonical_value_from_json(value: &Value) -> Result<CanonicalValue, S1SchemaError> {
    Ok(match value {
        Value::Null => CanonicalValue::Null,
        Value::Bool(value) => CanonicalValue::Bool(*value),
        Value::Number(number) => {
            if let Some(value) = number.as_i64() {
                CanonicalValue::I64(value)
            } else if let Some(value) = number.as_u64() {
                CanonicalValue::U64(value)
            } else {
                CanonicalValue::F64(number.as_f64().ok_or_else(|| {
                    S1SchemaError::Json(serde_json::Error::io(std::io::Error::other(
                        "JSON number is not representable as finite f64",
                    )))
                })?)
            }
        }
        Value::String(value) => CanonicalValue::String(value.clone()),
        Value::Array(values) => CanonicalValue::Array(
            values
                .iter()
                .map(canonical_value_from_json)
                .collect::<Result<Vec<_>, _>>()?,
        ),
        Value::Object(object) => {
            let mut entries = Vec::new();
            for (key, value) in object {
                push_unique_entry(&mut entries, key.clone(), canonical_value_from_json(value)?)?;
            }
            CanonicalValue::Object(entries)
        }
    })
}

fn write_canonical_value(value: &CanonicalValue, out: &mut Vec<u8>) -> Result<(), S1SchemaError> {
    match value {
        CanonicalValue::Null => out.extend_from_slice(b"null"),
        CanonicalValue::Bool(true) => out.extend_from_slice(b"true"),
        CanonicalValue::Bool(false) => out.extend_from_slice(b"false"),
        CanonicalValue::I64(value) => out.extend_from_slice(value.to_string().as_bytes()),
        CanonicalValue::U64(value) => out.extend_from_slice(value.to_string().as_bytes()),
        CanonicalValue::F64(value) => write_canonical_f64(*value, out)?,
        CanonicalValue::String(string) => out.extend_from_slice(
            serde_json::to_string(string)
                .map_err(S1SchemaError::Json)?
                .as_bytes(),
        ),
        CanonicalValue::Array(values) => {
            out.push(b'[');
            for (index, item) in values.iter().enumerate() {
                if index > 0 {
                    out.push(b',');
                }
                write_canonical_value(item, out)?;
            }
            out.push(b']');
        }
        CanonicalValue::Object(object) => write_canonical_object(object, out)?,
    }
    Ok(())
}

fn write_canonical_object(
    object: &[(String, CanonicalValue)],
    out: &mut Vec<u8>,
) -> Result<(), S1SchemaError> {
    let mut entries = object.iter().collect::<Vec<_>>();
    entries.sort_by(|(left, _), (right, _)| left.as_str().cmp(right.as_str()));

    out.push(b'{');
    for (index, (key, value)) in entries.into_iter().enumerate() {
        if index > 0 {
            out.push(b',');
        }
        out.extend_from_slice(
            serde_json::to_string(key)
                .map_err(S1SchemaError::Json)?
                .as_bytes(),
        );
        out.push(b':');
        write_canonical_value(value, out)?;
    }
    out.push(b'}');
    Ok(())
}

fn write_canonical_f64(value: f64, out: &mut Vec<u8>) -> Result<(), S1SchemaError> {
    if !value.is_finite() {
        return Err(S1SchemaError::NonFiniteFloat);
    }
    if value == 0.0 {
        out.extend_from_slice(b"0.0");
        return Ok(());
    }

    let encoded = canonical_f64_text(value)?;
    out.extend_from_slice(encoded.as_bytes());
    Ok(())
}

fn canonical_f64_text(value: f64) -> Result<String, S1SchemaError> {
    let mut encoded = serde_json::to_string(&value).map_err(S1SchemaError::Json)?;
    if let Some(exponent) = encoded.find("e+") {
        encoded.replace_range(exponent..exponent + 2, "e");
    }
    Ok(encoded)
}

fn reject_top_level_self_hash(value: &CanonicalValue) -> Result<(), S1SchemaError> {
    reject_top_level_self_hash_except(value, &[])
}

fn reject_top_level_self_hash_except(
    value: &CanonicalValue,
    allowed_fields: &[&str],
) -> Result<(), S1SchemaError> {
    if let CanonicalValue::Object(object) = value
        && let Some((field, _)) = object
            .iter()
            .find(|(key, _)| key.ends_with("_self_hash") && !allowed_fields.contains(&key.as_str()))
    {
        return Err(S1SchemaError::SelfHashFieldMustBeOmitted(field.clone()));
    }
    Ok(())
}

fn validate_self_hash_field_name(field: &str) -> Result<(), S1SchemaError> {
    if field.ends_with("_self_hash") {
        Ok(())
    } else {
        Err(S1SchemaError::InvalidSelfHashFieldName(field.to_owned()))
    }
}
