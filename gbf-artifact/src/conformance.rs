//! F-S3 conformance envelope schema.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use gbf_foundation::{CanonicalJson, DomainHash, Hash256};
use serde::{Deserialize, Deserializer, Serialize};

use crate::ids::ArtifactPath;
use crate::semantic_checkpoint::SemanticCheckpoint;

const CONFORMANCE_SCHEMA_ID: &str = "s3_conformance.v1";
const CONFORMANCE_SCHEMA_VERSION: &str = "1";
pub const CONFORMANCE_REAL_OWNER_BEAD: &str = "bd-35l3";
pub const S3_CONFORMANCE_SEED_COUNT: usize = 5;

pub type MetricId = ArtifactPath;

/// S3 conformance JSON artifact.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConformanceEnvelope {
    #[serde(deserialize_with = "deserialize_conformance_schema")]
    pub schema: String,
    pub workload_self_hash: Hash256,
    pub per_seed: Vec<SeedConformanceEnvelope>,
    pub overall: EnvelopeGate,
    pub quantization_gap_summary: QuantizationGapSummary,
    #[serde(deserialize_with = "deserialize_real_owner_bead")]
    pub real_owner_bead: String,
    pub conformance_self_hash: Hash256,
}

impl ConformanceEnvelope {
    pub fn new(
        workload_self_hash: Hash256,
        per_seed: Vec<SeedConformanceEnvelope>,
        overall: EnvelopeGate,
        quantization_gap_summary: QuantizationGapSummary,
    ) -> Result<Self, ConformanceError> {
        if per_seed.len() != S3_CONFORMANCE_SEED_COUNT {
            return Err(ConformanceError::SeedArityMismatch {
                expected: S3_CONFORMANCE_SEED_COUNT,
                actual: per_seed.len(),
            });
        }

        let mut envelope = Self {
            schema: CONFORMANCE_SCHEMA_ID.to_owned(),
            workload_self_hash,
            per_seed,
            overall,
            quantization_gap_summary,
            real_owner_bead: CONFORMANCE_REAL_OWNER_BEAD.to_owned(),
            conformance_self_hash: Hash256::ZERO,
        };
        envelope.conformance_self_hash = envelope.compute_self_hash()?;
        Ok(envelope)
    }

    /// Canonical JSON bytes for the conformance write boundary.
    ///
    /// This method intentionally rejects the typed F8-broken aggregation marker
    /// at serialization time; B19 owns the full `CanonicalConformanceWrite`
    /// encoder that will call this validation.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, ConformanceError> {
        self.validate_canonical_write()?;
        self.canonical_bytes_unchecked()
    }

    pub fn canonical_bytes_unchecked(&self) -> Result<Vec<u8>, ConformanceError> {
        CanonicalJson::to_vec(self).map_err(ConformanceError::CanonicalJson)
    }

    pub fn compute_self_hash(&self) -> Result<Hash256, ConformanceError> {
        let mut value = serde_json::to_value(self).map_err(ConformanceError::Json)?;
        value
            .as_object_mut()
            .ok_or(ConformanceError::ExpectedObjectForSelfHash)?
            .remove("conformance_self_hash");
        let canonical =
            CanonicalJson::value_to_vec(&value).map_err(ConformanceError::CanonicalJson)?;
        Self::domain()
            .hash_canonical_bytes(&canonical)
            .map_err(ConformanceError::CanonicalJson)
    }

    pub fn validate_canonical_write(&self) -> Result<(), ConformanceError> {
        for seed in &self.per_seed {
            for (metric_id, metric) in &seed.per_metric {
                if metric.aggregation_kind == AggregationKind::PromptWideSoftmaxForbidden {
                    return Err(ConformanceError::ForbiddenAggregationKind {
                        seed: seed.seed,
                        metric_id: metric_id.clone(),
                    });
                }
            }
        }
        Ok(())
    }

    #[must_use]
    pub const fn domain() -> DomainHash<'static> {
        DomainHash::new(
            "gbf-artifact",
            "ConformanceEnvelope",
            CONFORMANCE_SCHEMA_ID,
            CONFORMANCE_SCHEMA_VERSION,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeedConformanceEnvelope {
    pub seed: u64,
    pub bundle_self_hash: Hash256,
    pub artifact_self_hash: Hash256,
    pub overall: EnvelopeGate,
    pub per_checkpoint: BTreeMap<SemanticCheckpoint, EnvelopeGate>,
    pub per_metric: BTreeMap<MetricId, MetricGate>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvelopeGate {
    pub tolerance: f32,
    pub passed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetricGate {
    pub value: f32,
    pub aggregation_kind: AggregationKind,
    pub passed: bool,
}

/// Metric aggregation provenance annotation used to detect F8-broken-S3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AggregationKind {
    PerTokenPerVocabRow,
    PromptWideSoftmaxForbidden,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuantizationGapSummary {
    pub mean_per_token_max_abs_diff_phase_a: f32,
    pub mean_per_token_max_abs_diff_phase_d: f32,
    pub mean_per_token_kl: f32,
}

#[derive(Debug)]
pub enum ConformanceError {
    SeedArityMismatch {
        expected: usize,
        actual: usize,
    },
    ForbiddenAggregationKind {
        seed: u64,
        metric_id: MetricId,
    },
    PromptWideSoftmaxAggregation {
        seed: u64,
        prompt_id: String,
        metric_id: MetricId,
    },
    MissingCheckpoint {
        seed: u64,
        checkpoint: SemanticCheckpoint,
    },
    MissingMetric {
        seed: u64,
        metric_name: &'static str,
    },
    OverallToleranceTooStrict {
        overall: f32,
        max_seed_tolerance: f32,
    },
    SelfHashMismatch {
        expected: Hash256,
        observed: Hash256,
    },
    ExpectedObjectForSelfHash,
    Json(serde_json::Error),
    CanonicalJson(gbf_foundation::CanonicalJsonError),
}

impl fmt::Display for ConformanceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SeedArityMismatch { expected, actual } => {
                write!(
                    f,
                    "s3_conformance.v1 requires {expected} seeds, got {actual}"
                )
            }
            Self::ForbiddenAggregationKind { seed, metric_id } => {
                write!(
                    f,
                    "seed {seed} metric {metric_id} uses prompt-wide softmax aggregation"
                )
            }
            Self::PromptWideSoftmaxAggregation {
                seed,
                prompt_id,
                metric_id,
            } => write!(
                f,
                "seed {seed} prompt {prompt_id} metric {metric_id} uses prompt-wide softmax aggregation"
            ),
            Self::MissingCheckpoint { seed, checkpoint } => {
                write!(f, "seed {seed} is missing checkpoint {checkpoint:?}")
            }
            Self::MissingMetric { seed, metric_name } => {
                write!(f, "seed {seed} is missing required metric {metric_name}")
            }
            Self::OverallToleranceTooStrict {
                overall,
                max_seed_tolerance,
            } => write!(
                f,
                "overall tolerance {overall} is stricter than max seed tolerance {max_seed_tolerance}"
            ),
            Self::SelfHashMismatch { expected, observed } => write!(
                f,
                "conformance self-hash mismatch: expected {expected}, observed {observed}"
            ),
            Self::ExpectedObjectForSelfHash => {
                f.write_str("conformance self-hash requires a top-level object")
            }
            Self::Json(error) => write!(f, "{error}"),
            Self::CanonicalJson(error) => write!(f, "{error}"),
        }
    }
}

impl Error for ConformanceError {}

fn deserialize_conformance_schema<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_literal(deserializer, CONFORMANCE_SCHEMA_ID, "schema")
}

fn deserialize_real_owner_bead<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_literal(deserializer, CONFORMANCE_REAL_OWNER_BEAD, "real_owner_bead")
}

fn deserialize_literal<'de, D>(
    deserializer: D,
    expected: &'static str,
    field: &'static str,
) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    if value == expected {
        Ok(value)
    } else {
        Err(serde::de::Error::custom(format!(
            "expected {field} {expected:?}, got {value:?}"
        )))
    }
}
