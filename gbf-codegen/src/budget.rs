//! Static budget taxonomy facade for Stage 2.
//!
//! This bead only wires the failure taxonomy, placement model identity, and
//! diagnostic one-to-one helpers. The Stage 2 producers live in dependent
//! beads.

use gbf_policy::{PlacementProfile, ValidationDiagnostic};
use serde::{Deserialize, Serialize};

pub use gbf_policy::{
    BudgetFailure, PlacementInfeasibilityReason, SwitchProjectionSource, ValidationCode,
    budget_failure_diagnostic, budget_failure_diagnostic_with_provenance,
    budget_failure_diagnostics, budget_failure_diagnostics_with_provenance,
    budget_failure_matches_diagnostic, budget_failure_validation_code,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum StaticPlacementModel {
    StrictOnePerBank,
    BudgetedFirstFit,
    PackedExpertsFirstFitDecreasing,
}

impl StaticPlacementModel {
    #[must_use]
    pub fn for_profile(profile: PlacementProfile) -> Self {
        match profile {
            PlacementProfile::StrictOnePerBank => Self::StrictOnePerBank,
            PlacementProfile::Budgeted => Self::BudgetedFirstFit,
            PlacementProfile::PackedExperts => Self::PackedExpertsFirstFitDecreasing,
        }
    }
}

#[must_use]
pub fn placement_model_for_profile(profile: PlacementProfile) -> StaticPlacementModel {
    StaticPlacementModel::for_profile(profile)
}

#[must_use]
pub fn validation_diagnostic_for_budget_failure(failure: &BudgetFailure) -> ValidationDiagnostic {
    budget_failure_diagnostic(failure)
}

#[must_use]
pub fn validation_diagnostics_for_budget_failures(
    failures: &[BudgetFailure],
) -> Vec<ValidationDiagnostic> {
    budget_failure_diagnostics(failures)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gbf_foundation::{BudgetSlotId, ExpertId, FieldPath, Hash256, LayerId};
    use gbf_policy::{EvidenceRef, ReductionSiteId, ValidationDetail, ValidationOrigin};

    fn round_trip_failure(failure: BudgetFailure) {
        let encoded = serde_json::to_string(&failure).expect("budget failure serializes");
        let decoded: BudgetFailure =
            serde_json::from_str(&encoded).expect("budget failure deserializes");

        assert_eq!(decoded, failure);
    }

    fn all_failure_variants() -> Vec<BudgetFailure> {
        vec![
            BudgetFailure::MissingRuntimeChromeBudget,
            BudgetFailure::QuantGraphBudgetViewMalformed {
                field: FieldPath::from("budget_view.routing"),
            },
            BudgetFailure::ExpertExceedsSlot {
                layer: LayerId::new(0),
                expert: ExpertId::new(1),
                slot: BudgetSlotId::new(2),
                payload_bytes: 17_000,
                cap_bytes: 16_128,
                excess_bytes: 872,
            },
            BudgetFailure::CommonBankExceedsCap {
                assigned_bytes: 20_000,
                cap_bytes: 16_384,
                excess_bytes: 3_616,
            },
            BudgetFailure::WramPeakExceedsCap {
                peak: 8_300,
                cap: 8_192,
            },
            BudgetFailure::SramPeakExceedsCap {
                peak: 33_000,
                cap: 32_768,
            },
            BudgetFailure::HramPeakExceedsCap {
                peak: 144,
                cap: 127,
            },
            BudgetFailure::AccumulatorExceedsI32 {
                site: ReductionSiteId("ffn.0.acc".to_owned()),
                projected_max_abs: i32::MAX as u64 + 1,
            },
            BudgetFailure::BankSwitchesPerTokenOverCap {
                decision_value: 9,
                upper_bound: 9,
                cap: 5,
                source: SwitchProjectionSource::ConservativeStaticUpperBound,
            },
            BudgetFailure::SramPageSwitchesPerTokenOverCap {
                decision_value: 4,
                upper_bound: 4,
                cap: 2,
                source: SwitchProjectionSource::HintWeightedExpectedWithStaticCap,
            },
            BudgetFailure::PlacementProfileInfeasible {
                profile: PlacementProfile::Budgeted,
                reason: PlacementInfeasibilityReason::NoSlotsForClass,
            },
        ]
    }

    #[test]
    fn f_b4_budget_records_static_placement_model() {
        assert_eq!(
            placement_model_for_profile(PlacementProfile::StrictOnePerBank),
            StaticPlacementModel::StrictOnePerBank
        );
        assert_eq!(
            placement_model_for_profile(PlacementProfile::Budgeted),
            StaticPlacementModel::BudgetedFirstFit
        );
        assert_eq!(
            placement_model_for_profile(PlacementProfile::PackedExperts),
            StaticPlacementModel::PackedExpertsFirstFitDecreasing
        );

        assert_eq!(
            serde_json::to_value(StaticPlacementModel::PackedExpertsFirstFitDecreasing)
                .expect("placement model serializes"),
            serde_json::json!({"kind": "PackedExpertsFirstFitDecreasing"})
        );
    }

    #[test]
    fn f_b4_budget_failure_records_concrete_byte_counts() {
        let expert = BudgetFailure::ExpertExceedsSlot {
            layer: LayerId::new(0),
            expert: ExpertId::new(0),
            slot: BudgetSlotId::new(7),
            payload_bytes: 17_000,
            cap_bytes: 16_128,
            excess_bytes: 872,
        };
        let common = BudgetFailure::CommonBankExceedsCap {
            assigned_bytes: 20_000,
            cap_bytes: 16_384,
            excess_bytes: 3_616,
        };

        assert_eq!(
            budget_failure_validation_code(&expert),
            ValidationCode::BudgetExpertExceedsSlot {
                layer: LayerId::new(0),
                expert: ExpertId::new(0),
                slot: BudgetSlotId::new(7),
                payload_bytes: 17_000,
                cap_bytes: 16_128,
            }
        );
        assert_eq!(
            budget_failure_validation_code(&common),
            ValidationCode::BudgetCommonBankExceedsCap {
                assigned_bytes: 20_000,
                cap_bytes: 16_384,
            }
        );

        assert_eq!(
            serde_json::to_value(&expert).expect("expert failure serializes"),
            serde_json::json!({
                "kind": "ExpertExceedsSlot",
                "fields": {
                    "layer": 0,
                    "expert": 0,
                    "slot": 7,
                    "payload_bytes": 17000,
                    "cap_bytes": 16128,
                    "excess_bytes": 872
                }
            })
        );
        assert_eq!(
            serde_json::to_value(&common).expect("common failure serializes"),
            serde_json::json!({
                "kind": "CommonBankExceedsCap",
                "fields": {
                    "assigned_bytes": 20000,
                    "cap_bytes": 16384,
                    "excess_bytes": 3616
                }
            })
        );
    }

    #[test]
    fn f_b4_budget_failure_taxonomy_one_to_one_with_validation_code() {
        let failures = all_failure_variants();
        let diagnostics = validation_diagnostics_for_budget_failures(&failures);

        assert_eq!(diagnostics.len(), failures.len());
        for (failure, diagnostic) in failures.iter().zip(&diagnostics) {
            assert_eq!(diagnostic.origin, ValidationOrigin::Budget);
            assert_eq!(diagnostic.code, failure.validation_code());
            assert!(budget_failure_matches_diagnostic(failure, diagnostic));

            match failure {
                BudgetFailure::MissingRuntimeChromeBudget
                | BudgetFailure::QuantGraphBudgetViewMalformed { .. } => {
                    assert!(matches!(&diagnostic.detail, ValidationDetail::Field { .. }));
                }
                _ => {
                    assert!(matches!(
                        &diagnostic.detail,
                        ValidationDetail::Selector { .. }
                    ));
                }
            }
        }
    }

    #[test]
    fn f_b4_budget_failure_pins_selector_detail_strings() {
        let cases = [
            (
                BudgetFailure::ExpertExceedsSlot {
                    layer: LayerId::new(0),
                    expert: ExpertId::new(1),
                    slot: BudgetSlotId::new(2),
                    payload_bytes: 17_000,
                    cap_bytes: 16_128,
                    excess_bytes: 872,
                },
                "budget.expert[layer=0,expert=1,slot=2]",
            ),
            (
                BudgetFailure::CommonBankExceedsCap {
                    assigned_bytes: 20_000,
                    cap_bytes: 16_384,
                    excess_bytes: 3_616,
                },
                "budget.common_bank",
            ),
            (
                BudgetFailure::AccumulatorExceedsI32 {
                    site: ReductionSiteId("ffn.0.acc".to_owned()),
                    projected_max_abs: i32::MAX as u64 + 1,
                },
                "budget.accumulator[site=ffn.0.acc]",
            ),
            (
                BudgetFailure::BankSwitchesPerTokenOverCap {
                    decision_value: 9,
                    upper_bound: 9,
                    cap: 5,
                    source: SwitchProjectionSource::ConservativeStaticUpperBound,
                },
                "budget.switches.bank_per_token",
            ),
            (
                BudgetFailure::PlacementProfileInfeasible {
                    profile: PlacementProfile::PackedExperts,
                    reason: PlacementInfeasibilityReason::ExpertCountExceedsSlots,
                },
                "budget.placement[profile=packed_experts,reason=expert_count_exceeds_slots]",
            ),
        ];

        for (failure, selector) in cases {
            assert_eq!(
                serde_json::to_value(failure.diagnostic_detail())
                    .expect("selector detail serializes"),
                serde_json::json!({
                    "kind": "Selector",
                    "selector": selector
                })
            );
        }
    }

    #[test]
    fn f_b4_budget_failure_diagnostic_accepts_provenance() {
        let failure = BudgetFailure::CommonBankExceedsCap {
            assigned_bytes: 20_000,
            cap_bytes: 16_384,
            excess_bytes: 3_616,
        };
        let provenance = vec![EvidenceRef {
            kind: "Fixture".to_owned(),
            reference: "static-budget-input".to_owned(),
            hash: Some(Hash256::from_bytes([7; 32])),
        }];

        let diagnostic = budget_failure_diagnostic_with_provenance(&failure, provenance.clone());

        assert_eq!(diagnostic.provenance, provenance);
        assert!(budget_failure_matches_diagnostic(&failure, &diagnostic));
    }

    #[test]
    fn f_b4_budget_failure_missing_runtime_chrome_budget_round_trip() {
        round_trip_failure(BudgetFailure::MissingRuntimeChromeBudget);

        assert_eq!(
            serde_json::to_value(BudgetFailure::MissingRuntimeChromeBudget)
                .expect("missing budget failure serializes"),
            serde_json::json!({"kind": "MissingRuntimeChromeBudget"})
        );
    }

    #[test]
    fn f_b4_budget_failure_quant_graph_view_malformed_round_trip() {
        let failure = BudgetFailure::QuantGraphBudgetViewMalformed {
            field: FieldPath::from("budget_view.per_expert_payload"),
        };

        round_trip_failure(failure.clone());
        assert_eq!(
            budget_failure_validation_code(&failure),
            ValidationCode::BudgetQuantGraphViewMalformed {
                field: FieldPath::from("budget_view.per_expert_payload"),
            }
        );
        assert_eq!(
            serde_json::to_value(failure).expect("budget view malformed failure serializes"),
            serde_json::json!({
                "kind": "QuantGraphBudgetViewMalformed",
                "fields": {
                    "field": "budget_view.per_expert_payload"
                }
            })
        );
    }
}
