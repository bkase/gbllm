//! Downstream overlay eligibility lens over the emitted Stage 6 plan.

use crate::storage_plan::driver::StoragePlanCoreResult;
use crate::storage_plan::predicates::ValueRole;
use crate::storage_plan::types::{Materialization, StorageBinding, StorageClass};

/// Public Stage 6 storage plan product consumed by downstream stages.
pub type StoragePlan = StoragePlanCoreResult;

#[must_use]
pub fn is_overlay_eligible(sp: &StoragePlan, b: &StorageBinding) -> bool {
    matches!(
        b.materialization,
        Materialization::Materialize {
            class: StorageClass::RomConst,
            ..
        }
    ) && sp
        .provenance
        .bindings
        .get(&b.value)
        .and_then(|provenance| provenance.op_output_role.as_ref())
        .is_some_and(|role| {
            matches!(
                role,
                ValueRole::LutFragment | ValueRole::ExpertWeight | ValueRole::RouterWeight
            )
        })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::s3::infer_ir::{NodeId, ValueId};
    use crate::storage_plan::types::{
        AbstractLiveRange, AdmittingPredicateId, AliasClassId, BindingJustification,
        BindingProvenance, DecisionRuleId, LifetimeClass, StorageProvenance,
    };

    #[test]
    fn expert_weight_rom_const_binding_is_overlay_eligible() {
        let binding = binding(
            1,
            Materialization::Materialize {
                class: StorageClass::RomConst,
                lifetime: LifetimeClass::Persistent,
            },
        );
        let plan = plan_with_role(&binding, Some(ValueRole::ExpertWeight));

        assert!(is_overlay_eligible(&plan, &binding));
    }

    #[test]
    fn router_score_rom_const_binding_is_not_overlay_eligible() {
        let binding = binding(
            2,
            Materialization::Materialize {
                class: StorageClass::RomConst,
                lifetime: LifetimeClass::Slice,
            },
        );
        let plan = plan_with_role(&binding, Some(ValueRole::RouterScore));

        assert!(!is_overlay_eligible(&plan, &binding));
    }

    #[test]
    fn non_rom_const_materialization_is_not_overlay_eligible_even_for_lut_fragment() {
        let binding = binding(
            3,
            Materialization::Materialize {
                class: StorageClass::WramHot,
                lifetime: LifetimeClass::Slice,
            },
        );
        let plan = plan_with_role(&binding, Some(ValueRole::LutFragment));

        assert!(!is_overlay_eligible(&plan, &binding));
    }

    #[test]
    fn missing_provenance_role_is_not_overlay_eligible() {
        let binding = binding(
            4,
            Materialization::Materialize {
                class: StorageClass::RomConst,
                lifetime: LifetimeClass::Persistent,
            },
        );
        let plan = plan_with_role(&binding, None);

        assert!(!is_overlay_eligible(&plan, &binding));
    }

    fn plan_with_role(binding: &StorageBinding, role: Option<ValueRole>) -> StoragePlan {
        StoragePlan {
            bindings: BTreeMap::from([(binding.value, binding.clone())]),
            alias_classes: BTreeMap::new(),
            persist_pages: BTreeMap::new(),
            commit_groups: BTreeMap::new(),
            provenance: StorageProvenance {
                bindings: BTreeMap::from([(
                    binding.value,
                    BindingProvenance::new(
                        AdmittingPredicateId(1),
                        DecisionRuleId(1),
                        false,
                        Vec::new(),
                        role,
                        None,
                    ),
                )]),
                alias_classes: BTreeMap::new(),
                persist_pages: BTreeMap::new(),
                commit_groups: BTreeMap::new(),
            },
        }
    }

    fn binding(id: u32, materialization: Materialization) -> StorageBinding {
        StorageBinding {
            value: ValueId::new(id),
            materialization,
            alias_class: AliasClassId(0),
            live_range: AbstractLiveRange {
                def_node: NodeId::new(id),
                first_use_node: Some(NodeId::new(id)),
                last_use_node: Some(NodeId::new(id)),
                lifetime_class: LifetimeClass::Slice,
                checkpoint_stable: false,
            },
            justification: BindingJustification::DecisionRule(DecisionRuleId(1)),
        }
    }

    #[test]
    fn static_scan_rejects_policy_only_overlay_filters() {
        let source = include_str!("overlay_lens.rs");
        let forbidden = [
            ["overlay_", "excluded_", "set"].concat(),
            ["Overlay", "Region", "Size", "Ceiling"].concat(),
        ];

        for key in forbidden {
            assert!(
                !source.contains(&key),
                "overlay lens references unavailable policy path {key:?}"
            );
        }
    }
}
