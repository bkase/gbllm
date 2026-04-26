//! Training-side deployability preflight helpers.

use gbf_artifact::weight_plan::TernaryWeightPlan;
use gbf_foundation::ByteCost;
use gbf_model::budget::{
    ExpertBudgetError, ExpertSlotFit, StaticBudgetReport, compute_expert_bytes_checked,
};

pub fn compute_preflight_expert_bytes(
    plan: &TernaryWeightPlan,
    d_model: u32,
    d_ff: u32,
) -> Result<ByteCost, ExpertBudgetError> {
    compute_expert_bytes_checked(plan, d_model, d_ff)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExpertBudgetPreflightReport {
    static_budget: StaticBudgetReport,
}

impl ExpertBudgetPreflightReport {
    pub fn check_expert_slot(
        plan: &TernaryWeightPlan,
        d_model: u32,
        d_ff: u32,
        expert_slot_usable_bytes: ByteCost,
    ) -> Result<Self, ExpertBudgetError> {
        Ok(Self {
            static_budget: StaticBudgetReport::for_expert_checked(
                plan,
                d_model,
                d_ff,
                Some(expert_slot_usable_bytes),
            )?,
        })
    }

    #[must_use]
    pub const fn static_budget(self) -> StaticBudgetReport {
        self.static_budget
    }

    #[must_use]
    pub fn expert_bytes(self) -> ByteCost {
        self.static_budget.expert_bytes()
    }

    #[must_use]
    pub fn expert_slot_fit(self) -> ExpertSlotFit {
        self.static_budget
            .expert_slot_fit()
            .expect("preflight report always has an expert slot budget")
    }

    #[must_use]
    pub fn fits_expert_slot(self) -> bool {
        self.expert_slot_fit().fits()
    }
}

#[cfg(test)]
mod tests {
    use gbf_artifact::weight_plan::{
        ScaleFormat, ScaleGranularity, TernaryWeightPlan, ThresholdPlan, WeightEncoding,
    };

    use super::*;

    #[test]
    fn preflight_expert_budget_uses_model_compute_expert_bytes() {
        let plan = default_plan();
        let expected = compute_expert_bytes_checked(&plan, 128, 224).unwrap();

        assert_eq!(
            compute_preflight_expert_bytes(&plan, 128, 224),
            Ok(expected)
        );

        let report =
            ExpertBudgetPreflightReport::check_expert_slot(&plan, 128, 224, ByteCost::new(16_384))
                .unwrap();
        assert_eq!(report.expert_bytes(), expected);
        assert_eq!(report.static_budget().expert_bytes(), expected);
        assert_eq!(
            report.expert_slot_fit(),
            ExpertSlotFit::Fits {
                slack: ByteCost::new(1_294),
            }
        );
        assert!(report.fits_expert_slot());
    }

    #[test]
    fn preflight_reports_over_budget_experts_before_training() {
        let plan = default_plan();

        let report =
            ExpertBudgetPreflightReport::check_expert_slot(&plan, 128, 224, ByteCost::new(15_000))
                .unwrap();

        assert_eq!(
            report.expert_slot_fit(),
            ExpertSlotFit::Exceeds {
                over_by: ByteCost::new(90),
            }
        );
        assert!(!report.fits_expert_slot());
    }

    #[test]
    fn preflight_rejects_zero_expert_dimensions() {
        let plan = default_plan();

        assert_eq!(
            compute_preflight_expert_bytes(&plan, 0, 224),
            Err(ExpertBudgetError::EmptyDimension { field: "d_model" })
        );
        assert_eq!(
            ExpertBudgetPreflightReport::check_expert_slot(&plan, 128, 0, ByteCost::new(16_384),),
            Err(ExpertBudgetError::EmptyDimension { field: "d_ff" })
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
