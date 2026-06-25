//! S7 telemetry schemas.
//!
//! `RouterStepTelemetry` is the S7 v0.2 experiment-local schema/helper used
//! for O12 subscriber proof. It is not, by itself, proof that a production
//! `gbf-train` loop emits the event.

use std::fmt;

use gbf_artifact::{S7SchemaError, S7ScoreReport, S7Topology};
use gbf_data::charset_v1::{CharsetError, normalize_raw};
use gbf_foundation::{CanonicalJson, CanonicalJsonError, DomainHash, Hash256, SemVer};
use serde::{Deserialize, Deserializer, Serialize};

use crate::S7_LOG_TARGET;

/// Public event name for per-step S7 router telemetry.
pub const ROUTER_STEP_TELEMETRY_EVENT: &str = "s7.router.step";

/// Public schema id for per-step S7 router telemetry.
pub const ROUTER_STEP_TELEMETRY_SCHEMA: &str = "s7_router_step_telemetry.v1";

/// Public schema id for the S7 one-token emulator/oracle comparison report.
pub const EMULATOR_ONE_TOKEN_SCHEMA: &str = "s7_emulator_one_token.v1";

/// Version carried by `RouterStepTelemetry::schema_version`.
pub const ROUTER_STEP_TELEMETRY_SCHEMA_VERSION: SemVer = SemVer::new(1, 0, 0);

/// Tolerance used by finite-range telemetry validation.
pub const ROUTER_STEP_TELEMETRY_EPSILON: f32 = 1.0e-6;

const ROUTER_STEP_TELEMETRY_SELF_HASH_FIELD: &str = "telemetry_self_hash";
const ROUTER_STEP_TELEMETRY_SCHEMA_VERSION_ID: &str = "1";
const EMULATOR_ONE_TOKEN_SELF_HASH_FIELD: &str = "emulator_self_hash";
const EMULATOR_ONE_TOKEN_SCHEMA_VERSION_ID: &str = "1";
const EMULATOR_ONE_TOKEN_EPSILON: f32 = 1.0e-6;

/// Return the S7 scoring token count after `charset_v1` normalization.
pub fn charset_v1_normalized_token_count(val_bytes: &[u8]) -> Result<u64, S7ScoreFromBytesError> {
    let normalized = normalize_raw(val_bytes)?;
    u64::try_from(normalized.tokens.len()).map_err(|_| S7ScoreFromBytesError::TokenCountOverflow)
}

/// Build an `s7_score.v1` report from raw validation bytes.
pub fn s7_score_report_from_val_bytes(
    seed: u64,
    topology: S7Topology,
    checkpoint_sha: Hash256,
    corpus_val_sha: Hash256,
    val_bytes: &[u8],
    log2_sum: f64,
) -> Result<S7ScoreReport, S7ScoreFromBytesError> {
    let token_count = charset_v1_normalized_token_count(val_bytes)?;
    Ok(S7ScoreReport::new(
        seed,
        topology,
        checkpoint_sha,
        corpus_val_sha,
        token_count,
        log2_sum,
    )?
    .with_computed_self_hash()?)
}

/// S7 H10 one-token emulator report compared against the artifact-oracle route
/// tracer for the same fixed prompt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmulatorOneTokenReport {
    /// Schema literal.
    pub schema: String,
    /// Experiment seed.
    pub seed: u64,
    /// S7 topology under test.
    pub topology: S7Topology,
    /// Encoded ROM hash used by the emulator.
    pub encoded_rom_sha: Hash256,
    /// Fixed prompt hash.
    pub prompt_sha: Hash256,
    /// Artifact-oracle logits hash for the same fixed prompt.
    pub artifact_oracle_logits_sha: Hash256,
    /// Emulator logits hash for the same fixed prompt.
    pub emulator_logits_sha: Hash256,
    /// Pairwise max absolute logit difference.
    pub pairwise_max_abs_diff: f64,
    /// S5 pinned tolerance for one-token output comparison.
    pub s5_tolerance: f64,
    /// Bank switches per token observed by the emulator.
    pub observed_bank_switches_per_token: f32,
    /// Bank switches per token computed by the artifact-oracle route tracer.
    pub oracle_recorded_bank_switches: f32,
    /// Absolute bank-switch difference.
    pub bank_switch_diff: f32,
    /// True when the H10 bank-switch comparison is within the permitted prefix
    /// correction, or forced true for dense topology where the assertion is N/A.
    pub bank_switch_within_one: bool,
    /// Self-hash over canonical report bytes with this field omitted.
    pub emulator_self_hash: Hash256,
}

impl EmulatorOneTokenReport {
    /// Domain used for canonical self-hashing.
    #[must_use]
    pub const fn domain() -> DomainHash<'static> {
        DomainHash::new(
            "gbf-experiments",
            "EmulatorOneTokenReport",
            EMULATOR_ONE_TOKEN_SCHEMA,
            EMULATOR_ONE_TOKEN_SCHEMA_VERSION_ID,
        )
    }

    /// Construct an H10 report from the artifact-oracle route tracer output and
    /// emulator observation for the same fixed prompt.
    #[allow(clippy::too_many_arguments)]
    pub fn from_artifact_oracle_trace(
        seed: u64,
        topology: S7Topology,
        encoded_rom_sha: Hash256,
        prompt_sha: Hash256,
        artifact_oracle_logits_sha: Hash256,
        emulator_logits_sha: Hash256,
        pairwise_max_abs_diff: f64,
        s5_tolerance: f64,
        observed_bank_switches_per_token: f32,
        oracle_recorded_bank_switches: f32,
        n_blocks: u32,
    ) -> Result<Self, EmulatorOneTokenReportError> {
        let bank_switch_diff =
            (observed_bank_switches_per_token - oracle_recorded_bank_switches).abs();
        let bank_switch_within_one = match &topology {
            S7Topology::MoeTiny => bank_switch_diff <= 1.0 + EMULATOR_ONE_TOKEN_EPSILON,
            S7Topology::MoeTinyDenseMatched => true,
        };
        let report = Self {
            schema: EMULATOR_ONE_TOKEN_SCHEMA.to_owned(),
            seed,
            topology,
            encoded_rom_sha,
            prompt_sha,
            artifact_oracle_logits_sha,
            emulator_logits_sha,
            pairwise_max_abs_diff,
            s5_tolerance,
            observed_bank_switches_per_token,
            oracle_recorded_bank_switches,
            bank_switch_diff,
            bank_switch_within_one,
            emulator_self_hash: Hash256::ZERO,
        };
        report.validate_with_n_blocks(n_blocks)?;
        report.with_computed_self_hash()
    }

    /// Validate schema-level invariants that do not need external deployment
    /// context.
    pub fn validate(&self) -> Result<(), EmulatorOneTokenReportError> {
        self.validate_common()?;
        validate_report_bank_switch_lower_bound(
            "observed_bank_switches_per_token",
            self.observed_bank_switches_per_token,
        )?;
        validate_report_bank_switch_lower_bound(
            "oracle_recorded_bank_switches",
            self.oracle_recorded_bank_switches,
        )
    }

    /// Validate schema invariants plus the bank-switch upper bound.
    pub fn validate_with_n_blocks(&self, n_blocks: u32) -> Result<(), EmulatorOneTokenReportError> {
        self.validate_common()?;
        validate_report_bank_switch_with_n_blocks(
            "observed_bank_switches_per_token",
            self.observed_bank_switches_per_token,
            n_blocks,
        )?;
        validate_report_bank_switch_with_n_blocks(
            "oracle_recorded_bank_switches",
            self.oracle_recorded_bank_switches,
            n_blocks,
        )
    }

    /// Compute the canonical self-hash.
    pub fn computed_self_hash(&self) -> Result<Hash256, EmulatorOneTokenReportError> {
        Ok(gbf_foundation::self_hash_omitting_fields(
            Self::domain(),
            self,
            EMULATOR_ONE_TOKEN_SELF_HASH_FIELD,
            &[],
        )?)
    }

    /// Return a copy with `emulator_self_hash` recomputed.
    pub fn with_computed_self_hash(mut self) -> Result<Self, EmulatorOneTokenReportError> {
        self.emulator_self_hash = self.computed_self_hash()?;
        Ok(self)
    }

    /// Verify that the stored self-hash matches the report payload.
    pub fn verify_self_hash(&self) -> Result<(), EmulatorOneTokenReportError> {
        let expected = self.computed_self_hash()?;
        if self.emulator_self_hash != expected {
            return Err(EmulatorOneTokenReportError::SelfHashMismatch {
                expected,
                observed: self.emulator_self_hash,
            });
        }
        Ok(())
    }

    /// Canonical JSON bytes for this report.
    pub fn canonical_json_bytes(&self) -> Result<Vec<u8>, EmulatorOneTokenReportError> {
        Ok(CanonicalJson::to_vec(self)?)
    }

    fn validate_common(&self) -> Result<(), EmulatorOneTokenReportError> {
        if self.schema != EMULATOR_ONE_TOKEN_SCHEMA {
            return Err(EmulatorOneTokenReportError::UnexpectedSchema {
                expected: EMULATOR_ONE_TOKEN_SCHEMA,
                observed: self.schema.clone(),
            });
        }
        validate_report_finite_nonnegative_f64(
            "pairwise_max_abs_diff",
            self.pairwise_max_abs_diff,
        )?;
        validate_report_finite_nonnegative_f64("s5_tolerance", self.s5_tolerance)?;
        if self.pairwise_max_abs_diff > self.s5_tolerance {
            return Err(
                EmulatorOneTokenReportError::PairwiseDiffExceedsS5Tolerance {
                    pairwise_max_abs_diff: self.pairwise_max_abs_diff,
                    s5_tolerance: self.s5_tolerance,
                },
            );
        }

        let expected_diff =
            (self.observed_bank_switches_per_token - self.oracle_recorded_bank_switches).abs();
        if (self.bank_switch_diff - expected_diff).abs() > EMULATOR_ONE_TOKEN_EPSILON {
            return Err(EmulatorOneTokenReportError::BankSwitchDiffMismatch {
                observed: self.bank_switch_diff,
                expected: expected_diff,
            });
        }

        let expected_within_one = match &self.topology {
            S7Topology::MoeTiny => self.bank_switch_diff <= 1.0 + EMULATOR_ONE_TOKEN_EPSILON,
            S7Topology::MoeTinyDenseMatched => true,
        };
        if self.bank_switch_within_one != expected_within_one {
            return Err(EmulatorOneTokenReportError::BankSwitchWithinOneMismatch {
                observed: self.bank_switch_within_one,
                expected: expected_within_one,
            });
        }
        Ok(())
    }
}

/// Summary of max-softmax router confidence values.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConfidenceDist {
    /// Mean max-softmax confidence.
    pub mean: f32,
    /// Tenth percentile max-softmax confidence.
    pub p10: f32,
    /// Median max-softmax confidence.
    pub p50: f32,
    /// Ninetieth percentile max-softmax confidence.
    pub p90: f32,
}

impl<'de> Deserialize<'de> for ConfidenceDist {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            mean: f32,
            p10: f32,
            p50: f32,
            p90: f32,
        }

        let raw = Raw::deserialize(deserializer)?;
        Self::new(raw.mean, raw.p10, raw.p50, raw.p90).map_err(serde::de::Error::custom)
    }
}

impl ConfidenceDist {
    /// Construct a validated confidence distribution.
    pub fn new(mean: f32, p10: f32, p50: f32, p90: f32) -> Result<Self, RouterTelemetryError> {
        let dist = Self {
            mean,
            p10,
            p50,
            p90,
        };
        dist.validate()?;
        Ok(dist)
    }

    /// Validate confidence bounds and quantile ordering.
    pub fn validate(&self) -> Result<(), RouterTelemetryError> {
        validate_unit_interval("router_confidence_distribution.mean", self.mean)?;
        validate_unit_interval("router_confidence_distribution.p10", self.p10)?;
        validate_unit_interval("router_confidence_distribution.p50", self.p50)?;
        validate_unit_interval("router_confidence_distribution.p90", self.p90)?;
        if self.p10 > self.p50 || self.p50 > self.p90 {
            return Err(RouterTelemetryError::ConfidenceQuantilesOutOfOrder {
                p10: self.p10,
                p50: self.p50,
                p90: self.p90,
            });
        }
        Ok(())
    }
}

/// Per-step S7 router telemetry.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RouterStepTelemetry {
    /// Schema version carried in serialized telemetry.
    pub schema_version: SemVer,
    /// Experiment seed.
    pub seed: u64,
    /// Training step that produced this sample.
    pub train_step: u64,
    /// Layer-local router id.
    pub layer_id: u32,
    /// Shannon entropy of expert usage, in bits.
    pub expert_usage_entropy_bits: f32,
    /// Fraction of adjacent routed tokens that stayed on the same expert.
    pub same_expert_rate: f32,
    /// Distribution of max-softmax router confidence.
    pub router_confidence_distribution: ConfidenceDist,
    /// Token counts per layer-local expert.
    pub tokens_per_expert: Vec<u32>,
    /// Deploy-metric bank switches per token.
    pub bank_switches_per_token: f32,
    /// Self-hash over canonical telemetry bytes with this field omitted.
    pub telemetry_self_hash: Hash256,
}

impl<'de> Deserialize<'de> for RouterStepTelemetry {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            schema_version: SemVer,
            seed: u64,
            train_step: u64,
            layer_id: u32,
            expert_usage_entropy_bits: f32,
            same_expert_rate: f32,
            router_confidence_distribution: ConfidenceDist,
            tokens_per_expert: Vec<u32>,
            bank_switches_per_token: f32,
            telemetry_self_hash: Hash256,
        }

        let raw = Raw::deserialize(deserializer)?;
        let telemetry = Self {
            schema_version: raw.schema_version,
            seed: raw.seed,
            train_step: raw.train_step,
            layer_id: raw.layer_id,
            expert_usage_entropy_bits: raw.expert_usage_entropy_bits,
            same_expert_rate: raw.same_expert_rate,
            router_confidence_distribution: raw.router_confidence_distribution,
            tokens_per_expert: raw.tokens_per_expert,
            bank_switches_per_token: raw.bank_switches_per_token,
            telemetry_self_hash: raw.telemetry_self_hash,
        };
        telemetry.validate().map_err(serde::de::Error::custom)?;
        telemetry
            .verify_self_hash()
            .map_err(serde::de::Error::custom)?;
        Ok(telemetry)
    }
}

impl RouterStepTelemetry {
    /// Domain used for canonical self-hashing.
    #[must_use]
    pub const fn domain() -> DomainHash<'static> {
        DomainHash::new(
            "gbf-experiments",
            "RouterStepTelemetry",
            ROUTER_STEP_TELEMETRY_SCHEMA,
            ROUTER_STEP_TELEMETRY_SCHEMA_VERSION_ID,
        )
    }

    /// Construct a telemetry event and compute `expert_usage_entropy_bits` from
    /// `tokens_per_expert`.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        seed: u64,
        train_step: u64,
        layer_id: u32,
        same_expert_rate: f32,
        router_confidence_distribution: ConfidenceDist,
        tokens_per_expert: Vec<u32>,
        bank_switches_per_token: f32,
        n_blocks: u32,
    ) -> Result<Self, RouterTelemetryError> {
        let expert_usage_entropy_bits = entropy_bits_from_counts(&tokens_per_expert)?;
        Self::from_computed_entropy_bits(
            seed,
            train_step,
            layer_id,
            expert_usage_entropy_bits,
            same_expert_rate,
            router_confidence_distribution,
            tokens_per_expert,
            bank_switches_per_token,
            n_blocks,
        )
    }

    /// Construct telemetry from an already computed entropy value.
    ///
    /// This exists for tests and future producer adapters that compute entropy
    /// upstream; validation still enforces bits-range invariants.
    #[allow(clippy::too_many_arguments)]
    pub fn from_computed_entropy_bits(
        seed: u64,
        train_step: u64,
        layer_id: u32,
        expert_usage_entropy_bits: f32,
        same_expert_rate: f32,
        router_confidence_distribution: ConfidenceDist,
        tokens_per_expert: Vec<u32>,
        bank_switches_per_token: f32,
        n_blocks: u32,
    ) -> Result<Self, RouterTelemetryError> {
        let telemetry = Self {
            schema_version: ROUTER_STEP_TELEMETRY_SCHEMA_VERSION,
            seed,
            train_step,
            layer_id,
            expert_usage_entropy_bits,
            same_expert_rate,
            router_confidence_distribution,
            tokens_per_expert,
            bank_switches_per_token,
            telemetry_self_hash: Hash256::ZERO,
        };
        telemetry.validate_with_n_blocks(n_blocks)?;
        telemetry.with_computed_self_hash()
    }

    /// Validate schema-level invariants that do not need external deployment
    /// context.
    pub fn validate(&self) -> Result<(), RouterTelemetryError> {
        self.validate_common()?;
        validate_bank_switches_lower_bound(self.bank_switches_per_token)
    }

    /// Validate schema-level invariants plus the bank-switch upper bound.
    pub fn validate_with_n_blocks(&self, n_blocks: u32) -> Result<(), RouterTelemetryError> {
        self.validate_common()?;
        validate_bank_switches_with_n_blocks(self.bank_switches_per_token, n_blocks)
    }

    /// Compute the canonical self-hash.
    pub fn computed_self_hash(&self) -> Result<Hash256, RouterTelemetryError> {
        Ok(gbf_foundation::self_hash_omitting_fields(
            Self::domain(),
            self,
            ROUTER_STEP_TELEMETRY_SELF_HASH_FIELD,
            &[],
        )?)
    }

    /// Return a copy with `telemetry_self_hash` recomputed.
    pub fn with_computed_self_hash(mut self) -> Result<Self, RouterTelemetryError> {
        self.telemetry_self_hash = self.computed_self_hash()?;
        Ok(self)
    }

    /// Verify that the stored self-hash matches the telemetry payload.
    pub fn verify_self_hash(&self) -> Result<(), RouterTelemetryError> {
        let expected = self.computed_self_hash()?;
        if self.telemetry_self_hash != expected {
            return Err(RouterTelemetryError::SelfHashMismatch {
                expected,
                observed: self.telemetry_self_hash,
            });
        }
        Ok(())
    }

    /// Canonical JSON bytes for this telemetry event.
    pub fn canonical_json_bytes(&self) -> Result<Vec<u8>, RouterTelemetryError> {
        Ok(CanonicalJson::to_vec(self)?)
    }

    /// Canonical JSON string for subscriber payload round-trip checks.
    pub fn canonical_json_string(&self) -> Result<String, RouterTelemetryError> {
        String::from_utf8(self.canonical_json_bytes()?).map_err(|error| {
            RouterTelemetryError::CanonicalJsonUtf8 {
                detail: error.to_string(),
            }
        })
    }

    /// Emit the telemetry through tracing using the RFC-pinned event name.
    pub fn emit_trace(&self) -> Result<(), RouterTelemetryError> {
        self.validate()?;
        self.verify_self_hash()?;
        let telemetry_canonical_json = self.canonical_json_string()?;
        tracing::info!(
            target: S7_LOG_TARGET,
            event_name = ROUTER_STEP_TELEMETRY_EVENT,
            schema_version_major = self.schema_version.major,
            schema_version_minor = self.schema_version.minor,
            schema_version_patch = self.schema_version.patch,
            seed = self.seed,
            train_step = self.train_step,
            layer_id = self.layer_id,
            expert_usage_entropy_bits = self.expert_usage_entropy_bits,
            same_expert_rate = self.same_expert_rate,
            router_confidence_distribution = ?self.router_confidence_distribution,
            router_confidence_mean = self.router_confidence_distribution.mean,
            router_confidence_p10 = self.router_confidence_distribution.p10,
            router_confidence_p50 = self.router_confidence_distribution.p50,
            router_confidence_p90 = self.router_confidence_distribution.p90,
            tokens_per_expert = ?self.tokens_per_expert,
            bank_switches_per_token = self.bank_switches_per_token,
            telemetry_self_hash = %self.telemetry_self_hash,
            telemetry_canonical_json = telemetry_canonical_json.as_str(),
            "s7 router step telemetry"
        );
        Ok(())
    }

    fn validate_common(&self) -> Result<(), RouterTelemetryError> {
        if self.schema_version != ROUTER_STEP_TELEMETRY_SCHEMA_VERSION {
            return Err(RouterTelemetryError::UnexpectedSchemaVersion {
                expected: ROUTER_STEP_TELEMETRY_SCHEMA_VERSION,
                observed: self.schema_version,
            });
        }
        validate_expert_counts(&self.tokens_per_expert)?;
        validate_entropy_bits(self.expert_usage_entropy_bits, self.tokens_per_expert.len())?;
        validate_unit_interval("same_expert_rate", self.same_expert_rate)?;
        self.router_confidence_distribution.validate()?;
        Ok(())
    }
}

/// Compute Shannon entropy in bits from expert usage counts.
pub fn entropy_bits_from_counts(tokens_per_expert: &[u32]) -> Result<f32, RouterTelemetryError> {
    let total = validate_expert_counts(tokens_per_expert)? as f32;
    let entropy_bits = tokens_per_expert
        .iter()
        .copied()
        .filter(|count| *count > 0)
        .map(|count| {
            let probability = count as f32 / total;
            -probability * probability.log2()
        })
        .sum::<f32>();
    validate_entropy_bits(entropy_bits, tokens_per_expert.len())?;
    Ok(entropy_bits)
}

/// Errors raised by S7 H10 emulator one-token report helpers.
#[derive(Debug)]
pub enum EmulatorOneTokenReportError {
    /// Canonical JSON serialization or hashing failed.
    CanonicalJson(CanonicalJsonError),
    /// The schema literal did not match the pinned H10 report schema.
    UnexpectedSchema {
        /// Expected schema literal.
        expected: &'static str,
        /// Observed schema literal.
        observed: String,
    },
    /// A numeric field was not finite.
    NonFiniteValue {
        /// Field name.
        field: &'static str,
        /// Observed value.
        value: f64,
    },
    /// A numeric field was negative.
    NegativeValue {
        /// Field name.
        field: &'static str,
        /// Observed value.
        value: f64,
    },
    /// Emulator/oracle logits differed by more than the S5 tolerance.
    PairwiseDiffExceedsS5Tolerance {
        /// Observed pairwise max absolute difference.
        pairwise_max_abs_diff: f64,
        /// S5 pinned tolerance.
        s5_tolerance: f64,
    },
    /// Bank-switch value exceeded the deployment block count.
    BankSwitchesExceedBlocks {
        /// Field name.
        field: &'static str,
        /// Observed value.
        value: f32,
        /// Number of deployment blocks.
        n_blocks: u32,
    },
    /// Stored bank-switch difference did not match `abs(observed - oracle)`.
    BankSwitchDiffMismatch {
        /// Observed stored difference.
        observed: f32,
        /// Expected computed difference.
        expected: f32,
    },
    /// Stored H10 bank-switch predicate did not match the schema rule.
    BankSwitchWithinOneMismatch {
        /// Observed stored predicate.
        observed: bool,
        /// Expected predicate.
        expected: bool,
    },
    /// The stored self-hash did not match the payload.
    SelfHashMismatch {
        /// Expected self-hash.
        expected: Hash256,
        /// Observed self-hash.
        observed: Hash256,
    },
}

impl fmt::Display for EmulatorOneTokenReportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CanonicalJson(error) => write!(f, "{error}"),
            Self::UnexpectedSchema { expected, observed } => write!(
                f,
                "unexpected emulator one-token schema: expected {expected}, observed {observed}"
            ),
            Self::NonFiniteValue { field, value } => {
                write!(f, "{field} must be finite, observed {value}")
            }
            Self::NegativeValue { field, value } => {
                write!(f, "{field} must be non-negative, observed {value}")
            }
            Self::PairwiseDiffExceedsS5Tolerance {
                pairwise_max_abs_diff,
                s5_tolerance,
            } => write!(
                f,
                "pairwise_max_abs_diff {pairwise_max_abs_diff} exceeds s5_tolerance {s5_tolerance}"
            ),
            Self::BankSwitchesExceedBlocks {
                field,
                value,
                n_blocks,
            } => write!(
                f,
                "{field} must be <= n_blocks ({n_blocks}), observed {value}"
            ),
            Self::BankSwitchDiffMismatch { observed, expected } => write!(
                f,
                "bank_switch_diff mismatch: expected {expected}, observed {observed}"
            ),
            Self::BankSwitchWithinOneMismatch { observed, expected } => write!(
                f,
                "bank_switch_within_one mismatch: expected {expected}, observed {observed}"
            ),
            Self::SelfHashMismatch { expected, observed } => write!(
                f,
                "emulator_self_hash mismatch: expected {expected}, observed {observed}"
            ),
        }
    }
}

impl std::error::Error for EmulatorOneTokenReportError {}

impl From<CanonicalJsonError> for EmulatorOneTokenReportError {
    fn from(error: CanonicalJsonError) -> Self {
        Self::CanonicalJson(error)
    }
}

/// Errors raised by S7 router telemetry schema helpers.
#[derive(Debug)]
pub enum RouterTelemetryError {
    /// Canonical JSON serialization or hashing failed.
    CanonicalJson(CanonicalJsonError),
    /// Canonical JSON bytes were not UTF-8.
    CanonicalJsonUtf8 {
        /// UTF-8 conversion detail.
        detail: String,
    },
    /// The stored schema version did not match the pinned schema.
    UnexpectedSchemaVersion {
        /// Expected schema version.
        expected: SemVer,
        /// Observed schema version.
        observed: SemVer,
    },
    /// At least one expert slot is required.
    EmptyExpertSet,
    /// At least one token must be routed in the telemetry window.
    ZeroTokenCount,
    /// Token counts overflowed the schema accumulator.
    TokenCountOverflow,
    /// The entropy scalar was not finite.
    NonFiniteEntropyBits {
        /// Observed entropy value.
        value: f32,
    },
    /// The entropy scalar was outside `[0, log2(n_experts)]`.
    EntropyBitsOutOfRange {
        /// Observed entropy value.
        value: f32,
        /// Maximum legal entropy in bits.
        max: f32,
    },
    /// A rate-like field was not finite.
    NonFiniteRate {
        /// Field name.
        field: &'static str,
        /// Observed value.
        value: f32,
    },
    /// A rate-like field was outside `[0, 1]`.
    RateOutOfRange {
        /// Field name.
        field: &'static str,
        /// Observed value.
        value: f32,
    },
    /// Confidence quantiles were not monotonic.
    ConfidenceQuantilesOutOfOrder {
        /// Tenth percentile.
        p10: f32,
        /// Median.
        p50: f32,
        /// Ninetieth percentile.
        p90: f32,
    },
    /// Bank switches per token was not finite.
    NonFiniteBankSwitches {
        /// Observed value.
        value: f32,
    },
    /// Bank switches per token was negative.
    NegativeBankSwitches {
        /// Observed value.
        value: f32,
    },
    /// Bank switches per token exceeded the deployment block count.
    BankSwitchesExceedBlocks {
        /// Observed value.
        value: f32,
        /// Number of deployment blocks.
        n_blocks: u32,
    },
    /// The stored self-hash did not match the payload.
    SelfHashMismatch {
        /// Expected self-hash.
        expected: Hash256,
        /// Observed self-hash.
        observed: Hash256,
    },
}

impl fmt::Display for RouterTelemetryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CanonicalJson(error) => write!(f, "{error}"),
            Self::CanonicalJsonUtf8 { detail } => {
                write!(f, "router telemetry canonical JSON was not UTF-8: {detail}")
            }
            Self::UnexpectedSchemaVersion { expected, observed } => write!(
                f,
                "unexpected router telemetry schema version: expected {expected}, observed {observed}"
            ),
            Self::EmptyExpertSet => f.write_str("router telemetry requires at least one expert"),
            Self::ZeroTokenCount => {
                f.write_str("router telemetry requires at least one routed token")
            }
            Self::TokenCountOverflow => f.write_str("router telemetry token counts overflowed"),
            Self::NonFiniteEntropyBits { value } => {
                write!(
                    f,
                    "expert_usage_entropy_bits must be finite, observed {value}"
                )
            }
            Self::EntropyBitsOutOfRange { value, max } => write!(
                f,
                "expert_usage_entropy_bits must be in [0, {max}], observed {value}"
            ),
            Self::NonFiniteRate { field, value } => {
                write!(f, "{field} must be finite, observed {value}")
            }
            Self::RateOutOfRange { field, value } => {
                write!(f, "{field} must be in [0, 1], observed {value}")
            }
            Self::ConfidenceQuantilesOutOfOrder { p10, p50, p90 } => write!(
                f,
                "router confidence quantiles must satisfy p10 <= p50 <= p90, observed {p10}, {p50}, {p90}"
            ),
            Self::NonFiniteBankSwitches { value } => {
                write!(
                    f,
                    "bank_switches_per_token must be finite, observed {value}"
                )
            }
            Self::NegativeBankSwitches { value } => {
                write!(
                    f,
                    "bank_switches_per_token must be non-negative, observed {value}"
                )
            }
            Self::BankSwitchesExceedBlocks { value, n_blocks } => write!(
                f,
                "bank_switches_per_token must be <= n_blocks ({n_blocks}), observed {value}"
            ),
            Self::SelfHashMismatch { expected, observed } => write!(
                f,
                "telemetry_self_hash mismatch: expected {expected}, observed {observed}"
            ),
        }
    }
}

impl std::error::Error for RouterTelemetryError {}

impl From<CanonicalJsonError> for RouterTelemetryError {
    fn from(error: CanonicalJsonError) -> Self {
        Self::CanonicalJson(error)
    }
}

/// Errors raised while deriving an S7 score report from raw validation bytes.
#[derive(Debug)]
pub enum S7ScoreFromBytesError {
    /// Raw validation bytes failed `charset_v1` normalization.
    Charset(
        /// Underlying charset normalization error.
        CharsetError,
    ),
    /// The derived score report violated the public S7 schema.
    Schema(
        /// Underlying public S7 schema validation error.
        S7SchemaError,
    ),
    /// The normalized token count could not fit in the public `u64` field.
    TokenCountOverflow,
}

impl fmt::Display for S7ScoreFromBytesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Charset(error) => write!(f, "{error}"),
            Self::Schema(error) => write!(f, "{error}"),
            Self::TokenCountOverflow => f.write_str("charset_v1 token count overflowed u64"),
        }
    }
}

impl std::error::Error for S7ScoreFromBytesError {}

impl From<CharsetError> for S7ScoreFromBytesError {
    fn from(error: CharsetError) -> Self {
        Self::Charset(error)
    }
}

impl From<S7SchemaError> for S7ScoreFromBytesError {
    fn from(error: S7SchemaError) -> Self {
        Self::Schema(error)
    }
}

fn validate_expert_counts(tokens_per_expert: &[u32]) -> Result<u64, RouterTelemetryError> {
    if tokens_per_expert.is_empty() {
        return Err(RouterTelemetryError::EmptyExpertSet);
    }
    let total = tokens_per_expert.iter().try_fold(0_u64, |total, count| {
        total
            .checked_add(u64::from(*count))
            .ok_or(RouterTelemetryError::TokenCountOverflow)
    })?;
    if total == 0 {
        return Err(RouterTelemetryError::ZeroTokenCount);
    }
    Ok(total)
}

fn validate_entropy_bits(value: f32, n_experts: usize) -> Result<(), RouterTelemetryError> {
    if !value.is_finite() {
        return Err(RouterTelemetryError::NonFiniteEntropyBits { value });
    }
    let max = (n_experts as f32).log2();
    if value < -ROUTER_STEP_TELEMETRY_EPSILON || value > max + ROUTER_STEP_TELEMETRY_EPSILON {
        return Err(RouterTelemetryError::EntropyBitsOutOfRange { value, max });
    }
    Ok(())
}

fn validate_unit_interval(field: &'static str, value: f32) -> Result<(), RouterTelemetryError> {
    if !value.is_finite() {
        return Err(RouterTelemetryError::NonFiniteRate { field, value });
    }
    if !(0.0..=1.0).contains(&value) {
        return Err(RouterTelemetryError::RateOutOfRange { field, value });
    }
    Ok(())
}

fn validate_bank_switches_lower_bound(value: f32) -> Result<(), RouterTelemetryError> {
    if !value.is_finite() {
        return Err(RouterTelemetryError::NonFiniteBankSwitches { value });
    }
    if value < 0.0 {
        return Err(RouterTelemetryError::NegativeBankSwitches { value });
    }
    Ok(())
}

fn validate_bank_switches_with_n_blocks(
    value: f32,
    n_blocks: u32,
) -> Result<(), RouterTelemetryError> {
    validate_bank_switches_lower_bound(value)?;
    if value > n_blocks as f32 + ROUTER_STEP_TELEMETRY_EPSILON {
        return Err(RouterTelemetryError::BankSwitchesExceedBlocks { value, n_blocks });
    }
    Ok(())
}

fn validate_report_finite_nonnegative_f64(
    field: &'static str,
    value: f64,
) -> Result<(), EmulatorOneTokenReportError> {
    if !value.is_finite() {
        return Err(EmulatorOneTokenReportError::NonFiniteValue { field, value });
    }
    if value < 0.0 {
        return Err(EmulatorOneTokenReportError::NegativeValue { field, value });
    }
    Ok(())
}

fn validate_report_bank_switch_lower_bound(
    field: &'static str,
    value: f32,
) -> Result<(), EmulatorOneTokenReportError> {
    if !value.is_finite() {
        return Err(EmulatorOneTokenReportError::NonFiniteValue {
            field,
            value: f64::from(value),
        });
    }
    if value < 0.0 {
        return Err(EmulatorOneTokenReportError::NegativeValue {
            field,
            value: f64::from(value),
        });
    }
    Ok(())
}

fn validate_report_bank_switch_with_n_blocks(
    field: &'static str,
    value: f32,
    n_blocks: u32,
) -> Result<(), EmulatorOneTokenReportError> {
    validate_report_bank_switch_lower_bound(field, value)?;
    if value > n_blocks as f32 + EMULATOR_ONE_TOKEN_EPSILON {
        return Err(EmulatorOneTokenReportError::BankSwitchesExceedBlocks {
            field,
            value,
            n_blocks,
        });
    }
    Ok(())
}
