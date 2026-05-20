use std::collections::BTreeSet;

use gbf_codegen::s3::infer_ir::ValueId;
use gbf_codegen::storage_plan::{
    AliasIntent, BindingJustification, KnobDelta, LifetimeClass, Materialization, PlanningStage,
    RepairReason, StorageClass, StoragePlanCoreOutcome,
    StoreDiagnosticCode as StoragePlanDiagnosticCode, emit_storage_plan_json_bytes,
    emit_storage_plan_report, parse_storage_plan_report_bytes,
    storage_plan_core_output_canonical_bytes,
};
use gbf_codegen_tests::sc_violations::{
    self, StoragePlanProductionFixture, StoragePlanViolationFixture,
    StoragePlanViolationFixtureKind,
};
use gbf_codegen_tests::synth;

#[test]
fn rejection_class_all_store_fixtures_emit_exact_failed_reports() {
    let fixtures = sc_violations::all_store_violation_factories();
    assert_eq!(
        fixtures.len(),
        StoragePlanDiagnosticCode::ALL.len(),
        "bd-dkpz must keep one executable fixture for every STORE code"
    );
    let mut report_gaps = Vec::new();

    for fixture in fixtures {
        let expected_identity = fixture_input_identity(&fixture);
        let output = fixture.run().expect("production fixture runs");

        assert_eq!(
            output.outcome,
            StoragePlanCoreOutcome::Failed,
            "{} must reject",
            fixture.fixture_id
        );
        assert_eq!(
            output.diagnostics,
            vec![fixture.expected_code],
            "{} must emit exactly one named STORE code",
            fixture.fixture_id
        );
        assert!(
            output.result.is_none(),
            "{} must not emit a partial result on rejection",
            fixture.fixture_id
        );
        assert!(
            output.summary.is_none(),
            "{} must not emit a summary on rejection",
            fixture.fixture_id
        );
        assert_eq!(
            output.input_identity, expected_identity,
            "{} must preserve body.input_identity for rejected reports",
            fixture.fixture_id
        );
        assert!(
            !fixture.provenance_schema.is_empty(),
            "{} must declare typed provenance schema for {}",
            fixture.fixture_id,
            fixture.expected_code.as_str()
        );

        let report = emit_storage_plan_report(&output).expect("failed report emits");
        let body = &report.body.body;
        assert!(body.result.is_none());
        assert!(body.summary.is_none());
        assert_eq!(body.input_identity, expected_identity);
        assert_eq!(body.diagnostics.len(), 1);

        let code_json =
            serde_json::to_value(&body.diagnostics[0].code).expect("diagnostic code serializes");
        let code_text = code_json.to_string();
        if !code_text.contains(fixture.expected_code.name()) || !code_text.contains("provenance") {
            report_gaps.push(format!(
                "{} expected {} typed provenance, got {code_text}",
                fixture.fixture_id,
                fixture.expected_code.as_str()
            ));
        }
    }

    assert!(
        report_gaps.is_empty(),
        "production rejection fixtures must emit typed StoragePlan diagnostics:\n{}",
        report_gaps.join("\n")
    );
}

#[test]
fn po_1_minimal_driver_output_is_byte_identical_across_runs() {
    let input = synth::minimal_singleton_core_input();

    let first = gbf_codegen::storage_plan::build_storage_plan_core(&input);
    let second = gbf_codegen::storage_plan::build_storage_plan_core(&input);

    assert_eq!(first.outcome, StoragePlanCoreOutcome::Succeeded);
    assert!(first.diagnostics.is_empty());
    assert_eq!(
        storage_plan_core_output_canonical_bytes(&first).expect("first output canonicalizes"),
        storage_plan_core_output_canonical_bytes(&second).expect("second output canonicalizes"),
        "PO-1: pure storage-plan driver runs must be byte-identical for the same input"
    );
}

#[test]
fn po_2_binding_coverage_has_store_002_and_store_003_witnesses() {
    assert_store_witness(StoragePlanDiagnosticCode::StorageBindingCoverageGap);
    assert_store_witness(StoragePlanDiagnosticCode::StorageBindingDoubleBind);
}

#[test]
fn po_3_alias_equivalence_relation_is_functional_on_successful_plan() {
    let output =
        gbf_codegen::storage_plan::build_storage_plan_core(&synth::minimal_singleton_core_input());
    let result = output.result.expect("successful plan has result");

    for binding in result.bindings.values() {
        let class = result
            .alias_classes
            .get(&binding.alias_class)
            .expect("binding alias class exists");
        assert!(
            class
                .members()
                .iter()
                .any(|member| *member == binding.value),
            "alias equivalence must be reflexive for every binding"
        );
    }
}

#[test]
fn po_4_alias_intent_materialization_compatibility_has_store_013_witness() {
    assert_store_witness(StoragePlanDiagnosticCode::StorageAliasIntentMaterializationMismatch);
}

#[test]
fn po_5_alias_no_conflict_uses_live_range_overlap_store_014_witness() {
    assert_store_witness(StoragePlanDiagnosticCode::StorageAliasClassOverlapWithoutIntent);
}

#[test]
fn po_6_recompute_first_class_records_forced_rule_and_singleton_alias() {
    let mut input = synth::minimal_singleton_core_input();
    input.alias_forced_recompute_values.insert(ValueId::new(1));

    let output = gbf_codegen::storage_plan::build_storage_plan_core(&input);
    assert_eq!(output.outcome, StoragePlanCoreOutcome::Succeeded);
    let result = output.result.expect("successful plan has result");
    let binding = result
        .bindings
        .get(&ValueId::new(1))
        .expect("forced binding exists");
    assert_eq!(binding.materialization, Materialization::Recompute);
    assert_eq!(binding.justification, BindingJustification::ForcedRecompute);
    let class = result
        .alias_classes
        .get(&binding.alias_class)
        .expect("forced binding alias class exists");
    assert_eq!(class.intent(), AliasIntent::NoAlias);
    assert_eq!(class.members().len(), 1);
}

#[test]
fn po_7_recompute_alias_isolation_has_store_016_witness() {
    assert_store_witness(StoragePlanDiagnosticCode::StorageRecomputeAliasNotIsolated);
}

#[test]
fn po_8_recompute_observation_forbidden_has_store_006_witness() {
    assert_store_witness(StoragePlanDiagnosticCode::StorageRecomputeForbiddenForObservedValue);
}

#[test]
fn po_9_persist_plug_compatible_has_store_009_and_store_010_witnesses() {
    assert_store_witness(StoragePlanDiagnosticCode::StoragePersistPageNotReferenced);
    assert_store_witness(StoragePlanDiagnosticCode::StorageCommitGroupEmpty);
}

#[test]
fn po_10_persist_commit_group_well_formed_has_store_008_011_012_witnesses() {
    assert_store_witness(StoragePlanDiagnosticCode::StoragePersistBindingKindMismatch);
    assert_store_witness(StoragePlanDiagnosticCode::StorageCommitGroupKindMix);
    assert_store_witness(StoragePlanDiagnosticCode::StorageCommitGroupDurabilityMix);
}

#[test]
fn po_11_persist_alias_rotation_only_has_store_013_witness() {
    assert_store_witness(StoragePlanDiagnosticCode::StorageAliasIntentMaterializationMismatch);
}

#[test]
fn po_12_persist_sequence_state_v1_rejects_with_store_007() {
    assert_store_witness(StoragePlanDiagnosticCode::StoragePersistSequenceStateUnsupportedV1);
}

#[test]
fn po_13_lifetime_bounds_has_store_017_witness() {
    assert_store_witness(StoragePlanDiagnosticCode::StorageLifetimeAdmissibilityViolation);
}

#[test]
fn po_14_refinement_rerun_keeps_lifetimes_non_decreasing() {
    let baseline = synth::minimal_singleton_core_input();
    let refined = synth::InputBuilder::minimal()
        .with_promotion_level(synth::RecomputePromotionLevel::RecomputePureValues)
        .build_core();

    let baseline = gbf_codegen::storage_plan::build_storage_plan_core(&baseline)
        .result
        .expect("baseline succeeds");
    let refined = gbf_codegen::storage_plan::build_storage_plan_core(&refined)
        .result
        .expect("refined succeeds");

    for (value, before) in baseline.bindings {
        let after = refined
            .bindings
            .get(&value)
            .expect("refined binding still exists");
        if let (
            Materialization::Materialize {
                lifetime: before, ..
            },
            Materialization::Materialize {
                lifetime: after, ..
            },
        ) = (&before.materialization, &after.materialization)
        {
            assert!(after >= before, "lifetime decreased for {value:?}");
        }
    }
}

#[test]
fn po_15_storage_plan_json_has_no_spatial_enum_leak() {
    let output =
        gbf_codegen::storage_plan::build_storage_plan_core(&synth::minimal_singleton_core_input());
    let bytes = emit_storage_plan_json_bytes(&output).expect("storage_plan.json emits");
    let text = String::from_utf8(bytes).expect("json is utf8");

    for forbidden in [
        "byte_offset",
        "byte_alignment",
        "byte_address",
        "concrete_bank",
        "rom_bank",
        "sram_bank",
        "slice_id",
    ] {
        assert!(
            !text.contains(forbidden),
            "storage_plan.json leaked forbidden spatial key {forbidden}"
        );
    }
}

#[test]
fn po_16_self_hash_invariant_round_trips_on_emitted_json() {
    let output =
        gbf_codegen::storage_plan::build_storage_plan_core(&synth::minimal_singleton_core_input());
    let bytes = emit_storage_plan_json_bytes(&output).expect("self-hashed report emits");

    parse_storage_plan_report_bytes(&bytes).expect("self-hashed report parses");
}

#[test]
fn po_17_schema_unknown_field_rejects_at_parse_time() {
    let output =
        gbf_codegen::storage_plan::build_storage_plan_core(&synth::minimal_singleton_core_input());
    let mut value = serde_json::to_value(emit_storage_plan_report(&output).expect("report emits"))
        .expect("report serializes");
    value["body"]["body"]["unknown_storage_plan_field"] = serde_json::json!(true);
    let bytes = serde_json::to_vec(&value).expect("mutated report serializes");

    assert!(
        parse_storage_plan_report_bytes(&bytes).is_err(),
        "unknown storage_plan.json fields must be rejected"
    );
}

#[test]
fn po_18_renorm_loop_scratch_binds_hot_slice_storage() {
    let input = synth::InputBuilder::with_expert_weights(8)
        .with_router_decision_value()
        .with_renorm_loop_scratch(16)
        .build_core();

    let output = gbf_codegen::storage_plan::build_storage_plan_core(&input);
    let result = output.result.expect("routed FFN plan succeeds");
    for value in [ValueId::new(19), ValueId::new(20)] {
        let binding = result.bindings.get(&value).expect("renorm scratch binding");
        assert!(
            matches!(
                binding.materialization,
                Materialization::Materialize {
                    class: StorageClass::WramHot | StorageClass::HramHot,
                    lifetime: LifetimeClass::Slice,
                }
            ),
            "renorm scratch {value:?} must be hot Slice storage"
        );
    }
}

#[test]
fn po_19_range_plan_hash_binding_matches_expected_input_hashes() {
    let input = synth::minimal_singleton_core_input();

    assert_eq!(
        input.input_identity.range_plan_hash,
        input.expected_input_hashes.range_plan_hash
    );
    assert_store_witness(StoragePlanDiagnosticCode::StorageRangePlanHashMismatch);
}

#[test]
fn po_20_knob_bounds_honored_has_store_027_witness() {
    assert_store_witness(StoragePlanDiagnosticCode::StorageRepairProposalIllegal);
}

#[test]
fn po_21_knob_locks_suppress_storage_recompute_promotion() {
    let mut input = synth::minimal_singleton_core_input();
    input.repair_policy.soft_pressure_threshold_bytes = Some(0);
    input.predicate_env = std::mem::take(&mut input.predicate_env)
        .with_recompute_cycle_ceiling(8)
        .with_recompute_cost_estimate(ValueId::new(1), 5);
    input.repair_policy.storage_recompute_promotion_locked = true;

    let output = gbf_codegen::storage_plan::build_storage_plan_core(&input);
    assert_eq!(output.outcome, StoragePlanCoreOutcome::Succeeded);
    assert!(
        output
            .result
            .expect("locked plan succeeds")
            .repair_proposals
            .is_empty(),
        "locked StorageRecomputePromotion must suppress repair proposals"
    );
}

#[test]
fn po_22_forced_recompute_honored_and_rejection_paths_are_witnessed() {
    let mut input = synth::minimal_singleton_core_input();
    input.alias_forced_recompute_values.insert(ValueId::new(1));

    let output = gbf_codegen::storage_plan::build_storage_plan_core(&input);
    let binding = output
        .result
        .expect("forced recompute plan succeeds")
        .bindings
        .remove(&ValueId::new(1))
        .expect("forced binding exists");
    assert_eq!(binding.materialization, Materialization::Recompute);
    assert_eq!(binding.justification, BindingJustification::ForcedRecompute);
    assert_store_witness(StoragePlanDiagnosticCode::StorageForcedRecomputeNotAllowed);
}

#[test]
fn po_23_repair_proposal_shape_is_storage_plan_admissible() {
    let mut input = synth::minimal_singleton_core_input();
    input.repair_policy.soft_pressure_threshold_bytes = Some(0);
    input.predicate_env = std::mem::take(&mut input.predicate_env)
        .with_recompute_cycle_ceiling(8)
        .with_recompute_cost_estimate(ValueId::new(1), 5);

    let output = gbf_codegen::storage_plan::build_storage_plan_core(&input);
    let proposals = output
        .result
        .expect("repair-proposal input succeeds")
        .repair_proposals;
    assert!(
        !proposals.is_empty(),
        "PO-23 needs a non-empty proposal witness"
    );
    for proposal in proposals {
        assert_eq!(proposal.source, PlanningStage::StoragePlan);
        assert_eq!(proposal.reason, RepairReason::PromoteRecompute);
        assert!(proposal.estimated_cost.cycles.is_some());
        for change in proposal.tighten.changes {
            assert!(
                matches!(
                    change,
                    KnobDelta::PromoteRecomputeLevel { .. } | KnobDelta::ForceRecompute { .. }
                ),
                "unexpected repair proposal change {change:?}"
            );
        }
    }
}

#[test]
fn rejection_fixture_registry_keeps_stub_and_production_classes_disjoint() {
    let production_codes: BTreeSet<_> = sc_violations::production_backed_violation_factories()
        .into_iter()
        .map(|fixture| fixture.expected_code.as_str().to_owned())
        .collect();
    let synthetic_codes: BTreeSet<_> = sc_violations::synthetic_diagnostic_violation_factories()
        .into_iter()
        .map(|fixture| fixture.expected_code.as_str().to_owned())
        .collect();
    let stub_codes: BTreeSet<_> = sc_violations::stub_only_violation_factories()
        .into_iter()
        .map(|fixture| fixture.expected_code.as_str().to_owned())
        .collect();

    assert!(production_codes.is_disjoint(&stub_codes));
    assert!(production_codes.is_disjoint(&synthetic_codes));
    assert!(synthetic_codes.is_disjoint(&stub_codes));
    assert!(
        stub_codes.is_empty(),
        "bd-dkpz requires every STORE fixture to be executable, including STORE-034 and STORE-035"
    );
    assert!(
        !synthetic_codes.contains("STORE-014") && !synthetic_codes.contains("STORE-027"),
        "PO-5 and PO-20 must be driver-exercised witnesses"
    );
    assert!(
        synthetic_codes.contains("STORE-034") && synthetic_codes.contains("STORE-035"),
        "STORE-034/035 remain explicit synthetic diagnostics per bd-dkpz special cases"
    );
    assert_eq!(
        production_codes
            .union(&stub_codes)
            .chain(synthetic_codes.iter())
            .cloned()
            .collect::<BTreeSet<_>>(),
        (1..=35)
            .map(|number| format!("STORE-{number:03}"))
            .collect::<BTreeSet<_>>()
    );
}

fn fixture_input_identity(
    fixture: &StoragePlanViolationFixture,
) -> gbf_codegen::storage_plan::StoragePlanInputIdentity {
    match &fixture.kind {
        StoragePlanViolationFixtureKind::ProductionBacked(production) => {
            match production.as_ref() {
                StoragePlanProductionFixture::CoreDriver { core_input }
                | StoragePlanProductionFixture::SelfConsistency { core_input, .. }
                | StoragePlanProductionFixture::SyntheticDiagnostic { core_input, .. } => {
                    core_input.input_identity.clone()
                }
            }
        }
        StoragePlanViolationFixtureKind::StubOnly { .. } => {
            panic!("stub fixture has no production input identity")
        }
    }
}

fn assert_store_witness(code: StoragePlanDiagnosticCode) {
    let fixture = fixture_for_code(code);
    let output = fixture.run().expect("STORE witness runs");

    assert_eq!(
        output.outcome,
        StoragePlanCoreOutcome::Failed,
        "{} must reject",
        code.as_str()
    );
    assert_eq!(
        output.diagnostics,
        vec![code],
        "{} must emit exactly its named diagnostic",
        code.as_str()
    );
    assert!(
        output.result.is_none() && output.summary.is_none(),
        "{} must not emit partial result or summary",
        code.as_str()
    );
    let report = emit_storage_plan_report(&output).expect("STORE witness report emits");
    let diagnostic = report
        .body
        .body
        .diagnostics
        .first()
        .expect("report carries diagnostic");
    let encoded = serde_json::to_value(&diagnostic.code).expect("diagnostic serializes");
    let encoded = encoded.to_string();
    assert!(
        encoded.contains(code.name()) && encoded.contains("provenance"),
        "{} must carry typed diagnostic provenance, got {encoded}",
        code.as_str()
    );
}

fn fixture_for_code(code: StoragePlanDiagnosticCode) -> StoragePlanViolationFixture {
    sc_violations::all_store_violation_factories()
        .into_iter()
        .find(|fixture| fixture.expected_code == code)
        .unwrap_or_else(|| panic!("missing fixture for {}", code.as_str()))
}
