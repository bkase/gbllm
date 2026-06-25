//! F-S7 public artifact schemas.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use gbf_foundation::{
    CanonicalJson, CanonicalJsonError, DomainHash, ExpertId, Hash256, LayerId, SemVer,
    self_hash_omitting_fields,
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::matched_bytes_pin::MatchedBytesPin;
use crate::quant::QuantSpec;

pub const S7_SCHEMA_VERSION: SemVer = SemVer::new(1, 0, 0);
pub const S7_N_BLOCKS: u16 = 4;
pub const S7_N_EXPERTS: u16 = 4;
pub const S7_OPTIMIZER_STEPS: u64 = 20_000;
pub const S7_EVAL_EVERY_STEPS: u64 = 1_000;
pub const S7_SCORE_CHUNK_SIZE: u64 = 256;

pub const S7_RUN_LOG_SCHEMA: &str = "s7_run_log.v1";
pub const S7_SCORE_SCHEMA: &str = "s7_score.v1";
pub const S7_TEMPORAL_SWITCH_DIGEST_SCHEMA: &str = "s7_temporal_switch_digest.v1";
pub const S7_EXPERT_SLOT_AFFINITY_SCHEMA: &str = "s7_expert_slot_affinity.v1";
pub const S7_CLIP_SATURATION_DIGEST_SCHEMA: &str = "s7_clip_saturation_digest.v1";
pub const S7_EXPERT_PAYLOAD_DIGEST_SCHEMA: &str = "s7_expert_payload_digest.v1";
pub const S7_DENSE_VS_MOE_SCHEMA: &str = "s7_dense_vs_moe.v1";
pub const S7_FRONTIER_SCHEMA: &str = "s7_frontier.v1";

const S7_SCHEMA_DOMAIN_VERSION: &str = "1";
const Q8_8_ONE: u16 = 256;
const SCORE_BPC_EPSILON: f64 = 1.0e-12;
const S7_DENSE_VS_MOE_SEED_COUNT: usize = 5;
const S7_PARITY_BPC_MARGIN: f64 = 0.05;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum S7Topology {
    MoeTiny,
    MoeTinyDenseMatched,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParetoVerdict {
    #[serde(rename = "MoE-dominates")]
    MoeDominates,
    #[serde(rename = "dense-dominates")]
    DenseDominates,
    #[serde(rename = "MoE-wins-under-byte-equivalence")]
    MoeWinsUnderByteEquivalence,
    #[serde(rename = "Dense-wins-under-byte-equivalence")]
    DenseWinsUnderByteEquivalence,
    Incomparable,
    Tied,
}

impl ParetoVerdict {
    #[must_use]
    pub const fn confirms_h4(self) -> bool {
        matches!(self, Self::MoeDominates | Self::MoeWinsUnderByteEquivalence)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum S7ParityVerdict {
    Pass,
    Fail,
}

impl S7ParityVerdict {
    #[must_use]
    pub const fn passed(self) -> bool {
        matches!(self, Self::Pass)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum S7AggregateParityVerdict {
    #[serde(rename = "Pass-clean")]
    PassClean,
    #[serde(rename = "Fail-parity")]
    FailParity,
    #[serde(rename = "Fail-bytes")]
    FailBytes,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct S7DenseVsMoeParetoFields {
    pub schema: String,
    pub pareto_verdict: ParetoVerdict,
}

impl<'de> Deserialize<'de> for S7DenseVsMoeParetoFields {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            schema: String,
            pareto_verdict: ParetoVerdict,
        }

        let raw = Raw::deserialize(deserializer)?;
        Self::new(raw.pareto_verdict)
            .map(|mut fields| {
                fields.schema = raw.schema;
                fields
            })
            .and_then(|fields| {
                fields.validate()?;
                Ok(fields)
            })
            .map_err(serde::de::Error::custom)
    }
}

impl S7DenseVsMoeParetoFields {
    pub fn new(pareto_verdict: ParetoVerdict) -> Result<Self, S7SchemaError> {
        let fields = Self {
            schema: S7_DENSE_VS_MOE_SCHEMA.to_owned(),
            pareto_verdict,
        };
        fields.validate()?;
        Ok(fields)
    }

    pub fn validate(&self) -> Result<(), S7SchemaError> {
        validate_schema_literal("schema", &self.schema, S7_DENSE_VS_MOE_SCHEMA)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct S7FrontierParetoFields {
    pub schema: String,
    pub pareto_verdict: ParetoVerdict,
}

impl<'de> Deserialize<'de> for S7FrontierParetoFields {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            schema: String,
            pareto_verdict: ParetoVerdict,
        }

        let raw = Raw::deserialize(deserializer)?;
        Self::new(raw.pareto_verdict)
            .map(|mut fields| {
                fields.schema = raw.schema;
                fields
            })
            .and_then(|fields| {
                fields.validate()?;
                Ok(fields)
            })
            .map_err(serde::de::Error::custom)
    }
}

impl S7FrontierParetoFields {
    pub fn new(pareto_verdict: ParetoVerdict) -> Result<Self, S7SchemaError> {
        let fields = Self {
            schema: S7_FRONTIER_SCHEMA.to_owned(),
            pareto_verdict,
        };
        fields.validate()?;
        Ok(fields)
    }

    pub fn validate(&self) -> Result<(), S7SchemaError> {
        validate_schema_literal("schema", &self.schema, S7_FRONTIER_SCHEMA)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct S7ProjectedFit {
    pub deployed_bytes_total: u64,
    pub deployed_bytes_per_block: Vec<u64>,
}

impl<'de> Deserialize<'de> for S7ProjectedFit {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            deployed_bytes_total: u64,
            deployed_bytes_per_block: Vec<u64>,
        }

        let raw = Raw::deserialize(deserializer)?;
        Self::new(raw.deployed_bytes_total, raw.deployed_bytes_per_block)
            .map_err(serde::de::Error::custom)
    }
}

impl S7ProjectedFit {
    pub fn new(
        deployed_bytes_total: u64,
        deployed_bytes_per_block: Vec<u64>,
    ) -> Result<Self, S7SchemaError> {
        let fit = Self {
            deployed_bytes_total,
            deployed_bytes_per_block,
        };
        fit.validate()?;
        Ok(fit)
    }

    pub fn validate(&self) -> Result<(), S7SchemaError> {
        if self.deployed_bytes_per_block.len() != usize::from(S7_N_BLOCKS) {
            return Err(S7SchemaError::ProjectedFitBlockCountMismatch {
                observed: self.deployed_bytes_per_block.len(),
                expected: S7_N_BLOCKS,
            });
        }
        let block_total = self
            .deployed_bytes_per_block
            .iter()
            .try_fold(0_u64, |total, value| total.checked_add(*value))
            .ok_or(S7SchemaError::LengthOverflow)?;
        if block_total != self.deployed_bytes_total {
            return Err(S7SchemaError::ProjectedFitTotalMismatch {
                observed: self.deployed_bytes_total,
                expected: block_total,
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LambdaSwitch(String);

impl LambdaSwitch {
    pub fn new(value: impl Into<String>) -> Result<Self, S7SchemaError> {
        let value = value.into();
        validate_lambda_switch_text(&value)?;
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn validate(&self) -> Result<(), S7SchemaError> {
        validate_lambda_switch_text(&self.0)
    }
}

impl Serialize for LambdaSwitch {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for LambdaSwitch {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

/// H6 router-collapse guardrail verdict carried in the dense-vs-MoE summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum GuardrailVerdict {
    Pass,
    FailA,
    FailB,
    FailC,
    FailD,
    InconclusiveDiverged {
        lambda_switch: LambdaSwitch,
        step: u64,
    },
}

impl GuardrailVerdict {
    fn validate(&self) -> Result<(), S7SchemaError> {
        if let Self::InconclusiveDiverged { lambda_switch, .. } = self {
            lambda_switch.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SwitchStatsSummary {
    pub same_expert_rate_per_layer_q8_8: Vec<u16>,
    pub expert_usage_entropy_bits_mean: f32,
    pub bank_switches_per_token_mean: f32,
}

impl<'de> Deserialize<'de> for SwitchStatsSummary {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            same_expert_rate_per_layer_q8_8: Vec<u16>,
            expert_usage_entropy_bits_mean: f32,
            bank_switches_per_token_mean: f32,
        }

        let raw = Raw::deserialize(deserializer)?;
        Self::new(
            raw.same_expert_rate_per_layer_q8_8,
            raw.expert_usage_entropy_bits_mean,
            raw.bank_switches_per_token_mean,
        )
        .map_err(serde::de::Error::custom)
    }
}

impl SwitchStatsSummary {
    pub fn new(
        same_expert_rate_per_layer_q8_8: Vec<u16>,
        expert_usage_entropy_bits_mean: f32,
        bank_switches_per_token_mean: f32,
    ) -> Result<Self, S7SchemaError> {
        let summary = Self {
            same_expert_rate_per_layer_q8_8,
            expert_usage_entropy_bits_mean,
            bank_switches_per_token_mean,
        };
        summary.validate()?;
        Ok(summary)
    }

    pub fn validate(&self) -> Result<(), S7SchemaError> {
        if self.same_expert_rate_per_layer_q8_8.len() != usize::from(S7_N_BLOCKS) {
            return Err(S7SchemaError::SwitchStatsLayerCountMismatch {
                observed: self.same_expert_rate_per_layer_q8_8.len(),
                expected: S7_N_BLOCKS,
            });
        }
        for value in &self.same_expert_rate_per_layer_q8_8 {
            validate_q8_8("same_expert_rate_per_layer_q8_8", *value)?;
        }
        validate_finite_nonnegative_f32(
            "expert_usage_entropy_bits_mean",
            self.expert_usage_entropy_bits_mean,
        )?;
        validate_finite_nonnegative_f32(
            "bank_switches_per_token_mean",
            self.bank_switches_per_token_mean,
        )?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SweepSummary {
    pub bpc_at_lambda: BTreeMap<LambdaSwitch, f64>,
    pub entropy_at_lambda: BTreeMap<LambdaSwitch, f32>,
    pub guardrail_verdict: GuardrailVerdict,
}

impl<'de> Deserialize<'de> for SweepSummary {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            bpc_at_lambda: BTreeMap<LambdaSwitch, f64>,
            entropy_at_lambda: BTreeMap<LambdaSwitch, f32>,
            guardrail_verdict: GuardrailVerdict,
        }

        let raw = Raw::deserialize(deserializer)?;
        Self::new(
            raw.bpc_at_lambda,
            raw.entropy_at_lambda,
            raw.guardrail_verdict,
        )
        .map_err(serde::de::Error::custom)
    }
}

impl SweepSummary {
    pub fn new(
        bpc_at_lambda: BTreeMap<LambdaSwitch, f64>,
        entropy_at_lambda: BTreeMap<LambdaSwitch, f32>,
        guardrail_verdict: GuardrailVerdict,
    ) -> Result<Self, S7SchemaError> {
        let summary = Self {
            bpc_at_lambda,
            entropy_at_lambda,
            guardrail_verdict,
        };
        summary.validate()?;
        Ok(summary)
    }

    pub fn validate(&self) -> Result<(), S7SchemaError> {
        validate_nonempty_lambda_map("bpc_at_lambda", &self.bpc_at_lambda)?;
        validate_nonempty_lambda_map("entropy_at_lambda", &self.entropy_at_lambda)?;
        if self.bpc_at_lambda.keys().ne(self.entropy_at_lambda.keys()) {
            return Err(S7SchemaError::SweepSummaryLambdaSetMismatch);
        }
        for (lambda_switch, value) in &self.bpc_at_lambda {
            lambda_switch.validate()?;
            validate_finite_nonnegative_f64("bpc_at_lambda", *value)?;
        }
        for (lambda_switch, value) in &self.entropy_at_lambda {
            lambda_switch.validate()?;
            validate_finite_nonnegative_f32("entropy_at_lambda", *value)?;
        }
        self.guardrail_verdict.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct S7PerSeedComparison {
    pub seed: u64,
    pub val_bpc_moe: f64,
    pub val_bpc_dense: f64,
    pub delta: f64,
    pub parity_verdict: S7ParityVerdict,
}

impl<'de> Deserialize<'de> for S7PerSeedComparison {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            seed: u64,
            val_bpc_moe: f64,
            val_bpc_dense: f64,
            delta: f64,
            parity_verdict: S7ParityVerdict,
        }

        let raw = Raw::deserialize(deserializer)?;
        Self::new(
            raw.seed,
            raw.val_bpc_moe,
            raw.val_bpc_dense,
            raw.delta,
            raw.parity_verdict,
        )
        .map_err(serde::de::Error::custom)
    }
}

impl S7PerSeedComparison {
    pub fn new(
        seed: u64,
        val_bpc_moe: f64,
        val_bpc_dense: f64,
        delta: f64,
        parity_verdict: S7ParityVerdict,
    ) -> Result<Self, S7SchemaError> {
        let comparison = Self {
            seed,
            val_bpc_moe,
            val_bpc_dense,
            delta,
            parity_verdict,
        };
        comparison.validate()?;
        Ok(comparison)
    }

    pub fn validate(&self) -> Result<(), S7SchemaError> {
        validate_finite_nonnegative_f64("per_seed.val_bpc_moe", self.val_bpc_moe)?;
        validate_finite_nonnegative_f64("per_seed.val_bpc_dense", self.val_bpc_dense)?;
        validate_finite_f64("per_seed.delta", self.delta)?;
        let expected_delta = self.val_bpc_dense - self.val_bpc_moe;
        if !f64_close(self.delta, expected_delta) {
            return Err(S7SchemaError::PerSeedDeltaMismatch {
                seed: self.seed,
                observed: self.delta,
                expected: expected_delta,
            });
        }
        let expected_parity = derive_parity_verdict(self.val_bpc_moe, self.val_bpc_dense);
        if self.parity_verdict != expected_parity {
            return Err(S7SchemaError::PerSeedParityVerdictMismatch {
                seed: self.seed,
                observed: self.parity_verdict,
                expected: expected_parity,
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct S7DenseVsMoeComparisonReport {
    pub schema: String,
    pub moe_topology_hash: Hash256,
    pub dense_matched_topology_hash: Hash256,
    pub matched_bytes_pin: MatchedBytesPin,
    pub per_seed: Vec<S7PerSeedComparison>,
    pub median_val_bpc_moe: f64,
    pub median_val_bpc_dense: f64,
    pub deployed_bytes_total_moe: u64,
    pub deployed_bytes_total_dense: u64,
    pub bytes_diff: i64,
    pub bytes_within_tolerance: bool,
    pub aggregate_parity_verdict: S7AggregateParityVerdict,
    pub pareto_verdict: ParetoVerdict,
    pub switch_stats_summary: SwitchStatsSummary,
    pub sweep_summary: SweepSummary,
    pub comparison_self_hash: Hash256,
}

impl<'de> Deserialize<'de> for S7DenseVsMoeComparisonReport {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            schema: String,
            moe_topology_hash: Hash256,
            dense_matched_topology_hash: Hash256,
            matched_bytes_pin: MatchedBytesPin,
            per_seed: Vec<S7PerSeedComparison>,
            median_val_bpc_moe: f64,
            median_val_bpc_dense: f64,
            deployed_bytes_total_moe: u64,
            deployed_bytes_total_dense: u64,
            bytes_diff: i64,
            bytes_within_tolerance: bool,
            aggregate_parity_verdict: S7AggregateParityVerdict,
            pareto_verdict: ParetoVerdict,
            switch_stats_summary: SwitchStatsSummary,
            sweep_summary: SweepSummary,
            comparison_self_hash: Hash256,
        }

        let raw = Raw::deserialize(deserializer)?;
        let report = Self {
            schema: raw.schema,
            moe_topology_hash: raw.moe_topology_hash,
            dense_matched_topology_hash: raw.dense_matched_topology_hash,
            matched_bytes_pin: raw.matched_bytes_pin,
            per_seed: raw.per_seed,
            median_val_bpc_moe: raw.median_val_bpc_moe,
            median_val_bpc_dense: raw.median_val_bpc_dense,
            deployed_bytes_total_moe: raw.deployed_bytes_total_moe,
            deployed_bytes_total_dense: raw.deployed_bytes_total_dense,
            bytes_diff: raw.bytes_diff,
            bytes_within_tolerance: raw.bytes_within_tolerance,
            aggregate_parity_verdict: raw.aggregate_parity_verdict,
            pareto_verdict: raw.pareto_verdict,
            switch_stats_summary: raw.switch_stats_summary,
            sweep_summary: raw.sweep_summary,
            comparison_self_hash: raw.comparison_self_hash,
        };
        report.validate().map_err(serde::de::Error::custom)?;
        Ok(report)
    }
}

impl S7DenseVsMoeComparisonReport {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        moe_topology_hash: Hash256,
        dense_matched_topology_hash: Hash256,
        matched_bytes_pin: MatchedBytesPin,
        per_seed: Vec<S7PerSeedComparison>,
        median_val_bpc_moe: f64,
        median_val_bpc_dense: f64,
        deployed_bytes_total_moe: u64,
        deployed_bytes_total_dense: u64,
        bytes_diff: i64,
        bytes_within_tolerance: bool,
        aggregate_parity_verdict: S7AggregateParityVerdict,
        pareto_verdict: ParetoVerdict,
        switch_stats_summary: SwitchStatsSummary,
        sweep_summary: SweepSummary,
    ) -> Result<Self, S7SchemaError> {
        let report = Self {
            schema: S7_DENSE_VS_MOE_SCHEMA.to_owned(),
            moe_topology_hash,
            dense_matched_topology_hash,
            matched_bytes_pin,
            per_seed,
            median_val_bpc_moe,
            median_val_bpc_dense,
            deployed_bytes_total_moe,
            deployed_bytes_total_dense,
            bytes_diff,
            bytes_within_tolerance,
            aggregate_parity_verdict,
            pareto_verdict,
            switch_stats_summary,
            sweep_summary,
            comparison_self_hash: Hash256::ZERO,
        };
        report.validate()?;
        Ok(report)
    }

    pub fn validate(&self) -> Result<(), S7SchemaError> {
        validate_schema_literal("schema", &self.schema, S7_DENSE_VS_MOE_SCHEMA)?;
        self.matched_bytes_pin
            .verify_self_hash()
            .map_err(|_| S7SchemaError::MatchedBytesPinSelfHashInvalid)?;
        validate_dense_vs_moe_per_seed(&self.per_seed)?;
        validate_finite_nonnegative_f64("median_val_bpc_moe", self.median_val_bpc_moe)?;
        validate_finite_nonnegative_f64("median_val_bpc_dense", self.median_val_bpc_dense)?;
        validate_reported_median(
            "median_val_bpc_moe",
            self.median_val_bpc_moe,
            median_bpc(&self.per_seed, |entry| entry.val_bpc_moe),
        )?;
        validate_reported_median(
            "median_val_bpc_dense",
            self.median_val_bpc_dense,
            median_bpc(&self.per_seed, |entry| entry.val_bpc_dense),
        )?;
        validate_deployed_bytes_total(
            "matched_bytes_pin.b_deployed_total_moe",
            self.matched_bytes_pin.b_deployed_total_moe,
            self.deployed_bytes_total_moe,
        )?;
        validate_deployed_bytes_total(
            "matched_bytes_pin.b_deployed_total_dense",
            self.matched_bytes_pin.b_deployed_total_dense,
            self.deployed_bytes_total_dense,
        )?;
        let expected_bytes_diff = checked_signed_bytes_diff(
            self.deployed_bytes_total_dense,
            self.deployed_bytes_total_moe,
        )?;
        if self.bytes_diff != expected_bytes_diff {
            return Err(S7SchemaError::BytesDiffMismatch {
                observed: self.bytes_diff,
                expected: expected_bytes_diff,
            });
        }
        let expected_within_tolerance = self
            .deployed_bytes_total_dense
            .abs_diff(self.deployed_bytes_total_moe)
            <= self.matched_bytes_pin.tolerance_bytes;
        if self.bytes_within_tolerance != expected_within_tolerance {
            return Err(S7SchemaError::BytesWithinToleranceMismatch {
                observed: self.bytes_within_tolerance,
                expected: expected_within_tolerance,
            });
        }
        let expected_aggregate = derive_aggregate_parity_verdict(
            &self.per_seed,
            self.deployed_bytes_total_dense
                .abs_diff(self.deployed_bytes_total_moe),
            self.matched_bytes_pin.tolerance_bytes,
        );
        if self.aggregate_parity_verdict != expected_aggregate {
            return Err(S7SchemaError::AggregateParityVerdictMismatch {
                observed: self.aggregate_parity_verdict,
                expected: expected_aggregate,
            });
        }
        let expected_pareto = derive_pareto_verdict(
            self.median_val_bpc_moe,
            self.median_val_bpc_dense,
            self.deployed_bytes_total_moe,
            self.deployed_bytes_total_dense,
            self.matched_bytes_pin.tolerance_bytes,
        );
        if self.pareto_verdict != expected_pareto {
            return Err(S7SchemaError::ParetoVerdictMismatch {
                observed: self.pareto_verdict,
                expected: expected_pareto,
            });
        }
        self.switch_stats_summary.validate()?;
        self.sweep_summary.validate()?;
        Ok(())
    }

    pub fn computed_self_hash(&self) -> Result<Hash256, S7SchemaError> {
        self.validate()?;
        Ok(self_hash_omitting_fields(
            Self::domain(),
            self,
            "comparison_self_hash",
            &[],
        )?)
    }

    pub fn with_computed_self_hash(mut self) -> Result<Self, S7SchemaError> {
        self.comparison_self_hash = self.computed_self_hash()?;
        Ok(self)
    }

    pub fn verify_self_hash(&self) -> Result<(), S7SchemaError> {
        let expected = self.computed_self_hash()?;
        if self.comparison_self_hash != expected {
            return Err(S7SchemaError::SelfHashMismatch {
                field: "comparison_self_hash",
                expected,
                observed: self.comparison_self_hash,
            });
        }
        Ok(())
    }

    pub fn canonical_json_bytes(&self) -> Result<Vec<u8>, S7SchemaError> {
        self.validate()?;
        Ok(CanonicalJson::to_vec(self)?)
    }

    #[must_use]
    pub const fn domain() -> DomainHash<'static> {
        DomainHash::new(
            "gbf-artifact",
            "S7DenseVsMoeComparisonReport",
            S7_DENSE_VS_MOE_SCHEMA,
            S7_SCHEMA_DOMAIN_VERSION,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum S7Completion {
    Completed,
    DivergedAt { step: u64 },
    CollapsedAt { step: u64 },
}

impl S7Completion {
    fn expected_loss_steps(&self) -> Result<u64, S7SchemaError> {
        match self {
            Self::Completed => Ok(S7_OPTIMIZER_STEPS),
            Self::DivergedAt { step } | Self::CollapsedAt { step } => {
                if *step == 0 || *step > S7_OPTIMIZER_STEPS {
                    return Err(S7SchemaError::InvalidCompletionStep { step: *step });
                }
                Ok(*step)
            }
        }
    }

    fn expected_eval_points(&self) -> Result<usize, S7SchemaError> {
        let last_step = self.expected_loss_steps()?;
        let count = (last_step / S7_EVAL_EVERY_STEPS) + 1;
        usize::try_from(count).map_err(|_| S7SchemaError::LengthOverflow)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GradNormSummary {
    pub global_l2: f32,
    pub max_l2: f32,
    pub mean_l2: f32,
}

impl<'de> Deserialize<'de> for GradNormSummary {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            global_l2: f32,
            max_l2: f32,
            mean_l2: f32,
        }

        let raw = Raw::deserialize(deserializer)?;
        Self::new(raw.global_l2, raw.max_l2, raw.mean_l2).map_err(serde::de::Error::custom)
    }
}

impl GradNormSummary {
    pub fn new(global_l2: f32, max_l2: f32, mean_l2: f32) -> Result<Self, S7SchemaError> {
        let summary = Self {
            global_l2,
            max_l2,
            mean_l2,
        };
        summary.validate()?;
        Ok(summary)
    }

    pub fn validate(&self) -> Result<(), S7SchemaError> {
        validate_finite_nonnegative_f32("global_l2", self.global_l2)?;
        validate_finite_nonnegative_f32("max_l2", self.max_l2)?;
        validate_finite_nonnegative_f32("mean_l2", self.mean_l2)?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RawLossDiagnostics {
    pub lm_loss_raw: f32,
    pub distill_loss_raw: DistillRawDiagnostic,
    pub balance_loss_raw: f32,
    pub zrouter_loss_raw: f32,
    pub switch_loss_raw: f32,
    pub diagnostics_self_hash: Hash256,
}

impl<'de> Deserialize<'de> for RawLossDiagnostics {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            lm_loss_raw: f32,
            distill_loss_raw: DistillRawDiagnostic,
            balance_loss_raw: f32,
            zrouter_loss_raw: f32,
            switch_loss_raw: f32,
            diagnostics_self_hash: Hash256,
        }

        let raw = Raw::deserialize(deserializer)?;
        Self::new(
            raw.lm_loss_raw,
            raw.distill_loss_raw,
            raw.balance_loss_raw,
            raw.zrouter_loss_raw,
            raw.switch_loss_raw,
        )
        .map(|mut diagnostics| {
            diagnostics.diagnostics_self_hash = raw.diagnostics_self_hash;
            diagnostics
        })
        .map_err(serde::de::Error::custom)
    }
}

impl RawLossDiagnostics {
    pub fn new(
        lm_loss_raw: f32,
        distill_loss_raw: DistillRawDiagnostic,
        balance_loss_raw: f32,
        zrouter_loss_raw: f32,
        switch_loss_raw: f32,
    ) -> Result<Self, S7SchemaError> {
        validate_finite_nonnegative_f32("lm_loss_raw", lm_loss_raw)?;
        distill_loss_raw.validate()?;
        validate_finite_nonnegative_f32("balance_loss_raw", balance_loss_raw)?;
        validate_finite_nonnegative_f32("zrouter_loss_raw", zrouter_loss_raw)?;
        validate_unit_f32("switch_loss_raw", switch_loss_raw)?;
        Ok(Self {
            lm_loss_raw,
            distill_loss_raw,
            balance_loss_raw,
            zrouter_loss_raw,
            switch_loss_raw,
            diagnostics_self_hash: Hash256::ZERO,
        })
    }

    pub fn validate(&self) -> Result<(), S7SchemaError> {
        validate_finite_nonnegative_f32("lm_loss_raw", self.lm_loss_raw)?;
        self.distill_loss_raw.validate()?;
        validate_finite_nonnegative_f32("balance_loss_raw", self.balance_loss_raw)?;
        validate_finite_nonnegative_f32("zrouter_loss_raw", self.zrouter_loss_raw)?;
        validate_unit_f32("switch_loss_raw", self.switch_loss_raw)?;
        Ok(())
    }

    pub fn computed_self_hash(&self) -> Result<Hash256, S7SchemaError> {
        Ok(self_hash_omitting_fields(
            Self::domain(),
            self,
            "diagnostics_self_hash",
            &[],
        )?)
    }

    pub fn with_computed_self_hash(mut self) -> Result<Self, S7SchemaError> {
        self.diagnostics_self_hash = self.computed_self_hash()?;
        Ok(self)
    }

    pub fn canonical_json_bytes(&self) -> Result<Vec<u8>, S7SchemaError> {
        Ok(CanonicalJson::to_vec(self)?)
    }

    #[must_use]
    pub const fn domain() -> DomainHash<'static> {
        DomainHash::new(
            "gbf-artifact",
            "RawLossDiagnostics",
            "s7_raw_loss_diagnostics.v1",
            S7_SCHEMA_DOMAIN_VERSION,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DistillRawDiagnostic {
    NotAvailable { reason: String, phase: TrainPhase },
    Value { loss: f32 },
}

impl DistillRawDiagnostic {
    fn validate(&self) -> Result<(), S7SchemaError> {
        match self {
            Self::NotAvailable { reason, .. } => {
                if reason.is_empty() {
                    return Err(S7SchemaError::EmptyReason);
                }
            }
            Self::Value { loss } => validate_finite_nonnegative_f32("distill_loss_raw", *loss)?,
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrainPhase {
    PhaseA,
    PhaseB,
    PhaseC,
    PhaseD,
    PhaseE,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TransitionEntry {
    pub from_expert: ExpertId,
    pub to_expert: ExpertId,
    pub mass_q8_8: u16,
}

impl<'de> Deserialize<'de> for TransitionEntry {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            from_expert: ExpertId,
            to_expert: ExpertId,
            mass_q8_8: u16,
        }

        let raw = Raw::deserialize(deserializer)?;
        Self::new(raw.from_expert, raw.to_expert, raw.mass_q8_8).map_err(serde::de::Error::custom)
    }
}

impl TransitionEntry {
    pub fn new(
        from_expert: ExpertId,
        to_expert: ExpertId,
        mass_q8_8: u16,
    ) -> Result<Self, S7SchemaError> {
        validate_expert_id(from_expert, S7_N_EXPERTS)?;
        validate_expert_id(to_expert, S7_N_EXPERTS)?;
        validate_q8_8("transition_mass.mass_q8_8", mass_q8_8)?;
        Ok(Self {
            from_expert,
            to_expert,
            mass_q8_8,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TemporalSwitchDigest {
    pub schema_version: SemVer,
    pub layer_id: LayerId,
    pub n_experts: u16,
    pub same_expert_rate_q8_8: u16,
    pub transition_mass: Vec<TransitionEntry>,
    pub digest_self_hash: Hash256,
}

impl<'de> Deserialize<'de> for TemporalSwitchDigest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            schema_version: SemVer,
            layer_id: LayerId,
            n_experts: u16,
            same_expert_rate_q8_8: u16,
            transition_mass: Vec<TransitionEntry>,
            digest_self_hash: Hash256,
        }

        let raw = Raw::deserialize(deserializer)?;
        Self::new(
            raw.layer_id,
            raw.n_experts,
            raw.same_expert_rate_q8_8,
            raw.transition_mass,
        )
        .map(|mut digest| {
            digest.schema_version = raw.schema_version;
            digest.digest_self_hash = raw.digest_self_hash;
            digest
        })
        .and_then(|digest| {
            digest.validate()?;
            Ok(digest)
        })
        .map_err(serde::de::Error::custom)
    }
}

impl TemporalSwitchDigest {
    pub fn new(
        layer_id: LayerId,
        n_experts: u16,
        same_expert_rate_q8_8: u16,
        transition_mass: Vec<TransitionEntry>,
    ) -> Result<Self, S7SchemaError> {
        let digest = Self {
            schema_version: S7_SCHEMA_VERSION,
            layer_id,
            n_experts,
            same_expert_rate_q8_8,
            transition_mass,
            digest_self_hash: Hash256::ZERO,
        };
        digest.validate()?;
        Ok(digest)
    }

    pub fn validate(&self) -> Result<(), S7SchemaError> {
        validate_schema_version(self.schema_version)?;
        validate_layer_id(self.layer_id)?;
        validate_n_experts(self.n_experts)?;
        validate_q8_8("same_expert_rate_q8_8", self.same_expert_rate_q8_8)?;
        validate_transition_mass(self.n_experts, &self.transition_mass)?;
        Ok(())
    }

    pub fn computed_self_hash(&self) -> Result<Hash256, S7SchemaError> {
        Ok(self_hash_omitting_fields(
            Self::domain(),
            self,
            "digest_self_hash",
            &[],
        )?)
    }

    pub fn with_computed_self_hash(mut self) -> Result<Self, S7SchemaError> {
        self.digest_self_hash = self.computed_self_hash()?;
        Ok(self)
    }

    pub fn canonical_json_bytes(&self) -> Result<Vec<u8>, S7SchemaError> {
        Ok(CanonicalJson::to_vec(self)?)
    }

    #[must_use]
    pub const fn domain() -> DomainHash<'static> {
        DomainHash::new(
            "gbf-artifact",
            "TemporalSwitchDigest",
            S7_TEMPORAL_SWITCH_DIGEST_SCHEMA,
            S7_SCHEMA_DOMAIN_VERSION,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UnorderedExpertPair {
    pub lo: ExpertId,
    pub hi: ExpertId,
}

impl<'de> Deserialize<'de> for UnorderedExpertPair {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            lo: ExpertId,
            hi: ExpertId,
        }

        let raw = Raw::deserialize(deserializer)?;
        Self::new(raw.lo, raw.hi).map_err(serde::de::Error::custom)
    }
}

impl UnorderedExpertPair {
    pub fn new(left: ExpertId, right: ExpertId) -> Result<Self, S7SchemaError> {
        validate_expert_id(left, S7_N_EXPERTS)?;
        validate_expert_id(right, S7_N_EXPERTS)?;
        let (lo, hi) = if left <= right {
            (left, right)
        } else {
            (right, left)
        };
        Ok(Self { lo, hi })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalizedAffinity {
    pub pair: UnorderedExpertPair,
    pub affinity_score: u16,
}

impl<'de> Deserialize<'de> for CanonicalizedAffinity {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            pair: UnorderedExpertPair,
            affinity_score: u16,
        }

        let raw = Raw::deserialize(deserializer)?;
        Self::new(raw.pair, raw.affinity_score).map_err(serde::de::Error::custom)
    }
}

impl CanonicalizedAffinity {
    pub fn new(pair: UnorderedExpertPair, affinity_score: u16) -> Result<Self, S7SchemaError> {
        validate_q8_8("affinity_score", affinity_score)?;
        Ok(Self {
            pair,
            affinity_score,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExpertSlotAffinity {
    pub schema_version: SemVer,
    pub layer_id: LayerId,
    pub affinities: Vec<CanonicalizedAffinity>,
    pub affinity_self_hash: Hash256,
}

impl<'de> Deserialize<'de> for ExpertSlotAffinity {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            schema_version: SemVer,
            layer_id: LayerId,
            affinities: Vec<CanonicalizedAffinity>,
            affinity_self_hash: Hash256,
        }

        let raw = Raw::deserialize(deserializer)?;
        Self::new(raw.layer_id, raw.affinities)
            .map(|mut affinity| {
                affinity.schema_version = raw.schema_version;
                affinity.affinity_self_hash = raw.affinity_self_hash;
                affinity
            })
            .and_then(|affinity| {
                affinity.validate()?;
                Ok(affinity)
            })
            .map_err(serde::de::Error::custom)
    }
}

impl ExpertSlotAffinity {
    pub fn new(
        layer_id: LayerId,
        affinities: Vec<CanonicalizedAffinity>,
    ) -> Result<Self, S7SchemaError> {
        let affinity = Self {
            schema_version: S7_SCHEMA_VERSION,
            layer_id,
            affinities,
            affinity_self_hash: Hash256::ZERO,
        };
        affinity.validate()?;
        Ok(affinity)
    }

    pub fn from_temporal_switch_digest(
        digest: &TemporalSwitchDigest,
    ) -> Result<Self, S7SchemaError> {
        digest.validate()?;
        Self::from_directional_transitions(digest.layer_id, digest.transition_mass.iter().copied())
    }

    pub fn from_directional_transitions(
        layer_id: LayerId,
        transition_mass: impl IntoIterator<Item = TransitionEntry>,
    ) -> Result<Self, S7SchemaError> {
        validate_layer_id(layer_id)?;
        let mut summed = BTreeMap::<UnorderedExpertPair, u32>::new();
        for entry in transition_mass {
            let pair = UnorderedExpertPair::new(entry.from_expert, entry.to_expert)?;
            let total = summed.entry(pair).or_insert(0);
            *total = total.saturating_add(u32::from(entry.mass_q8_8));
        }

        let affinities = summed
            .into_iter()
            .map(|(pair, raw_score)| {
                let clamped = raw_score.min(u32::from(Q8_8_ONE));
                let score = u16::try_from(clamped).map_err(|_| S7SchemaError::LengthOverflow)?;
                CanonicalizedAffinity::new(pair, score)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Self::new(layer_id, affinities)
    }

    pub fn validate(&self) -> Result<(), S7SchemaError> {
        validate_schema_version(self.schema_version)?;
        validate_layer_id(self.layer_id)?;
        let mut seen = BTreeSet::new();
        for affinity in &self.affinities {
            validate_q8_8("affinity_score", affinity.affinity_score)?;
            if !seen.insert(affinity.pair) {
                return Err(S7SchemaError::DuplicateAffinityPair {
                    lo: affinity.pair.lo,
                    hi: affinity.pair.hi,
                });
            }
        }
        Ok(())
    }

    pub fn computed_self_hash(&self) -> Result<Hash256, S7SchemaError> {
        Ok(self_hash_omitting_fields(
            Self::domain(),
            self,
            "affinity_self_hash",
            &[],
        )?)
    }

    pub fn with_computed_self_hash(mut self) -> Result<Self, S7SchemaError> {
        self.affinity_self_hash = self.computed_self_hash()?;
        Ok(self)
    }

    pub fn canonical_json_bytes(&self) -> Result<Vec<u8>, S7SchemaError> {
        Ok(CanonicalJson::to_vec(self)?)
    }

    #[must_use]
    pub const fn domain() -> DomainHash<'static> {
        DomainHash::new(
            "gbf-artifact",
            "ExpertSlotAffinity",
            S7_EXPERT_SLOT_AFFINITY_SCHEMA,
            S7_SCHEMA_DOMAIN_VERSION,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClipSaturationDigest {
    pub schema_version: SemVer,
    pub layer_id: LayerId,
    pub saturation_rate_q8_8: u16,
    pub clip_bound_observed: f32,
    pub digest_self_hash: Hash256,
}

impl<'de> Deserialize<'de> for ClipSaturationDigest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            schema_version: SemVer,
            layer_id: LayerId,
            saturation_rate_q8_8: u16,
            clip_bound_observed: f32,
            digest_self_hash: Hash256,
        }

        let raw = Raw::deserialize(deserializer)?;
        Self::new(
            raw.layer_id,
            raw.saturation_rate_q8_8,
            raw.clip_bound_observed,
        )
        .map(|mut digest| {
            digest.schema_version = raw.schema_version;
            digest.digest_self_hash = raw.digest_self_hash;
            digest
        })
        .and_then(|digest| {
            digest.validate()?;
            Ok(digest)
        })
        .map_err(serde::de::Error::custom)
    }
}

impl ClipSaturationDigest {
    pub fn new(
        layer_id: LayerId,
        saturation_rate_q8_8: u16,
        clip_bound_observed: f32,
    ) -> Result<Self, S7SchemaError> {
        let digest = Self {
            schema_version: S7_SCHEMA_VERSION,
            layer_id,
            saturation_rate_q8_8,
            clip_bound_observed,
            digest_self_hash: Hash256::ZERO,
        };
        digest.validate()?;
        Ok(digest)
    }

    pub fn validate(&self) -> Result<(), S7SchemaError> {
        validate_schema_version(self.schema_version)?;
        validate_layer_id(self.layer_id)?;
        validate_q8_8("saturation_rate_q8_8", self.saturation_rate_q8_8)?;
        validate_positive_f32("clip_bound_observed", self.clip_bound_observed)?;
        Ok(())
    }

    pub fn computed_self_hash(&self) -> Result<Hash256, S7SchemaError> {
        Ok(self_hash_omitting_fields(
            Self::domain(),
            self,
            "digest_self_hash",
            &[],
        )?)
    }

    pub fn with_computed_self_hash(mut self) -> Result<Self, S7SchemaError> {
        self.digest_self_hash = self.computed_self_hash()?;
        Ok(self)
    }

    #[must_use]
    pub const fn domain() -> DomainHash<'static> {
        DomainHash::new(
            "gbf-artifact",
            "ClipSaturationDigest",
            S7_CLIP_SATURATION_DIGEST_SCHEMA,
            S7_SCHEMA_DOMAIN_VERSION,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExpertPayloadEntry {
    pub expert_id: ExpertId,
    pub byte_count: u32,
    pub weight_quant: QuantSpec,
}

impl<'de> Deserialize<'de> for ExpertPayloadEntry {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            expert_id: ExpertId,
            byte_count: u32,
            weight_quant: QuantSpec,
        }

        let raw = Raw::deserialize(deserializer)?;
        Self::new(raw.expert_id, raw.byte_count, raw.weight_quant).map_err(serde::de::Error::custom)
    }
}

impl ExpertPayloadEntry {
    pub fn new(
        expert_id: ExpertId,
        byte_count: u32,
        weight_quant: QuantSpec,
    ) -> Result<Self, S7SchemaError> {
        validate_expert_id(expert_id, S7_N_EXPERTS)?;
        if byte_count == 0 {
            return Err(S7SchemaError::ZeroByteCount { expert_id });
        }
        Ok(Self {
            expert_id,
            byte_count,
            weight_quant,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExpertPayloadDigest {
    pub schema_version: SemVer,
    pub layer_id: LayerId,
    pub artifact_path: String,
    pub entries: Vec<ExpertPayloadEntry>,
    pub digest_self_hash: Hash256,
}

impl<'de> Deserialize<'de> for ExpertPayloadDigest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            schema_version: SemVer,
            layer_id: LayerId,
            artifact_path: String,
            entries: Vec<ExpertPayloadEntry>,
            digest_self_hash: Hash256,
        }

        let raw = Raw::deserialize(deserializer)?;
        Self::new(raw.layer_id, raw.artifact_path, raw.entries)
            .map(|mut digest| {
                digest.schema_version = raw.schema_version;
                digest.digest_self_hash = raw.digest_self_hash;
                digest
            })
            .and_then(|digest| {
                digest.validate()?;
                Ok(digest)
            })
            .map_err(serde::de::Error::custom)
    }
}

impl ExpertPayloadDigest {
    pub fn new(
        layer_id: LayerId,
        artifact_path: impl Into<String>,
        entries: Vec<ExpertPayloadEntry>,
    ) -> Result<Self, S7SchemaError> {
        let digest = Self {
            schema_version: S7_SCHEMA_VERSION,
            layer_id,
            artifact_path: artifact_path.into(),
            entries,
            digest_self_hash: Hash256::ZERO,
        };
        digest.validate()?;
        Ok(digest)
    }

    pub fn validate(&self) -> Result<(), S7SchemaError> {
        validate_schema_version(self.schema_version)?;
        validate_layer_id(self.layer_id)?;
        if self.artifact_path.is_empty() {
            return Err(S7SchemaError::EmptyArtifactPath);
        }
        if self.entries.len() != usize::from(S7_N_EXPERTS) {
            return Err(S7SchemaError::WrongExpertPayloadEntryCount {
                observed: self.entries.len(),
                expected: S7_N_EXPERTS,
            });
        }

        let mut seen = BTreeSet::new();
        for entry in &self.entries {
            validate_expert_id(entry.expert_id, S7_N_EXPERTS)?;
            if entry.byte_count == 0 {
                return Err(S7SchemaError::ZeroByteCount {
                    expert_id: entry.expert_id,
                });
            }
            if !seen.insert(entry.expert_id) {
                return Err(S7SchemaError::DuplicatePayloadExpert {
                    expert_id: entry.expert_id,
                });
            }
        }
        for expert in 0..S7_N_EXPERTS {
            let expert_id = ExpertId::new(expert);
            if !seen.contains(&expert_id) {
                return Err(S7SchemaError::MissingPayloadExpert { expert_id });
            }
        }
        Ok(())
    }

    pub fn computed_self_hash(&self) -> Result<Hash256, S7SchemaError> {
        Ok(self_hash_omitting_fields(
            Self::domain(),
            self,
            "digest_self_hash",
            &[],
        )?)
    }

    pub fn with_computed_self_hash(mut self) -> Result<Self, S7SchemaError> {
        self.digest_self_hash = self.computed_self_hash()?;
        Ok(self)
    }

    #[must_use]
    pub const fn domain() -> DomainHash<'static> {
        DomainHash::new(
            "gbf-artifact",
            "ExpertPayloadDigest",
            S7_EXPERT_PAYLOAD_DIGEST_SCHEMA,
            S7_SCHEMA_DOMAIN_VERSION,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct S7RunLog {
    pub schema: String,
    pub seed: u64,
    pub topology: S7Topology,
    pub train_config_hash: Hash256,
    pub model_topology_hash: Hash256,
    pub router_config_hash: Option<Hash256>,
    pub expert_block_config_hash: Option<Hash256>,
    pub loss_config_hash: Hash256,
    pub phase_schedule_hash: Hash256,
    pub frozen_teacher_checkpoint_sha: Option<Hash256>,
    pub losses: Vec<(u64, RawLossDiagnostics)>,
    pub grad_norms: Vec<(u64, GradNormSummary)>,
    pub eval_points: Vec<(u64, f64)>,
    pub final_grad_norms: GradNormSummary,
    pub completion: S7Completion,
    pub run_log_self_hash: Hash256,
}

impl<'de> Deserialize<'de> for S7RunLog {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            schema: String,
            seed: u64,
            topology: S7Topology,
            train_config_hash: Hash256,
            model_topology_hash: Hash256,
            router_config_hash: Option<Hash256>,
            expert_block_config_hash: Option<Hash256>,
            loss_config_hash: Hash256,
            phase_schedule_hash: Hash256,
            frozen_teacher_checkpoint_sha: Option<Hash256>,
            losses: Vec<(u64, RawLossDiagnostics)>,
            grad_norms: Vec<(u64, GradNormSummary)>,
            eval_points: Vec<(u64, f64)>,
            final_grad_norms: GradNormSummary,
            completion: S7Completion,
            run_log_self_hash: Hash256,
        }

        let raw = Raw::deserialize(deserializer)?;
        let run_log = Self {
            schema: raw.schema,
            seed: raw.seed,
            topology: raw.topology,
            train_config_hash: raw.train_config_hash,
            model_topology_hash: raw.model_topology_hash,
            router_config_hash: raw.router_config_hash,
            expert_block_config_hash: raw.expert_block_config_hash,
            loss_config_hash: raw.loss_config_hash,
            phase_schedule_hash: raw.phase_schedule_hash,
            frozen_teacher_checkpoint_sha: raw.frozen_teacher_checkpoint_sha,
            losses: raw.losses,
            grad_norms: raw.grad_norms,
            eval_points: raw.eval_points,
            final_grad_norms: raw.final_grad_norms,
            completion: raw.completion,
            run_log_self_hash: raw.run_log_self_hash,
        };
        run_log.validate().map_err(serde::de::Error::custom)?;
        Ok(run_log)
    }
}

impl S7RunLog {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        seed: u64,
        topology: S7Topology,
        train_config_hash: Hash256,
        model_topology_hash: Hash256,
        router_config_hash: Option<Hash256>,
        expert_block_config_hash: Option<Hash256>,
        loss_config_hash: Hash256,
        phase_schedule_hash: Hash256,
        frozen_teacher_checkpoint_sha: Option<Hash256>,
        losses: Vec<(u64, RawLossDiagnostics)>,
        grad_norms: Vec<(u64, GradNormSummary)>,
        eval_points: Vec<(u64, f64)>,
        final_grad_norms: GradNormSummary,
        completion: S7Completion,
    ) -> Result<Self, S7SchemaError> {
        let run_log = Self {
            schema: S7_RUN_LOG_SCHEMA.to_owned(),
            seed,
            topology,
            train_config_hash,
            model_topology_hash,
            router_config_hash,
            expert_block_config_hash,
            loss_config_hash,
            phase_schedule_hash,
            frozen_teacher_checkpoint_sha,
            losses,
            grad_norms,
            eval_points,
            final_grad_norms,
            completion,
            run_log_self_hash: Hash256::ZERO,
        };
        run_log.validate()?;
        Ok(run_log)
    }

    pub fn validate(&self) -> Result<(), S7SchemaError> {
        validate_schema_literal("schema", &self.schema, S7_RUN_LOG_SCHEMA)?;
        if self.topology == S7Topology::MoeTinyDenseMatched
            && (self.router_config_hash.is_some() || self.expert_block_config_hash.is_some())
        {
            return Err(S7SchemaError::DenseRunHasRouterOrExpertHashes);
        }
        if self.frozen_teacher_checkpoint_sha.is_none() {
            return Err(S7SchemaError::MissingFrozenTeacherCheckpoint);
        }
        let expected_loss_steps = self.completion.expected_loss_steps()?;
        let expected_steps =
            usize::try_from(expected_loss_steps).map_err(|_| S7SchemaError::LengthOverflow)?;
        if self.losses.len() != expected_steps {
            return Err(S7SchemaError::RunLogLossLengthMismatch {
                observed: self.losses.len(),
                expected: expected_steps,
            });
        }
        if self.grad_norms.len() != self.losses.len() {
            return Err(S7SchemaError::RunLogGradNormLengthMismatch {
                losses: self.losses.len(),
                grad_norms: self.grad_norms.len(),
            });
        }
        self.final_grad_norms.validate()?;
        let expected_eval_points = self.completion.expected_eval_points()?;
        if self.eval_points.len() != expected_eval_points {
            return Err(S7SchemaError::RunLogEvalLengthMismatch {
                observed: self.eval_points.len(),
                expected: expected_eval_points,
            });
        }
        for ((loss_step, diagnostics), (grad_step, grad_norms)) in
            self.losses.iter().zip(&self.grad_norms)
        {
            if loss_step != grad_step {
                return Err(S7SchemaError::RunLogStepMismatch {
                    loss_step: *loss_step,
                    grad_step: *grad_step,
                });
            }
            diagnostics.validate()?;
            grad_norms.validate()?;
        }
        for (step_index, (step, _)) in self.losses.iter().enumerate() {
            let expected_step =
                u64::try_from(step_index + 1).map_err(|_| S7SchemaError::LengthOverflow)?;
            if *step != expected_step {
                return Err(S7SchemaError::RunLogUnexpectedLossStep {
                    observed: *step,
                    expected: expected_step,
                });
            }
        }
        for (eval_index, (step, bpc)) in self.eval_points.iter().enumerate() {
            let expected_step = u64::try_from(eval_index)
                .map_err(|_| S7SchemaError::LengthOverflow)?
                .checked_mul(S7_EVAL_EVERY_STEPS)
                .ok_or(S7SchemaError::LengthOverflow)?;
            if *step != expected_step {
                return Err(S7SchemaError::RunLogUnexpectedEvalStep {
                    observed: *step,
                    expected: expected_step,
                });
            }
            if *step > expected_loss_steps {
                return Err(S7SchemaError::EvalStepOutOfRange { step: *step });
            }
            if *step > S7_OPTIMIZER_STEPS {
                return Err(S7SchemaError::EvalStepOutOfRange { step: *step });
            }
            validate_finite_nonnegative_f64("eval_points.bpc", *bpc)?;
        }
        Ok(())
    }

    pub fn computed_self_hash(&self) -> Result<Hash256, S7SchemaError> {
        self.validate()?;
        Ok(self_hash_omitting_fields(
            Self::domain(),
            self,
            "run_log_self_hash",
            &[],
        )?)
    }

    pub fn with_computed_self_hash(mut self) -> Result<Self, S7SchemaError> {
        self.run_log_self_hash = self.computed_self_hash()?;
        Ok(self)
    }

    pub fn canonical_json_bytes(&self) -> Result<Vec<u8>, S7SchemaError> {
        self.validate()?;
        Ok(CanonicalJson::to_vec(self)?)
    }

    #[must_use]
    pub const fn domain() -> DomainHash<'static> {
        DomainHash::new(
            "gbf-artifact",
            "S7RunLog",
            S7_RUN_LOG_SCHEMA,
            S7_SCHEMA_DOMAIN_VERSION,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct S7ScoreReport {
    pub schema: String,
    pub seed: u64,
    pub topology: S7Topology,
    pub checkpoint_sha: Hash256,
    pub corpus_val_sha: Hash256,
    pub chunk_size: u64,
    pub token_count: u64,
    pub log2_sum: f64,
    pub bpc: f64,
    pub score_self_hash: Hash256,
}

impl<'de> Deserialize<'de> for S7ScoreReport {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            schema: String,
            seed: u64,
            topology: S7Topology,
            checkpoint_sha: Hash256,
            corpus_val_sha: Hash256,
            chunk_size: u64,
            token_count: u64,
            log2_sum: f64,
            bpc: f64,
            score_self_hash: Hash256,
        }

        let raw = Raw::deserialize(deserializer)?;
        let report = Self {
            schema: raw.schema,
            seed: raw.seed,
            topology: raw.topology,
            checkpoint_sha: raw.checkpoint_sha,
            corpus_val_sha: raw.corpus_val_sha,
            chunk_size: raw.chunk_size,
            token_count: raw.token_count,
            log2_sum: raw.log2_sum,
            bpc: raw.bpc,
            score_self_hash: raw.score_self_hash,
        };
        report.validate().map_err(serde::de::Error::custom)?;
        Ok(report)
    }
}

impl S7ScoreReport {
    pub fn new(
        seed: u64,
        topology: S7Topology,
        checkpoint_sha: Hash256,
        corpus_val_sha: Hash256,
        charset_v1_token_count: u64,
        log2_sum: f64,
    ) -> Result<Self, S7SchemaError> {
        if charset_v1_token_count == 0 {
            return Err(S7SchemaError::ZeroTokenCount);
        }
        validate_finite_nonnegative_f64("log2_sum", log2_sum)?;
        let bpc = log2_sum / charset_v1_token_count as f64;
        let report = Self {
            schema: S7_SCORE_SCHEMA.to_owned(),
            seed,
            topology,
            checkpoint_sha,
            corpus_val_sha,
            chunk_size: S7_SCORE_CHUNK_SIZE,
            token_count: charset_v1_token_count,
            log2_sum,
            bpc,
            score_self_hash: Hash256::ZERO,
        };
        report.validate()?;
        Ok(report)
    }

    pub fn validate(&self) -> Result<(), S7SchemaError> {
        validate_schema_literal("schema", &self.schema, S7_SCORE_SCHEMA)?;
        if self.chunk_size != S7_SCORE_CHUNK_SIZE {
            return Err(S7SchemaError::UnexpectedChunkSize {
                observed: self.chunk_size,
                expected: S7_SCORE_CHUNK_SIZE,
            });
        }
        if self.token_count == 0 {
            return Err(S7SchemaError::ZeroTokenCount);
        }
        validate_finite_nonnegative_f64("log2_sum", self.log2_sum)?;
        validate_finite_nonnegative_f64("bpc", self.bpc)?;
        let expected = self.log2_sum / self.token_count as f64;
        if (self.bpc - expected).abs() > SCORE_BPC_EPSILON {
            return Err(S7SchemaError::BpcMismatch {
                observed: self.bpc,
                expected,
            });
        }
        Ok(())
    }

    pub fn computed_self_hash(&self) -> Result<Hash256, S7SchemaError> {
        Ok(self_hash_omitting_fields(
            Self::domain(),
            self,
            "score_self_hash",
            &[],
        )?)
    }

    pub fn with_computed_self_hash(mut self) -> Result<Self, S7SchemaError> {
        self.score_self_hash = self.computed_self_hash()?;
        Ok(self)
    }

    pub fn canonical_json_bytes(&self) -> Result<Vec<u8>, S7SchemaError> {
        Ok(CanonicalJson::to_vec(self)?)
    }

    #[must_use]
    pub const fn domain() -> DomainHash<'static> {
        DomainHash::new(
            "gbf-artifact",
            "S7ScoreReport",
            S7_SCORE_SCHEMA,
            S7_SCHEMA_DOMAIN_VERSION,
        )
    }
}

#[derive(Debug)]
pub enum S7SchemaError {
    CanonicalJson(CanonicalJsonError),
    UnexpectedSchemaVersion {
        observed: SemVer,
        expected: SemVer,
    },
    UnexpectedSchemaLiteral {
        field: &'static str,
        observed: String,
        expected: &'static str,
    },
    LayerIdOutOfRange {
        layer_id: LayerId,
        n_blocks: u16,
    },
    ExpertIdOutOfRange {
        expert_id: ExpertId,
        n_experts: u16,
    },
    UnexpectedExpertCount {
        observed: u16,
        expected: u16,
    },
    Q8_8OutOfRange {
        field: &'static str,
        value: u16,
    },
    TransitionMassExceedsOne {
        total_q8_8: u32,
    },
    DuplicateTransitionEntry {
        from_expert: ExpertId,
        to_expert: ExpertId,
    },
    DuplicateAffinityPair {
        lo: ExpertId,
        hi: ExpertId,
    },
    EmptyArtifactPath,
    WrongExpertPayloadEntryCount {
        observed: usize,
        expected: u16,
    },
    ZeroByteCount {
        expert_id: ExpertId,
    },
    DuplicatePayloadExpert {
        expert_id: ExpertId,
    },
    MissingPayloadExpert {
        expert_id: ExpertId,
    },
    NonFiniteF32 {
        field: &'static str,
        value: f32,
    },
    NonFiniteF64 {
        field: &'static str,
        value: f64,
    },
    NegativeF32 {
        field: &'static str,
        value: f32,
    },
    NegativeF64 {
        field: &'static str,
        value: f64,
    },
    F32OutOfUnitInterval {
        field: &'static str,
        value: f32,
    },
    NonPositiveF32 {
        field: &'static str,
        value: f32,
    },
    EmptyReason,
    InvalidCompletionStep {
        step: u64,
    },
    RunLogLossLengthMismatch {
        observed: usize,
        expected: usize,
    },
    RunLogGradNormLengthMismatch {
        losses: usize,
        grad_norms: usize,
    },
    RunLogEvalLengthMismatch {
        observed: usize,
        expected: usize,
    },
    RunLogStepMismatch {
        loss_step: u64,
        grad_step: u64,
    },
    RunLogUnexpectedLossStep {
        observed: u64,
        expected: u64,
    },
    RunLogUnexpectedEvalStep {
        observed: u64,
        expected: u64,
    },
    EvalStepOutOfRange {
        step: u64,
    },
    DenseRunHasRouterOrExpertHashes,
    MissingFrozenTeacherCheckpoint,
    UnexpectedChunkSize {
        observed: u64,
        expected: u64,
    },
    ZeroTokenCount,
    BpcMismatch {
        observed: f64,
        expected: f64,
    },
    ProjectedFitBlockCountMismatch {
        observed: usize,
        expected: u16,
    },
    ProjectedFitTotalMismatch {
        observed: u64,
        expected: u64,
    },
    SwitchStatsLayerCountMismatch {
        observed: usize,
        expected: u16,
    },
    EmptyLambdaSwitch,
    InvalidLambdaSwitch {
        value: String,
    },
    EmptySweepSummaryMap {
        field: &'static str,
    },
    SweepSummaryLambdaSetMismatch,
    DenseVsMoePerSeedLengthMismatch {
        observed: usize,
        expected: usize,
    },
    DuplicateDenseVsMoeSeed {
        seed: u64,
    },
    MissingDenseVsMoeSeed {
        seed: u64,
    },
    PerSeedDeltaMismatch {
        seed: u64,
        observed: f64,
        expected: f64,
    },
    PerSeedParityVerdictMismatch {
        seed: u64,
        observed: S7ParityVerdict,
        expected: S7ParityVerdict,
    },
    MedianBpcMismatch {
        field: &'static str,
        observed: f64,
        expected: f64,
    },
    DeployedBytesTotalMismatch {
        field: &'static str,
        observed: u64,
        expected: u64,
    },
    BytesDiffMismatch {
        observed: i64,
        expected: i64,
    },
    BytesDiffOverflow {
        dense: u64,
        moe: u64,
    },
    BytesWithinToleranceMismatch {
        observed: bool,
        expected: bool,
    },
    AggregateParityVerdictMismatch {
        observed: S7AggregateParityVerdict,
        expected: S7AggregateParityVerdict,
    },
    ParetoVerdictMismatch {
        observed: ParetoVerdict,
        expected: ParetoVerdict,
    },
    MatchedBytesPinSelfHashInvalid,
    SelfHashMismatch {
        field: &'static str,
        expected: Hash256,
        observed: Hash256,
    },
    LengthOverflow,
}

impl fmt::Display for S7SchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CanonicalJson(error) => write!(f, "{error}"),
            Self::UnexpectedSchemaVersion { observed, expected } => {
                write!(f, "expected schema_version {expected}, got {observed}")
            }
            Self::UnexpectedSchemaLiteral {
                field,
                observed,
                expected,
            } => write!(f, "expected {field}={expected:?}, got {observed:?}"),
            Self::LayerIdOutOfRange { layer_id, n_blocks } => {
                write!(f, "layer_id {layer_id} must be in 0..{n_blocks}")
            }
            Self::ExpertIdOutOfRange {
                expert_id,
                n_experts,
            } => write!(f, "expert_id {expert_id} must be in 0..{n_experts}"),
            Self::UnexpectedExpertCount { observed, expected } => {
                write!(f, "expected n_experts={expected}, got {observed}")
            }
            Self::Q8_8OutOfRange { field, value } => {
                write!(f, "{field} must be <= 256, got {value}")
            }
            Self::TransitionMassExceedsOne { total_q8_8 } => {
                write!(f, "transition_mass must sum to <= 256, got {total_q8_8}")
            }
            Self::DuplicateTransitionEntry {
                from_expert,
                to_expert,
            } => write!(
                f,
                "duplicate directional transition {from_expert}->{to_expert}"
            ),
            Self::DuplicateAffinityPair { lo, hi } => {
                write!(f, "duplicate affinity pair {lo}/{hi}")
            }
            Self::EmptyArtifactPath => f.write_str("artifact_path must not be empty"),
            Self::WrongExpertPayloadEntryCount { observed, expected } => write!(
                f,
                "expert payload entries length must be {expected}, got {observed}"
            ),
            Self::ZeroByteCount { expert_id } => {
                write!(f, "expert {expert_id} byte_count must be > 0")
            }
            Self::DuplicatePayloadExpert { expert_id } => {
                write!(f, "duplicate expert payload entry for expert {expert_id}")
            }
            Self::MissingPayloadExpert { expert_id } => {
                write!(f, "missing expert payload entry for expert {expert_id}")
            }
            Self::NonFiniteF32 { field, value } => write!(f, "{field} must be finite, got {value}"),
            Self::NonFiniteF64 { field, value } => write!(f, "{field} must be finite, got {value}"),
            Self::NegativeF32 { field, value } => {
                write!(f, "{field} must be non-negative, got {value}")
            }
            Self::NegativeF64 { field, value } => {
                write!(f, "{field} must be non-negative, got {value}")
            }
            Self::F32OutOfUnitInterval { field, value } => {
                write!(f, "{field} must be in [0, 1], got {value}")
            }
            Self::NonPositiveF32 { field, value } => write!(f, "{field} must be > 0, got {value}"),
            Self::EmptyReason => f.write_str("distill NotAvailable reason must not be empty"),
            Self::InvalidCompletionStep { step } => {
                write!(f, "partial completion step must be in 1..={S7_OPTIMIZER_STEPS}, got {step}")
            }
            Self::RunLogLossLengthMismatch { observed, expected } => write!(
                f,
                "run log losses length must be {expected}, got {observed}"
            ),
            Self::RunLogGradNormLengthMismatch { losses, grad_norms } => write!(
                f,
                "run log grad_norms length {grad_norms} must match losses length {losses}"
            ),
            Self::RunLogEvalLengthMismatch { observed, expected } => write!(
                f,
                "run log eval_points length must be {expected}, got {observed}"
            ),
            Self::RunLogStepMismatch {
                loss_step,
                grad_step,
            } => write!(
                f,
                "run log loss step {loss_step} does not match grad_norm step {grad_step}"
            ),
            Self::RunLogUnexpectedLossStep { observed, expected } => write!(
                f,
                "run log loss step must be contiguous; expected {expected}, got {observed}"
            ),
            Self::RunLogUnexpectedEvalStep { observed, expected } => write!(
                f,
                "run log eval step must follow the eval cadence; expected {expected}, got {observed}"
            ),
            Self::EvalStepOutOfRange { step } => {
                write!(f, "eval step {step} exceeds optimizer steps")
            }
            Self::DenseRunHasRouterOrExpertHashes => f.write_str(
                "dense S7 run logs must carry JSON null router_config_hash and expert_block_config_hash",
            ),
            Self::MissingFrozenTeacherCheckpoint => f.write_str(
                "S7 run logs must carry frozen_teacher_checkpoint_sha after the Phase A boundary",
            ),
            Self::UnexpectedChunkSize { observed, expected } => {
                write!(f, "expected chunk_size={expected}, got {observed}")
            }
            Self::ZeroTokenCount => f.write_str("token_count must be > 0"),
            Self::BpcMismatch { observed, expected } => {
                write!(f, "bpc {observed} does not match log2_sum/token_count {expected}")
            }
            Self::ProjectedFitBlockCountMismatch { observed, expected } => write!(
                f,
                "projected fit deployed_bytes_per_block length must be {expected}, got {observed}"
            ),
            Self::ProjectedFitTotalMismatch { observed, expected } => write!(
                f,
                "projected fit deployed_bytes_total {observed} does not equal per-block sum {expected}"
            ),
            Self::SwitchStatsLayerCountMismatch { observed, expected } => write!(
                f,
                "switch stats same_expert_rate_per_layer_q8_8 length must be {expected}, got {observed}"
            ),
            Self::EmptyLambdaSwitch => f.write_str("lambda_switch key must not be empty"),
            Self::InvalidLambdaSwitch { value } => {
                write!(f, "lambda_switch key {value:?} is not a finite non-negative f32")
            }
            Self::EmptySweepSummaryMap { field } => {
                write!(f, "sweep_summary.{field} must not be empty")
            }
            Self::SweepSummaryLambdaSetMismatch => f.write_str(
                "sweep_summary bpc_at_lambda and entropy_at_lambda must carry identical lambda_switch keys",
            ),
            Self::DenseVsMoePerSeedLengthMismatch { observed, expected } => write!(
                f,
                "dense-vs-MoE per_seed length must be {expected}, got {observed}"
            ),
            Self::DuplicateDenseVsMoeSeed { seed } => {
                write!(f, "duplicate dense-vs-MoE per_seed row for seed {seed}")
            }
            Self::MissingDenseVsMoeSeed { seed } => {
                write!(f, "missing dense-vs-MoE per_seed row for seed {seed}")
            }
            Self::PerSeedDeltaMismatch {
                seed,
                observed,
                expected,
            } => write!(
                f,
                "dense-vs-MoE seed {seed} delta {observed} does not match dense-minus-MoE {expected}"
            ),
            Self::PerSeedParityVerdictMismatch {
                seed,
                observed,
                expected,
            } => write!(
                f,
                "dense-vs-MoE seed {seed} parity verdict {observed:?} does not match derived {expected:?}"
            ),
            Self::MedianBpcMismatch {
                field,
                observed,
                expected,
            } => write!(
                f,
                "{field} {observed} does not match per-seed median {expected}"
            ),
            Self::DeployedBytesTotalMismatch {
                field,
                observed,
                expected,
            } => write!(
                f,
                "{field} deployed byte total {observed} does not match report total {expected}"
            ),
            Self::BytesDiffMismatch { observed, expected } => write!(
                f,
                "bytes_diff {observed} does not match dense-minus-MoE deployed bytes {expected}"
            ),
            Self::BytesDiffOverflow { dense, moe } => write!(
                f,
                "dense-minus-MoE deployed bytes overflow i64: dense={dense}, moe={moe}"
            ),
            Self::BytesWithinToleranceMismatch { observed, expected } => write!(
                f,
                "bytes_within_tolerance {observed} does not match derived {expected}"
            ),
            Self::AggregateParityVerdictMismatch { observed, expected } => write!(
                f,
                "aggregate_parity_verdict {observed:?} does not match derived {expected:?}"
            ),
            Self::ParetoVerdictMismatch { observed, expected } => write!(
                f,
                "pareto_verdict {observed:?} does not match derived {expected:?}"
            ),
            Self::MatchedBytesPinSelfHashInvalid => {
                f.write_str("matched_bytes_pin self-hash did not verify")
            }
            Self::SelfHashMismatch {
                field,
                expected,
                observed,
            } => write!(
                f,
                "{field} mismatch: expected {expected}, observed {observed}"
            ),
            Self::LengthOverflow => f.write_str("schema length conversion overflowed"),
        }
    }
}

impl std::error::Error for S7SchemaError {}

impl From<CanonicalJsonError> for S7SchemaError {
    fn from(error: CanonicalJsonError) -> Self {
        Self::CanonicalJson(error)
    }
}

fn validate_schema_version(version: SemVer) -> Result<(), S7SchemaError> {
    if version != S7_SCHEMA_VERSION {
        return Err(S7SchemaError::UnexpectedSchemaVersion {
            observed: version,
            expected: S7_SCHEMA_VERSION,
        });
    }
    Ok(())
}

fn validate_schema_literal(
    field: &'static str,
    observed: &str,
    expected: &'static str,
) -> Result<(), S7SchemaError> {
    if observed != expected {
        return Err(S7SchemaError::UnexpectedSchemaLiteral {
            field,
            observed: observed.to_owned(),
            expected,
        });
    }
    Ok(())
}

fn validate_layer_id(layer_id: LayerId) -> Result<(), S7SchemaError> {
    if layer_id.get() >= S7_N_BLOCKS {
        return Err(S7SchemaError::LayerIdOutOfRange {
            layer_id,
            n_blocks: S7_N_BLOCKS,
        });
    }
    Ok(())
}

fn validate_expert_id(expert_id: ExpertId, n_experts: u16) -> Result<(), S7SchemaError> {
    if expert_id.get() >= n_experts {
        return Err(S7SchemaError::ExpertIdOutOfRange {
            expert_id,
            n_experts,
        });
    }
    Ok(())
}

fn validate_n_experts(n_experts: u16) -> Result<(), S7SchemaError> {
    if n_experts != S7_N_EXPERTS {
        return Err(S7SchemaError::UnexpectedExpertCount {
            observed: n_experts,
            expected: S7_N_EXPERTS,
        });
    }
    Ok(())
}

fn validate_q8_8(field: &'static str, value: u16) -> Result<(), S7SchemaError> {
    if value > Q8_8_ONE {
        return Err(S7SchemaError::Q8_8OutOfRange { field, value });
    }
    Ok(())
}

fn validate_lambda_switch_text(value: &str) -> Result<(), S7SchemaError> {
    if value.is_empty() {
        return Err(S7SchemaError::EmptyLambdaSwitch);
    }
    let parsed = value
        .parse::<f32>()
        .map_err(|_| S7SchemaError::InvalidLambdaSwitch {
            value: value.to_owned(),
        })?;
    if !parsed.is_finite() || parsed < 0.0 {
        return Err(S7SchemaError::InvalidLambdaSwitch {
            value: value.to_owned(),
        });
    }
    Ok(())
}

fn validate_nonempty_lambda_map<T>(
    field: &'static str,
    map: &BTreeMap<LambdaSwitch, T>,
) -> Result<(), S7SchemaError> {
    if map.is_empty() {
        return Err(S7SchemaError::EmptySweepSummaryMap { field });
    }
    for lambda_switch in map.keys() {
        lambda_switch.validate()?;
    }
    Ok(())
}

fn validate_transition_mass(
    n_experts: u16,
    transition_mass: &[TransitionEntry],
) -> Result<(), S7SchemaError> {
    let mut total_q8_8 = 0_u32;
    let mut seen = BTreeSet::new();
    for entry in transition_mass {
        validate_expert_id(entry.from_expert, n_experts)?;
        validate_expert_id(entry.to_expert, n_experts)?;
        validate_q8_8("transition_mass.mass_q8_8", entry.mass_q8_8)?;
        total_q8_8 += u32::from(entry.mass_q8_8);
        if !seen.insert((entry.from_expert, entry.to_expert)) {
            return Err(S7SchemaError::DuplicateTransitionEntry {
                from_expert: entry.from_expert,
                to_expert: entry.to_expert,
            });
        }
    }
    if total_q8_8 > u32::from(Q8_8_ONE) {
        return Err(S7SchemaError::TransitionMassExceedsOne { total_q8_8 });
    }
    Ok(())
}

fn validate_finite_nonnegative_f32(field: &'static str, value: f32) -> Result<(), S7SchemaError> {
    if !value.is_finite() {
        return Err(S7SchemaError::NonFiniteF32 { field, value });
    }
    if value < 0.0 {
        return Err(S7SchemaError::NegativeF32 { field, value });
    }
    Ok(())
}

fn validate_finite_nonnegative_f64(field: &'static str, value: f64) -> Result<(), S7SchemaError> {
    if !value.is_finite() {
        return Err(S7SchemaError::NonFiniteF64 { field, value });
    }
    if value < 0.0 {
        return Err(S7SchemaError::NegativeF64 { field, value });
    }
    Ok(())
}

fn validate_finite_f64(field: &'static str, value: f64) -> Result<(), S7SchemaError> {
    if !value.is_finite() {
        return Err(S7SchemaError::NonFiniteF64 { field, value });
    }
    Ok(())
}

fn validate_dense_vs_moe_per_seed(per_seed: &[S7PerSeedComparison]) -> Result<(), S7SchemaError> {
    if per_seed.len() != S7_DENSE_VS_MOE_SEED_COUNT {
        return Err(S7SchemaError::DenseVsMoePerSeedLengthMismatch {
            observed: per_seed.len(),
            expected: S7_DENSE_VS_MOE_SEED_COUNT,
        });
    }

    let mut seen = BTreeSet::new();
    for entry in per_seed {
        entry.validate()?;
        if !seen.insert(entry.seed) {
            return Err(S7SchemaError::DuplicateDenseVsMoeSeed { seed: entry.seed });
        }
    }
    for seed in 0..S7_DENSE_VS_MOE_SEED_COUNT {
        let seed = u64::try_from(seed).map_err(|_| S7SchemaError::LengthOverflow)?;
        if !seen.contains(&seed) {
            return Err(S7SchemaError::MissingDenseVsMoeSeed { seed });
        }
    }
    Ok(())
}

fn validate_reported_median(
    field: &'static str,
    observed: f64,
    expected: f64,
) -> Result<(), S7SchemaError> {
    if !f64_close(observed, expected) {
        return Err(S7SchemaError::MedianBpcMismatch {
            field,
            observed,
            expected,
        });
    }
    Ok(())
}

fn validate_deployed_bytes_total(
    field: &'static str,
    observed: u64,
    expected: u64,
) -> Result<(), S7SchemaError> {
    if observed != expected {
        return Err(S7SchemaError::DeployedBytesTotalMismatch {
            field,
            observed,
            expected,
        });
    }
    Ok(())
}

fn derive_parity_verdict(val_bpc_moe: f64, val_bpc_dense: f64) -> S7ParityVerdict {
    if val_bpc_moe < val_bpc_dense - S7_PARITY_BPC_MARGIN {
        S7ParityVerdict::Pass
    } else {
        S7ParityVerdict::Fail
    }
}

fn derive_aggregate_parity_verdict(
    per_seed: &[S7PerSeedComparison],
    bytes_diff: u64,
    tolerance_bytes: u64,
) -> S7AggregateParityVerdict {
    if bytes_diff > tolerance_bytes {
        S7AggregateParityVerdict::FailBytes
    } else if per_seed.iter().all(|entry| entry.parity_verdict.passed()) {
        S7AggregateParityVerdict::PassClean
    } else {
        S7AggregateParityVerdict::FailParity
    }
}

fn derive_pareto_verdict(
    median_val_bpc_moe: f64,
    median_val_bpc_dense: f64,
    deployed_bytes_total_moe: u64,
    deployed_bytes_total_dense: u64,
    tolerance_bytes: u64,
) -> ParetoVerdict {
    let bpc_equal = f64_close(median_val_bpc_moe, median_val_bpc_dense);
    let bpc_moe_less = median_val_bpc_moe < median_val_bpc_dense && !bpc_equal;
    let bpc_dense_less = median_val_bpc_dense < median_val_bpc_moe && !bpc_equal;
    let bpc_le_moe = bpc_moe_less || bpc_equal;
    let bpc_le_dense = bpc_dense_less || bpc_equal;
    let by_le_moe = deployed_bytes_total_moe <= deployed_bytes_total_dense;
    let by_le_dense = deployed_bytes_total_dense <= deployed_bytes_total_moe;

    if bpc_le_moe
        && by_le_moe
        && (bpc_moe_less || deployed_bytes_total_moe < deployed_bytes_total_dense)
    {
        return ParetoVerdict::MoeDominates;
    }

    if bpc_le_dense
        && by_le_dense
        && (bpc_dense_less || deployed_bytes_total_dense < deployed_bytes_total_moe)
    {
        return ParetoVerdict::DenseDominates;
    }

    if bpc_equal && deployed_bytes_total_moe == deployed_bytes_total_dense {
        return ParetoVerdict::Tied;
    }

    let bytes_equivalent =
        deployed_bytes_total_moe.abs_diff(deployed_bytes_total_dense) <= tolerance_bytes;

    if bytes_equivalent && bpc_moe_less {
        return ParetoVerdict::MoeWinsUnderByteEquivalence;
    }

    if bytes_equivalent && bpc_dense_less {
        return ParetoVerdict::DenseWinsUnderByteEquivalence;
    }

    ParetoVerdict::Incomparable
}

fn median_bpc(
    per_seed: &[S7PerSeedComparison],
    select: impl Fn(&S7PerSeedComparison) -> f64,
) -> f64 {
    let mut values = per_seed.iter().map(select).collect::<Vec<_>>();
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

fn checked_signed_bytes_diff(dense: u64, moe: u64) -> Result<i64, S7SchemaError> {
    let diff = i128::from(dense) - i128::from(moe);
    i64::try_from(diff).map_err(|_| S7SchemaError::BytesDiffOverflow { dense, moe })
}

fn f64_close(left: f64, right: f64) -> bool {
    (left - right).abs() <= SCORE_BPC_EPSILON
}

fn validate_unit_f32(field: &'static str, value: f32) -> Result<(), S7SchemaError> {
    validate_finite_nonnegative_f32(field, value)?;
    if value > 1.0 {
        return Err(S7SchemaError::F32OutOfUnitInterval { field, value });
    }
    Ok(())
}

fn validate_positive_f32(field: &'static str, value: f32) -> Result<(), S7SchemaError> {
    if !value.is_finite() {
        return Err(S7SchemaError::NonFiniteF32 { field, value });
    }
    if value <= 0.0 {
        return Err(S7SchemaError::NonPositiveF32 { field, value });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn temporal_switch_digest_public_shape_and_directional_invariants_are_pinned() {
        let digest = TemporalSwitchDigest::new(
            LayerId::new(2),
            S7_N_EXPERTS,
            128,
            vec![
                TransitionEntry::new(ExpertId::new(1), ExpertId::new(2), 40).unwrap(),
                TransitionEntry::new(ExpertId::new(2), ExpertId::new(1), 41).unwrap(),
            ],
        )
        .unwrap()
        .with_computed_self_hash()
        .unwrap();

        assert_eq!(
            serde_json::to_value(&digest).unwrap(),
            json!({
                "schema_version": { "major": 1, "minor": 0, "patch": 0 },
                "layer_id": 2,
                "n_experts": 4,
                "same_expert_rate_q8_8": 128,
                "transition_mass": [
                    { "from_expert": 1, "to_expert": 2, "mass_q8_8": 40 },
                    { "from_expert": 2, "to_expert": 1, "mass_q8_8": 41 },
                ],
                "digest_self_hash": digest.digest_self_hash,
            })
        );

        let decoded: TemporalSwitchDigest =
            serde_json::from_value(serde_json::to_value(&digest).unwrap()).unwrap();
        assert_eq!(decoded, digest);

        let duplicate_directional: Result<TemporalSwitchDigest, _> =
            serde_json::from_value(json!({
                "schema_version": { "major": 1, "minor": 0, "patch": 0 },
                "layer_id": 0,
                "n_experts": 4,
                "same_expert_rate_q8_8": 0,
                "transition_mass": [
                    { "from_expert": 0, "to_expert": 1, "mass_q8_8": 1 },
                    { "from_expert": 0, "to_expert": 1, "mass_q8_8": 1 },
                ],
                "digest_self_hash": Hash256::ZERO,
            }));
        assert!(duplicate_directional.is_err());

        let aggregate_overflow: Result<TemporalSwitchDigest, _> = serde_json::from_value(json!({
            "schema_version": { "major": 1, "minor": 0, "patch": 0 },
            "layer_id": 0,
            "n_experts": 4,
            "same_expert_rate_q8_8": 0,
            "transition_mass": [
                { "from_expert": 0, "to_expert": 1, "mass_q8_8": 200 },
                { "from_expert": 1, "to_expert": 0, "mass_q8_8": 57 },
            ],
            "digest_self_hash": Hash256::ZERO,
        }));
        assert!(aggregate_overflow.is_err());
    }

    #[test]
    fn expert_slot_affinity_canonicalizes_and_sums_directional_transitions() {
        let digest = TemporalSwitchDigest::new(
            LayerId::new(1),
            S7_N_EXPERTS,
            0,
            vec![
                TransitionEntry::new(ExpertId::new(3), ExpertId::new(1), 70).unwrap(),
                TransitionEntry::new(ExpertId::new(1), ExpertId::new(3), 90).unwrap(),
                TransitionEntry::new(ExpertId::new(2), ExpertId::new(2), 12).unwrap(),
            ],
        )
        .unwrap();
        let affinity = ExpertSlotAffinity::from_temporal_switch_digest(&digest).unwrap();

        assert_eq!(
            serde_json::to_value(&affinity).unwrap(),
            json!({
                "schema_version": { "major": 1, "minor": 0, "patch": 0 },
                "layer_id": 1,
                "affinities": [
                    {
                        "pair": { "lo": 1, "hi": 3 },
                        "affinity_score": 160,
                    },
                    {
                        "pair": { "lo": 2, "hi": 2 },
                        "affinity_score": 12,
                    },
                ],
                "affinity_self_hash": Hash256::ZERO,
            })
        );

        let decoded: ExpertSlotAffinity =
            serde_json::from_value(serde_json::to_value(&affinity).unwrap()).unwrap();
        assert_eq!(decoded, affinity);

        let canonicalized_pair: UnorderedExpertPair =
            serde_json::from_value(json!({ "lo": 3, "hi": 1 })).unwrap();
        assert_eq!(
            canonicalized_pair,
            UnorderedExpertPair {
                lo: ExpertId::new(1),
                hi: ExpertId::new(3),
            }
        );

        let saturated = ExpertSlotAffinity::from_directional_transitions(
            LayerId::new(0),
            [
                TransitionEntry::new(ExpertId::new(0), ExpertId::new(1), 200).unwrap(),
                TransitionEntry::new(ExpertId::new(1), ExpertId::new(0), 200).unwrap(),
                TransitionEntry::new(ExpertId::new(0), ExpertId::new(1), 200).unwrap(),
            ],
        )
        .unwrap();
        assert_eq!(saturated.affinities[0].affinity_score, 256);
    }

    #[test]
    fn layer_and_expert_bounds_reject_off_by_one_values_in_construction_and_json() {
        assert!(matches!(
            TemporalSwitchDigest::new(LayerId::new(4), S7_N_EXPERTS, 0, Vec::new()),
            Err(S7SchemaError::LayerIdOutOfRange { .. })
        ));
        assert!(matches!(
            TransitionEntry::new(ExpertId::new(4), ExpertId::new(0), 1),
            Err(S7SchemaError::ExpertIdOutOfRange { .. })
        ));

        let invalid_layer: Result<TemporalSwitchDigest, _> = serde_json::from_value(json!({
            "schema_version": { "major": 1, "minor": 0, "patch": 0 },
            "layer_id": 4,
            "n_experts": 4,
            "same_expert_rate_q8_8": 0,
            "transition_mass": [],
            "digest_self_hash": Hash256::ZERO,
        }));
        assert!(invalid_layer.is_err());
    }

    #[test]
    fn expert_payload_digest_exhausts_layer_local_expert_ids() {
        let entries = (0..S7_N_EXPERTS)
            .map(|expert| {
                ExpertPayloadEntry::new(ExpertId::new(expert), 128, QuantSpec::default()).unwrap()
            })
            .collect::<Vec<_>>();
        let digest =
            ExpertPayloadDigest::new(LayerId::new(0), "model.layers.0.experts", entries.clone())
                .unwrap();

        assert_eq!(
            serde_json::to_value(&digest).unwrap(),
            json!({
                "schema_version": { "major": 1, "minor": 0, "patch": 0 },
                "layer_id": 0,
                "artifact_path": "model.layers.0.experts",
                "entries": [
                    { "expert_id": 0, "byte_count": 128, "weight_quant": QuantSpec::default() },
                    { "expert_id": 1, "byte_count": 128, "weight_quant": QuantSpec::default() },
                    { "expert_id": 2, "byte_count": 128, "weight_quant": QuantSpec::default() },
                    { "expert_id": 3, "byte_count": 128, "weight_quant": QuantSpec::default() },
                ],
                "digest_self_hash": Hash256::ZERO,
            })
        );

        let mut missing = entries;
        missing.pop();
        assert!(ExpertPayloadDigest::new(LayerId::new(0), "path", missing).is_err());

        let invalid_json: Result<ExpertPayloadDigest, _> = serde_json::from_value(json!({
            "schema_version": { "major": 1, "minor": 0, "patch": 0 },
            "layer_id": 0,
            "artifact_path": "model.layers.0.experts",
            "entries": [
                { "expert_id": 0, "byte_count": 128, "weight_quant": QuantSpec::default() },
                { "expert_id": 1, "byte_count": 128, "weight_quant": QuantSpec::default() },
                { "expert_id": 2, "byte_count": 128, "weight_quant": QuantSpec::default() },
                { "expert_id": 4, "byte_count": 128, "weight_quant": QuantSpec::default() },
            ],
            "digest_self_hash": Hash256::ZERO,
        }));
        assert!(invalid_json.is_err());
    }

    #[test]
    fn raw_loss_diagnostics_distill_raw_diagnostic_shape_is_pinned() {
        let unavailable = RawLossDiagnostics::new(
            1.0,
            DistillRawDiagnostic::NotAvailable {
                reason: "no_frozen_teacher".to_owned(),
                phase: TrainPhase::PhaseA,
            },
            0.5,
            0.25,
            0.75,
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(&unavailable).unwrap(),
            json!({
                "lm_loss_raw": 1.0,
                "distill_loss_raw": {
                    "kind": "not_available",
                    "reason": "no_frozen_teacher",
                    "phase": "phase_a",
                },
                "balance_loss_raw": 0.5,
                "zrouter_loss_raw": 0.25,
                "switch_loss_raw": 0.75,
                "diagnostics_self_hash": Hash256::ZERO,
            })
        );

        let value = RawLossDiagnostics::new(
            1.0,
            DistillRawDiagnostic::Value { loss: 0.125 },
            0.5,
            0.25,
            0.75,
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(&value).unwrap()["distill_loss_raw"],
            json!({
                "kind": "value",
                "loss": 0.125,
            })
        );

        let numeric_distill_raw: Result<RawLossDiagnostics, _> = serde_json::from_value(json!({
            "lm_loss_raw": 1.0,
            "distill_loss_raw": 0.0,
            "balance_loss_raw": 0.1,
            "zrouter_loss_raw": 0.2,
            "switch_loss_raw": 0.3,
            "diagnostics_self_hash": Hash256::ZERO,
        }));
        assert!(numeric_distill_raw.is_err());
    }

    #[test]
    fn run_log_completion_rules_and_grad_norms_are_validated_in_both_paths() {
        let run_log = dense_run_log_for_completion(7, S7Completion::DivergedAt { step: 2 })
            .with_computed_self_hash()
            .unwrap();
        assert_eq!(
            serde_json::to_value(&run_log).unwrap(),
            json!({
                "schema": "s7_run_log.v1",
                "seed": 7,
                "topology": "MoeTinyDenseMatched",
                "train_config_hash": Hash256::ZERO,
                "model_topology_hash": Hash256::ZERO,
                "router_config_hash": null,
                "expert_block_config_hash": null,
                "loss_config_hash": Hash256::ZERO,
                "phase_schedule_hash": Hash256::ZERO,
                "frozen_teacher_checkpoint_sha": Hash256::ZERO,
                "losses": [
                    [1, sample_losses()],
                    [2, sample_losses()],
                ],
                "grad_norms": [
                    [1, sample_grad_norms()],
                    [2, sample_grad_norms()],
                ],
                "eval_points": [[0, 2.0]],
                "final_grad_norms": sample_grad_norms(),
                "completion": { "kind": "diverged_at", "step": 2 },
                "run_log_self_hash": run_log.run_log_self_hash,
            })
        );

        let decoded: S7RunLog = serde_json::from_value(serde_json::to_value(&run_log).unwrap())
            .expect("run log shape deserializes");
        assert_eq!(decoded, run_log);

        let mut malformed = serde_json::to_value(&run_log).unwrap();
        malformed["grad_norms"] = json!([[1, sample_grad_norms()]]);
        let decoded: Result<S7RunLog, _> = serde_json::from_value(malformed);
        assert!(decoded.is_err());

        let mut missing_teacher = serde_json::to_value(&run_log).unwrap();
        missing_teacher["frozen_teacher_checkpoint_sha"] = serde_json::Value::Null;
        let decoded: Result<S7RunLog, _> = serde_json::from_value(missing_teacher);
        assert!(decoded.is_err());
    }

    #[test]
    fn run_log_canonical_round_trip_covers_all_completion_variants() {
        let run_logs = [
            dense_run_log_for_completion(1, S7Completion::Completed),
            dense_run_log_for_completion(2, S7Completion::DivergedAt { step: 2 }),
            moe_run_log_for_completion(3, S7Completion::CollapsedAt { step: 1_000 }),
        ];

        for run_log in run_logs {
            let run_log = run_log.with_computed_self_hash().unwrap();
            let bytes = run_log.canonical_json_bytes().unwrap();
            let decoded: S7RunLog = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(decoded, run_log);
            assert_eq!(
                run_log.run_log_self_hash,
                run_log.computed_self_hash().unwrap()
            );

            match run_log.completion {
                S7Completion::Completed => {
                    assert_eq!(run_log.losses.len(), S7_OPTIMIZER_STEPS as usize);
                    assert_eq!(run_log.grad_norms.len(), S7_OPTIMIZER_STEPS as usize);
                    assert_eq!(
                        run_log.eval_points.len(),
                        (S7_OPTIMIZER_STEPS / S7_EVAL_EVERY_STEPS + 1) as usize
                    );
                }
                S7Completion::DivergedAt { step } => {
                    assert_eq!(run_log.losses.len(), step as usize);
                    assert_eq!(run_log.grad_norms.len(), step as usize);
                    assert_eq!(run_log.eval_points, vec![(0, 2.0)]);
                }
                S7Completion::CollapsedAt { step } => {
                    assert_eq!(run_log.losses.len(), step as usize);
                    assert_eq!(run_log.grad_norms.len(), step as usize);
                    assert_eq!(run_log.eval_points, vec![(0, 2.0), (1_000, 2.0)]);
                }
            }
        }
    }

    #[test]
    fn run_log_rejects_non_conditional_lengths_and_bad_eval_cadence() {
        let mut completed = dense_run_log_for_completion(1, S7Completion::Completed);
        completed.losses.pop();
        completed.grad_norms.pop();
        assert!(matches!(
            completed.validate(),
            Err(S7SchemaError::RunLogLossLengthMismatch {
                observed,
                expected
            }) if observed == S7_OPTIMIZER_STEPS as usize - 1
                && expected == S7_OPTIMIZER_STEPS as usize
        ));

        let mut collapsed =
            moe_run_log_for_completion(3, S7Completion::CollapsedAt { step: 1_000 });
        collapsed.eval_points.pop();
        assert!(matches!(
            collapsed.validate(),
            Err(S7SchemaError::RunLogEvalLengthMismatch {
                observed: 1,
                expected: 2,
            })
        ));

        let mut wrong_eval_step =
            moe_run_log_for_completion(3, S7Completion::CollapsedAt { step: 1_000 });
        wrong_eval_step.eval_points[1].0 = 999;
        assert!(matches!(
            wrong_eval_step.validate(),
            Err(S7SchemaError::RunLogUnexpectedEvalStep {
                observed: 999,
                expected: 1_000,
            })
        ));

        let mut mismatched_grad_step =
            dense_run_log_for_completion(2, S7Completion::DivergedAt { step: 2 });
        mismatched_grad_step.grad_norms[1].0 = 3;
        assert!(matches!(
            mismatched_grad_step.validate(),
            Err(S7SchemaError::RunLogStepMismatch {
                loss_step: 2,
                grad_step: 3,
            })
        ));
    }

    #[test]
    fn run_log_revalidates_grad_norm_summaries_for_rl_finite() {
        let mut run_log = dense_run_log_for_completion(2, S7Completion::DivergedAt { step: 2 });
        run_log.grad_norms[1].1.global_l2 = f32::INFINITY;
        assert!(matches!(
            run_log.validate(),
            Err(S7SchemaError::NonFiniteF32 {
                field: "global_l2",
                ..
            })
        ));
        assert!(run_log.canonical_json_bytes().is_err());

        let mut run_log = dense_run_log_for_completion(2, S7Completion::DivergedAt { step: 2 });
        run_log.final_grad_norms.mean_l2 = -0.1;
        assert!(matches!(
            run_log.validate(),
            Err(S7SchemaError::NegativeF32 {
                field: "mean_l2",
                ..
            })
        ));
        assert!(run_log.computed_self_hash().is_err());
    }

    #[test]
    fn score_report_shape_and_bpc_are_pinned() {
        let report = S7ScoreReport::new(
            3,
            S7Topology::MoeTiny,
            Hash256::ZERO,
            Hash256::ZERO,
            5,
            10.0,
        )
        .unwrap()
        .with_computed_self_hash()
        .unwrap();

        assert_eq!(
            serde_json::to_value(&report).unwrap(),
            json!({
                "schema": "s7_score.v1",
                "seed": 3,
                "topology": "MoeTiny",
                "checkpoint_sha": Hash256::ZERO,
                "corpus_val_sha": Hash256::ZERO,
                "chunk_size": 256,
                "token_count": 5,
                "log2_sum": 10.0,
                "bpc": 2.0,
                "score_self_hash": report.score_self_hash,
            })
        );

        let mut malformed = serde_json::to_value(&report).unwrap();
        malformed["bpc"] = json!(3.0);
        let decoded: Result<S7ScoreReport, _> = serde_json::from_value(malformed);
        assert!(decoded.is_err());
    }

    #[test]
    fn pareto_verdict_public_strings_and_consumer_fields_are_pinned() {
        let variants = [
            (ParetoVerdict::MoeDominates, "MoE-dominates", true),
            (ParetoVerdict::DenseDominates, "dense-dominates", false),
            (
                ParetoVerdict::MoeWinsUnderByteEquivalence,
                "MoE-wins-under-byte-equivalence",
                true,
            ),
            (
                ParetoVerdict::DenseWinsUnderByteEquivalence,
                "Dense-wins-under-byte-equivalence",
                false,
            ),
            (ParetoVerdict::Incomparable, "Incomparable", false),
            (ParetoVerdict::Tied, "Tied", false),
        ];

        for (verdict, wire, h4_confirmed) in variants {
            assert_eq!(serde_json::to_value(verdict).unwrap(), json!(wire));
            assert_eq!(
                serde_json::from_value::<ParetoVerdict>(json!(wire)).unwrap(),
                verdict
            );
            assert_eq!(verdict.confirms_h4(), h4_confirmed);

            let dense_vs_moe = S7DenseVsMoeParetoFields::new(verdict).unwrap();
            assert_eq!(
                serde_json::to_value(&dense_vs_moe).unwrap(),
                json!({
                    "schema": "s7_dense_vs_moe.v1",
                    "pareto_verdict": wire,
                })
            );
            assert_eq!(
                serde_json::from_value::<S7DenseVsMoeParetoFields>(
                    serde_json::to_value(&dense_vs_moe).unwrap()
                )
                .unwrap(),
                dense_vs_moe
            );

            let frontier = S7FrontierParetoFields::new(verdict).unwrap();
            assert_eq!(
                serde_json::to_value(&frontier).unwrap(),
                json!({
                    "schema": "s7_frontier.v1",
                    "pareto_verdict": wire,
                })
            );
            assert_eq!(
                serde_json::from_value::<S7FrontierParetoFields>(
                    serde_json::to_value(&frontier).unwrap()
                )
                .unwrap(),
                frontier
            );
        }

        let bad_schema: Result<S7DenseVsMoeParetoFields, _> = serde_json::from_value(json!({
            "schema": "s7_frontier.v1",
            "pareto_verdict": "MoE-dominates",
        }));
        assert!(bad_schema.is_err());

        let unknown_variant: Result<ParetoVerdict, _> =
            serde_json::from_value(json!("MoE-dominates-with-tolerance"));
        assert!(unknown_variant.is_err());
    }

    #[test]
    fn dense_vs_moe_report_public_shape_and_self_hash_are_pinned() {
        let report = dense_vs_moe_report().with_computed_self_hash().unwrap();

        report.verify_self_hash().unwrap();
        assert_ne!(report.comparison_self_hash, Hash256::ZERO);

        let encoded = report.canonical_json_bytes().unwrap();
        let decoded: S7DenseVsMoeComparisonReport = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded, report);
        decoded.verify_self_hash().unwrap();

        let matched_bytes_pin = serde_json::to_value(&report.matched_bytes_pin).unwrap();
        assert_eq!(
            serde_json::to_value(&report).unwrap(),
            json!({
                "schema": "s7_dense_vs_moe.v1",
                "moe_topology_hash": Hash256::ZERO,
                "dense_matched_topology_hash": Hash256::ZERO,
                "matched_bytes_pin": matched_bytes_pin,
                "per_seed": [
                    {
                        "seed": 0,
                        "val_bpc_moe": 1.0,
                        "val_bpc_dense": 1.25,
                        "delta": 0.25,
                        "parity_verdict": "Pass",
                    },
                    {
                        "seed": 1,
                        "val_bpc_moe": 1.125,
                        "val_bpc_dense": 1.375,
                        "delta": 0.25,
                        "parity_verdict": "Pass",
                    },
                    {
                        "seed": 2,
                        "val_bpc_moe": 1.25,
                        "val_bpc_dense": 1.5,
                        "delta": 0.25,
                        "parity_verdict": "Pass",
                    },
                    {
                        "seed": 3,
                        "val_bpc_moe": 1.375,
                        "val_bpc_dense": 1.625,
                        "delta": 0.25,
                        "parity_verdict": "Pass",
                    },
                    {
                        "seed": 4,
                        "val_bpc_moe": 1.5,
                        "val_bpc_dense": 1.75,
                        "delta": 0.25,
                        "parity_verdict": "Pass",
                    },
                ],
                "median_val_bpc_moe": 1.25,
                "median_val_bpc_dense": 1.5,
                "deployed_bytes_total_moe": 1000,
                "deployed_bytes_total_dense": 1010,
                "bytes_diff": 10,
                "bytes_within_tolerance": true,
                "aggregate_parity_verdict": "Pass-clean",
                "pareto_verdict": "MoE-dominates",
                "switch_stats_summary": {
                    "same_expert_rate_per_layer_q8_8": [256, 128, 64, 0],
                    "expert_usage_entropy_bits_mean": 1.75,
                    "bank_switches_per_token_mean": 0.5,
                },
                "sweep_summary": {
                    "bpc_at_lambda": {
                        "0.0": 1.02,
                        "0.05": 1.03,
                        "5.0": 1.5,
                    },
                    "entropy_at_lambda": {
                        "0.0": 1.9500000476837158,
                        "0.05": 1.899999976158142,
                        "5.0": 1.2000000476837158,
                    },
                    "guardrail_verdict": { "kind": "pass" },
                },
                "comparison_self_hash": report.comparison_self_hash,
            })
        );
    }

    #[test]
    fn dense_vs_moe_report_constructs_all_aggregate_parity_verdicts() {
        let pass = dense_vs_moe_report();
        assert_eq!(
            pass.aggregate_parity_verdict,
            S7AggregateParityVerdict::PassClean
        );

        let fail_bytes = dense_vs_moe_report_from_rows(
            comparison_rows(1.0, 1.25),
            1000,
            1030,
            20,
            ParetoVerdict::MoeDominates,
        );
        assert_eq!(
            fail_bytes.aggregate_parity_verdict,
            S7AggregateParityVerdict::FailBytes
        );

        let fail_parity = dense_vs_moe_report_from_rows(
            comparison_rows(1.20, 1.25),
            1000,
            1010,
            20,
            ParetoVerdict::MoeDominates,
        );
        assert_eq!(
            fail_parity.aggregate_parity_verdict,
            S7AggregateParityVerdict::FailParity
        );
    }

    #[test]
    fn dense_vs_moe_per_seed_parity_fails_at_exact_margin() {
        let edge = S7PerSeedComparison::new(0, 1.20, 1.25, 0.05, S7ParityVerdict::Fail)
            .expect("exact 0.05 BPC margin does not pass parity");
        assert_eq!(edge.parity_verdict, S7ParityVerdict::Fail);

        let bad_pass = S7PerSeedComparison::new(0, 1.20, 1.25, 0.05, S7ParityVerdict::Pass);
        assert!(matches!(
            bad_pass,
            Err(S7SchemaError::PerSeedParityVerdictMismatch {
                seed: 0,
                observed: S7ParityVerdict::Pass,
                expected: S7ParityVerdict::Fail,
            })
        ));
    }

    #[test]
    fn dense_vs_moe_report_constructs_all_pareto_verdict_variants() {
        let cases = [
            (
                ParetoVerdict::MoeDominates,
                comparison_rows(1.0, 1.25),
                1000,
                1010,
                20,
            ),
            (
                ParetoVerdict::DenseDominates,
                comparison_rows(1.25, 1.0),
                1010,
                1000,
                20,
            ),
            (
                ParetoVerdict::MoeWinsUnderByteEquivalence,
                comparison_rows(1.0, 1.25),
                1010,
                1000,
                20,
            ),
            (
                ParetoVerdict::DenseWinsUnderByteEquivalence,
                comparison_rows(1.25, 1.0),
                1000,
                1010,
                20,
            ),
            (
                ParetoVerdict::Incomparable,
                comparison_rows(1.0, 1.25),
                1100,
                1000,
                20,
            ),
            (
                ParetoVerdict::Tied,
                comparison_rows(1.25, 1.25),
                1000,
                1000,
                20,
            ),
        ];

        for (expected, rows, moe_bytes, dense_bytes, tolerance_bytes) in cases {
            let report = dense_vs_moe_report_from_rows(
                rows,
                moe_bytes,
                dense_bytes,
                tolerance_bytes,
                expected,
            );
            assert_eq!(report.pareto_verdict, expected);
        }

        assert_eq!(
            derive_pareto_verdict(1.0, 1.0 + SCORE_BPC_EPSILON / 2.0, 1000, 1000, 20),
            ParetoVerdict::Tied
        );
    }

    #[test]
    fn dense_vs_moe_report_accepts_unsorted_per_seed_rows_when_medians_match() {
        let report = dense_vs_moe_report_from_rows(
            vec![
                per_seed_comparison(4, 1.50, 1.75),
                per_seed_comparison(2, 1.25, 1.50),
                per_seed_comparison(0, 1.00, 1.25),
                per_seed_comparison(3, 1.375, 1.625),
                per_seed_comparison(1, 1.125, 1.375),
            ],
            1000,
            1010,
            20,
            ParetoVerdict::MoeDominates,
        );

        assert_eq!(report.median_val_bpc_moe, 1.25);
        assert_eq!(report.median_val_bpc_dense, 1.5);
    }

    #[test]
    fn dense_vs_moe_summary_fields_validate_rates_lengths_and_maps() {
        assert!(matches!(
            SwitchStatsSummary::new(vec![256, 128, 64], 1.75, 0.5),
            Err(S7SchemaError::SwitchStatsLayerCountMismatch { .. })
        ));
        assert!(matches!(
            SwitchStatsSummary::new(vec![256, 257, 64, 0], 1.75, 0.5),
            Err(S7SchemaError::Q8_8OutOfRange {
                field: "same_expert_rate_per_layer_q8_8",
                value: 257,
            })
        ));
        assert!(matches!(
            SwitchStatsSummary::new(vec![256, 128, 64, 0], f32::NAN, 0.5),
            Err(S7SchemaError::NonFiniteF32 {
                field: "expert_usage_entropy_bits_mean",
                ..
            })
        ));

        assert!(matches!(
            SweepSummary::new(
                BTreeMap::new(),
                lambda_entropy_map(),
                GuardrailVerdict::Pass
            ),
            Err(S7SchemaError::EmptySweepSummaryMap {
                field: "bpc_at_lambda",
            })
        ));

        let mut mismatched_entropy = lambda_entropy_map();
        mismatched_entropy.remove(&lambda("5.0"));
        assert!(matches!(
            SweepSummary::new(lambda_bpc_map(), mismatched_entropy, GuardrailVerdict::Pass,),
            Err(S7SchemaError::SweepSummaryLambdaSetMismatch)
        ));

        let mut bad_bpc = lambda_bpc_map();
        *bad_bpc.get_mut(&lambda("0.05")).unwrap() = f64::INFINITY;
        assert!(matches!(
            SweepSummary::new(bad_bpc, lambda_entropy_map(), GuardrailVerdict::Pass),
            Err(S7SchemaError::NonFiniteF64 {
                field: "bpc_at_lambda",
                ..
            })
        ));

        let invalid_lambda: Result<LambdaSwitch, _> = serde_json::from_value(json!("-0.1"));
        assert!(matches!(invalid_lambda, Err(_)));
    }

    #[test]
    fn dense_vs_moe_report_rejects_non_canonical_derived_fields() {
        let mut bad = dense_vs_moe_report();
        bad.bytes_within_tolerance = false;
        assert!(matches!(
            bad.validate(),
            Err(S7SchemaError::BytesWithinToleranceMismatch {
                observed: false,
                expected: true,
            })
        ));

        let mut bad = dense_vs_moe_report();
        bad.aggregate_parity_verdict = S7AggregateParityVerdict::FailParity;
        assert!(matches!(
            bad.validate(),
            Err(S7SchemaError::AggregateParityVerdictMismatch { .. })
        ));

        let mut bad = dense_vs_moe_report();
        bad.pareto_verdict = ParetoVerdict::Incomparable;
        assert!(matches!(
            bad.validate(),
            Err(S7SchemaError::ParetoVerdictMismatch { .. })
        ));

        let mut bad = dense_vs_moe_report();
        bad.per_seed[0].delta = 0.0;
        assert!(matches!(
            bad.validate(),
            Err(S7SchemaError::PerSeedDeltaMismatch { seed: 0, .. })
        ));

        let mut bad = dense_vs_moe_report();
        bad.median_val_bpc_moe = 9.0;
        assert!(matches!(
            bad.validate(),
            Err(S7SchemaError::MedianBpcMismatch {
                field: "median_val_bpc_moe",
                ..
            })
        ));

        let mut bad = dense_vs_moe_report().with_computed_self_hash().unwrap();
        bad.moe_topology_hash = Hash256::from_bytes([1; 32]);
        assert!(matches!(
            bad.verify_self_hash(),
            Err(S7SchemaError::SelfHashMismatch {
                field: "comparison_self_hash",
                ..
            })
        ));
    }

    #[test]
    fn dense_vs_moe_report_deserialization_revalidates_shape() {
        let report = dense_vs_moe_report().with_computed_self_hash().unwrap();

        let mut value = serde_json::to_value(&report).unwrap();
        value["per_seed"] = json!([]);
        let decoded: Result<S7DenseVsMoeComparisonReport, _> = serde_json::from_value(value);
        assert!(decoded.is_err());

        let mut bad_pin = dense_vs_moe_report();
        bad_pin.matched_bytes_pin.b_deployed_total_moe += 1;
        assert!(matches!(
            bad_pin.validate(),
            Err(S7SchemaError::MatchedBytesPinSelfHashInvalid)
        ));
    }

    fn dense_run_log_for_completion(seed: u64, completion: S7Completion) -> S7RunLog {
        run_log_for_completion(seed, S7Topology::MoeTinyDenseMatched, completion)
    }

    fn dense_vs_moe_report() -> S7DenseVsMoeComparisonReport {
        dense_vs_moe_report_from_rows(
            comparison_rows(1.25, 1.5),
            1000,
            1010,
            20,
            ParetoVerdict::MoeDominates,
        )
    }

    fn dense_vs_moe_report_from_rows(
        per_seed: Vec<S7PerSeedComparison>,
        deployed_bytes_total_moe: u64,
        deployed_bytes_total_dense: u64,
        tolerance_bytes: u64,
        pareto_verdict: ParetoVerdict,
    ) -> S7DenseVsMoeComparisonReport {
        let pin = MatchedBytesPin {
            formula_version: SemVer::new(0, 2, 0),
            d_ff_dense_resolved: 572,
            bias_policy: gbf_policy::BiasPolicy::Q8_8PerOutput,
            b_experts_total: 900,
            b_router_overhead_total: 100,
            b_dense_ffn_total: deployed_bytes_total_dense,
            b_deployed_total_moe: deployed_bytes_total_moe,
            b_deployed_total_dense: deployed_bytes_total_dense,
            tolerance_bytes,
            matched_bytes_self_hash: Hash256::ZERO,
        }
        .with_computed_self_hash()
        .unwrap();

        S7DenseVsMoeComparisonReport::new(
            Hash256::ZERO,
            Hash256::ZERO,
            pin,
            per_seed.clone(),
            median_bpc(&per_seed, |entry| entry.val_bpc_moe),
            median_bpc(&per_seed, |entry| entry.val_bpc_dense),
            deployed_bytes_total_moe,
            deployed_bytes_total_dense,
            checked_signed_bytes_diff(deployed_bytes_total_dense, deployed_bytes_total_moe)
                .unwrap(),
            deployed_bytes_total_dense.abs_diff(deployed_bytes_total_moe) <= tolerance_bytes,
            derive_aggregate_parity_verdict(
                &per_seed,
                deployed_bytes_total_dense.abs_diff(deployed_bytes_total_moe),
                tolerance_bytes,
            ),
            pareto_verdict,
            switch_stats_summary(),
            sweep_summary(),
        )
        .unwrap()
    }

    fn comparison_rows(moe_median: f64, dense_median: f64) -> Vec<S7PerSeedComparison> {
        [-0.25, -0.125, 0.0, 0.125, 0.25]
            .into_iter()
            .enumerate()
            .map(|(seed, offset)| {
                per_seed_comparison(
                    u64::try_from(seed).unwrap(),
                    moe_median + offset,
                    dense_median + offset,
                )
            })
            .collect()
    }

    fn per_seed_comparison(seed: u64, val_bpc_moe: f64, val_bpc_dense: f64) -> S7PerSeedComparison {
        let parity = derive_parity_verdict(val_bpc_moe, val_bpc_dense);
        S7PerSeedComparison::new(
            seed,
            val_bpc_moe,
            val_bpc_dense,
            val_bpc_dense - val_bpc_moe,
            parity,
        )
        .unwrap()
    }

    fn switch_stats_summary() -> SwitchStatsSummary {
        SwitchStatsSummary::new(vec![256, 128, 64, 0], 1.75, 0.5).unwrap()
    }

    fn sweep_summary() -> SweepSummary {
        SweepSummary::new(
            lambda_bpc_map(),
            lambda_entropy_map(),
            GuardrailVerdict::Pass,
        )
        .unwrap()
    }

    fn lambda_bpc_map() -> BTreeMap<LambdaSwitch, f64> {
        BTreeMap::from([
            (lambda("0.0"), 1.02),
            (lambda("0.05"), 1.03),
            (lambda("5.0"), 1.5),
        ])
    }

    fn lambda_entropy_map() -> BTreeMap<LambdaSwitch, f32> {
        BTreeMap::from([
            (lambda("0.0"), 1.95),
            (lambda("0.05"), 1.9),
            (lambda("5.0"), 1.2),
        ])
    }

    fn lambda(value: &str) -> LambdaSwitch {
        LambdaSwitch::new(value).unwrap()
    }

    fn moe_run_log_for_completion(seed: u64, completion: S7Completion) -> S7RunLog {
        run_log_for_completion(seed, S7Topology::MoeTiny, completion)
    }

    fn run_log_for_completion(
        seed: u64,
        topology: S7Topology,
        completion: S7Completion,
    ) -> S7RunLog {
        let last_completed_step = completion.expected_loss_steps().unwrap();
        let losses = (1..=last_completed_step)
            .map(|step| (step, sample_losses()))
            .collect::<Vec<_>>();
        let grad_norms = (1..=last_completed_step)
            .map(|step| (step, sample_grad_norms()))
            .collect::<Vec<_>>();
        let eval_points = (0..completion.expected_eval_points().unwrap())
            .map(|index| (u64::try_from(index).unwrap() * S7_EVAL_EVERY_STEPS, 2.0))
            .collect::<Vec<_>>();
        let (router_config_hash, expert_block_config_hash) = match &topology {
            S7Topology::MoeTiny => (Some(Hash256::ZERO), Some(Hash256::ZERO)),
            S7Topology::MoeTinyDenseMatched => (None, None),
        };
        S7RunLog::new(
            seed,
            topology,
            Hash256::ZERO,
            Hash256::ZERO,
            router_config_hash,
            expert_block_config_hash,
            Hash256::ZERO,
            Hash256::ZERO,
            Some(Hash256::ZERO),
            losses,
            grad_norms,
            eval_points,
            sample_grad_norms(),
            completion,
        )
        .unwrap()
    }

    fn sample_losses() -> RawLossDiagnostics {
        RawLossDiagnostics::new(
            1.0,
            DistillRawDiagnostic::NotAvailable {
                reason: "no_frozen_teacher".to_owned(),
                phase: TrainPhase::PhaseA,
            },
            0.1,
            0.2,
            0.3,
        )
        .unwrap()
    }

    fn sample_grad_norms() -> GradNormSummary {
        GradNormSummary::new(1.0, 0.8, 0.4).unwrap()
    }
}
