//! Explicit deployed weight numeric plans.

use std::num::NonZeroU16;

use gbf_foundation::ByteCost;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WeightEncoding {
    /// Dense ternary values packed row-major at four values per byte.
    ///
    /// The first matrix element occupies the least-significant two bits of the
    /// first byte, then the next element advances by two bits. Codes are
    /// `00 = 0`, `01 = +1`, `10 = -1`, and `11` is reserved. Padding trits at
    /// the end of the last byte must be encoded as `00`.
    Ternary2,
    /// Dense ternary values represented as two row-major bitplanes.
    ///
    /// The first bitplane is the non-zero mask and the second bitplane is the
    /// negative sign mask. Bits are least-significant-bit first within each
    /// byte. A sign bit without the corresponding non-zero bit is invalid, and
    /// padding bits in both planes must be zero.
    SparseTernaryBitplanes,
    /// One-bit signed weights packed row-major, least-significant-bit first.
    ///
    /// `1` represents `+1`, `0` represents `-1`, and zero-valued ternary
    /// weights are not representable. Padding bits must be zero.
    Binary1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TernaryWeightPlan {
    pub encoding: WeightEncoding,
    pub scale_granularity: ScaleGranularity,
    pub scale_format: ScaleFormat,
    pub threshold: ThresholdPlan,
}

impl TernaryWeightPlan {
    #[must_use]
    pub const fn new(
        encoding: WeightEncoding,
        scale_granularity: ScaleGranularity,
        scale_format: ScaleFormat,
        threshold: ThresholdPlan,
    ) -> Self {
        Self {
            encoding,
            scale_granularity,
            scale_format,
            threshold,
        }
    }

    #[must_use]
    pub fn compute_byte_cost(&self, rows: u32, cols: u32) -> ByteCost {
        if rows == 0 || cols == 0 {
            return ByteCost::ZERO;
        }

        let weights = self.encoding.weight_bytes(rows, cols);
        let scales = self
            .scale_granularity
            .scale_count(rows, cols)
            .saturating_mul(u128::from(self.scale_format.byte_len()));

        ByteCost::new(saturating_u64(weights.saturating_add(scales)))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[allow(clippy::enum_variant_names)]
pub enum ScaleGranularity {
    PerTensor,
    PerOutputRow,
    PerGroup(NonZeroU16),
}

impl ScaleGranularity {
    #[must_use]
    pub fn per_group(group_size: u16) -> Option<Self> {
        NonZeroU16::new(group_size).map(Self::PerGroup)
    }

    fn scale_count(self, rows: u32, cols: u32) -> u128 {
        match self {
            Self::PerTensor => 1,
            Self::PerOutputRow => u128::from(rows),
            Self::PerGroup(group_size) => ceil_div(
                u128::from(rows) * u128::from(cols),
                u128::from(group_size.get()),
            ),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ScaleFormat {
    Q8_8,
    Q4_4,
    Pow2,
}

impl ScaleFormat {
    #[must_use]
    pub const fn byte_len(self) -> u8 {
        match self {
            Self::Q8_8 => 2,
            Self::Q4_4 | Self::Pow2 => 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ThresholdPlan {
    FixedQ8_8,
    AnnealedGlobalThenPerOutputRow,
    LearnedPerGroup(NonZeroU16),
}

impl ThresholdPlan {
    #[must_use]
    pub fn learned_per_group(group_size: u16) -> Option<Self> {
        NonZeroU16::new(group_size).map(Self::LearnedPerGroup)
    }
}

impl WeightEncoding {
    fn weight_bytes(self, rows: u32, cols: u32) -> u128 {
        let weights = u128::from(rows) * u128::from(cols);
        match self {
            Self::Ternary2 => ceil_div(weights.saturating_mul(2), 8),
            Self::SparseTernaryBitplanes => ceil_div(weights, 8).saturating_mul(2),
            Self::Binary1 => ceil_div(weights, 8),
        }
    }
}

fn ceil_div(numerator: u128, denominator: u128) -> u128 {
    numerator.div_ceil(denominator)
}

fn saturating_u64(value: u128) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ternary_weight_plan_computes_per_output_row_q8_8_cost() {
        let plan = TernaryWeightPlan::new(
            WeightEncoding::Ternary2,
            ScaleGranularity::PerOutputRow,
            ScaleFormat::Q8_8,
            ThresholdPlan::FixedQ8_8,
        );

        assert_eq!(plan.compute_byte_cost(64, 128), ByteCost::new(2176));
    }

    #[test]
    fn ternary_weight_plan_computes_scale_granularity_costs() {
        let per_tensor = TernaryWeightPlan::new(
            WeightEncoding::Ternary2,
            ScaleGranularity::PerTensor,
            ScaleFormat::Q8_8,
            ThresholdPlan::FixedQ8_8,
        );
        let per_group = TernaryWeightPlan::new(
            WeightEncoding::Ternary2,
            ScaleGranularity::per_group(16).unwrap(),
            ScaleFormat::Q4_4,
            ThresholdPlan::learned_per_group(16).unwrap(),
        );

        assert_eq!(per_tensor.compute_byte_cost(2, 8), ByteCost::new(6));
        assert_eq!(per_group.compute_byte_cost(2, 8), ByteCost::new(5));
        assert_eq!(per_tensor.compute_byte_cost(0, 8), ByteCost::ZERO);
    }

    #[test]
    fn ternary_weight_plan_rejects_zero_group_sizes_at_construction() {
        assert_eq!(ScaleGranularity::per_group(0), None);
        assert_eq!(ThresholdPlan::learned_per_group(0), None);
    }

    #[test]
    fn ternary_weight_plan_cost_is_monotonic_for_shape_growth() {
        let plan = TernaryWeightPlan::new(
            WeightEncoding::Ternary2,
            ScaleGranularity::PerOutputRow,
            ScaleFormat::Q8_8,
            ThresholdPlan::FixedQ8_8,
        );

        for rows in 1..16 {
            for cols in 1..16 {
                let base = plan.compute_byte_cost(rows, cols);
                assert!(plan.compute_byte_cost(rows + 1, cols) >= base);
                assert!(plan.compute_byte_cost(rows, cols + 1) >= base);
            }
        }
    }

    #[test]
    fn ternary_weight_plan_round_trips_through_serde() {
        let plan = TernaryWeightPlan::new(
            WeightEncoding::Ternary2,
            ScaleGranularity::per_group(32).unwrap(),
            ScaleFormat::Pow2,
            ThresholdPlan::AnnealedGlobalThenPerOutputRow,
        );

        let encoded = serde_json::to_string(&plan).expect("plan serializes");
        let decoded: TernaryWeightPlan = serde_json::from_str(&encoded).expect("plan deserializes");

        assert_eq!(decoded, plan);
    }
}
