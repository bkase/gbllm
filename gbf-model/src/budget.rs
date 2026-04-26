//! Target-independent model byte-budget formulas.

use std::error::Error;
use std::fmt;

use gbf_artifact::weight_plan::TernaryWeightPlan;
use gbf_foundation::ByteCost;

pub const ESTIMATED_EXPERT_TILE_HEADER_BYTES: ByteCost = ByteCost::new(32);
pub const ESTIMATED_EXPERT_ALIGNMENT_PADDING_BYTES: ByteCost = ByteCost::new(18);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExpertBudgetMetadata {
    tile_header_bytes: ByteCost,
    alignment_padding_bytes: ByteCost,
}

impl ExpertBudgetMetadata {
    #[must_use]
    pub const fn new(tile_header_bytes: ByteCost, alignment_padding_bytes: ByteCost) -> Self {
        Self {
            tile_header_bytes,
            alignment_padding_bytes,
        }
    }

    #[must_use]
    pub const fn zero() -> Self {
        Self::new(ByteCost::ZERO, ByteCost::ZERO)
    }

    #[must_use]
    pub const fn tile_header_bytes(self) -> ByteCost {
        self.tile_header_bytes
    }

    #[must_use]
    pub const fn alignment_padding_bytes(self) -> ByteCost {
        self.alignment_padding_bytes
    }

    #[must_use]
    pub fn total(self) -> ByteCost {
        self.tile_header_bytes + self.alignment_padding_bytes
    }
}

impl Default for ExpertBudgetMetadata {
    fn default() -> Self {
        Self::new(
            ESTIMATED_EXPERT_TILE_HEADER_BYTES,
            ESTIMATED_EXPERT_ALIGNMENT_PADDING_BYTES,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExpertByteBreakdown {
    d_model: u32,
    d_ff: u32,
    up_projection_bytes: ByteCost,
    down_projection_bytes: ByteCost,
    metadata: ExpertBudgetMetadata,
}

impl ExpertByteBreakdown {
    #[must_use]
    pub const fn d_model(self) -> u32 {
        self.d_model
    }

    #[must_use]
    pub const fn d_ff(self) -> u32 {
        self.d_ff
    }

    #[must_use]
    pub const fn up_projection_bytes(self) -> ByteCost {
        self.up_projection_bytes
    }

    #[must_use]
    pub const fn down_projection_bytes(self) -> ByteCost {
        self.down_projection_bytes
    }

    #[must_use]
    pub const fn metadata(self) -> ExpertBudgetMetadata {
        self.metadata
    }

    #[must_use]
    pub fn projection_bytes(self) -> ByteCost {
        self.up_projection_bytes + self.down_projection_bytes
    }

    #[must_use]
    pub fn total(self) -> ByteCost {
        self.projection_bytes() + self.metadata.total()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpertBudgetError {
    EmptyDimension { field: &'static str },
}

impl fmt::Display for ExpertBudgetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyDimension { field } => write!(f, "{field} must be nonzero"),
        }
    }
}

impl Error for ExpertBudgetError {}

#[must_use]
pub fn compute_expert_bytes(plan: &TernaryWeightPlan, d_model: u32, d_ff: u32) -> ByteCost {
    compute_expert_byte_breakdown(plan, d_model, d_ff).total()
}

pub fn compute_expert_bytes_checked(
    plan: &TernaryWeightPlan,
    d_model: u32,
    d_ff: u32,
) -> Result<ByteCost, ExpertBudgetError> {
    Ok(compute_expert_byte_breakdown_checked(plan, d_model, d_ff)?.total())
}

#[must_use]
pub fn compute_expert_byte_breakdown(
    plan: &TernaryWeightPlan,
    d_model: u32,
    d_ff: u32,
) -> ExpertByteBreakdown {
    compute_expert_byte_breakdown_with_metadata(
        plan,
        d_model,
        d_ff,
        ExpertBudgetMetadata::default(),
    )
}

pub fn compute_expert_byte_breakdown_checked(
    plan: &TernaryWeightPlan,
    d_model: u32,
    d_ff: u32,
) -> Result<ExpertByteBreakdown, ExpertBudgetError> {
    validate_nonzero("d_model", d_model)?;
    validate_nonzero("d_ff", d_ff)?;
    Ok(compute_expert_byte_breakdown(plan, d_model, d_ff))
}

#[must_use]
pub fn compute_expert_byte_breakdown_with_metadata(
    plan: &TernaryWeightPlan,
    d_model: u32,
    d_ff: u32,
    metadata: ExpertBudgetMetadata,
) -> ExpertByteBreakdown {
    if d_model == 0 || d_ff == 0 {
        return ExpertByteBreakdown {
            d_model,
            d_ff,
            up_projection_bytes: ByteCost::ZERO,
            down_projection_bytes: ByteCost::ZERO,
            metadata: ExpertBudgetMetadata::zero(),
        };
    }

    let up_projection_bytes = plan.compute_byte_cost(d_ff, d_model);
    let down_projection_bytes = plan.compute_byte_cost(d_model, d_ff);

    ExpertByteBreakdown {
        d_model,
        d_ff,
        up_projection_bytes,
        down_projection_bytes,
        metadata,
    }
}

#[must_use]
pub(crate) fn compute_glu_expert_bytes_for_diagnostic(
    plan: &TernaryWeightPlan,
    d_model: u32,
    d_ff: u32,
) -> ByteCost {
    if d_model == 0 || d_ff == 0 {
        return ByteCost::ZERO;
    }

    let up_projection_bytes = plan.compute_byte_cost(d_ff, d_model);
    let gate_projection_bytes = plan.compute_byte_cost(d_ff, d_model);
    let down_projection_bytes = plan.compute_byte_cost(d_model, d_ff);

    up_projection_bytes
        + gate_projection_bytes
        + down_projection_bytes
        + ExpertBudgetMetadata::default().total()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StaticBudgetReport {
    expert: ExpertByteBreakdown,
    expert_slot_usable_bytes: Option<ByteCost>,
}

impl StaticBudgetReport {
    #[must_use]
    pub fn for_expert(
        plan: &TernaryWeightPlan,
        d_model: u32,
        d_ff: u32,
        expert_slot_usable_bytes: Option<ByteCost>,
    ) -> Self {
        Self {
            expert: compute_expert_byte_breakdown(plan, d_model, d_ff),
            expert_slot_usable_bytes,
        }
    }

    pub fn for_expert_checked(
        plan: &TernaryWeightPlan,
        d_model: u32,
        d_ff: u32,
        expert_slot_usable_bytes: Option<ByteCost>,
    ) -> Result<Self, ExpertBudgetError> {
        Ok(Self {
            expert: compute_expert_byte_breakdown_checked(plan, d_model, d_ff)?,
            expert_slot_usable_bytes,
        })
    }

    #[must_use]
    pub const fn expert_breakdown(self) -> ExpertByteBreakdown {
        self.expert
    }

    #[must_use]
    pub fn expert_bytes(self) -> ByteCost {
        self.expert.total()
    }

    #[must_use]
    pub const fn expert_slot_usable_bytes(self) -> Option<ByteCost> {
        self.expert_slot_usable_bytes
    }

    #[must_use]
    pub fn expert_slot_fit(self) -> Option<ExpertSlotFit> {
        let usable = self.expert_slot_usable_bytes?;
        let expert_bytes = self.expert_bytes();
        Some(if usable >= expert_bytes {
            ExpertSlotFit::Fits {
                slack: usable - expert_bytes,
            }
        } else {
            ExpertSlotFit::Exceeds {
                over_by: expert_bytes - usable,
            }
        })
    }

    #[must_use]
    pub fn fits_expert_slot(self) -> Option<bool> {
        self.expert_slot_fit().map(ExpertSlotFit::fits)
    }
}

fn validate_nonzero(field: &'static str, value: u32) -> Result<(), ExpertBudgetError> {
    if value == 0 {
        return Err(ExpertBudgetError::EmptyDimension { field });
    }

    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpertSlotFit {
    Fits { slack: ByteCost },
    Exceeds { over_by: ByteCost },
}

impl ExpertSlotFit {
    #[must_use]
    pub const fn fits(self) -> bool {
        matches!(self, Self::Fits { .. })
    }
}

#[cfg(test)]
mod tests {
    use gbf_artifact::weight_plan::{
        ScaleFormat, ScaleGranularity, TernaryWeightPlan, ThresholdPlan, WeightEncoding,
    };

    use super::*;

    #[test]
    fn budget_compute_expert_bytes_delegates_to_ternary_weight_plan() {
        let plan = default_plan();

        let breakdown = compute_expert_byte_breakdown(&plan, 256, 512);

        assert_eq!(
            breakdown.up_projection_bytes(),
            plan.compute_byte_cost(512, 256)
        );
        assert_eq!(
            breakdown.down_projection_bytes(),
            plan.compute_byte_cost(256, 512)
        );
        assert_eq!(breakdown.up_projection_bytes(), ByteCost::new(33_792));
        assert_eq!(breakdown.down_projection_bytes(), ByteCost::new(33_280));
        assert_eq!(breakdown.metadata().total(), ByteCost::new(50));
        assert_eq!(breakdown.total(), ByteCost::new(67_122));
        assert_eq!(compute_expert_bytes(&plan, 256, 512), breakdown.total());
    }

    #[test]
    fn budget_known_default_expert_matches_bank_warning_scale() {
        let plan = default_plan();

        let breakdown = compute_expert_byte_breakdown(&plan, 128, 224);

        assert_eq!(breakdown.up_projection_bytes(), ByteCost::new(7_616));
        assert_eq!(breakdown.down_projection_bytes(), ByteCost::new(7_424));
        assert_eq!(breakdown.projection_bytes(), ByteCost::new(15_040));
        assert_eq!(breakdown.metadata().total(), ByteCost::new(50));
        assert_eq!(breakdown.total(), ByteCost::new(15_090));
        assert_eq!(
            compute_glu_expert_bytes_for_diagnostic(&plan, 128, 224),
            ByteCost::new(22_706)
        );
        assert_eq!(
            compute_glu_expert_bytes_for_diagnostic(&plan, 128, 224) - breakdown.total(),
            ByteCost::new(7_616)
        );
    }

    #[test]
    fn budget_static_report_uses_canonical_compute_function() {
        let plan = default_plan();
        let report = StaticBudgetReport::for_expert(&plan, 128, 224, Some(ByteCost::new(16_384)));

        assert_eq!(report.expert_bytes(), compute_expert_bytes(&plan, 128, 224));
        assert_eq!(
            report.expert_breakdown(),
            compute_expert_byte_breakdown(&plan, 128, 224)
        );
        assert_eq!(
            report.expert_slot_fit(),
            Some(ExpertSlotFit::Fits {
                slack: ByteCost::new(1_294),
            })
        );
        assert_eq!(report.fits_expert_slot(), Some(true));

        let over_budget =
            StaticBudgetReport::for_expert(&plan, 128, 224, Some(ByteCost::new(15_000)));
        assert_eq!(
            over_budget.expert_slot_fit(),
            Some(ExpertSlotFit::Exceeds {
                over_by: ByteCost::new(90),
            })
        );
        assert_eq!(over_budget.fits_expert_slot(), Some(false));
    }

    #[test]
    fn budget_zero_dimensions_do_not_charge_metadata() {
        let plan = default_plan();

        assert_eq!(compute_expert_bytes(&plan, 0, 224), ByteCost::ZERO);
        assert_eq!(
            compute_expert_bytes_checked(&plan, 0, 224),
            Err(ExpertBudgetError::EmptyDimension { field: "d_model" })
        );
        assert_eq!(
            compute_expert_byte_breakdown_checked(&plan, 128, 0),
            Err(ExpertBudgetError::EmptyDimension { field: "d_ff" })
        );
        assert_eq!(
            StaticBudgetReport::for_expert_checked(&plan, 0, 224, Some(ByteCost::new(16_384))),
            Err(ExpertBudgetError::EmptyDimension { field: "d_model" })
        );
        assert_eq!(
            compute_glu_expert_bytes_for_diagnostic(&plan, 128, 0),
            ByteCost::ZERO
        );
    }

    #[test]
    fn budget_supports_non_default_weight_plans_without_reimplementing_formula() {
        let plan = TernaryWeightPlan::new(
            WeightEncoding::SparseTernaryBitplanes,
            ScaleGranularity::per_group(16).unwrap(),
            ScaleFormat::Q4_4,
            ThresholdPlan::learned_per_group(16).unwrap(),
        );

        let breakdown = compute_expert_byte_breakdown(&plan, 17, 19);

        assert_eq!(
            breakdown.total(),
            plan.compute_byte_cost(19, 17)
                + plan.compute_byte_cost(17, 19)
                + ExpertBudgetMetadata::default().total()
        );
    }

    fn default_plan() -> TernaryWeightPlan {
        TernaryWeightPlan::new(
            WeightEncoding::Ternary2,
            ScaleGranularity::PerOutputRow,
            ScaleFormat::Q8_8,
            ThresholdPlan::FixedQ8_8,
        )
    }
}
