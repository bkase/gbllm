//! `static_budget.v1` report helpers.

pub use gbf_policy::StaticFitInterpretation;
use gbf_policy::{
    BudgetFailure, EvidenceRef, ValidationDiagnostic, budget_failure_diagnostic,
    budget_failure_diagnostic_with_provenance, budget_failure_matches_diagnostic,
};

pub type BudgetFailureRecord = BudgetFailure;
pub type ValidationDiagnosticRecord = ValidationDiagnostic;

/// Return the binary Stage-2 interpretation for a `decision.fits` value.
///
/// `fits = true` only means the report passed necessary static checks. It is
/// not a final deployability claim; F-B10, F-B12, F-B13, and final layout
/// remain authoritative.
#[must_use]
pub const fn static_fit_interpretation_for_fits(fits: bool) -> StaticFitInterpretation {
    if fits {
        StaticFitInterpretation::PassesNecessaryStaticChecks
    } else {
        StaticFitInterpretation::FailsNecessaryStaticChecks
    }
}

#[must_use]
pub const fn decision_interpretation_matches_fits(
    fits: bool,
    interpretation: StaticFitInterpretation,
) -> bool {
    matches!(
        (fits, interpretation),
        (true, StaticFitInterpretation::PassesNecessaryStaticChecks)
            | (false, StaticFitInterpretation::FailsNecessaryStaticChecks)
    )
}

#[must_use]
pub fn diagnostics_for_budget_failures(
    failures: &[BudgetFailureRecord],
) -> Vec<ValidationDiagnosticRecord> {
    failures.iter().map(budget_failure_diagnostic).collect()
}

#[must_use]
pub fn diagnostics_for_budget_failures_with_provenance(
    failures: &[BudgetFailureRecord],
    provenance: Vec<EvidenceRef>,
) -> Vec<ValidationDiagnosticRecord> {
    failures
        .iter()
        .map(|failure| budget_failure_diagnostic_with_provenance(failure, provenance.clone()))
        .collect()
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
    use gbf_foundation::{BudgetSlotId, ExpertId, FieldPath, Hash256, LayerId};
    use gbf_policy::{
        EvidenceRef, PlacementInfeasibilityReason, PlacementProfile, ReductionSiteId,
        SwitchProjectionSource, ValidationCode, ValidationDetail, ValidationOrigin,
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

        let mut mismatched = diagnostics.clone();
        mismatched.swap(0, 1);
        assert!(!failure_diagnostics_are_one_to_one(&failures, &mismatched));

        let mut selector_mismatched = diagnostics;
        selector_mismatched.swap(2, 3);
        assert!(!failure_diagnostics_are_one_to_one(
            &failures,
            &selector_mismatched
        ));
    }

    #[test]
    fn f_b4_static_budget_v1_provenance_helper_preserves_one_to_one_mapping() {
        let failures = all_failure_variants();
        let provenance = vec![EvidenceRef {
            kind: "Fixture".to_owned(),
            reference: "static-budget-report".to_owned(),
            hash: Some(Hash256::from_bytes([4; 32])),
        }];
        let diagnostics =
            diagnostics_for_budget_failures_with_provenance(&failures, provenance.clone());

        assert!(failure_diagnostics_are_one_to_one(&failures, &diagnostics));
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.provenance == provenance)
        );
    }

    #[test]
    fn f_b4_static_budget_v1_decision_interpretation_invariant() {
        assert_eq!(
            static_fit_interpretation_for_fits(true),
            StaticFitInterpretation::PassesNecessaryStaticChecks
        );
        assert_eq!(
            static_fit_interpretation_for_fits(false),
            StaticFitInterpretation::FailsNecessaryStaticChecks
        );

        assert!(decision_interpretation_matches_fits(
            true,
            StaticFitInterpretation::PassesNecessaryStaticChecks
        ));
        assert!(decision_interpretation_matches_fits(
            false,
            StaticFitInterpretation::FailsNecessaryStaticChecks
        ));
        assert!(!decision_interpretation_matches_fits(
            true,
            StaticFitInterpretation::FailsNecessaryStaticChecks
        ));
        assert!(!decision_interpretation_matches_fits(
            false,
            StaticFitInterpretation::PassesNecessaryStaticChecks
        ));

        assert_eq!(
            serde_json::to_value(StaticFitInterpretation::PassesNecessaryStaticChecks)
                .expect("interpretation serializes"),
            serde_json::json!({"kind": "PassesNecessaryStaticChecks"})
        );
    }
}
