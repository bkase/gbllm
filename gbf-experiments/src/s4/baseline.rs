//! S4 Gutenberg baseline surface.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use gbf_artifact::TextCharSeq;
use gbf_foundation::{CanonicalJson, DomainHash, Hash256, sha256};
use serde::ser::SerializeMap;
use serde::{Deserialize, Serialize};

use crate::S4_LOG_TARGET;
use crate::s3::baseline::{
    BaselineError, CountsSummary, KN_MAX_ORDER, KN_RESET_CHUNK_SIZE, KnBaselineInputs, KnDiscounts,
    s3_fit_kn5,
};

/// Schema id for the S4 Gutenberg KN-5 baseline report.
pub const S4_BASELINE_GUTENBERG_SCHEMA: &str = "s4_baseline_gutenberg.v1";

/// S4 baseline tracing target.
pub const S4_BASELINE_LOG_TARGET: &str = "gbf_experiments::s4::baseline";

const S4_BASELINE_GUTENBERG_SCHEMA_VERSION: &str = "1";
const KN_SMOOTHING: &str = "modified_kneser_ney_d_rule";
const BASELINE_DOMAIN: DomainHash<'static> = DomainHash::new(
    "gbf-experiments",
    "S4BaselineGutenbergReport",
    S4_BASELINE_GUTENBERG_SCHEMA,
    S4_BASELINE_GUTENBERG_SCHEMA_VERSION,
);

/// Inputs to `s4_fit_kn5_gutenberg`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S4BaselineInputs {
    /// TinyStories manifest self-hash carried for report lineage.
    pub tinystories_manifest_self_hash: Hash256,
    /// Gutenberg manifest self-hash.
    pub gutenberg_manifest_self_hash: Hash256,
    /// SHA-256 expected for `corpus_train`.
    pub corpus_train_sha: Hash256,
    /// SHA-256 expected for `corpus_val`.
    pub corpus_val_sha: Hash256,
    /// Normalized Gutenberg train token stream.
    pub corpus_train: TextCharSeq,
    /// Normalized Gutenberg validation token stream.
    pub corpus_val: TextCharSeq,
}

/// Kneser-Ney parameters inherited from S3.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S4KneserNeyParams {
    /// S3-pinned smoothing family.
    pub smoothing: String,
    /// S3-pinned max order, always 5.
    pub max_order: u64,
    /// Reset-context scoring chunk size, always 128.
    pub reset_context_chunk_size: u64,
    /// Per-order D-rule discounts fitted by the inherited S3 machinery.
    #[serde(
        serialize_with = "serialize_discount_map",
        deserialize_with = "deserialize_discount_map"
    )]
    pub discounts: BTreeMap<u64, KnDiscounts>,
}

impl S4KneserNeyParams {
    fn from_s3(discounts: BTreeMap<u64, KnDiscounts>) -> Self {
        Self {
            smoothing: KN_SMOOTHING.to_owned(),
            max_order: KN_MAX_ORDER as u64,
            reset_context_chunk_size: KN_RESET_CHUNK_SIZE as u64,
            discounts,
        }
    }

    fn validate(&self) -> Result<(), S4BaselineError> {
        if self.smoothing != KN_SMOOTHING {
            return Err(S4BaselineError::InvalidKnParams {
                field: "kn_params.smoothing",
            });
        }
        if self.max_order != KN_MAX_ORDER as u64 {
            return Err(S4BaselineError::InvalidKnParams {
                field: "kn_params.max_order",
            });
        }
        if self.reset_context_chunk_size != KN_RESET_CHUNK_SIZE as u64 {
            return Err(S4BaselineError::InvalidKnParams {
                field: "kn_params.reset_context_chunk_size",
            });
        }
        let expected_orders = [2_u64, 3, 4, 5];
        if self.discounts.keys().copied().collect::<Vec<_>>() != expected_orders.to_vec() {
            return Err(S4BaselineError::InvalidKnParams {
                field: "kn_params.discounts",
            });
        }
        for discounts in self.discounts.values() {
            validate_finite_nonnegative("kn_params.discounts[].y_k", discounts.y_k)?;
            validate_finite_nonnegative("kn_params.discounts[].d_1", discounts.d_1)?;
            validate_finite_nonnegative("kn_params.discounts[].d_2", discounts.d_2)?;
            validate_finite_nonnegative("kn_params.discounts[].d_3p", discounts.d_3p)?;
        }
        Ok(())
    }
}

/// `s4_baseline_gutenberg.v1` report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S4BaselineGutenbergReport {
    /// Schema id, always `s4_baseline_gutenberg.v1`.
    pub schema: String,
    /// TinyStories manifest self-hash.
    pub tinystories_manifest_self_hash: Hash256,
    /// Gutenberg manifest self-hash.
    pub gutenberg_manifest_self_hash: Hash256,
    /// SHA-256 of the Gutenberg training corpus bytes.
    pub corpus_train_sha: Hash256,
    /// SHA-256 of the Gutenberg validation corpus bytes.
    pub corpus_val_sha: Hash256,
    /// Inherited S3 Kneser-Ney parameters.
    pub kn_params: S4KneserNeyParams,
    /// KN-5 reset-context validation bpc.
    pub bpc_kn5: f64,
    /// KN-3 reset-context validation bpc.
    pub bpc_kn3: f64,
    /// Unigram reset-context validation bpc.
    pub bpc_unigram: f64,
    /// Count cardinality summary.
    pub counts_summary: CountsSummary,
    /// SHA-256 of the inherited canonical counts blob.
    pub counts_blob_sha256: Hash256,
    /// Self-hash over canonical JSON with this field omitted.
    pub baseline_gutenberg_self_hash: Hash256,
}

impl S4BaselineGutenbergReport {
    /// Canonical JSON bytes including `baseline_gutenberg_self_hash`.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, S4BaselineError> {
        self.validate_canonical_write()?;
        CanonicalJson::to_vec(self).map_err(S4BaselineError::CanonicalJson)
    }

    /// Compute the report self-hash with `baseline_gutenberg_self_hash` omitted.
    pub fn compute_self_hash(&self) -> Result<Hash256, S4BaselineError> {
        let mut value = serde_json::to_value(self).map_err(S4BaselineError::Json)?;
        value
            .as_object_mut()
            .ok_or(S4BaselineError::ExpectedObjectForSelfHash)?
            .remove("baseline_gutenberg_self_hash");
        let canonical =
            CanonicalJson::value_to_vec(&value).map_err(S4BaselineError::CanonicalJson)?;
        BASELINE_DOMAIN
            .hash_canonical_bytes(&canonical)
            .map_err(S4BaselineError::CanonicalJson)
    }

    /// Validate structure and self-hash.
    pub fn validate_canonical_write(&self) -> Result<(), S4BaselineError> {
        self.validate_structure()?;
        let recomputed = self.compute_self_hash()?;
        if recomputed != self.baseline_gutenberg_self_hash {
            return Err(S4BaselineError::SelfHashMismatch {
                expected: recomputed,
                observed: self.baseline_gutenberg_self_hash,
            });
        }
        Ok(())
    }

    fn validate_structure(&self) -> Result<(), S4BaselineError> {
        if self.schema != S4_BASELINE_GUTENBERG_SCHEMA {
            return Err(S4BaselineError::InvalidSchema {
                observed: self.schema.clone(),
            });
        }
        self.kn_params.validate()?;
        validate_finite_nonnegative("bpc_kn5", self.bpc_kn5)?;
        validate_finite_nonnegative("bpc_kn3", self.bpc_kn3)?;
        validate_finite_nonnegative("bpc_unigram", self.bpc_unigram)?;
        Ok(())
    }
}

/// Fit the inherited S3 KN-5 baseline on Gutenberg train and score Gutenberg val.
pub fn s4_fit_kn5_gutenberg(
    inputs: S4BaselineInputs,
) -> Result<S4BaselineGutenbergReport, S4BaselineError> {
    validate_hash(
        "corpus_train_sha",
        inputs.corpus_train_sha,
        sha256(inputs.corpus_train.as_slice()),
    )?;
    validate_hash(
        "corpus_val_sha",
        inputs.corpus_val_sha,
        sha256(inputs.corpus_val.as_slice()),
    )?;

    tracing::info!(
        target: S4_BASELINE_LOG_TARGET,
        event_name = "s4::baseline::fit_started",
        inherited_from = "s3_fit_kn5",
        train_char_count = inputs.corpus_train.len() as u64,
        val_char_count = inputs.corpus_val.len() as u64,
        max_order = KN_MAX_ORDER as u64,
        reset_context_chunk_size = KN_RESET_CHUNK_SIZE as u64,
        s4_log_target = S4_LOG_TARGET,
        "s4 gutenberg baseline fit started"
    );

    let product = s3_fit_kn5(KnBaselineInputs {
        train_post: inputs.corpus_train,
        val_post: inputs.corpus_val,
    })?;

    let mut report = S4BaselineGutenbergReport {
        schema: S4_BASELINE_GUTENBERG_SCHEMA.to_owned(),
        tinystories_manifest_self_hash: inputs.tinystories_manifest_self_hash,
        gutenberg_manifest_self_hash: inputs.gutenberg_manifest_self_hash,
        corpus_train_sha: inputs.corpus_train_sha,
        corpus_val_sha: inputs.corpus_val_sha,
        kn_params: S4KneserNeyParams::from_s3(product.discounts),
        bpc_kn5: product.bpc_kn5_val,
        bpc_kn3: product.bpc_kn3_val,
        bpc_unigram: product.bpc_kn1_val,
        counts_summary: product.counts_summary,
        counts_blob_sha256: product.counts_blob_sha256,
        baseline_gutenberg_self_hash: Hash256::ZERO,
    };
    report.validate_structure()?;
    report.baseline_gutenberg_self_hash = report.compute_self_hash()?;

    tracing::info!(
        target: S4_BASELINE_LOG_TARGET,
        event_name = "s4::baseline::emitted",
        baseline_gutenberg_self_hash = %report.baseline_gutenberg_self_hash,
        counts_blob_sha256 = %report.counts_blob_sha256,
        bpc_kn5 = report.bpc_kn5,
        bpc_kn3 = report.bpc_kn3,
        bpc_unigram = report.bpc_unigram,
        "s4 gutenberg baseline emitted"
    );

    Ok(report)
}

fn validate_hash(
    field: &'static str,
    expected: Hash256,
    observed: Hash256,
) -> Result<(), S4BaselineError> {
    if expected == observed {
        Ok(())
    } else {
        Err(S4BaselineError::CorpusHashMismatch {
            field,
            expected,
            observed,
        })
    }
}

fn validate_finite_nonnegative(field: &'static str, value: f64) -> Result<(), S4BaselineError> {
    if value.is_finite() && value >= 0.0 {
        Ok(())
    } else {
        Err(S4BaselineError::NonFiniteOrNegative { field, value })
    }
}

fn serialize_discount_map<S>(
    discounts: &BTreeMap<u64, KnDiscounts>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let mut map = serializer.serialize_map(Some(discounts.len()))?;
    for (order, discounts) in discounts {
        map.serialize_entry(&order.to_string(), discounts)?;
    }
    map.end()
}

fn deserialize_discount_map<'de, D>(deserializer: D) -> Result<BTreeMap<u64, KnDiscounts>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = BTreeMap::<String, KnDiscounts>::deserialize(deserializer)?;
    let mut discounts = BTreeMap::new();
    for (order, value) in raw {
        let order = order.parse::<u64>().map_err(serde::de::Error::custom)?;
        discounts.insert(order, value);
    }
    Ok(discounts)
}

/// Errors from S4 Gutenberg baseline fitting and report validation.
#[derive(Debug)]
pub enum S4BaselineError {
    /// Product schema did not match `s4_baseline_gutenberg.v1`.
    InvalidSchema {
        /// Observed schema value.
        observed: String,
    },
    /// Caller-supplied split hash did not match the normalized corpus bytes.
    CorpusHashMismatch {
        /// Field name.
        field: &'static str,
        /// Expected caller-provided hash.
        expected: Hash256,
        /// Observed recomputed hash.
        observed: Hash256,
    },
    /// KN params drifted away from the inherited S3 pins.
    InvalidKnParams {
        /// Invalid field name.
        field: &'static str,
    },
    /// A BPC or numeric parameter was not finite and non-negative.
    NonFiniteOrNegative {
        /// Field name.
        field: &'static str,
        /// Observed value.
        value: f64,
    },
    /// Stored report self-hash differed from recomputation.
    SelfHashMismatch {
        /// Expected recomputed self-hash.
        expected: Hash256,
        /// Observed stored self-hash.
        observed: Hash256,
    },
    /// Self-hash computation expected a top-level object.
    ExpectedObjectForSelfHash,
    /// Inherited S3 baseline failed.
    S3Baseline(BaselineError),
    /// JSON serialization failed.
    Json(serde_json::Error),
    /// Canonical JSON serialization failed.
    CanonicalJson(gbf_foundation::CanonicalJsonError),
}

impl fmt::Display for S4BaselineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSchema { observed } => {
                write!(
                    f,
                    "expected s4_baseline_gutenberg.v1 schema, got {observed:?}"
                )
            }
            Self::CorpusHashMismatch {
                field,
                expected,
                observed,
            } => write!(
                f,
                "{field} mismatch for s4_baseline_gutenberg.v1: expected {expected}, observed {observed}"
            ),
            Self::InvalidKnParams { field } => {
                write!(f, "S4 Gutenberg baseline has invalid inherited {field}")
            }
            Self::NonFiniteOrNegative { field, value } => {
                write!(f, "{field} must be finite and non-negative, got {value}")
            }
            Self::SelfHashMismatch { expected, observed } => write!(
                f,
                "s4_baseline_gutenberg.v1 self-hash mismatch: expected {expected}, observed {observed}"
            ),
            Self::ExpectedObjectForSelfHash => {
                f.write_str("S4 Gutenberg baseline self-hash requires a top-level object")
            }
            Self::S3Baseline(error) => write!(f, "{error}"),
            Self::Json(error) => write!(f, "{error}"),
            Self::CanonicalJson(error) => write!(f, "{error}"),
        }
    }
}

impl Error for S4BaselineError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::S3Baseline(error) => Some(error),
            Self::Json(error) => Some(error),
            Self::CanonicalJson(error) => Some(error),
            _ => None,
        }
    }
}

impl From<BaselineError> for S4BaselineError {
    fn from(error: BaselineError) -> Self {
        Self::S3Baseline(error)
    }
}
