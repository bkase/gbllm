use std::collections::BTreeSet;
use std::path::Path;

use gbf_codegen::s1::quant_graph::DeterminismClass;
use gbf_codegen::s3::infer_ir::{NodeId, ValueId};
use gbf_codegen::sram_page_plan::{
    ColdSpillResidency, PageId, PageResidency, PersistManifestResidency, PersistentPageGeometry,
    SRAM_PAGE_PLAN_SCHEMA_VERSION, SequenceStreamId, SramEpochId, SramPagePlanBindingInput,
    SramPagePlanCacheKeyInputs, SramPagePlanEpochInput, SramPagePlanInputHashes,
    SramPagePlanInputIdentity, SramPagePlanInputs, SramPagePlanOutcome, SramResidencyRole,
    SramSwitchCaps, build_sram_page_plan, emit_sram_cert_json_bytes, emit_sram_cert_report,
    emit_sram_page_plan_json_bytes, emit_sram_page_plan_report, is_yield_safe_at,
    parse_sram_cert_report_bytes, parse_sram_page_plan_report_bytes,
    validate_sram_page_plan_json_surface,
};
use gbf_codegen::storage_plan::types::{
    AbstractLiveRange, AliasClassId, BindingJustification, CommitGroupId, DecisionRuleId,
    LifetimeClass, Materialization, PersistPageId, StorageBinding,
};
use gbf_codegen::window::NodeAnchorRange;
use gbf_foundation::{
    BudgetSlotId, CompileProfileId, Hash256, TargetProfileId, canonical_json_bytes_omitting_fields,
};
use gbf_policy::{
    BudgetSlotClass, PlacementProfile, RomBudgetSlot, RuntimeChromeBudget, RuntimeMemoryCapSection,
    SramKnob, SramPageAggression, SramPagePlanDiagnosticCode, SramSpillPolicy, ValidationCode,
};
use gbf_report::{canonicalize, round_trip_self_hash};
use serde_json::json;

#[test]
fn sram_page_plan_pass_fixtures_cover_single_multi_and_yield_resume() {
    assert_fixture("pass", "single_stream_basic");
    let single = build_sram_page_plan(&fixture_inputs(vec![
        binding(2, 0, 1, 512, PageResidency::AnyPageInBudget, false),
        binding(1, 0, 1, 256, PageResidency::SamePageAsLastMember, true),
    ]));
    assert_eq!(single.outcome, SramPagePlanOutcome::Succeeded);
    let single_plan = single.result.as_ref().expect("single-stream plan");
    assert_eq!(single_plan.budgets.page_count, 1);
    assert_eq!(single_plan.budgets.stream_count, 1);
    assert_eq!(single_plan.bindings[0].page, single_plan.bindings[1].page);
    assert_eq!(single_plan.active_sets.len(), 1);
    assert_eq!(single_plan.active_sets[0].bytes_in_use, 768);
    assert_eq!(
        single_plan.active_sets[0].commit_boundaries_in_range.len(),
        1
    );
    assert_eq!(single_plan.commit_boundaries.len(), 1);
    assert_eq!(single_plan.page_rotations.len(), 1);
    assert_eq!(
        single_plan
            .projections
            .projected_sram_page_switches_per_token,
        1
    );

    assert_fixture("pass", "multi_stream");
    let multi = build_sram_page_plan(&fixture_inputs(vec![
        binding(1, 0, 1, 128, PageResidency::AnyPageInBudget, false),
        binding(2, 1, 2, 128, PageResidency::AnyPageInBudget, false),
    ]));
    assert_eq!(multi.outcome, SramPagePlanOutcome::Succeeded);
    let multi_plan = multi.result.as_ref().expect("multi-stream plan");
    assert_eq!(multi_plan.budgets.stream_count, 2);
    assert_ne!(multi_plan.bindings[0].page, multi_plan.bindings[1].page);
    assert_eq!(multi_plan.active_sets.len(), 2);
    assert_eq!(multi_plan.commit_boundaries.len(), 2);
    assert_eq!(multi_plan.page_rotations.len(), 2);
    assert_eq!(
        multi_plan
            .projections
            .projected_sram_page_switches_per_token,
        2
    );

    assert_fixture("pass", "yield_resume");
    let yield_resume = build_sram_page_plan(&fixture_inputs(vec![binding(
        1,
        0,
        1,
        128,
        PageResidency::SamePageAsLastMember,
        true,
    )]));
    assert_eq!(yield_resume.outcome, SramPagePlanOutcome::Succeeded);

    assert_fixture("pass", "spill_eager_manifest_residency");
    let mut spill_eager = fixture_inputs(vec![binding(
        1,
        7,
        1,
        128,
        PageResidency::AnyPageInBudget,
        false,
    )]);
    spill_eager.policy.spill_policy = SramSpillPolicy::SpillEager;
    spill_eager.runtime_chrome_budget.sram_reserved = 2 * page_size_bytes(spill_eager.geometry);
    let spill_eager = build_sram_page_plan(&spill_eager);
    assert_eq!(spill_eager.outcome, SramPagePlanOutcome::Succeeded);
    let spill_plan = spill_eager.result.as_ref().expect("spill-eager plan");
    assert_eq!(
        spill_plan.spill_policy.persist_manifest_residency,
        PersistManifestResidency::DedicatedManifestPage
    );
    assert_eq!(
        spill_plan.spill_policy.cold_spill_residency,
        ColdSpillResidency::BoundedColdSpill { max_pages: 2 }
    );
    let boundary = &spill_plan.commit_boundaries[0];
    assert!(!boundary.member_pages.contains(&boundary.manifest_page));
    assert_eq!(
        spill_plan
            .bindings
            .iter()
            .filter(|binding| binding.residency_role == SramResidencyRole::SramPagedSpill)
            .count(),
        2
    );

    assert_fixture("pass", "sram_cert_claims");
    let report = emit_sram_page_plan_report(&spill_eager).expect("spill report");
    let cert = emit_sram_cert_report(&spill_eager, report.report_self_hash)
        .expect("cert emits")
        .expect("success emits cert");
    assert_eq!(
        cert.claim.sram_plan_self_hash,
        spill_plan.sram_page_plan_self_hash
    );
    assert!(cert.claim.single_page_invariant_holds);
    assert!(cert.claim.all_persists_resolved);
    assert!(cert.claim.page_switches_per_token_within_cap);
    assert_eq!(cert.claim.page_switches_per_token, 2);
    assert_eq!(cert.claim.page_switches_cap, 4);
    assert_eq!(cert.evidence.page_rotation_count, 2);
}

#[test]
fn sram_page_plan_fixtures_cover_roles_serialization_and_yield_predicate() {
    assert_fixture("pass", "residency_roles");
    let mut continuation = binding(5, 5, 1, 32, PageResidency::AnyPageInBudget, false);
    set_persist_page_and_rule(&mut continuation, 0x2000_0005, 5);
    let mut transcript = binding(6, 6, 1, 32, PageResidency::AnyPageInBudget, false);
    set_persist_page_and_rule(&mut transcript, 0x3000_0006, 6);
    let mut harness = binding(7, 7, 1, 32, PageResidency::AnyPageInBudget, false);
    set_persist_page_and_rule(&mut harness, 0x4000_0007, 7);
    let mut trace = binding(8, 8, 1, 32, PageResidency::AnyPageInBudget, false);
    set_persist_page_and_rule(&mut trace, 0x5000_0008, 7);
    let output = build_sram_page_plan(&fixture_inputs(vec![
        continuation,
        transcript,
        harness,
        trace,
    ]));
    assert_eq!(output.outcome, SramPagePlanOutcome::Succeeded);
    let report = emit_sram_page_plan_report(&output).expect("report emits");
    let cert = emit_sram_cert_report(&output, report.report_self_hash)
        .expect("cert emits")
        .expect("success emits cert");
    assert_eq!(cert.evidence.persistent_kind_distribution.continuation, 1);
    assert_eq!(cert.evidence.persistent_kind_distribution.transcript, 1);
    assert_eq!(cert.evidence.persistent_kind_distribution.harness, 1);
    assert_eq!(cert.evidence.persistent_kind_distribution.trace, 1);

    assert_fixture("pass", "canonical_serialization_order");
    let mut order_inputs = fixture_inputs(vec![
        binding(
            2,
            70,
            1,
            32,
            PageResidency::FixedPage { page: PageId(1) },
            false,
        ),
        binding(
            1,
            70,
            1,
            32,
            PageResidency::FixedPage { page: PageId(2) },
            false,
        ),
    ]);
    order_inputs.epochs = vec![
        SramPagePlanEpochInput {
            epoch: SramEpochId(0),
            op_range: NodeAnchorRange {
                first_node: NodeId::new(1),
                last_node: NodeId::new(2),
            },
        },
        SramPagePlanEpochInput {
            epoch: SramEpochId(1),
            op_range: NodeAnchorRange {
                first_node: NodeId::new(2),
                last_node: NodeId::new(3),
            },
        },
    ];
    let output = build_sram_page_plan(&order_inputs);
    assert_eq!(output.outcome, SramPagePlanOutcome::Succeeded);
    let boundary = &output.result.as_ref().expect("plan").commit_boundaries[0];
    assert_eq!(
        boundary.serialization_order,
        vec![ValueId::new(1), ValueId::new(2)]
    );
    assert!(is_yield_safe_at(boundary, boundary.after_epoch));
}

#[test]
fn sram_page_plan_reject_fixtures_cover_allocator_and_diagnostics() {
    assert_fixture("reject", "target_profile_layout_unsupported");
    let mut target_layout = fixture_inputs(vec![]);
    target_layout.geometry.payload_bytes = 0;
    target_layout.expected_geometry.payload_bytes = 0;
    assert_has_code(
        &build_sram_page_plan(&target_layout),
        SramPagePlanDiagnosticCode::SramTargetProfileLayoutUnsupported,
    );

    assert_fixture("reject", "policy_projection_mismatch");
    let policy_mismatch = fixture_inputs(vec![binding(
        1,
        0,
        1,
        0,
        PageResidency::AnyPageInBudget,
        false,
    )]);
    assert_has_code(
        &build_sram_page_plan(&policy_mismatch),
        SramPagePlanDiagnosticCode::SramPolicyProjectionMismatch,
    );

    assert_fixture("reject", "cross_stream_page_sharing");
    let cross_stream = fixture_inputs(vec![
        binding(
            1,
            0,
            1,
            128,
            PageResidency::FixedPage { page: PageId(7) },
            false,
        ),
        binding(
            2,
            1,
            2,
            128,
            PageResidency::FixedPage { page: PageId(7) },
            false,
        ),
    ]);
    assert_has_code(
        &build_sram_page_plan(&cross_stream),
        SramPagePlanDiagnosticCode::SramCrossStreamPageSharing,
    );

    assert_fixture("reject", "page_id_exhaustion");
    let page_exhaustion = fixture_inputs(
        (0..=256)
            .map(|index| {
                binding(
                    index + 1,
                    index,
                    1,
                    1,
                    PageResidency::AnyPageInBudget,
                    false,
                )
            })
            .collect(),
    );
    assert_has_code(
        &build_sram_page_plan(&page_exhaustion),
        SramPagePlanDiagnosticCode::SramTargetProfileLayoutUnsupported,
    );

    assert_fixture("reject", "distinct_page_exhaustion");
    let mut distinct_inputs = (0..=255)
        .map(|page| {
            binding(
                page + 1,
                1,
                1,
                1,
                PageResidency::FixedPage {
                    page: PageId(page as u8),
                },
                false,
            )
        })
        .collect::<Vec<_>>();
    distinct_inputs.push(binding(
        1000,
        2,
        1,
        1,
        PageResidency::DistinctFromCommitGroup {
            commit_group: CommitGroupId(1),
        },
        false,
    ));
    assert_has_code(
        &build_sram_page_plan(&fixture_inputs(distinct_inputs)),
        SramPagePlanDiagnosticCode::SramTargetProfileLayoutUnsupported,
    );

    assert_fixture("reject", "multi_page_epoch");
    let mut multi_page_epoch = fixture_inputs(vec![
        binding(
            1,
            10,
            1,
            128,
            PageResidency::FixedPage { page: PageId(1) },
            false,
        ),
        binding(
            2,
            11,
            1,
            128,
            PageResidency::FixedPage { page: PageId(2) },
            false,
        ),
    ]);
    multi_page_epoch.epochs = vec![SramPagePlanEpochInput {
        epoch: SramEpochId(0),
        op_range: NodeAnchorRange {
            first_node: NodeId::new(1),
            last_node: NodeId::new(3),
        },
    }];
    assert_has_code(
        &build_sram_page_plan(&multi_page_epoch),
        SramPagePlanDiagnosticCode::SramPolicyProjectionMismatch,
    );

    assert_fixture("reject", "empty_working_set_epoch");
    let mut empty_epoch = fixture_inputs(vec![binding(
        1,
        12,
        1,
        128,
        PageResidency::AnyPageInBudget,
        false,
    )]);
    empty_epoch.epochs = vec![SramPagePlanEpochInput {
        epoch: SramEpochId(0),
        op_range: NodeAnchorRange {
            first_node: NodeId::new(10),
            last_node: NodeId::new(11),
        },
    }];
    assert_has_code(
        &build_sram_page_plan(&empty_epoch),
        SramPagePlanDiagnosticCode::SramPolicyProjectionMismatch,
    );

    assert_fixture("reject", "spill_policy_manifest_conflict");
    let mut spill_conflict = fixture_inputs(vec![binding(
        1,
        12,
        1,
        128,
        PageResidency::AnyPageInBudget,
        false,
    )]);
    spill_conflict.policy.spill_policy = SramSpillPolicy::SpillEager;
    spill_conflict.runtime_chrome_budget.sram_reserved = page_size_bytes(spill_conflict.geometry);
    assert_has_code(
        &build_sram_page_plan(&spill_conflict),
        SramPagePlanDiagnosticCode::SramPolicyProjectionMismatch,
    );

    assert_fixture("reject", "page_switch_cap_exceeded");
    let mut cap_failure = fixture_inputs(vec![
        binding(
            1,
            20,
            1,
            128,
            PageResidency::FixedPage { page: PageId(1) },
            false,
        ),
        binding(
            2,
            21,
            1,
            128,
            PageResidency::FixedPage { page: PageId(2) },
            false,
        ),
    ]);
    cap_failure.switch_caps.max_sram_page_switches_per_token = 1;
    assert_has_code(
        &build_sram_page_plan(&cap_failure),
        SramPagePlanDiagnosticCode::SramPolicyProjectionMismatch,
    );
}

#[test]
fn sram_page_plan_forbidden_surface_fixture_covers_closed_codes() {
    assert_fixture("reject", "forbidden_surface");
    assert_json_surface_code(
        json!({ "product": { "tag": "SectionRole" } }),
        SramPagePlanDiagnosticCode::SramSectionRoleLeaked,
    );
    assert_json_surface_code(
        json!({ "lease": "SliceId" }),
        SramPagePlanDiagnosticCode::SramSchedulingFieldLeaked,
    );
    assert_json_surface_code(
        json!({ "repair_proposals": [] }),
        SramPagePlanDiagnosticCode::SramRepairProvenanceForbidden,
    );
    assert_json_surface_code(
        json!({ "residency": "AnyPageInBudget" }),
        SramPagePlanDiagnosticCode::SramResidencyUnresolved,
    );
}

#[test]
fn sram_page_plan_report_fixture_round_trips_and_is_byte_deterministic() {
    assert_fixture("pass", "report_round_trip");
    let output = build_sram_page_plan(&fixture_inputs(vec![
        binding(3, 2, 2, 64, PageResidency::AnyPageInBudget, false),
        binding(1, 0, 1, 64, PageResidency::AnyPageInBudget, false),
        binding(2, 0, 1, 64, PageResidency::SamePageAsLastMember, true),
    ]));
    assert_eq!(output.outcome, SramPagePlanOutcome::Succeeded);

    let first = emit_sram_page_plan_json_bytes(&output).expect("first report");
    let second = emit_sram_page_plan_json_bytes(&output).expect("second report");
    assert_eq!(first, second);

    let parsed = parse_sram_page_plan_report_bytes(&first).expect("parsed report");
    round_trip_self_hash(&parsed).expect("report self-hash round trip");
    assert_eq!(canonicalize(&parsed).expect("canonical report"), first);
    assert_eq!(
        parsed
            .body
            .result
            .as_ref()
            .expect("product")
            .sram_page_plan_self_hash,
        output
            .result
            .as_ref()
            .expect("source product")
            .sram_page_plan_self_hash
    );
}

#[test]
fn sram_cert_fixture_emits_only_for_success_and_is_byte_deterministic() {
    let output = build_sram_page_plan(&fixture_inputs(vec![
        binding(
            1,
            30,
            1,
            64,
            PageResidency::FixedPage { page: PageId(1) },
            false,
        ),
        binding(
            2,
            31,
            1,
            64,
            PageResidency::FixedPage { page: PageId(2) },
            false,
        ),
    ]));
    assert_eq!(output.outcome, SramPagePlanOutcome::Succeeded);
    let report = emit_sram_page_plan_report(&output).expect("report emits");
    let first = emit_sram_cert_json_bytes(&output, report.report_self_hash)
        .expect("first cert")
        .expect("success cert");
    let second = emit_sram_cert_json_bytes(&output, report.report_self_hash)
        .expect("second cert")
        .expect("success cert again");
    assert_eq!(first, second);

    let parsed = parse_sram_cert_report_bytes(&first).expect("cert parses");
    assert_eq!(
        canonical_json_bytes_omitting_fields(&parsed, &[]).expect("canonical cert"),
        first
    );
    assert_eq!(parsed.claim.page_switches_per_token, 2);
    assert_eq!(parsed.claim.page_switches_cap, 4);
    assert!(parsed.claim.page_switches_per_token_within_cap);

    let mut cap_failure = fixture_inputs(vec![
        binding(
            1,
            40,
            1,
            64,
            PageResidency::FixedPage { page: PageId(1) },
            false,
        ),
        binding(
            2,
            41,
            1,
            64,
            PageResidency::FixedPage { page: PageId(2) },
            false,
        ),
    ]);
    cap_failure.switch_caps.max_sram_page_switches_per_token = 1;
    let failed = build_sram_page_plan(&cap_failure);
    assert_eq!(failed.outcome, SramPagePlanOutcome::Failed);
    let failed_report = emit_sram_page_plan_report(&failed).expect("failed report emits");
    assert!(
        emit_sram_cert_json_bytes(&failed, failed_report.report_self_hash)
            .expect("failed cert query")
            .is_none()
    );
}

#[test]
fn sram_page_plan_k7_fixture_is_sensitive_to_inputs_features_and_pass_version() {
    assert_fixture("pass", "k7_sensitivity");
    let identity = fixture_inputs(vec![]).input_identity;
    let base = SramPagePlanCacheKeyInputs::from_input_identity(&identity, hash(42));
    let base_key = base.cache_key().expect("base key");

    let mut changed = base.clone();
    changed.storage_plan_self_hash = hash(50);
    assert_ne!(base_key, changed.cache_key().expect("storage"));

    let mut changed = base.clone();
    changed.observation_plan_self_hash = hash(51);
    assert_ne!(base_key, changed.cache_key().expect("observation"));

    let mut changed = base.clone();
    changed.range_plan_self_hash = hash(52);
    assert_ne!(base_key, changed.cache_key().expect("range"));

    let mut changed = base.clone();
    changed.runtime_chrome_budget_hash = hash(53);
    assert_ne!(base_key, changed.cache_key().expect("runtime budget"));

    let mut changed = base.clone();
    changed.target_profile_hash = hash(54);
    assert_ne!(base_key, changed.cache_key().expect("target profile"));

    let mut changed = base.clone();
    changed.sram_page_plan_policy_projection_hash = hash(55);
    assert_ne!(base_key, changed.cache_key().expect("policy projection"));

    let mut changed = base.clone();
    changed.crate_feature_set_hash = hash(56);
    assert_ne!(base_key, changed.cache_key().expect("feature set"));

    let mut changed = base;
    changed.pass_version = "stage7/v2".to_owned();
    assert_ne!(base_key, changed.cache_key().expect("pass version"));
}

fn assert_json_surface_code(value: serde_json::Value, expected: SramPagePlanDiagnosticCode) {
    let diagnostic = validate_sram_page_plan_json_surface(&value).expect("diagnostic");
    assert!(
        matches!(diagnostic.code, ValidationCode::SramPagePlan { code, .. } if code == expected),
        "missing {expected:?} in {diagnostic:?}"
    );
}

fn assert_has_code(
    output: &gbf_codegen::sram_page_plan::SramPagePlanOutput,
    expected: SramPagePlanDiagnosticCode,
) {
    assert_eq!(output.outcome, SramPagePlanOutcome::Failed);
    assert!(
        output.diagnostics.iter().any(|diagnostic| matches!(
            diagnostic.code,
            ValidationCode::SramPagePlan { code, .. } if code == expected
        )),
        "missing {expected:?} in {:?}",
        output.diagnostics
    );
}

fn assert_fixture(kind: &str, name: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("sram_page_plan")
        .join(kind)
        .join(name)
        .join("README.md");
    assert!(
        path.exists(),
        "missing fixture marker at {}",
        path.display()
    );
}

fn fixture_inputs(bindings: Vec<SramPagePlanBindingInput>) -> SramPagePlanInputs {
    let hashes = SramPagePlanInputHashes {
        storage_plan_self_hash: hash(1),
        observation_plan_self_hash: hash(2),
        range_plan_self_hash: hash(3),
        runtime_chrome_budget_hash: hash(4),
        target_profile_hash: hash(5),
        sram_page_plan_policy_projection_hash: hash(6),
    };
    SramPagePlanInputs {
        input_identity: SramPagePlanInputIdentity {
            storage_plan_self_hash: hashes.storage_plan_self_hash,
            observation_plan_self_hash: hashes.observation_plan_self_hash,
            range_plan_self_hash: hashes.range_plan_self_hash,
            runtime_chrome_budget_hash: hashes.runtime_chrome_budget_hash,
            target_profile_hash: hashes.target_profile_hash,
            sram_page_plan_policy_projection_hash: hashes.sram_page_plan_policy_projection_hash,
            determinism: DeterminismClass::Deterministic,
            schema_version: SRAM_PAGE_PLAN_SCHEMA_VERSION,
        },
        expected_input_hashes: hashes,
        runtime_chrome_budget: RuntimeChromeBudget {
            target: TargetProfileId::from("dmg-mbc5"),
            profile: CompileProfileId::from("Bringup"),
            runtime_nucleus_hash: hash(7),
            rom_slots: vec![RomBudgetSlot {
                id: BudgetSlotId::new(0),
                class: BudgetSlotClass::CommonBank,
                usable_bytes: 16 * 1024,
                reserved_slack: 0,
                placement_caps: BTreeSet::from([PlacementProfile::Budgeted]),
            }],
            memory_caps: RuntimeMemoryCapSection {
                wram_usable_bytes: 8 * 1024,
                sram_usable_bytes: 32 * 1024,
                hram_usable_bytes: 127,
                source_target_profile_hash: hash(8),
            },
            wram_reserved: 0,
            sram_reserved: 512,
        },
        policy: SramKnob {
            page_aggression: SramPageAggression::Preserve,
            spill_policy: SramSpillPolicy::NoSpill,
        },
        switch_caps: SramSwitchCaps {
            max_sram_page_switches_per_token: 4,
        },
        geometry: PersistentPageGeometry::dmg_mbc5_8k(),
        expected_geometry: PersistentPageGeometry::dmg_mbc5_8k(),
        epochs: Vec::new(),
        bindings,
    }
}

fn binding(
    value_id: u32,
    commit_group: u32,
    stream: u32,
    payload_bytes: u32,
    residency: PageResidency,
    yield_resume: bool,
) -> SramPagePlanBindingInput {
    let value = ValueId::new(value_id);
    SramPagePlanBindingInput {
        binding: StorageBinding {
            value,
            materialization: Materialization::Persist {
                page: PersistPageId(commit_group),
                commit_group: CommitGroupId(commit_group),
            },
            alias_class: AliasClassId(value_id),
            live_range: AbstractLiveRange {
                def_node: NodeId::new(value_id),
                first_use_node: Some(NodeId::new(value_id)),
                last_use_node: Some(NodeId::new(value_id + 1)),
                lifetime_class: LifetimeClass::Persistent,
                checkpoint_stable: true,
            },
            justification: BindingJustification::DecisionRule(DecisionRuleId(1)),
        },
        payload_bytes,
        sequence_stream: SequenceStreamId(stream),
        residency,
        yield_resume,
    }
}

fn set_persist_page_and_rule(binding: &mut SramPagePlanBindingInput, page: u32, rule: u32) {
    let Materialization::Persist {
        page: persist_page, ..
    } = &mut binding.binding.materialization
    else {
        panic!("expected persist binding");
    };
    *persist_page = PersistPageId(page);
    binding.binding.justification = BindingJustification::DecisionRule(DecisionRuleId(rule));
}

fn hash(byte: u8) -> Hash256 {
    Hash256::from_bytes([byte; 32])
}

fn page_size_bytes(geometry: PersistentPageGeometry) -> u32 {
    geometry
        .payload_bytes
        .saturating_add(u32::from(geometry.header_bytes))
        .saturating_add(u32::from(geometry.commit_word_bytes))
}
