//! Canonical S3 notation types.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use gbf_artifact::{ClassifierView, DecodeMode, TextCharSeq, TiedEmbeddingAlias};
use gbf_data::charset_v1::{CharsetProduct, DropEvent};
use gbf_foundation::{CanonicalJson, CanonicalJsonError, Hash256};
use serde::{Deserialize, Serialize};

/// Schema id for S3 per-run phase-log JSONL entries.
pub const S3_PHASE_LOG_SCHEMA: &str = "s3_phase_log.v1";
/// Schema id for S3 reference bundle export metadata.
pub const S3_BUNDLE_SCHEMA: &str = "s3_bundle.v1";
/// Schema id for S3 model artifact export metadata.
pub const S3_ARTIFACT_SCHEMA: &str = "s3_artifact.v1";
/// Schema id for S3 v0_success run products.
pub const S3_V0_SUCCESS_SCHEMA: &str = "s3_v0_success.v1";
/// Tracing event name for S3 phase-log row production.
pub const EVENT_NAME_S3_PHASE_LOG: &str = "s3_phase_log.v1";
/// Student freeze event boundary after step 10000 optimizer update completes.
pub const S3_STUDENT_FREEZE_EVENT_STEP: GlobalStep = 10_001;

#[cfg(any(
    feature = "phase-a",
    feature = "ablation",
    feature = "s2-full",
    feature = "s2-ablation",
    feature = "falsify"
))]
pub use crate::s2::schema::{GlobalStep, HypothesisStatus};

#[cfg(not(any(
    feature = "phase-a",
    feature = "ablation",
    feature = "s2-full",
    feature = "s2-ablation",
    feature = "falsify"
)))]
/// Global 1-indexed optimizer step counter across the S3 run.
pub type GlobalStep = u64;

#[cfg(not(any(
    feature = "phase-a",
    feature = "ablation",
    feature = "s2-full",
    feature = "s2-ablation",
    feature = "falsify"
)))]
/// Schema-only fallback matching the inherited S2 hypothesis status shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum HypothesisStatus {
    /// Hypothesis confirmed.
    Confirmed,
    /// Hypothesis refuted.
    Refuted,
    /// Hypothesis was not evaluated because an earlier gate stopped.
    NotEvaluatedDueToPriorGate {
        /// Human-readable prior gate reason.
        reason: String,
    },
}

#[cfg(not(any(
    feature = "phase-a",
    feature = "ablation",
    feature = "s2-full",
    feature = "s2-ablation",
    feature = "falsify"
)))]
impl HypothesisStatus {
    /// True when the hypothesis reached a binary closure verdict.
    #[must_use]
    pub const fn is_binary_closure_verdict(&self) -> bool {
        matches!(self, Self::Confirmed | Self::Refuted)
    }
}

/// S3 runtime build identity.
#[allow(non_camel_case_types)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum S3BuildKind {
    /// v0_success run with real F-C1/F-C2 oracle backends.
    s3_v0_success_real_oracle,
    /// v0_success run with named S3 fallback oracle backends.
    s3_v0_success_fallback_oracle,
    /// Test-only adversarial oracle build for H6 falsification.
    s3_oracle_adversarial,
}

impl S3BuildKind {
    /// All S3 build kinds in canonical matrix order.
    pub const ALL: [Self; 3] = [
        Self::s3_v0_success_real_oracle,
        Self::s3_v0_success_fallback_oracle,
        Self::s3_oracle_adversarial,
    ];

    /// Stable schema/logging label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::s3_v0_success_real_oracle => "s3-v0-success-real-oracle",
            Self::s3_v0_success_fallback_oracle => "s3-v0-success-fallback-oracle",
            Self::s3_oracle_adversarial => "s3-oracle-adversarial",
        }
    }
}

/// Emit the public field schema for `s3_v0_success.v1`.
#[must_use]
pub fn s3_v0_success_schema() -> serde_json::Value {
    serde_json::json!({
        "schema": S3_V0_SUCCESS_SCHEMA,
        "fields": [
            "schema",
            "workload_self_hash",
            "baseline_self_hash",
            "chrome_budget_self_hash",
            "per_seed",
            "suspicious_low_bpc",
            "overall_pass",
            "v0_success_self_hash"
        ],
        "per_seed_fields": [
            "seed",
            "val_bpc_char_fp",
            "val_bpc_char_ternary",
            "bpc_gain_vs_kn5",
            "bpc_quant_gap",
            "per_prompt_generation",
            "artifact_deployable_bytes",
            "fits_chrome_budget",
            "Q1_holds",
            "Q2_holds",
            "Q3_holds",
            "Q4_holds",
            "Q5_holds",
            "Q6_holds",
            "pass"
        ]
    })
}

/// S3 report outcome tag from RFC section 10.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum S3Outcome {
    /// H1 through H7 all confirmed with real oracle backends.
    #[serde(rename = "Pass-clean")]
    PassClean,
    /// Closure gates passed with at least one named fallback oracle backend.
    #[serde(rename = "Pass-with-fallback-oracle")]
    PassWithFallbackOracle,
    /// H1 charset validation was refuted.
    #[serde(rename = "Fail-charset")]
    FailCharset,
    /// H2 Kneser-Ney baseline validation was refuted.
    #[serde(rename = "Fail-baseline")]
    FailBaseline,
    /// H3 quality gate was refuted without the suspicious-low-bpc sentinel.
    #[serde(rename = "Fail-quality")]
    FailQuality,
    /// Median validation bpc_char was suspiciously low.
    #[serde(rename = "Fail-suspicious")]
    FailSuspicious,
    /// H4 live-vs-oracle agreement was refuted.
    #[serde(rename = "Fail-oracle-agreement")]
    FailOracleAgreement,
    /// H5 bundle or artifact export determinism/shape was refuted.
    #[serde(rename = "Fail-bundle")]
    FailBundle,
    /// H6 QuantSpec resolution was refuted.
    #[serde(rename = "Fail-quantspec")]
    FailQuantspec,
    /// Any seed/build diverged.
    #[serde(rename = "Fail-substrate")]
    FailSubstrate,
    /// H7 F4 phase carry-through was refuted.
    #[serde(rename = "Fail-phase")]
    FailPhase,
    /// S3 falsification suite failed.
    #[serde(rename = "Fail-falsification")]
    FailFalsification,
    /// Public API drift check failed.
    #[serde(rename = "Fail-api-drift")]
    FailApiDrift,
    /// Inherited S1/F-S2 oracle re-run regressed under the S3 binary.
    #[serde(rename = "Fail-metric")]
    FailMetric,
    /// Pre-registration proof failed.
    #[serde(rename = "Fail-preregistration")]
    FailPreregistration,
    /// Required S3 artifact was missing or self-hash invalid.
    #[serde(rename = "Fail-artifact")]
    FailArtifact,
    /// Required non-gating artifact was missing.
    #[serde(rename = "Fail-incomplete")]
    FailIncomplete,
}

impl S3Outcome {
    /// All S3 report outcome tags currently accepted by the schema.
    pub const ALL: [Self; 17] = [
        Self::PassClean,
        Self::PassWithFallbackOracle,
        Self::FailCharset,
        Self::FailBaseline,
        Self::FailQuality,
        Self::FailSuspicious,
        Self::FailOracleAgreement,
        Self::FailBundle,
        Self::FailQuantspec,
        Self::FailSubstrate,
        Self::FailPhase,
        Self::FailFalsification,
        Self::FailApiDrift,
        Self::FailMetric,
        Self::FailPreregistration,
        Self::FailArtifact,
        Self::FailIncomplete,
    ];
}

/// S3 decision tag from RFC section 10.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum S3Decision {
    /// Proceed to S4 without deferred oracle clauses.
    #[serde(rename = "ProceedToS4")]
    ProceedToS4,
    /// Proceed to S4 while carrying a named real-oracle deferred clause.
    #[serde(rename = "ProceedToS4-with-deferred-clause")]
    ProceedToS4WithDeferredClause,
    /// Investigation is required before S3 can close.
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

/// One of the seven F-S3 hypotheses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum S3Hypothesis {
    /// H1 charset v1 correctness.
    H1,
    /// H2 Kneser-Ney baseline correctness.
    H2,
    /// H3 v0_success quality gate.
    H3,
    /// H4 live-vs-oracle surface agreement.
    H4,
    /// H5 bundle and artifact export determinism.
    H5,
    /// H6 QuantSpec resolution and adversarial oracle direction.
    H6,
    /// H7 F4 phase carry-through.
    H7,
}

impl S3Hypothesis {
    /// All seven S3 hypotheses in canonical closure order.
    pub const ALL: [Self; 7] = [
        Self::H1,
        Self::H2,
        Self::H3,
        Self::H4,
        Self::H5,
        Self::H6,
        Self::H7,
    ];
}

/// Completion state recorded in `s3_report.v1` per seed/build cell.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "PascalCase", deny_unknown_fields)]
pub enum S3Completion {
    /// The run completed its requested optimizer steps.
    Completed,
    /// The run observed divergence at the recorded train step.
    DivergedAt {
        /// First diverged global train step.
        step: GlobalStep,
    },
    /// The run product was not reached because an earlier gate stopped.
    NotReached,
}

/// Named S3 fallback oracle backend used during a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum OracleFallbackTag {
    /// Fixture-local denotational oracle fallback.
    S3DenotationalFallback,
    /// Fixture-local artifact oracle fallback.
    S3ArtifactFallback,
    /// Fixture-local live observations derived from oracle output.
    S3LiveObservationFixture,
}

impl From<gbf_oracle::phase_surface_agreement::OracleFallbackTag> for OracleFallbackTag {
    fn from(value: gbf_oracle::phase_surface_agreement::OracleFallbackTag) -> Self {
        match value {
            gbf_oracle::phase_surface_agreement::OracleFallbackTag::S3DenotationalFallback => {
                Self::S3DenotationalFallback
            }
            gbf_oracle::phase_surface_agreement::OracleFallbackTag::S3ArtifactFallback => {
                Self::S3ArtifactFallback
            }
            gbf_oracle::phase_surface_agreement::OracleFallbackTag::S3LiveObservationFixture => {
                Self::S3LiveObservationFixture
            }
        }
    }
}

/// Canonical `s3_charset_v1.v1` product record emitted by the charset gate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CharsetProductRecord {
    /// Pinned schema literal.
    #[serde(deserialize_with = "deserialize_charset_product_schema")]
    pub schema: String,
    /// Normalized train character stream.
    pub train_post: TextCharSeq,
    /// Normalized validation character stream.
    pub val_post: TextCharSeq,
    /// SHA-256 of `train_post` bytes.
    pub train_post_sha256: Hash256,
    /// SHA-256 of `val_post` bytes.
    pub val_post_sha256: Hash256,
    /// Pinned `LexicalSpec_v1` self-hash.
    pub charset_v1_sha256: Hash256,
    /// Dropped-example rate for the train split.
    pub unmappable_example_drop_rate_train: f64,
    /// Dropped-example rate for the validation split.
    pub unmappable_example_drop_rate_val: f64,
    /// Dropped-token rate for the train split.
    pub unmappable_char_drop_rate_train: f64,
    /// Dropped-token rate for the validation split.
    pub unmappable_char_drop_rate_val: f64,
    /// Per-example drop events.
    pub drop_log: Vec<DropEvent>,
    /// Self-hash of this record.
    pub charset_self_hash: Hash256,
}

impl From<CharsetProduct> for CharsetProductRecord {
    fn from(product: CharsetProduct) -> Self {
        Self {
            schema: "s3_charset_v1.v1".to_owned(),
            train_post: product.train_post,
            val_post: product.val_post,
            train_post_sha256: product.train_post_sha256,
            val_post_sha256: product.val_post_sha256,
            charset_v1_sha256: product.charset_v1_sha256,
            unmappable_example_drop_rate_train: product.unmappable_example_drop_rate_train,
            unmappable_example_drop_rate_val: product.unmappable_example_drop_rate_val,
            unmappable_char_drop_rate_train: product.unmappable_char_drop_rate_train,
            unmappable_char_drop_rate_val: product.unmappable_char_drop_rate_val,
            drop_log: product.drop_log,
            charset_self_hash: product.charset_self_hash,
        }
    }
}

/// `s3_phase_log.v1` event row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event_kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum S3PhaseLogEvent {
    /// Student snapshot freeze at the S3 post-step-10000 boundary.
    StudentFreeze {
        /// Pinned schema literal.
        schema: String,
        /// Boundary step recorded in the phase log.
        step: GlobalStep,
        /// Frozen student storage fingerprint.
        student_storage_fingerprint: String,
        /// Frozen student weight fingerprint.
        student_weight_fingerprint: String,
    },
}

impl S3PhaseLogEvent {
    /// Construct the canonical student-freeze phase-log event.
    pub fn student_freeze(
        student_storage_fingerprint: impl Into<String>,
        student_weight_fingerprint: impl Into<String>,
    ) -> Result<Self, S3PhaseLogError> {
        let event = Self::StudentFreeze {
            schema: S3_PHASE_LOG_SCHEMA.to_owned(),
            step: S3_STUDENT_FREEZE_EVENT_STEP,
            student_storage_fingerprint: student_storage_fingerprint.into(),
            student_weight_fingerprint: student_weight_fingerprint.into(),
        };
        event.validate()?;
        Ok(event)
    }

    /// Validate schema id, boundary, and required fingerprint fields.
    pub fn validate(&self) -> Result<(), S3PhaseLogError> {
        match self {
            Self::StudentFreeze {
                schema,
                step,
                student_storage_fingerprint,
                student_weight_fingerprint,
            } => {
                if schema != S3_PHASE_LOG_SCHEMA {
                    return Err(S3PhaseLogError::InvalidSchema {
                        expected: S3_PHASE_LOG_SCHEMA,
                        observed: schema.clone(),
                    });
                }
                if *step != S3_STUDENT_FREEZE_EVENT_STEP {
                    return Err(S3PhaseLogError::InvalidStudentFreezeStep { observed: *step });
                }
                validate_s3_phase_log_nonempty(
                    "student_storage_fingerprint",
                    student_storage_fingerprint,
                )?;
                validate_s3_phase_log_nonempty(
                    "student_weight_fingerprint",
                    student_weight_fingerprint,
                )?;
                Ok(())
            }
        }
    }

    /// Canonical JSON bytes for one phase-log row.
    pub fn canonical_json_bytes(&self) -> Result<Vec<u8>, S3PhaseLogError> {
        self.validate()?;
        CanonicalJson::to_vec(self).map_err(S3PhaseLogError::Canonical)
    }

    /// Canonical JSONL line for one phase-log row.
    pub fn canonical_json_line(&self) -> Result<Vec<u8>, S3PhaseLogError> {
        let mut bytes = self.canonical_json_bytes()?;
        bytes.push(b'\n');
        Ok(bytes)
    }

    /// The event kind string stored in JSONL.
    #[must_use]
    pub const fn event_kind(&self) -> &'static str {
        match self {
            Self::StudentFreeze { .. } => "student_freeze",
        }
    }

    /// The boundary step stored in JSONL.
    #[must_use]
    pub const fn step(&self) -> GlobalStep {
        match self {
            Self::StudentFreeze { step, .. } => *step,
        }
    }
}

/// Emit a subscriber-capturable S3 phase-log event row.
pub fn emit_s3_phase_log_event(event: &S3PhaseLogEvent) -> Result<(), S3PhaseLogError> {
    event.validate()?;
    match event {
        S3PhaseLogEvent::StudentFreeze {
            step,
            student_storage_fingerprint,
            student_weight_fingerprint,
            ..
        } => {
            tracing::info!(
                target: crate::S3_LOG_TARGET,
                event_name = EVENT_NAME_S3_PHASE_LOG,
                schema = S3_PHASE_LOG_SCHEMA,
                event_kind = "student_freeze",
                step = *step,
                student_storage_fingerprint = %student_storage_fingerprint,
                student_weight_fingerprint = %student_weight_fingerprint,
            );
        }
    }
    Ok(())
}

/// Canonical JSONL bytes for ordered `s3_phase_log.v1` events.
pub fn s3_phase_log_jsonl_bytes(events: &[S3PhaseLogEvent]) -> Result<Vec<u8>, S3PhaseLogError> {
    let mut bytes = Vec::new();
    for event in events {
        bytes.extend_from_slice(&event.canonical_json_line()?);
    }
    Ok(bytes)
}

/// Errors produced by S3 phase-log schema helpers.
#[derive(Debug)]
pub enum S3PhaseLogError {
    /// Row schema id did not match `s3_phase_log.v1`.
    InvalidSchema {
        /// Expected schema literal.
        expected: &'static str,
        /// Observed schema literal.
        observed: String,
    },
    /// Student freeze event was not recorded at step 10001.
    InvalidStudentFreezeStep {
        /// Observed step.
        observed: GlobalStep,
    },
    /// Required string field was empty.
    EmptyField {
        /// Field name.
        name: &'static str,
    },
    /// Canonical JSON encoding failed.
    Canonical(CanonicalJsonError),
}

impl fmt::Display for S3PhaseLogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSchema { expected, observed } => {
                write!(
                    f,
                    "expected S3 phase-log schema {expected:?}, got {observed:?}"
                )
            }
            Self::InvalidStudentFreezeStep { observed } => {
                write!(
                    f,
                    "student_freeze event must occur at step 10001, got {observed}"
                )
            }
            Self::EmptyField { name } => write!(f, "{name} must not be empty"),
            Self::Canonical(error) => write!(f, "failed to encode S3 phase log: {error}"),
        }
    }
}

impl Error for S3PhaseLogError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Canonical(error) => Some(error),
            Self::InvalidSchema { .. }
            | Self::InvalidStudentFreezeStep { .. }
            | Self::EmptyField { .. } => None,
        }
    }
}

/// Determinism class recorded by `s3_bundle.v1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum S3BundleDeterminismClass {
    /// The canonical bundle writer is expected to be bit-exact.
    BitExact,
}

/// Program validation summary stored in `s3_bundle.v1`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S3BundleProgramValidation {
    /// Structural graph/opset validation result.
    pub structural_valid: bool,
    /// Maximum absolute logit difference over the agreement prompt subset.
    pub semantic_max_logit_abs_diff: f32,
    /// Whether every agreement prompt matched the live teacher argmax token.
    pub argmax_token_all_match: bool,
}

impl S3BundleProgramValidation {
    /// True when both structural and semantic checks passed.
    #[must_use]
    pub fn prompt_subset_pass(&self, tolerance: f32) -> bool {
        self.structural_valid
            && self.semantic_max_logit_abs_diff <= tolerance
            && self.argmax_token_all_match
    }
}

/// Tied embedding/classifier metadata stored in `s3_bundle.v1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S3BundleTiedEmbeddingAlias {
    /// Whether the embedding and classifier share one tensor payload.
    pub shared: bool,
    /// Canonical id of the embedding tensor.
    pub embedding_canonical_id: String,
    /// Canonical id used by the classifier.
    pub classifier_canonical_id: String,
}

impl From<&TiedEmbeddingAlias> for S3BundleTiedEmbeddingAlias {
    fn from(alias: &TiedEmbeddingAlias) -> Self {
        Self {
            shared: alias.shared,
            embedding_canonical_id: alias.embedding_canonical_id.as_str().to_owned(),
            classifier_canonical_id: alias.classifier_canonical_id.as_str().to_owned(),
        }
    }
}

/// Canonical `s3_bundle.v1` metadata emitted by bundle export.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S3BundleMetadata {
    /// Pinned schema literal.
    pub schema: String,
    /// Deterministic S3 seed.
    pub seed: u64,
    /// Hash of the frozen teacher source snapshot.
    pub frozen_teacher_sha: Hash256,
    /// Pinned lexical spec self-hash.
    pub lexical_self_hash: Hash256,
    /// Hash of the sequence semantics paired with this bundle.
    pub sequence_semantics_hash: Hash256,
    /// Decode capability set accepted by the bundle.
    pub decode_caps: Vec<DecodeMode>,
    /// Export visitor id.
    pub export_visitor_id: String,
    /// Export visitor version hash.
    pub export_visitor_hash: Hash256,
    /// Determinism class for the canonical writer.
    pub determinism_class: S3BundleDeterminismClass,
    /// Self-hash stored in the reference bundle.
    pub bundle_self_hash: Hash256,
    /// SHA-256 of the canonical bundle payload bytes.
    pub canonical_bundle_payload_sha: Hash256,
    /// Program validation summary.
    pub program_validation: S3BundleProgramValidation,
    /// Tied embedding/classifier alias metadata, if the model shares storage.
    pub tied_embedding_alias: Option<S3BundleTiedEmbeddingAlias>,
}

impl S3BundleMetadata {
    /// Construct and validate canonical bundle metadata.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        seed: u64,
        frozen_teacher_sha: Hash256,
        lexical_self_hash: Hash256,
        sequence_semantics_hash: Hash256,
        decode_caps: Vec<DecodeMode>,
        export_visitor_id: impl Into<String>,
        export_visitor_hash: Hash256,
        bundle_self_hash: Hash256,
        canonical_bundle_payload_sha: Hash256,
        program_validation: S3BundleProgramValidation,
        tied_embedding_alias: Option<S3BundleTiedEmbeddingAlias>,
    ) -> Result<Self, S3BundleSchemaError> {
        let metadata = Self {
            schema: S3_BUNDLE_SCHEMA.to_owned(),
            seed,
            frozen_teacher_sha,
            lexical_self_hash,
            sequence_semantics_hash,
            decode_caps,
            export_visitor_id: export_visitor_id.into(),
            export_visitor_hash,
            determinism_class: S3BundleDeterminismClass::BitExact,
            bundle_self_hash,
            canonical_bundle_payload_sha,
            program_validation,
            tied_embedding_alias,
        };
        metadata.validate()?;
        Ok(metadata)
    }

    /// Validate the schema id and required fields.
    pub fn validate(&self) -> Result<(), S3BundleSchemaError> {
        if self.schema != S3_BUNDLE_SCHEMA {
            return Err(S3BundleSchemaError::InvalidSchema {
                expected: S3_BUNDLE_SCHEMA,
                observed: self.schema.clone(),
            });
        }
        if self.decode_caps.is_empty() {
            return Err(S3BundleSchemaError::EmptyField {
                name: "decode_caps",
            });
        }
        validate_s3_bundle_nonempty("export_visitor_id", &self.export_visitor_id)?;
        if let Some(alias) = &self.tied_embedding_alias {
            validate_s3_bundle_nonempty("embedding_canonical_id", &alias.embedding_canonical_id)?;
            validate_s3_bundle_nonempty("classifier_canonical_id", &alias.classifier_canonical_id)?;
        }
        Ok(())
    }

    /// Canonical JSON bytes for the metadata row.
    pub fn canonical_json_bytes(&self) -> Result<Vec<u8>, S3BundleSchemaError> {
        self.validate()?;
        CanonicalJson::to_vec(self).map_err(S3BundleSchemaError::Canonical)
    }
}

/// Errors produced by `s3_bundle.v1` schema helpers.
#[derive(Debug)]
pub enum S3BundleSchemaError {
    /// Metadata schema id did not match `s3_bundle.v1`.
    InvalidSchema {
        /// Expected schema literal.
        expected: &'static str,
        /// Observed schema literal.
        observed: String,
    },
    /// Required field was empty.
    EmptyField {
        /// Field name.
        name: &'static str,
    },
    /// Canonical JSON encoding failed.
    Canonical(CanonicalJsonError),
}

impl fmt::Display for S3BundleSchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSchema { expected, observed } => {
                write!(
                    f,
                    "expected S3 bundle schema {expected:?}, got {observed:?}"
                )
            }
            Self::EmptyField { name } => write!(f, "{name} must not be empty"),
            Self::Canonical(error) => write!(f, "failed to encode S3 bundle metadata: {error}"),
        }
    }
}

impl Error for S3BundleSchemaError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Canonical(error) => Some(error),
            Self::InvalidSchema { .. } | Self::EmptyField { .. } => None,
        }
    }
}

/// Determinism class recorded by `s3_artifact.v1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum S3ArtifactDeterminismClass {
    /// The canonical artifact writer is expected to be bit-exact.
    BitExact,
}

/// QuantSpec resolution summary stored in `s3_artifact.v1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S3ArtifactWeightResolutionSummary {
    /// Number of deployable logical weight tensors requiring resolution.
    pub total_tensors: u32,
    /// Number of tensors resolved through `QuantSpec_S3::weight_quant`.
    pub tensors_resolved_via_quant_spec: u32,
    /// Number of tensors resolved by naming fallback. Must remain zero.
    pub tensors_resolved_via_naming: u32,
}

impl S3ArtifactWeightResolutionSummary {
    /// True when no tensor used naming fallback resolution.
    #[must_use]
    pub const fn no_naming_resolution(&self) -> bool {
        self.tensors_resolved_via_naming == 0
    }
}

/// Tied embedding/classifier metadata stored in `s3_artifact.v1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S3ArtifactTiedEmbeddingAlias {
    /// Whether the embedding and classifier share one tensor payload.
    pub shared: bool,
    /// Canonical id of the embedding tensor.
    pub embedding_canonical_id: String,
    /// Canonical id used by the classifier.
    pub classifier_canonical_id: String,
    /// Classifier view over the shared embedding tensor.
    pub classifier_view: ClassifierView,
}

impl From<&TiedEmbeddingAlias> for S3ArtifactTiedEmbeddingAlias {
    fn from(alias: &TiedEmbeddingAlias) -> Self {
        Self {
            shared: alias.shared,
            embedding_canonical_id: alias.embedding_canonical_id.as_str().to_owned(),
            classifier_canonical_id: alias.classifier_canonical_id.as_str().to_owned(),
            classifier_view: alias.classifier_view,
        }
    }
}

/// Canonical `s3_artifact.v1` metadata emitted by artifact export.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S3ArtifactMetadata {
    /// Pinned schema literal.
    pub schema: String,
    /// Deterministic S3 seed.
    pub seed: u64,
    /// Hash of the frozen student source snapshot.
    pub student_checkpoint_sha: Hash256,
    /// Pinned lexical spec self-hash.
    pub lexical_self_hash: Hash256,
    /// Hash of the quantization spec paired with this artifact.
    pub quant_spec_hash: Hash256,
    /// Decode capability set accepted by the artifact.
    pub decode_caps: Vec<DecodeMode>,
    /// Export visitor id.
    pub export_visitor_id: String,
    /// Export visitor version hash.
    pub export_visitor_hash: Hash256,
    /// Determinism class for the canonical writer.
    pub determinism_class: S3ArtifactDeterminismClass,
    /// Self-hash stored in the model artifact.
    pub artifact_self_hash: Hash256,
    /// SHA-256 of the canonical artifact payload bytes.
    pub canonical_artifact_payload_sha: Hash256,
    /// SHA-256 of canonical aux sidecar references.
    pub canonical_aux_payload_sha: Hash256,
    /// Deployable byte total used by the Q6 chrome-budget gate.
    pub artifact_deployable_bytes: u64,
    /// QuantSpec resolution summary.
    pub weight_resolution_summary: S3ArtifactWeightResolutionSummary,
    /// Tied embedding/classifier alias metadata, if the model shares storage.
    pub tied_embedding_alias: Option<S3ArtifactTiedEmbeddingAlias>,
}

impl S3ArtifactMetadata {
    /// Construct and validate canonical artifact metadata.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        seed: u64,
        student_checkpoint_sha: Hash256,
        lexical_self_hash: Hash256,
        quant_spec_hash: Hash256,
        decode_caps: Vec<DecodeMode>,
        export_visitor_id: impl Into<String>,
        export_visitor_hash: Hash256,
        artifact_self_hash: Hash256,
        canonical_artifact_payload_sha: Hash256,
        canonical_aux_payload_sha: Hash256,
        artifact_deployable_bytes: u64,
        weight_resolution_summary: S3ArtifactWeightResolutionSummary,
        tied_embedding_alias: Option<S3ArtifactTiedEmbeddingAlias>,
    ) -> Result<Self, S3ArtifactSchemaError> {
        let metadata = Self {
            schema: S3_ARTIFACT_SCHEMA.to_owned(),
            seed,
            student_checkpoint_sha,
            lexical_self_hash,
            quant_spec_hash,
            decode_caps,
            export_visitor_id: export_visitor_id.into(),
            export_visitor_hash,
            determinism_class: S3ArtifactDeterminismClass::BitExact,
            artifact_self_hash,
            canonical_artifact_payload_sha,
            canonical_aux_payload_sha,
            artifact_deployable_bytes,
            weight_resolution_summary,
            tied_embedding_alias,
        };
        metadata.validate()?;
        Ok(metadata)
    }

    /// Validate the schema id and required fields.
    pub fn validate(&self) -> Result<(), S3ArtifactSchemaError> {
        if self.schema != S3_ARTIFACT_SCHEMA {
            return Err(S3ArtifactSchemaError::InvalidSchema {
                expected: S3_ARTIFACT_SCHEMA,
                observed: self.schema.clone(),
            });
        }
        if self.decode_caps.is_empty() {
            return Err(S3ArtifactSchemaError::EmptyField {
                name: "decode_caps",
            });
        }
        validate_s3_artifact_nonempty("export_visitor_id", &self.export_visitor_id)?;
        if !self.weight_resolution_summary.no_naming_resolution() {
            return Err(S3ArtifactSchemaError::NamingResolutionUsed {
                observed: self.weight_resolution_summary.tensors_resolved_via_naming,
            });
        }
        if let Some(alias) = &self.tied_embedding_alias {
            validate_s3_artifact_nonempty("embedding_canonical_id", &alias.embedding_canonical_id)?;
            validate_s3_artifact_nonempty(
                "classifier_canonical_id",
                &alias.classifier_canonical_id,
            )?;
            if alias.shared && alias.embedding_canonical_id != alias.classifier_canonical_id {
                return Err(S3ArtifactSchemaError::TiedAliasSplit);
            }
        }
        Ok(())
    }

    /// Canonical JSON bytes for the metadata row.
    pub fn canonical_json_bytes(&self) -> Result<Vec<u8>, S3ArtifactSchemaError> {
        self.validate()?;
        CanonicalJson::to_vec(self).map_err(S3ArtifactSchemaError::Canonical)
    }
}

/// Errors produced by `s3_artifact.v1` schema helpers.
#[derive(Debug)]
pub enum S3ArtifactSchemaError {
    /// Metadata schema id did not match `s3_artifact.v1`.
    InvalidSchema {
        /// Expected schema literal.
        expected: &'static str,
        /// Observed schema literal.
        observed: String,
    },
    /// Required field was empty.
    EmptyField {
        /// Field name.
        name: &'static str,
    },
    /// A tied alias claimed sharing while splitting canonical ids.
    TiedAliasSplit,
    /// A tensor was resolved by naming fallback.
    NamingResolutionUsed {
        /// Observed non-zero naming-resolution count.
        observed: u32,
    },
    /// Canonical JSON encoding failed.
    Canonical(CanonicalJsonError),
}

impl fmt::Display for S3ArtifactSchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSchema { expected, observed } => write!(
                f,
                "expected S3 artifact schema {expected:?}, got {observed:?}"
            ),
            Self::EmptyField { name } => write!(f, "{name} must not be empty"),
            Self::TiedAliasSplit => {
                f.write_str("s3_artifact.v1 tied alias must preserve one canonical tensor id")
            }
            Self::NamingResolutionUsed { observed } => write!(
                f,
                "s3_artifact.v1 forbids naming resolution, observed {observed}"
            ),
            Self::Canonical(error) => write!(f, "failed to encode S3 artifact metadata: {error}"),
        }
    }
}

impl Error for S3ArtifactSchemaError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Canonical(error) => Some(error),
            Self::InvalidSchema { .. }
            | Self::EmptyField { .. }
            | Self::TiedAliasSplit
            | Self::NamingResolutionUsed { .. } => None,
        }
    }
}

/// Canonical `s3_baseline_kn5.v1` product record.
pub type BaselineKnReport = crate::s3::baseline::KnBaselineProduct;

/// Canonical `s3_score.v1` product record.
pub type S3ScoreReport = crate::s3::score::ScoreCharProduct;

/// Verifier and early-gate inputs consumed by the S3 outcome dispatcher.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S3VerifierBundle {
    /// Pre-registration proof gate.
    pub preregistration_passed: bool,
    /// Required artifact presence and self-hash gate.
    pub artifact_integrity_passed: bool,
    /// S1 and F-S2 oracle re-run under the S3 binary.
    pub oracle_re_run_passed: bool,
    /// Public API drift checker result.
    pub api_drift_check_passed: bool,
    /// S3 falsification suite result.
    pub falsification_s3_passed: bool,
    /// Bundle export determinism gate.
    pub bundle_determinism_passed: bool,
    /// Artifact export determinism gate.
    pub artifact_determinism_passed: bool,
    /// Charset normalization/idempotence gate.
    pub charset_idempotence_passed: bool,
    /// Kneser-Ney oracle gate.
    pub kn_oracle_passed: bool,
    /// Live-vs-oracle agreement gate.
    pub oracle_agreement_passed: bool,
    /// QuantSpec resolution gate.
    pub quantspec_resolution_passed: bool,
    /// Whether all methodological controls needed by later hypotheses exist.
    pub methodological_controls_present: bool,
    /// Suspicious-low median bpc_char sentinel for any scored build.
    pub suspicious_low_bpc: bool,
    /// Per-seed/build completion states across the S3 build matrix.
    pub completions: Vec<S3Completion>,
    /// Explicit verdict status for all seven hypotheses.
    pub hypothesis_statuses: BTreeMap<S3Hypothesis, HypothesisStatus>,
    /// Named fallback oracle backends used by the run.
    pub oracle_fallback_used: Vec<OracleFallbackTag>,
}

impl S3VerifierBundle {
    /// Construct a closure-candidate bundle with all gates and hypotheses passing.
    #[must_use]
    pub fn closure_candidate() -> Self {
        Self {
            preregistration_passed: true,
            artifact_integrity_passed: true,
            oracle_re_run_passed: true,
            api_drift_check_passed: true,
            falsification_s3_passed: true,
            bundle_determinism_passed: true,
            artifact_determinism_passed: true,
            charset_idempotence_passed: true,
            kn_oracle_passed: true,
            oracle_agreement_passed: true,
            quantspec_resolution_passed: true,
            methodological_controls_present: true,
            suspicious_low_bpc: false,
            completions: vec![S3Completion::Completed; 15],
            hypothesis_statuses: all_confirmed_hypotheses(),
            oracle_fallback_used: Vec::new(),
        }
    }

    /// Return the status for one S3 hypothesis.
    #[must_use]
    pub fn status(&self, hypothesis: S3Hypothesis) -> HypothesisStatus {
        self.hypothesis_statuses
            .get(&hypothesis)
            .cloned()
            .unwrap_or_else(|| HypothesisStatus::NotEvaluatedDueToPriorGate {
                reason: "missing hypothesis status".to_owned(),
            })
    }

    /// True if any seed/build diverged.
    #[must_use]
    pub fn any_seed_diverged(&self) -> bool {
        self.completions
            .iter()
            .any(|completion| matches!(completion, S3Completion::DivergedAt { .. }))
    }

    /// True if any seed/build was not reached.
    #[must_use]
    pub fn any_not_reached(&self) -> bool {
        self.completions
            .iter()
            .any(|completion| matches!(completion, S3Completion::NotReached))
    }

    /// Return the first not-evaluated hypothesis status, if any.
    #[must_use]
    pub fn first_not_evaluated(&self) -> Option<(S3Hypothesis, HypothesisStatus)> {
        S3Hypothesis::ALL.into_iter().find_map(|hypothesis| {
            let status = self.status(hypothesis);
            (!status.is_binary_closure_verdict()).then_some((hypothesis, status))
        })
    }
}

fn all_confirmed_hypotheses() -> BTreeMap<S3Hypothesis, HypothesisStatus> {
    S3Hypothesis::ALL
        .into_iter()
        .map(|hypothesis| (hypothesis, HypothesisStatus::Confirmed))
        .collect()
}

fn deserialize_charset_product_schema<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    if value == "s3_charset_v1.v1" {
        Ok(value)
    } else {
        Err(serde::de::Error::custom(format_args!(
            "expected schema id {:?}, got {value:?}",
            "s3_charset_v1.v1"
        )))
    }
}

fn validate_s3_phase_log_nonempty(name: &'static str, value: &str) -> Result<(), S3PhaseLogError> {
    if value.trim().is_empty() {
        return Err(S3PhaseLogError::EmptyField { name });
    }
    Ok(())
}

fn validate_s3_bundle_nonempty(name: &'static str, value: &str) -> Result<(), S3BundleSchemaError> {
    if value.trim().is_empty() {
        return Err(S3BundleSchemaError::EmptyField { name });
    }
    Ok(())
}

fn validate_s3_artifact_nonempty(
    name: &'static str,
    value: &str,
) -> Result<(), S3ArtifactSchemaError> {
    if value.trim().is_empty() {
        return Err(S3ArtifactSchemaError::EmptyField { name });
    }
    Ok(())
}
