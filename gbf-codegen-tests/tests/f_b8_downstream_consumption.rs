use std::collections::BTreeMap;

use gbf_codegen::s3::infer_ir::{NodeId, ValueId};
use gbf_codegen::storage_plan::{
    AbstractLiveRange, AdmittingPredicateId, AliasClass, AliasClassId, AliasIntent,
    BindingJustification, BindingProvenance, CommitAtomicityClass, CommitGroupDecl, CommitGroupId,
    CommitGroupProvenance, CommitGroupReason, DecisionRuleId, DurabilityClass, LifetimeClass,
    Materialization, NonEmptySortedSet, PersistKind, PersistPageDecl, PersistPageId,
    PersistPageProvenance, PersistPageSource, PersistSchemaPin, StorageBinding, StorageClass,
    StoragePlanCacheKeyInputs, StoragePlanCoreOutcome, StoragePlanCoreResult,
    StoragePlanReportResult, StorageProvenance, ValueRole, build_storage_plan_core,
    emit_storage_plan_json_bytes, is_overlay_eligible, parse_storage_plan_report_bytes,
};
use gbf_codegen_tests::synth;
use serde_json::json;

#[test]
fn downstream_overlay_plan_reads_eligible_roles_from_storage_plan_only() {
    let plan = storage_plan_with_bindings([
        rom_const_binding(1, ValueRole::ExpertWeight),
        rom_const_binding(2, ValueRole::RouterWeight),
        rom_const_binding(3, ValueRole::LutFragment),
        rom_const_binding(4, ValueRole::RouterScore),
    ]);

    let eligible: Vec<_> = plan
        .bindings
        .values()
        .filter(|binding| is_overlay_eligible(&plan, binding))
        .map(|binding| binding.value)
        .collect();

    assert_eq!(
        eligible,
        vec![ValueId::new(1), ValueId::new(2), ValueId::new(3)],
        "F-B11 must consume op_output_role from StoragePlan provenance without consulting GbInferIR"
    );
}

#[test]
fn downstream_stage_smokes_extract_storage_plan_fields_without_ir() {
    let mut plan = storage_plan_with_bindings([
        materialized_binding(10, StorageClass::SramPaged, ValueRole::Activation),
        persist_binding(11, ValueRole::SequenceStateSlot),
        materialized_binding(12, StorageClass::RomConst, ValueRole::RouterWeight),
        materialized_binding(13, StorageClass::WramHot, ValueRole::Accumulator),
    ]);
    plan.persist_pages.insert(
        PersistPageId(11),
        PersistPageDecl {
            id: PersistPageId(11),
            kind: PersistKind::SequenceState,
            durability: DurabilityClass::Critical,
            schema_pin: PersistSchemaPin {
                state_schema: 1,
                requires_semantic_state_hash: true,
                requires_resume_abi_hash: false,
                requires_build_identity_hash: false,
            },
        },
    );
    plan.provenance.persist_pages.insert(
        PersistPageId(11),
        PersistPageProvenance {
            source: PersistPageSource::SequenceStateSlot { layer: 0, slot: 0 },
        },
    );
    plan.commit_groups.insert(
        CommitGroupId(11),
        CommitGroupDecl {
            id: CommitGroupId(11),
            members: NonEmptySortedSet::new([PersistPageId(11)])
                .expect("persist page set is non-empty"),
            kind_set: [PersistKind::SequenceState].into_iter().collect(),
            atomicity: CommitAtomicityClass::AllOrNothing,
        },
    );
    plan.provenance.commit_groups.insert(
        CommitGroupId(11),
        CommitGroupProvenance::new(CommitGroupReason::PerSequenceStateSlot, Vec::new()),
    );

    let sram_page_plan: Vec<_> = plan
        .bindings
        .values()
        .filter_map(|binding| match binding.materialization {
            Materialization::Materialize {
                class: StorageClass::SramPaged,
                ..
            } => Some((
                binding.value,
                format!("SramPageId({})", binding.value.get()),
            )),
            Materialization::Persist { page, .. } => {
                Some((binding.value, format!("SramPageId({})", page.0)))
            }
            Materialization::Recompute
            | Materialization::Materialize {
                class: StorageClass::WramHot | StorageClass::HramHot | StorageClass::RomConst,
                ..
            } => None,
        })
        .collect();
    assert_eq!(
        sram_page_plan,
        vec![
            (ValueId::new(10), "SramPageId(10)".to_owned()),
            (ValueId::new(11), "SramPageId(11)".to_owned()),
        ],
        "F-B9 shape must expose SramPaged and Persist bindings directly"
    );

    let rom_window_plan: Vec<_> = plan
        .bindings
        .values()
        .filter_map(|binding| match binding.materialization {
            Materialization::Materialize {
                class: StorageClass::RomConst,
                ..
            } => Some((binding.value, "BankClass::Const")),
            _ => None,
        })
        .collect();
    assert_eq!(
        rom_window_plan,
        vec![(ValueId::new(12), "BankClass::Const")],
        "F-B10 shape must expose RomConst bindings directly"
    );

    let arena_ranges: Vec<_> = plan
        .bindings
        .values()
        .filter_map(|binding| match binding.materialization {
            Materialization::Materialize { .. } => Some((
                binding.value,
                binding.live_range.def_node.get()..=binding.live_range.last_use_node.unwrap().get(),
            )),
            Materialization::Recompute | Materialization::Persist { .. } => None,
        })
        .collect();
    assert_eq!(
        arena_ranges,
        vec![
            (ValueId::new(10), 10..=10),
            (ValueId::new(12), 12..=12),
            (ValueId::new(13), 13..=13),
        ],
        "F-B12 shape must expose materialized ValueId keyed ranges directly"
    );

    let resource_state_cert = json!({
        "bindings": plan.bindings.values().map(|binding| {
            let alias_class = plan.alias_classes.get(&binding.alias_class).unwrap();
            json!({
                "value_id": binding.value.get(),
                "alias_class": binding.alias_class.0,
                "alias_intent": format!("{:?}", alias_class.intent()),
            })
        }).collect::<Vec<_>>(),
    });
    assert_eq!(
        resource_state_cert["bindings"][0]["alias_intent"], "NoAlias",
        "F-B13 must read bindings.alias_class plus alias_classes[A].intent"
    );
    assert!(
        resource_state_cert.get("alias_intents").is_none(),
        "F-B13 smoke must not depend on the removed alias_intents map"
    );
}

#[test]
fn downstream_stage_smokes_parse_routed_ffn_fixture_json_without_ir() {
    let plan = routed_ffn_fixture_result_from_json();

    let sram_page_plan: Vec<_> = plan
        .bindings
        .iter()
        .filter_map(|entry| match entry.value.materialization {
            Materialization::Materialize {
                class: StorageClass::SramPaged,
                ..
            } => Some((entry.key, format!("SramPageId({})", entry.key.get()))),
            Materialization::Persist { page, .. } => {
                Some((entry.key, format!("SramPageId({})", page.0)))
            }
            Materialization::Recompute
            | Materialization::Materialize {
                class: StorageClass::WramHot | StorageClass::HramHot | StorageClass::RomConst,
                ..
            } => None,
        })
        .collect();
    assert!(
        sram_page_plan.is_empty(),
        "routed-FFN fixture has no SRAM/Persist bindings, and F-B9 can read that directly"
    );

    let rom_window_plan: Vec<_> = plan
        .bindings
        .iter()
        .filter_map(|entry| match entry.value.materialization {
            Materialization::Materialize {
                class: StorageClass::RomConst,
                ..
            } => Some(entry.key),
            _ => None,
        })
        .collect();
    assert!(
        rom_window_plan.contains(&ValueId::new(13)),
        "F-B10 must read RomConst routed expert weights from storage_plan.json"
    );

    let arena_ranges: Vec<_> = plan
        .bindings
        .iter()
        .filter_map(|entry| match entry.value.materialization {
            Materialization::Materialize { .. } => Some((
                entry.key,
                entry.value.live_range.def_node.get()
                    ..=entry.value.live_range.last_use_node.unwrap().get(),
            )),
            Materialization::Recompute | Materialization::Persist { .. } => None,
        })
        .collect();
    assert!(
        arena_ranges.len() >= plan.bindings.len(),
        "F-B12 can derive materialized ValueId-keyed ranges from the parsed report"
    );

    let alias_intents: BTreeMap<_, _> = plan
        .alias_classes
        .iter()
        .map(|entry| (entry.key, entry.value.intent))
        .collect();
    let resource_state_cert = json!({
        "bindings": plan.bindings.iter().map(|entry| {
            json!({
                "value_id": entry.key.get(),
                "alias_class": entry.value.alias_class.0,
                "alias_intent": format!("{:?}", alias_intents[&entry.value.alias_class]),
            })
        }).collect::<Vec<_>>(),
    });
    assert!(
        resource_state_cert["bindings"]
            .as_array()
            .expect("bindings are an array")
            .iter()
            .any(|binding| binding["alias_intent"] == "ScratchReuse"),
        "F-B13 must read alias intent through bindings.alias_class and alias_classes[A].intent"
    );
}

#[test]
fn stage_cache_k6_uses_domain_hash_input_shape_without_storage_result() {
    let input = synth::minimal_singleton_core_input();
    let cache_inputs = StoragePlanCacheKeyInputs::from_input_identity(&input.input_identity)
        .expect("K6 cache inputs derive from identity");
    let key = cache_inputs.cache_key().expect("K6 cache key hashes");
    let replayed_key = StoragePlanCacheKeyInputs::from_input_identity(&input.input_identity)
        .expect("K6 replay cache inputs derive from identity")
        .cache_key()
        .expect("K6 replay cache key hashes");
    let json = serde_json::to_value(&cache_inputs).expect("K6 cache input serializes");

    assert_eq!(key, replayed_key, "K6 cache keys must be reproducible");
    assert!(
        !serde_json::to_value(key)
            .expect("K6 key serializes")
            .is_null()
    );
    for field in [
        "quant_graph_hash",
        "infer_ir_hash",
        "observation_plan_hash",
        "range_plan_hash",
        "policy_hash",
        "determinism",
        "schema",
        "schema_version",
        "decision_rule_set_hash",
        "persist_compat_hash",
        "alias_rule_set_hash",
    ] {
        assert!(json.get(field).is_some(), "missing K6 field {field}");
    }
    assert!(json.get("report_self_hash").is_none());
    assert!(json.get("result").is_none());
}

fn routed_ffn_fixture_result_from_json() -> StoragePlanReportResult {
    let input = synth::InputBuilder::with_expert_weights(8)
        .with_router_decision_value()
        .with_renorm_loop_scratch(16)
        .build_core();
    let output = build_storage_plan_core(&input);
    assert_eq!(output.outcome, StoragePlanCoreOutcome::Succeeded);
    let bytes = emit_storage_plan_json_bytes(&output).expect("routed fixture emits JSON");
    let report = parse_storage_plan_report_bytes(&bytes).expect("routed fixture JSON parses");
    report
        .body
        .body
        .result
        .expect("routed fixture report carries result")
}

fn storage_plan_with_bindings<const N: usize>(
    entries: [(StorageBinding, ValueRole); N],
) -> StoragePlanCoreResult {
    let mut bindings = BTreeMap::new();
    let mut alias_classes = BTreeMap::new();
    let mut provenance = BTreeMap::new();

    for (binding, role) in entries {
        let alias_class =
            AliasClass::from_members(binding.alias_class, [binding.value], AliasIntent::NoAlias)
                .expect("singleton alias class is valid");
        alias_classes.insert(binding.alias_class, alias_class);
        provenance.insert(
            binding.value,
            BindingProvenance::new(
                AdmittingPredicateId(1),
                DecisionRuleId(1),
                false,
                Vec::new(),
                Some(role),
                None,
            ),
        );
        bindings.insert(binding.value, binding);
    }

    StoragePlanCoreResult {
        bindings,
        alias_classes,
        persist_pages: BTreeMap::new(),
        commit_groups: BTreeMap::new(),
        repair_proposals: Vec::new(),
        provenance: StorageProvenance {
            bindings: provenance,
            alias_classes: BTreeMap::new(),
            persist_pages: BTreeMap::new(),
            commit_groups: BTreeMap::new(),
        },
    }
}

fn rom_const_binding(id: u32, role: ValueRole) -> (StorageBinding, ValueRole) {
    materialized_binding(id, StorageClass::RomConst, role)
}

fn materialized_binding(
    id: u32,
    class: StorageClass,
    role: ValueRole,
) -> (StorageBinding, ValueRole) {
    (
        binding(
            id,
            Materialization::Materialize {
                class,
                lifetime: LifetimeClass::Persistent,
            },
        ),
        role,
    )
}

fn persist_binding(id: u32, role: ValueRole) -> (StorageBinding, ValueRole) {
    (
        binding(
            id,
            Materialization::Persist {
                page: PersistPageId(id),
                commit_group: CommitGroupId(id),
            },
        ),
        role,
    )
}

fn binding(id: u32, materialization: Materialization) -> StorageBinding {
    StorageBinding {
        value: ValueId::new(id),
        materialization,
        alias_class: AliasClassId(id),
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
