//! `static_budget.v1` report helpers.

use gbf_policy::{
    BudgetFailure, ValidationDiagnostic, budget_failure_diagnostic,
    budget_failure_matches_diagnostic,
};

pub type BudgetFailureRecord = BudgetFailure;
pub type ValidationDiagnosticRecord = ValidationDiagnostic;

#[must_use]
pub fn diagnostics_for_budget_failures(
    failures: &[BudgetFailureRecord],
) -> Vec<ValidationDiagnosticRecord> {
    failures.iter().map(budget_failure_diagnostic).collect()
}

#[must_use]
pub fn failure_diagnostics_are_one_to_one(
    failures: &[BudgetFailureRecord],
    diagnostics: &[ValidationDiagnosticRecord],
) -> bool {
    failures.len() == diagnostics.len()
        && failures
            .iter()
            .zip(diagnostics)
            .all(|(failure, diagnostic)| budget_failure_matches_diagnostic(failure, diagnostic))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gbf_foundation::{BudgetSlotId, ExpertId, FieldPath, LayerId};
    use gbf_policy::{
        PlacementInfeasibilityReason, PlacementProfile, ReductionSiteId, SwitchProjectionSource,
        ValidationCode, ValidationDetail, ValidationOrigin,
    };

    fn all_failure_variants() -> Vec<BudgetFailureRecord> {
        vec![
            BudgetFailureRecord::MissingRuntimeChromeBudget,
            BudgetFailureRecord::QuantGraphBudgetViewMalformed {
                field: FieldPath::from("budget_view.experts[0].rows"),
            },
            BudgetFailureRecord::ExpertExceedsSlot {
                layer: LayerId::new(1),
                expert: ExpertId::new(2),
                slot: BudgetSlotId::new(3),
                payload_bytes: 17_000,
                cap_bytes: 16_128,
                excess_bytes: 872,
            },
            BudgetFailureRecord::CommonBankExceedsCap {
                assigned_bytes: 20_000,
                cap_bytes: 16_384,
                excess_bytes: 3_616,
            },
            BudgetFailureRecord::WramPeakExceedsCap {
                peak: 8_300,
                cap: 8_192,
            },
            BudgetFailureRecord::SramPeakExceedsCap {
                peak: 33_000,
                cap: 32_768,
            },
            BudgetFailureRecord::HramPeakExceedsCap {
                peak: 144,
                cap: 127,
            },
            BudgetFailureRecord::AccumulatorExceedsI32 {
                site: ReductionSiteId("ffn.0.acc".to_owned()),
                projected_max_abs: i32::MAX as u64 + 1,
            },
            BudgetFailureRecord::BankSwitchesPerTokenOverCap {
                decision_value: 9,
                upper_bound: 9,
                cap: 5,
                source: SwitchProjectionSource::ConservativeStaticUpperBound,
            },
            BudgetFailureRecord::SramPageSwitchesPerTokenOverCap {
                decision_value: 4,
                upper_bound: 4,
                cap: 2,
                source: SwitchProjectionSource::ConservativeStaticUpperBound,
            },
            BudgetFailureRecord::PlacementProfileInfeasible {
                profile: PlacementProfile::PackedExperts,
                reason: PlacementInfeasibilityReason::ExpertCountExceedsSlots,
            },
        ]
    }

    #[test]
    fn f_b4_static_budget_v1_failure_diagnostic_one_to_one() {
        let failures = all_failure_variants();
        let diagnostics = diagnostics_for_budget_failures(&failures);

        assert_eq!(diagnostics.len(), failures.len());
        assert!(failure_diagnostics_are_one_to_one(&failures, &diagnostics));

        for (failure, diagnostic) in failures.iter().zip(&diagnostics) {
            assert_eq!(diagnostic.origin, ValidationOrigin::Budget);
            assert_eq!(diagnostic.code, failure.validation_code());
        }

        assert!(matches!(
            &diagnostics[0].code,
            ValidationCode::BudgetMissingRuntimeChromeBudget
        ));
        assert!(matches!(
            &diagnostics[0].detail,
            ValidationDetail::Field { .. }
        ));
        assert!(matches!(
            &diagnostics[2].detail,
            ValidationDetail::Selector { .. }
        ));

        let mut missing = diagnostics.clone();
        missing.pop();
        assert!(!failure_diagnostics_are_one_to_one(&failures, &missing));

        let mut mismatched = diagnostics;
        mismatched.swap(0, 1);
        assert!(!failure_diagnostics_are_one_to_one(&failures, &mismatched));
    }
}
