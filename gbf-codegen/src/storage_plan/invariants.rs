//! Self-consistency checks for emitted Stage 6 storage plans.

use std::collections::{BTreeMap, BTreeSet};

use gbf_foundation::{EvidenceRef, Hash256};
use gbf_policy::{
    StoragePlanDiagnosticCode, StoragePlanDiagnosticProvenance, ValidationDiagnostic,
};
use gbf_report::{ReportBody, ReportEnvelope, ReportSelfHashError, compute_self_hash};
use serde::Serialize;
use serde_json::Value;

use crate::s3::infer_ir::ValueId;
use crate::storage_plan::diagnostics::storage_plan_diagnostic;
use crate::storage_plan::lifetime::{LifetimeBounds, validate_lifetime_bounds};
use crate::storage_plan::types::{
    AliasClass, AliasClassId, AliasIntent, CommitGroupDecl, CommitGroupId, Materialization,
    PersistPageDecl, PersistPageId, STORAGE_PLAN_INPUT_HASH_MISMATCH_SPECS, StorageBinding,
    StoragePlanInputHashes, StoragePlanInputIdentity,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoragePlanConsistencyContext {
    pub expected_values: BTreeSet<ValueId>,
    pub lifetime_bounds: BTreeMap<ValueId, LifetimeBounds>,
    pub expected_input_hashes: StoragePlanInputHashes,
}

#[derive(Debug, Clone, Copy)]
pub struct StoragePlanConsistencyView<'a> {
    pub input_identity: &'a StoragePlanInputIdentity,
    pub bindings: &'a [StorageBinding],
    pub alias_classes: &'a BTreeMap<AliasClassId, AliasClass>,
    pub persist_pages: &'a BTreeMap<PersistPageId, PersistPageDecl>,
    pub commit_groups: &'a BTreeMap<CommitGroupId, CommitGroupDecl>,
    pub json_value: Option<&'a Value>,
}

pub fn validate_storage_plan_self_consistency(
    context: &StoragePlanConsistencyContext,
    view: StoragePlanConsistencyView<'_>,
) -> Vec<ValidationDiagnostic> {
    let mut diagnostics = Vec::new();
    check_binding_coverage_and_functionality(context, view.bindings, &mut diagnostics);

    let binding_by_value = first_bindings_by_value(view.bindings);
    check_alias_classes(view.alias_classes, &binding_by_value, &mut diagnostics);
    check_alias_membership(view.bindings, view.alias_classes, &mut diagnostics);
    check_materialization_alias_rules(view.bindings, view.alias_classes, &mut diagnostics);
    check_persist_pages(view.persist_pages, &binding_by_value, &mut diagnostics);
    check_commit_groups(view.commit_groups, &binding_by_value, &mut diagnostics);
    check_lifetime_bounds(context, view.bindings, &mut diagnostics);
    check_input_identity(context, view.input_identity, &mut diagnostics);

    if let Some(value) = view.json_value {
        check_closed_spatial_surface(value, &mut diagnostics);
    }

    diagnostics
}

pub fn validate_storage_plan_report_self_hash<R>(
    envelope: &ReportEnvelope<R>,
) -> Result<(), ReportSelfHashError>
where
    R: ReportBody + Serialize,
{
    let expected = compute_self_hash(envelope)?;
    if envelope.report_self_hash == expected {
        Ok(())
    } else {
        Err(ReportSelfHashError::HashMismatch {
            expected,
            observed: envelope.report_self_hash,
        })
    }
}

pub fn serialized_body_contains_report_self_hash<T>(body: &T) -> Result<bool, serde_json::Error>
where
    T: Serialize,
{
    let value = serde_json::to_value(body)?;
    Ok(value_contains_report_self_hash(&value))
}

pub fn closed_spatial_surface_diagnostics(value: &Value) -> Vec<ValidationDiagnostic> {
    let mut diagnostics = Vec::new();
    check_closed_spatial_surface(value, &mut diagnostics);
    diagnostics
}

fn check_binding_coverage_and_functionality(
    context: &StoragePlanConsistencyContext,
    bindings: &[StorageBinding],
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    let mut counts = BTreeMap::<ValueId, u32>::new();
    for binding in bindings {
        *counts.entry(binding.value).or_default() += 1;
    }

    for value in &context.expected_values {
        match counts.get(value).copied().unwrap_or_default() {
            0 => diagnostics.push(diagnostic(
                StoragePlanDiagnosticCode::StorageBindingCoverageGap,
                StoragePlanDiagnosticProvenance::ValueProducer {
                    value_id: value.get(),
                    producer_node: 0,
                },
            )),
            1 => {}
            binding_count => diagnostics.push(diagnostic(
                StoragePlanDiagnosticCode::StorageBindingDoubleBind,
                StoragePlanDiagnosticProvenance::BindingSet {
                    value_id: value.get(),
                    binding_count,
                },
            )),
        }
    }

    for (value, binding_count) in counts {
        if !context.expected_values.contains(&value) && binding_count > 1 {
            diagnostics.push(diagnostic(
                StoragePlanDiagnosticCode::StorageBindingDoubleBind,
                StoragePlanDiagnosticProvenance::BindingSet {
                    value_id: value.get(),
                    binding_count,
                },
            ));
        }
    }
}

fn check_alias_classes(
    alias_classes: &BTreeMap<AliasClassId, AliasClass>,
    binding_by_value: &BTreeMap<ValueId, &StorageBinding>,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    for (id, class) in alias_classes {
        if class.id() != id {
            diagnostics.push(alias_membership_diagnostic(
                "SC3",
                class
                    .members()
                    .iter()
                    .next()
                    .copied()
                    .unwrap_or(ValueId::new(0)),
                *id,
            ));
        }

        for member in class.members() {
            match binding_by_value.get(member) {
                Some(binding) if binding.alias_class == *id => {}
                _ => diagnostics.push(alias_membership_diagnostic("SC4", *member, *id)),
            }
        }
    }
}

fn check_alias_membership(
    bindings: &[StorageBinding],
    alias_classes: &BTreeMap<AliasClassId, AliasClass>,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    for binding in bindings {
        if !alias_classes.contains_key(&binding.alias_class) {
            diagnostics.push(alias_membership_diagnostic(
                "SC4",
                binding.value,
                binding.alias_class,
            ));
        }
    }
}

fn check_materialization_alias_rules(
    bindings: &[StorageBinding],
    alias_classes: &BTreeMap<AliasClassId, AliasClass>,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    for binding in bindings {
        let Some(class) = alias_classes.get(&binding.alias_class) else {
            continue;
        };
        match binding.materialization {
            Materialization::Recompute => {
                if !class.members().is_singleton() || class.intent() != AliasIntent::NoAlias {
                    diagnostics.push(diagnostic(
                        StoragePlanDiagnosticCode::StorageRecomputeAliasNotIsolated,
                        StoragePlanDiagnosticProvenance::RecomputeAlias {
                            value_id: binding.value.get(),
                            alias_class_id: binding.alias_class.0,
                        },
                    ));
                }
            }
            Materialization::Persist { .. } => {
                if !class.members().is_singleton() && class.intent() != AliasIntent::PersistRotation
                {
                    diagnostics.push(diagnostic(
                        StoragePlanDiagnosticCode::StorageAliasIntentMaterializationMismatch,
                        StoragePlanDiagnosticProvenance::AliasMaterialization {
                            alias_class_id: binding.alias_class.0,
                            members: class.members().iter().map(|value| value.get()).collect(),
                            intent: format!("{:?}", class.intent()),
                            materializations: vec!["Persist".to_owned()],
                        },
                    ));
                }
            }
            Materialization::Materialize { .. } => {}
        }
    }
}

fn check_persist_pages(
    persist_pages: &BTreeMap<PersistPageId, PersistPageDecl>,
    binding_by_value: &BTreeMap<ValueId, &StorageBinding>,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    let referenced_pages = referenced_pages(binding_by_value.values().copied());
    for (id, page) in persist_pages {
        if page.id != *id || !referenced_pages.contains(id) {
            diagnostics.push(diagnostic(
                StoragePlanDiagnosticCode::StoragePersistPageNotReferenced,
                StoragePlanDiagnosticProvenance::PersistPage {
                    invariant: "SC7".to_owned(),
                    persist_page_id: id.0,
                },
            ));
        }
    }
}

fn check_commit_groups(
    commit_groups: &BTreeMap<CommitGroupId, CommitGroupDecl>,
    binding_by_value: &BTreeMap<ValueId, &StorageBinding>,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    let members_by_group = referenced_pages_by_group(binding_by_value.values().copied());
    for (id, group) in commit_groups {
        let referenced = members_by_group.get(id).cloned().unwrap_or_default();
        if group.id != *id || referenced.is_empty() || group.members.as_btree_set() != &referenced {
            let invariant = if referenced.is_empty() { "SC8" } else { "SC9" };
            diagnostics.push(diagnostic(
                StoragePlanDiagnosticCode::StorageCommitGroupEmpty,
                StoragePlanDiagnosticProvenance::CommitGroup {
                    invariant: invariant.to_owned(),
                    commit_group_id: id.0,
                },
            ));
        }
    }
}

fn check_lifetime_bounds(
    context: &StoragePlanConsistencyContext,
    bindings: &[StorageBinding],
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    for binding in bindings {
        let Some(bounds) = context.lifetime_bounds.get(&binding.value) else {
            continue;
        };
        if let Err(diagnostic) =
            validate_lifetime_bounds(binding.value, &binding.materialization, bounds)
        {
            diagnostics.push(*diagnostic);
        }
    }
}

fn check_input_identity(
    context: &StoragePlanConsistencyContext,
    identity: &StoragePlanInputIdentity,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    for spec in STORAGE_PLAN_INPUT_HASH_MISMATCH_SPECS {
        push_hash_mismatch(
            diagnostics,
            spec.storage_code,
            spec.identity_field,
            identity.hash_for_product(spec.product),
            context.expected_input_hashes.hash_for_product(spec.product),
        );
    }
}

fn push_hash_mismatch(
    diagnostics: &mut Vec<ValidationDiagnostic>,
    code: StoragePlanDiagnosticCode,
    product: &str,
    recorded: Hash256,
    computed: Hash256,
) {
    if recorded != computed {
        diagnostics.push(diagnostic(
            code,
            StoragePlanDiagnosticProvenance::HashMismatch {
                product: product.to_owned(),
                recorded,
                computed,
            },
        ));
    }
}

fn check_closed_spatial_surface(value: &Value, diagnostics: &mut Vec<ValidationDiagnostic>) {
    let forbidden = forbidden_stage6_spatial_surface();
    scan_json_value(value, "$", &forbidden, diagnostics);
}

fn scan_json_value(
    value: &Value,
    path: &str,
    forbidden: &BTreeSet<String>,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    match value {
        Value::Object(map) => {
            for (key, nested) in map {
                let nested_path = format!("{path}.{key}");
                if forbidden.contains(key) {
                    diagnostics.push(json_path_diagnostic(&nested_path, key));
                }
                scan_json_value(nested, &nested_path, forbidden, diagnostics);
            }
        }
        Value::Array(values) => {
            for (index, nested) in values.iter().enumerate() {
                scan_json_value(nested, &format!("{path}[{index}]"), forbidden, diagnostics);
            }
        }
        Value::String(tag) if forbidden.contains(tag) => {
            diagnostics.push(json_path_diagnostic(path, tag));
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

fn value_contains_report_self_hash(value: &Value) -> bool {
    match value {
        Value::Object(map) => map.iter().any(|(key, nested)| {
            key == "report_self_hash" || value_contains_report_self_hash(nested)
        }),
        Value::Array(values) => values.iter().any(value_contains_report_self_hash),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => false,
    }
}

fn json_path_diagnostic(path: &str, field_or_tag: &str) -> ValidationDiagnostic {
    diagnostic(
        StoragePlanDiagnosticCode::StorageForbiddenSpatialEnumLeak,
        StoragePlanDiagnosticProvenance::JsonPath {
            json_path: path.to_owned(),
            field_or_tag: field_or_tag.to_owned(),
        },
    )
}

fn forbidden_stage6_spatial_surface() -> BTreeSet<String> {
    // SC11 is an exact-key/tag scan over the canonical Stage 6 JSON spelling.
    // Case variants are not aliases unless the public schema adopts them.
    [
        closed_key(&["byte_", "offset"]),
        closed_key(&["byte_", "alignment"]),
        closed_key(&["byte_", "address"]),
        closed_key(&["concrete_", "bank"]),
        closed_key(&["rom_", "bank"]),
        closed_key(&["sram_", "bank"]),
        closed_key(&["slice_", "id"]),
        closed_key(&["lease_", "id"]),
        closed_key(&["overlay_", "region"]),
        closed_key(&["overlay_", "install"]),
        closed_key(&["page_", "byte_", "address"]),
        closed_key(&["kernel_", "resid", "ency"]),
        closed_key(&["sram_", "page_", "family_", "id"]),
        closed_key(&["sram_", "working_", "set_", "id"]),
        closed_key(&["Resource", "Vector"]),
        closed_key(&["Sched", "Slice"]),
        closed_key(&["Resid", "ency", "Epoch"]),
        closed_key(&["Overlay", "Id"]),
        closed_key(&["Overlay", "Install"]),
        closed_key(&["Kernel", "Resid", "ency"]),
        closed_key(&["Bank", "Class"]),
        closed_key(&["Rom", "Bank"]),
        closed_key(&["Sram", "Bank"]),
        closed_key(&["Resid", "ency"]),
    ]
    .into_iter()
    .collect()
}

fn closed_key(parts: &[&str]) -> String {
    parts.concat()
}

fn first_bindings_by_value(bindings: &[StorageBinding]) -> BTreeMap<ValueId, &StorageBinding> {
    let mut by_value = BTreeMap::new();
    for binding in bindings {
        by_value.entry(binding.value).or_insert(binding);
    }
    by_value
}

fn referenced_pages<'a>(
    bindings: impl IntoIterator<Item = &'a StorageBinding>,
) -> BTreeSet<PersistPageId> {
    bindings
        .into_iter()
        .filter_map(|binding| match binding.materialization {
            Materialization::Persist { page, .. } => Some(page),
            Materialization::Recompute | Materialization::Materialize { .. } => None,
        })
        .collect()
}

fn referenced_pages_by_group<'a>(
    bindings: impl IntoIterator<Item = &'a StorageBinding>,
) -> BTreeMap<CommitGroupId, BTreeSet<PersistPageId>> {
    let mut by_group = BTreeMap::<CommitGroupId, BTreeSet<PersistPageId>>::new();
    for binding in bindings {
        if let Materialization::Persist { page, commit_group } = binding.materialization {
            by_group.entry(commit_group).or_default().insert(page);
        }
    }
    by_group
}

fn alias_membership_diagnostic(
    invariant: &str,
    value: ValueId,
    alias_class: AliasClassId,
) -> ValidationDiagnostic {
    diagnostic(
        StoragePlanDiagnosticCode::StorageAliasClassMembershipFunctionalViolation,
        StoragePlanDiagnosticProvenance::AliasMembership {
            invariant: invariant.to_owned(),
            value_id: value.get(),
            alias_class_id: alias_class.0,
        },
    )
}

fn diagnostic(
    code: StoragePlanDiagnosticCode,
    provenance: StoragePlanDiagnosticProvenance,
) -> ValidationDiagnostic {
    storage_plan_diagnostic(code, provenance, Vec::<EvidenceRef>::new())
        .expect("self-consistency diagnostic provenance is fixed")
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use gbf_foundation::{EvidenceRef, Hash256};
    use gbf_policy::{StoragePlanDiagnosticProvenance, ValidationCode, ValidationDiagnostic};
    use gbf_report::{ReportEnvelope, ReportOutcome};
    use serde_json::json;

    use super::*;
    use crate::s1::quant_graph::DeterminismClass;
    use crate::s3::infer_ir::{NodeId, ValueId};
    use crate::storage_plan::emitter::{StoragePlanReportBody, StoragePlanReportEnvelopeBody};
    use crate::storage_plan::lifetime::{LifetimeBound, LifetimeBoundSource};
    use crate::storage_plan::types::{
        AbstractLiveRange, AdmittingPredicateId, AliasClass, AliasClassError, AliasClassProvenance,
        BindingJustification, BindingProvenance, CommitAtomicityClass, CommitGroupId,
        CommitGroupProvenance, CommitGroupReason, DecisionRuleId, DurabilityClass, LifetimeClass,
        NonEmptySortedSet, PersistKind, PersistPageId, PersistSchemaPin, STORAGE_PLAN_SCHEMA_ID,
        STORAGE_PLAN_SCHEMA_VERSION, StorageClass,
    };

    #[test]
    fn sc1_binding_coverage_gap_fires_store_002() {
        let result = fixture_result();
        let mut context = fixture_context();
        context.expected_values.insert(ValueId::new(9));
        let bindings = binding_vec(&result);

        let diagnostics =
            validate_storage_plan_self_consistency(&context, view(&result, &bindings));

        assert_has_store(
            &diagnostics,
            StoragePlanDiagnosticCode::StorageBindingCoverageGap,
        );
    }

    #[test]
    fn sc2_double_bind_fires_store_003() {
        let result = fixture_result();
        let mut bindings: Vec<_> = result.bindings.values().cloned().collect();
        bindings.push(result.bindings[&ValueId::new(1)].clone());

        let diagnostics = validate_storage_plan_self_consistency(
            &fixture_context(),
            StoragePlanConsistencyView {
                bindings: &bindings,
                ..view(&result, &bindings)
            },
        );

        assert_has_store(
            &diagnostics,
            StoragePlanDiagnosticCode::StorageBindingDoubleBind,
        );
    }

    #[test]
    fn sc3_alias_class_key_mismatch_fires_store_015() {
        let mut result = fixture_result();
        let class = result
            .alias_classes
            .remove(&AliasClassId(0))
            .expect("class");
        result.alias_classes.insert(AliasClassId(7), class);
        let bindings = binding_vec(&result);

        let diagnostics =
            validate_storage_plan_self_consistency(&fixture_context(), view(&result, &bindings));

        assert_has_store(
            &diagnostics,
            StoragePlanDiagnosticCode::StorageAliasClassMembershipFunctionalViolation,
        );
        assert_has_provenance_invariant(&diagnostics, "SC3");
    }

    #[test]
    fn sc4_binding_alias_reference_miss_fires_store_015() {
        let mut result = fixture_result();
        result
            .bindings
            .get_mut(&ValueId::new(1))
            .expect("binding")
            .alias_class = AliasClassId(99);
        let bindings = binding_vec(&result);

        let diagnostics =
            validate_storage_plan_self_consistency(&fixture_context(), view(&result, &bindings));

        assert_has_store(
            &diagnostics,
            StoragePlanDiagnosticCode::StorageAliasClassMembershipFunctionalViolation,
        );
    }

    #[test]
    fn sc5_recompute_alias_isolation_fires_store_016() {
        let mut result = fixture_result();
        result
            .bindings
            .get_mut(&ValueId::new(1))
            .expect("binding")
            .materialization = Materialization::Recompute;
        let bindings = binding_vec(&result);

        let diagnostics =
            validate_storage_plan_self_consistency(&fixture_context(), view(&result, &bindings));

        assert_has_store(
            &diagnostics,
            StoragePlanDiagnosticCode::StorageRecomputeAliasNotIsolated,
        );
    }

    #[test]
    fn sc5_singleton_recompute_requires_noalias_intent() {
        let accepted =
            AliasClass::from_members(AliasClassId(9), [ValueId::new(9)], AliasIntent::NoAlias)
                .expect("singleton NoAlias class is valid");
        assert_eq!(accepted.intent(), AliasIntent::NoAlias);

        let rejected = AliasClass::from_members(
            AliasClassId(10),
            [ValueId::new(10)],
            AliasIntent::ScratchReuse,
        )
        .expect_err("singleton non-NoAlias must not bypass SC5 isolation");
        assert!(matches!(
            rejected,
            AliasClassError::SingletonMustBeNoAlias {
                intent: AliasIntent::ScratchReuse
            }
        ));
    }

    #[test]
    fn sc6_persist_alias_rotation_only_fires_store_013() {
        let mut result = fixture_result();
        for value in [1, 2] {
            result
                .bindings
                .get_mut(&ValueId::new(value))
                .expect("binding")
                .materialization = Materialization::Persist {
                page: PersistPageId(value),
                commit_group: CommitGroupId(value),
            };
        }
        let bindings = binding_vec(&result);

        let diagnostics =
            validate_storage_plan_self_consistency(&fixture_context(), view(&result, &bindings));

        assert_has_store(
            &diagnostics,
            StoragePlanDiagnosticCode::StorageAliasIntentMaterializationMismatch,
        );
    }

    #[test]
    fn sc7_unreferenced_persist_page_fires_store_009() {
        let mut result = fixture_result();
        result.persist_pages.insert(
            PersistPageId(8),
            PersistPageDecl {
                id: PersistPageId(8),
                kind: PersistKind::Trace,
                durability: DurabilityClass::BestEffort,
                schema_pin: schema_pin(),
            },
        );
        let bindings = binding_vec(&result);

        let diagnostics =
            validate_storage_plan_self_consistency(&fixture_context(), view(&result, &bindings));

        assert_has_store(
            &diagnostics,
            StoragePlanDiagnosticCode::StoragePersistPageNotReferenced,
        );
        assert_has_provenance_invariant(&diagnostics, "SC7");
    }

    #[test]
    fn sc8_unreferenced_commit_group_fires_store_010() {
        let mut result = fixture_result();
        result.commit_groups.insert(
            CommitGroupId(8),
            CommitGroupDecl {
                id: CommitGroupId(8),
                members: NonEmptySortedSet::new([PersistPageId(8)]).expect("member"),
                kind_set: BTreeSet::from([PersistKind::Trace]),
                atomicity: CommitAtomicityClass::AllOrNothing,
            },
        );
        let bindings = binding_vec(&result);

        let diagnostics =
            validate_storage_plan_self_consistency(&fixture_context(), view(&result, &bindings));

        assert_has_store(
            &diagnostics,
            StoragePlanDiagnosticCode::StorageCommitGroupEmpty,
        );
        assert_has_provenance_invariant(&diagnostics, "SC8");
    }

    #[test]
    fn sc9_commit_group_member_mismatch_fires_store_010() {
        let mut result = fixture_result();
        result
            .commit_groups
            .get_mut(&CommitGroupId(1))
            .expect("group")
            .members = NonEmptySortedSet::new([PersistPageId(99)]).expect("member");
        let bindings = binding_vec(&result);

        let diagnostics =
            validate_storage_plan_self_consistency(&fixture_context(), view(&result, &bindings));

        assert_has_store(
            &diagnostics,
            StoragePlanDiagnosticCode::StorageCommitGroupEmpty,
        );
        assert_has_provenance_invariant(&diagnostics, "SC9");
    }

    #[test]
    fn sc10_lifetime_lower_and_upper_bound_failures_fire_store_017() {
        let mut result = fixture_result();
        result
            .bindings
            .get_mut(&ValueId::new(1))
            .expect("binding")
            .materialization = Materialization::Materialize {
            class: StorageClass::WramHot,
            lifetime: LifetimeClass::Slice,
        };
        let mut context = fixture_context();
        context.lifetime_bounds.insert(
            ValueId::new(1),
            bounds(LifetimeClass::Token, LifetimeClass::Persistent),
        );
        let lower_bindings = binding_vec(&result);
        let lower =
            validate_storage_plan_self_consistency(&context, view(&result, &lower_bindings));

        result
            .bindings
            .get_mut(&ValueId::new(1))
            .expect("binding")
            .materialization = Materialization::Materialize {
            class: StorageClass::WramHot,
            lifetime: LifetimeClass::Persistent,
        };
        context.lifetime_bounds.insert(
            ValueId::new(1),
            bounds(LifetimeClass::Slice, LifetimeClass::Token),
        );
        let upper_bindings = binding_vec(&result);
        let upper =
            validate_storage_plan_self_consistency(&context, view(&result, &upper_bindings));

        assert_has_store(
            &lower,
            StoragePlanDiagnosticCode::StorageLifetimeAdmissibilityViolation,
        );
        assert_has_store(
            &upper,
            StoragePlanDiagnosticCode::StorageLifetimeAdmissibilityViolation,
        );
    }

    #[test]
    fn sc11_closed_membership_scan_allows_legal_storage_tags() {
        let legal = json!({
            "class": "WramHot",
            "other": ["SramPaged", "RomConst"],
            "note": closed_key(&["byte_", "offset", "_suffix"]),
            "case_variant_key": { "ByteOffset": 4 },
            "case_variant_tag": closed_key(&["overlay", "id"])
        });
        let illegal = json!({
            "class": "WramHot",
            closed_key(&["byte_", "offset"]): 4,
            "tag": closed_key(&["Overlay", "Id"])
        });

        assert!(closed_spatial_surface_diagnostics(&legal).is_empty());
        assert_has_store(
            &closed_spatial_surface_diagnostics(&illegal),
            StoragePlanDiagnosticCode::StorageForbiddenSpatialEnumLeak,
        );
    }

    #[test]
    fn sc12_input_identity_mismatch_fires_store_020() {
        let result = fixture_result();
        let mut context = fixture_context();
        context.expected_input_hashes.range_plan_hash = hash(0x99);
        let bindings = binding_vec(&result);

        let diagnostics =
            validate_storage_plan_self_consistency(&context, view(&result, &bindings));

        assert_has_store(
            &diagnostics,
            StoragePlanDiagnosticCode::StorageRangePlanHashMismatch,
        );
    }

    #[test]
    fn sc12_report_hash_is_envelope_level_and_not_body_level() {
        let body = StoragePlanReportEnvelopeBody {
            body: StoragePlanReportBody {
                outcome: ReportOutcome::Failed,
                result: None,
                diagnostics: vec![diagnostic(
                    StoragePlanDiagnosticCode::StorageRangePlanHashMismatch,
                    StoragePlanDiagnosticProvenance::HashMismatch {
                        product: "range_plan_hash".to_owned(),
                        recorded: hash(0x99),
                        computed: hash(0x04),
                    },
                )],
                input_identity: identity(),
                summary: None,
            },
        };
        let envelope = ReportEnvelope::new(ReportOutcome::Failed, body.clone())
            .expect("envelope")
            .with_computed_self_hash()
            .expect("self hash");

        validate_storage_plan_report_self_hash(&envelope).expect("self hash validates");
        assert!(
            !serialized_body_contains_report_self_hash(&body).expect("body serializes"),
            "production storage plan body must not duplicate the envelope self hash"
        );
    }

    #[test]
    fn provenance_evidence_is_sorted_after_construction_and_deserialization() {
        let unsorted_evidence = vec![evidence("proof", "z"), evidence("proof", "a")];

        let binding = BindingProvenance::new(
            AdmittingPredicateId(1),
            DecisionRuleId(1),
            false,
            unsorted_evidence.clone(),
            None,
            None,
        );
        let alias = AliasClassProvenance::new(AliasIntent::NoAlias, unsorted_evidence.clone());
        let group =
            CommitGroupProvenance::new(CommitGroupReason::Independent, unsorted_evidence.clone());

        assert_sorted(&binding.evidence);
        assert_sorted(&alias.evidence);
        assert_sorted(&group.evidence);

        let binding: BindingProvenance = serde_json::from_value(json!({
            "admitting_predicate": 1,
            "decision_rule": 1,
            "policy_refinement_applied": false,
            "evidence": unsorted_evidence,
            "op_output_role": null,
            "op_output_format": null
        }))
        .expect("binding provenance deserializes");
        let alias: AliasClassProvenance = serde_json::from_value(json!({
            "admitting_intent": "NoAlias",
            "evidence": [evidence("proof", "z"), evidence("proof", "a")]
        }))
        .expect("alias provenance deserializes");
        let group: CommitGroupProvenance = serde_json::from_value(json!({
            "reason": "Independent",
            "evidence": [evidence("proof", "z"), evidence("proof", "a")]
        }))
        .expect("commit group provenance deserializes");

        assert_sorted(&binding.evidence);
        assert_sorted(&alias.evidence);
        assert_sorted(&group.evidence);
    }

    #[derive(Debug, Clone)]
    struct FixtureResult {
        input_identity: StoragePlanInputIdentity,
        bindings: BTreeMap<ValueId, StorageBinding>,
        alias_classes: BTreeMap<AliasClassId, AliasClass>,
        persist_pages: BTreeMap<PersistPageId, PersistPageDecl>,
        commit_groups: BTreeMap<CommitGroupId, CommitGroupDecl>,
    }

    fn fixture_result() -> FixtureResult {
        let alias_class = AliasClass::from_members(
            AliasClassId(0),
            [ValueId::new(1), ValueId::new(2)],
            AliasIntent::ScratchReuse,
        )
        .expect("class");
        let mut bindings = BTreeMap::new();
        bindings.insert(ValueId::new(1), materialized_binding(1, AliasClassId(0)));
        bindings.insert(ValueId::new(2), materialized_binding(2, AliasClassId(0)));
        bindings.insert(
            ValueId::new(3),
            StorageBinding {
                value: ValueId::new(3),
                materialization: Materialization::Persist {
                    page: PersistPageId(1),
                    commit_group: CommitGroupId(1),
                },
                alias_class: AliasClassId(1),
                live_range: live_range(3),
                justification: BindingJustification::DecisionRule(DecisionRuleId(1)),
            },
        );

        FixtureResult {
            input_identity: identity(),
            bindings,
            alias_classes: BTreeMap::from([
                (AliasClassId(0), alias_class),
                (
                    AliasClassId(1),
                    AliasClass::from_members(
                        AliasClassId(1),
                        [ValueId::new(3)],
                        AliasIntent::NoAlias,
                    )
                    .expect("singleton class"),
                ),
            ]),
            persist_pages: BTreeMap::from([(
                PersistPageId(1),
                PersistPageDecl {
                    id: PersistPageId(1),
                    kind: PersistKind::Trace,
                    durability: DurabilityClass::BestEffort,
                    schema_pin: schema_pin(),
                },
            )]),
            commit_groups: BTreeMap::from([(
                CommitGroupId(1),
                CommitGroupDecl {
                    id: CommitGroupId(1),
                    members: NonEmptySortedSet::new([PersistPageId(1)]).expect("member"),
                    kind_set: BTreeSet::from([PersistKind::Trace]),
                    atomicity: CommitAtomicityClass::AllOrNothing,
                },
            )]),
        }
    }

    fn binding_vec(result: &FixtureResult) -> Vec<StorageBinding> {
        result.bindings.values().cloned().collect()
    }

    fn view<'a>(
        result: &'a FixtureResult,
        bindings: &'a [StorageBinding],
    ) -> StoragePlanConsistencyView<'a> {
        StoragePlanConsistencyView {
            input_identity: &result.input_identity,
            bindings,
            alias_classes: &result.alias_classes,
            persist_pages: &result.persist_pages,
            commit_groups: &result.commit_groups,
            json_value: None,
        }
    }

    fn fixture_context() -> StoragePlanConsistencyContext {
        StoragePlanConsistencyContext {
            expected_values: BTreeSet::from([ValueId::new(1), ValueId::new(2), ValueId::new(3)]),
            lifetime_bounds: BTreeMap::from([
                (
                    ValueId::new(1),
                    bounds(LifetimeClass::Slice, LifetimeClass::Persistent),
                ),
                (
                    ValueId::new(2),
                    bounds(LifetimeClass::Slice, LifetimeClass::Persistent),
                ),
                (
                    ValueId::new(3),
                    bounds(LifetimeClass::Persistent, LifetimeClass::Persistent),
                ),
            ]),
            expected_input_hashes: input_hashes(),
        }
    }

    fn materialized_binding(value: u32, alias_class: AliasClassId) -> StorageBinding {
        StorageBinding {
            value: ValueId::new(value),
            materialization: Materialization::Materialize {
                class: StorageClass::WramHot,
                lifetime: LifetimeClass::Slice,
            },
            alias_class,
            live_range: live_range(value),
            justification: BindingJustification::DecisionRule(DecisionRuleId(1)),
        }
    }

    fn live_range(node: u32) -> AbstractLiveRange {
        AbstractLiveRange {
            def_node: NodeId::new(node),
            first_use_node: Some(NodeId::new(node + 10)),
            last_use_node: Some(NodeId::new(node + 10)),
            lifetime_class: LifetimeClass::Slice,
            checkpoint_stable: false,
        }
    }

    fn bounds(min: LifetimeClass, max: LifetimeClass) -> LifetimeBounds {
        LifetimeBounds {
            min_required: LifetimeBound {
                lifetime: min,
                source: LifetimeBoundSource::DefaultSlice,
            },
            max_admissible: LifetimeBound {
                lifetime: max,
                source: LifetimeBoundSource::DefUseWidth,
            },
        }
    }

    fn schema_pin() -> PersistSchemaPin {
        PersistSchemaPin {
            state_schema: 3,
            requires_semantic_state_hash: false,
            requires_resume_abi_hash: false,
            requires_build_identity_hash: true,
        }
    }

    fn identity() -> StoragePlanInputIdentity {
        StoragePlanInputIdentity {
            quant_graph_hash: hash(1),
            infer_ir_hash: hash(2),
            observation_plan_hash: hash(3),
            range_plan_hash: hash(4),
            policy_hash: hash(5),
            determinism: DeterminismClass::Deterministic,
            schema: STORAGE_PLAN_SCHEMA_ID.into(),
            schema_version: STORAGE_PLAN_SCHEMA_VERSION,
        }
    }

    fn input_hashes() -> StoragePlanInputHashes {
        StoragePlanInputHashes {
            quant_graph_hash: hash(1),
            infer_ir_hash: hash(2),
            observation_plan_hash: hash(3),
            range_plan_hash: hash(4),
            policy_hash: hash(5),
        }
    }

    fn hash(byte: u8) -> Hash256 {
        Hash256::from_bytes([byte; 32])
    }

    fn assert_has_store(diagnostics: &[ValidationDiagnostic], expected: StoragePlanDiagnosticCode) {
        assert!(
            diagnostics.iter().any(|diagnostic| matches!(
                diagnostic.code,
                ValidationCode::StoragePlan { code, .. } if code == expected
            )),
            "missing {expected:?} in {diagnostics:#?}"
        );
    }

    fn assert_has_provenance_invariant(diagnostics: &[ValidationDiagnostic], invariant: &str) {
        assert!(
            diagnostics
                .iter()
                .filter_map(storage_plan_provenance)
                .any(|provenance| provenance_invariant(provenance) == Some(invariant)),
            "missing provenance invariant {invariant} in {diagnostics:#?}"
        );
    }

    fn storage_plan_provenance(
        diagnostic: &ValidationDiagnostic,
    ) -> Option<&StoragePlanDiagnosticProvenance> {
        match &diagnostic.code {
            ValidationCode::StoragePlan { provenance, .. } => Some(provenance),
            _ => None,
        }
    }

    fn provenance_invariant(provenance: &StoragePlanDiagnosticProvenance) -> Option<&str> {
        match provenance {
            StoragePlanDiagnosticProvenance::AliasMembership { invariant, .. }
            | StoragePlanDiagnosticProvenance::PersistPage { invariant, .. }
            | StoragePlanDiagnosticProvenance::CommitGroup { invariant, .. } => Some(invariant),
            _ => None,
        }
    }

    fn evidence(kind: &str, reference: &str) -> EvidenceRef {
        EvidenceRef {
            kind: kind.to_owned(),
            reference: reference.to_owned(),
            hash: None,
        }
    }

    fn assert_sorted(evidence: &[EvidenceRef]) {
        assert!(
            evidence.windows(2).all(|window| window[0] <= window[1]),
            "evidence is not sorted: {evidence:#?}"
        );
    }
}
