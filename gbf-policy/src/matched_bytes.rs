//! F-S7 matched-deployed-bytes accounting helpers.

use std::fmt;
use std::str::FromStr;

use gbf_foundation::{ByteCost, SemVer};
use serde::{Deserialize, Serialize};

/// D6 matched-deployed-bytes formula version used by S7 v0.2.
pub const MATCHED_BYTES_FORMULA_VERSION: SemVer = SemVer::new(0, 2, 0);

/// Current S7 matched-bytes ternary metadata policy value.
///
/// This is the metadata byte estimate used by the D6/O11 accounting contract,
/// not a claim about the final F-A4 packed-layout encoding. The committed
/// `matched_bytes.json` pin is the preregistration point for this value; if the
/// deployed metadata contract changes, update this policy value and the pin
/// together.
pub const S7_TERNARY_METADATA_BYTES: ByteCost = ByteCost::new(50);

/// Canonical S7 deployment bias policy.
pub const S7_CANONICAL_BIAS_POLICY: BiasPolicy = BiasPolicy::Q8_8PerOutput;

/// Canonical S7 single ROM bank byte width.
pub const S7_ONE_BANK_BYTES: ByteCost =
    ByteCost::new(gbf_hw::memory::SWITCHABLE_BANK_SIZE_BYTES as u64);

/// D7 high-precision router parameter payload width for matched-byte accounting.
pub const S7_ROUTER_HIGH_PRECISION_BYTES_PER_PARAM: u8 = 4;

/// D6 maximum admissible dense FFN width.
pub const S7_D_FF_DENSE_MAX: u16 = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinearShape {
    pub out_rows: u32,
    pub in_cols: u32,
}

impl LinearShape {
    #[must_use]
    pub const fn new(out_rows: u32, in_cols: u32) -> Self {
        Self { out_rows, in_cols }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BiasPolicy {
    NotDeployed,
    Folded,
    Q8_8PerOutput,
    Fp16PerOutput,
}

impl BiasPolicy {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotDeployed => "not_deployed",
            Self::Folded => "folded",
            Self::Q8_8PerOutput => "q8_8_per_output",
            Self::Fp16PerOutput => "fp16_per_output",
        }
    }
}

impl fmt::Display for BiasPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for BiasPolicy {
    type Err = BiasPolicyParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "not_deployed" => Ok(Self::NotDeployed),
            "folded" => Ok(Self::Folded),
            "q8_8_per_output" => Ok(Self::Q8_8PerOutput),
            "fp16_per_output" => Ok(Self::Fp16PerOutput),
            _ => Err(BiasPolicyParseError {
                value: value.to_owned(),
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BiasPolicyParseError {
    value: String,
}

impl BiasPolicyParseError {
    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }
}

impl fmt::Display for BiasPolicyParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "unknown bias_policy {:?}; expected one of not_deployed, folded, q8_8_per_output, fp16_per_output",
            self.value
        )
    }
}

impl std::error::Error for BiasPolicyParseError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MatchedBytesPolicy {
    pub formula_version: SemVer,
    pub ternary_metadata_bytes: ByteCost,
    pub bias_policy: BiasPolicy,
    pub one_bank_bytes: ByteCost,
    pub router_parameter_bytes: u8,
}

impl MatchedBytesPolicy {
    #[must_use]
    pub const fn new(ternary_metadata_bytes: ByteCost, bias_policy: BiasPolicy) -> Self {
        Self::from_parts(
            MATCHED_BYTES_FORMULA_VERSION,
            ternary_metadata_bytes,
            bias_policy,
            S7_ONE_BANK_BYTES,
            S7_ROUTER_HIGH_PRECISION_BYTES_PER_PARAM,
        )
    }

    #[must_use]
    pub const fn from_parts(
        formula_version: SemVer,
        ternary_metadata_bytes: ByteCost,
        bias_policy: BiasPolicy,
        one_bank_bytes: ByteCost,
        router_parameter_bytes: u8,
    ) -> Self {
        Self {
            formula_version,
            ternary_metadata_bytes,
            bias_policy,
            one_bank_bytes,
            router_parameter_bytes,
        }
    }

    #[must_use]
    pub const fn s7_canonical() -> Self {
        Self::new(S7_TERNARY_METADATA_BYTES, S7_CANONICAL_BIAS_POLICY)
    }
}

impl Default for MatchedBytesPolicy {
    fn default() -> Self {
        Self::s7_canonical()
    }
}

#[must_use]
pub fn compute_weight_byte_cost(linear: LinearShape, policy: MatchedBytesPolicy) -> ByteCost {
    if linear.out_rows == 0 || linear.in_cols == 0 {
        return ByteCost::ZERO;
    }

    let weights = u128::from(linear.out_rows) * u128::from(linear.in_cols);
    let ternary_bytes = weights.div_ceil(4);
    let scale_bytes = u128::from(linear.out_rows) * 2;
    let metadata_bytes = u128::from(policy.ternary_metadata_bytes.as_u64());

    ByteCost::new(saturating_u64(
        ternary_bytes
            .saturating_add(scale_bytes)
            .saturating_add(metadata_bytes),
    ))
}

#[must_use]
pub fn bias_byte_cost(linear: LinearShape, bias_policy: BiasPolicy) -> ByteCost {
    if linear.out_rows == 0 {
        return ByteCost::ZERO;
    }

    match bias_policy {
        BiasPolicy::NotDeployed | BiasPolicy::Folded => ByteCost::ZERO,
        BiasPolicy::Q8_8PerOutput | BiasPolicy::Fp16PerOutput => {
            ByteCost::new(u64::from(linear.out_rows).saturating_mul(2))
        }
    }
}

#[must_use]
pub fn compute_linear_deployed_byte_cost(
    linear: LinearShape,
    policy: MatchedBytesPolicy,
) -> ByteCost {
    compute_weight_byte_cost(linear, policy) + bias_byte_cost(linear, policy.bias_policy)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MatchedBytesConfig {
    pub d_model: u16,
    pub d_ff_moe: u16,
    pub n_blocks: u8,
    pub n_experts: u8,
    pub router_rank: u8,
    pub d_ff_dense_min: u16,
    pub d_ff_dense_max: u16,
    pub common_deployed_bytes: ByteCost,
}

impl MatchedBytesConfig {
    #[must_use]
    pub const fn s7_moe_tiny() -> Self {
        Self {
            d_model: 64,
            d_ff_moe: 128,
            n_blocks: 4,
            n_experts: 4,
            router_rank: 4,
            d_ff_dense_min: 64,
            d_ff_dense_max: S7_D_FF_DENSE_MAX,
            common_deployed_bytes: ByteCost::ZERO,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MatchedBytesSolution {
    pub d_ff_dense: u16,
    pub b_experts_total: ByteCost,
    pub b_router_overhead_total: ByteCost,
    pub b_dense_ffn_total: ByteCost,
    pub b_deployed_total_moe: ByteCost,
    pub b_deployed_total_dense: ByteCost,
    pub tolerance_bytes: ByteCost,
}

impl MatchedBytesSolution {
    #[must_use]
    pub fn deployed_bytes_diff(self) -> i128 {
        i128::from(self.b_deployed_total_moe.as_u64())
            - i128::from(self.b_deployed_total_dense.as_u64())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MatchedBytesError {
    EmptyDimension {
        field: &'static str,
    },
    InvalidDenseRange {
        min: u16,
        max: u16,
    },
    InvalidPolicy {
        field: &'static str,
        value: u64,
    },
    MatchedBytesInfeasible {
        target_bytes: ByteCost,
        tolerance_bytes: ByteCost,
        min_d_ff_dense: u16,
        max_d_ff_dense: u16,
    },
}

impl fmt::Display for MatchedBytesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyDimension { field } => write!(f, "{field} must be nonzero"),
            Self::InvalidDenseRange { min, max } => {
                write!(f, "d_ff_dense range must be ordered, got {min}..={max}")
            }
            Self::InvalidPolicy { field, value } => {
                write!(
                    f,
                    "matched-bytes policy field {field} must be nonzero, got {value}"
                )
            }
            Self::MatchedBytesInfeasible {
                target_bytes,
                tolerance_bytes,
                min_d_ff_dense,
                max_d_ff_dense,
            } => write!(
                f,
                "no d_ff_dense in {min_d_ff_dense}..={max_d_ff_dense} matches target {target_bytes} within tolerance {tolerance_bytes}"
            ),
        }
    }
}

impl std::error::Error for MatchedBytesError {}

pub fn solve_d_ff_dense(
    config: MatchedBytesConfig,
    policy: MatchedBytesPolicy,
) -> Result<MatchedBytesSolution, MatchedBytesError> {
    validate_config(config)?;
    validate_policy(policy)?;

    let b_experts_total = compute_moe_experts_total(config, policy);
    let b_router_overhead_total = compute_router_overhead_total(config, policy);
    let b_deployed_total_moe =
        config.common_deployed_bytes + b_experts_total + b_router_overhead_total;
    let tolerance_bytes = d6_tolerance_bytes(b_deployed_total_moe, policy);

    let mut best: Option<MatchedBytesSolution> = None;
    for d_ff_dense in config.d_ff_dense_min..=config.d_ff_dense_max {
        let b_dense_ffn_total = compute_dense_ffn_total(config, d_ff_dense, policy);
        let b_deployed_total_dense = config.common_deployed_bytes + b_dense_ffn_total;
        let diff = abs_diff_bytes(b_deployed_total_dense, b_deployed_total_moe);
        if diff > tolerance_bytes.as_u64() {
            continue;
        }

        let candidate = MatchedBytesSolution {
            d_ff_dense,
            b_experts_total,
            b_router_overhead_total,
            b_dense_ffn_total,
            b_deployed_total_moe,
            b_deployed_total_dense,
            tolerance_bytes,
        };

        if best.is_none_or(|current| candidate_is_better(candidate, current)) {
            best = Some(candidate);
        }
    }

    best.ok_or(MatchedBytesError::MatchedBytesInfeasible {
        target_bytes: b_deployed_total_moe,
        tolerance_bytes,
        min_d_ff_dense: config.d_ff_dense_min,
        max_d_ff_dense: config.d_ff_dense_max,
    })
}

#[must_use]
pub fn compute_moe_experts_total(
    config: MatchedBytesConfig,
    policy: MatchedBytesPolicy,
) -> ByteCost {
    let up = compute_linear_deployed_byte_cost(
        LinearShape::new(u32::from(config.d_ff_moe), u32::from(config.d_model)),
        policy,
    );
    let down = compute_linear_deployed_byte_cost(
        LinearShape::new(u32::from(config.d_model), u32::from(config.d_ff_moe)),
        policy,
    );
    multiply_byte_cost(
        up + down,
        u64::from(config.n_blocks) * u64::from(config.n_experts),
    )
}

#[must_use]
pub fn compute_dense_ffn_total(
    config: MatchedBytesConfig,
    d_ff_dense: u16,
    policy: MatchedBytesPolicy,
) -> ByteCost {
    let up = compute_linear_deployed_byte_cost(
        LinearShape::new(u32::from(d_ff_dense), u32::from(config.d_model)),
        policy,
    );
    let down = compute_linear_deployed_byte_cost(
        LinearShape::new(u32::from(config.d_model), u32::from(d_ff_dense)),
        policy,
    );
    multiply_byte_cost(up + down, u64::from(config.n_blocks))
}

#[must_use]
pub fn compute_router_overhead_total(
    config: MatchedBytesConfig,
    policy: MatchedBytesPolicy,
) -> ByteCost {
    let params = u128::from(config.d_model)
        .saturating_mul(u128::from(config.router_rank))
        .saturating_add(
            u128::from(config.router_rank).saturating_mul(u128::from(config.n_experts)),
        );
    let total = params
        .saturating_mul(u128::from(policy.router_parameter_bytes))
        .saturating_mul(u128::from(config.n_blocks));
    ByteCost::new(saturating_u64(total))
}

#[must_use]
pub fn d6_tolerance_bytes(reference_bytes: ByteCost, policy: MatchedBytesPolicy) -> ByteCost {
    let ten_percent = reference_bytes.as_u64().div_ceil(10);
    let four_banks = policy.one_bank_bytes.as_u64().saturating_mul(4);
    ByteCost::new(ten_percent.max(four_banks))
}

fn validate_config(config: MatchedBytesConfig) -> Result<(), MatchedBytesError> {
    for (field, value) in [
        ("d_model", u64::from(config.d_model)),
        ("d_ff_moe", u64::from(config.d_ff_moe)),
        ("n_blocks", u64::from(config.n_blocks)),
        ("n_experts", u64::from(config.n_experts)),
        ("router_rank", u64::from(config.router_rank)),
        ("d_ff_dense_min", u64::from(config.d_ff_dense_min)),
        ("d_ff_dense_max", u64::from(config.d_ff_dense_max)),
    ] {
        if value == 0 {
            return Err(MatchedBytesError::EmptyDimension { field });
        }
    }
    if config.d_ff_dense_min > config.d_ff_dense_max {
        return Err(MatchedBytesError::InvalidDenseRange {
            min: config.d_ff_dense_min,
            max: config.d_ff_dense_max,
        });
    }
    Ok(())
}

fn validate_policy(policy: MatchedBytesPolicy) -> Result<(), MatchedBytesError> {
    for (field, value) in [
        ("one_bank_bytes", policy.one_bank_bytes.as_u64()),
        (
            "router_parameter_bytes",
            u64::from(policy.router_parameter_bytes),
        ),
    ] {
        if value == 0 {
            return Err(MatchedBytesError::InvalidPolicy { field, value });
        }
    }
    Ok(())
}

fn candidate_is_better(candidate: MatchedBytesSolution, current: MatchedBytesSolution) -> bool {
    let candidate_diff = abs_diff_bytes(
        candidate.b_deployed_total_dense,
        candidate.b_deployed_total_moe,
    );
    let current_diff = abs_diff_bytes(current.b_deployed_total_dense, current.b_deployed_total_moe);
    if candidate_diff != current_diff {
        return candidate_diff < current_diff;
    }

    let candidate_favors_dense = candidate.b_deployed_total_dense >= candidate.b_deployed_total_moe;
    let current_favors_dense = current.b_deployed_total_dense >= current.b_deployed_total_moe;
    if candidate_favors_dense != current_favors_dense {
        return candidate_favors_dense;
    }

    candidate.d_ff_dense < current.d_ff_dense
}

fn abs_diff_bytes(left: ByteCost, right: ByteCost) -> u64 {
    left.as_u64().abs_diff(right.as_u64())
}

fn multiply_byte_cost(cost: ByteCost, factor: u64) -> ByteCost {
    ByteCost::new(cost.as_u64().saturating_mul(factor))
}

fn saturating_u64(value: u128) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}
