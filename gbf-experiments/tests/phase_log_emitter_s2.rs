mod common;

use common::tracing_capture::{TraceCapture, captured_events, with_trace_capture};
use gbf_experiments::s2::run::hardness::hardness_for_global_step;
use gbf_experiments::s2::run::scheduler::{PhasePlan, phase_for_global_step};
use gbf_experiments::s2::schema::{
    GlobalStep, HardnessTriple, PhaseEffectiveLambda, PhaseEffectiveLambdaValues, PhaseEntry,
    PhaseEvent, PhaseLog, QuantHardnessOverride, RouterTrainMode, S2BuildKind, TrainConfigS2Full,
    quant_hardness_override_for_build_kind, write_phase_log_artifacts,
};
use gbf_foundation::Hash256;

#[test]
fn phase_log_full_ternary_validates_pl0_through_pl8_and_writes_artifacts() {
    let entries = phase_entries(S2BuildKind::s2_ternary_full);
    let header = PhaseLog::new(
        0,
        S2BuildKind::s2_ternary_full,
        hash(1),
        vec![4000, 5000, 8000, 10000],
        &entries,
    )
    .expect("phase log");
    let temp = tempfile::tempdir().expect("tempdir");
    let header_path = temp.path().join("phase-log.json");
    let jsonl_path = temp.path().join("phase-log.jsonl");
    let capture = TraceCapture::default();

    with_trace_capture(&capture, || {
        write_phase_log_artifacts(&header_path, &jsonl_path, &header, &entries)
            .expect("phase log write");
    });

    assert_eq!(entries.len(), 10_000);
    assert_eq!(
        header.phase_log_self_hash,
        header.computed_self_hash(&entries).expect("phase hash")
    );
    assert_eq!(
        std::fs::read(&jsonl_path).expect("jsonl"),
        PhaseLog::canonical_jsonl_bytes(&entries).expect("jsonl bytes")
    );
    assert_eq!(
        std::fs::read(&header_path).expect("header"),
        gbf_experiments::s1::schema::S1CanonicalJson::to_vec(&header).expect("header json")
    );
    let events = captured_events(&capture);
    assert_eq!(
        events
            .iter()
            .filter(|event| event.name == "phase_log_finalized")
            .count(),
        1
    );
    let flush_last_steps = events
        .iter()
        .filter(|event| event.name == "phase_log_flush")
        .map(|event| event.fields.get("last_step").cloned())
        .collect::<Vec<_>>();
    assert_eq!(
        flush_last_steps,
        vec![
            Some(serde_json::json!(4_000)),
            Some(serde_json::json!(5_000)),
            Some(serde_json::json!(8_000)),
            Some(serde_json::json!(10_000)),
        ]
    );
    assert!(events.iter().any(|event| event.name == "phase_log_flush"
        && event.fields.get("last_step") == Some(&serde_json::json!(10_000))));
    insta::assert_snapshot!(
        "phase_log_s2__header_ternary_seed0",
        String::from_utf8(gbf_experiments::s1::schema::S1CanonicalJson::to_vec(&header).unwrap())
            .unwrap()
    );
    insta::assert_snapshot!(
        "phase_log_s2__jsonl_first_3_steps",
        first_jsonl_lines(&entries, 3)
    );
}

#[test]
fn phase_log_pl0_hash_changes_when_entry_content_is_tampered() {
    let entries = phase_entries(S2BuildKind::s2_ternary_full);
    let header = PhaseLog::new(
        0,
        S2BuildKind::s2_ternary_full,
        hash(1),
        vec![4000, 5000, 8000, 10000],
        &entries,
    )
    .expect("phase log");
    let original_hash = header
        .computed_self_hash(&entries)
        .expect("original phase hash");
    let original_jsonl = PhaseLog::canonical_jsonl_bytes(&entries).expect("original jsonl");

    let mut tampered = entries;
    tampered[41].train_loss = 1.25;
    let tampered_hash = header
        .computed_self_hash(&tampered)
        .expect("tampered phase hash");
    let tampered_jsonl = PhaseLog::canonical_jsonl_bytes(&tampered).expect("tampered jsonl");

    assert_ne!(original_jsonl, tampered_jsonl);
    assert_ne!(original_hash, tampered_hash);
    assert_ne!(header.phase_log_self_hash, tampered_hash);
}

#[test]
fn phase_log_rejects_tampered_entry_count_and_non_finite_train_loss() {
    let entries = phase_entries(S2BuildKind::s2_ternary_full);
    let header = PhaseLog::new(
        0,
        S2BuildKind::s2_ternary_full,
        hash(1),
        vec![4000, 5000, 8000, 10000],
        &entries,
    )
    .expect("phase log");

    assert!(header.validate(&entries[..9_999]).is_err());

    let mut tampered = entries;
    tampered[99].train_loss = f32::NAN;
    assert!(header.validate(&tampered).is_err());
}

#[test]
fn phase_log_ablation_has_no_transitions_or_teacher_freeze() {
    let entries = phase_entries(S2BuildKind::s2_ablation);
    let header = PhaseLog::new(0, S2BuildKind::s2_ablation, hash(2), vec![4000], &entries)
        .expect("ablation phase log");

    assert_eq!(entries.len(), 4_000);
    assert!(entries.iter().all(|entry| entry.events.is_empty()));
    assert!(
        entries
            .iter()
            .all(|entry| entry.hardness == HardnessTriple::all_off())
    );
    header
        .validate(&entries)
        .expect("ablation phase log validates");
}

fn phase_entries(build_kind: S2BuildKind) -> Vec<PhaseEntry> {
    let cfg = TrainConfigS2Full::pinned();
    let plan = if build_kind == S2BuildKind::s2_ablation {
        PhasePlan::phase_a_only()
    } else {
        PhasePlan::full_s2()
    };
    let steps = if build_kind == S2BuildKind::s2_ablation {
        4_000
    } else {
        10_000
    };
    (1..=steps)
        .map(|step| phase_entry(step, build_kind, &cfg, &plan))
        .collect()
}

fn phase_entry(
    step: GlobalStep,
    build_kind: S2BuildKind,
    cfg: &TrainConfigS2Full,
    plan: &PhasePlan,
) -> PhaseEntry {
    let phase = phase_for_global_step(step, plan).expect("phase");
    let hardness = hardness_for_global_step(
        step,
        plan,
        quant_hardness_override_for_build_kind(build_kind),
    )
    .expect("hardness");
    PhaseEntry {
        step,
        phase,
        hardness,
        router_mode: RouterTrainMode::NoRouter,
        lambda_effective: phase_lambdas(step, build_kind, hardness, cfg),
        teacher_frozen: build_kind != S2BuildKind::s2_ablation && step > 4_000,
        train_loss: 1.0,
        grad_norm: 0.5,
        distill_loss: (build_kind != S2BuildKind::s2_ablation && step > 5_000).then_some(0.125),
        events: phase_events(step, build_kind),
    }
}

fn phase_lambdas(
    step: GlobalStep,
    build_kind: S2BuildKind,
    hardness: HardnessTriple,
    cfg: &TrainConfigS2Full,
) -> PhaseEffectiveLambda {
    PhaseEffectiveLambda::new(PhaseEffectiveLambdaValues {
        lambda_distill: if matches!(
            build_kind,
            S2BuildKind::s2_ternary_full | S2BuildKind::s2_fp_full
        ) && step > 5_000
        {
            1.0
        } else {
            0.0
        },
        lambda_balance: 0.0,
        lambda_zrouter: 0.0,
        lambda_switch: 0.0,
        lambda_range: if matches!(
            build_kind,
            S2BuildKind::s2_ternary_full | S2BuildKind::s2_ternary_nodistill
        ) && (hardness.activation_qat
            != QuantHardnessOverride::AllOff.apply(hardness).activation_qat
            || hardness.norm_qat != QuantHardnessOverride::AllOff.apply(hardness).norm_qat)
        {
            cfg.lambda_range
        } else {
            0.0
        },
        lambda_zero: if matches!(
            build_kind,
            S2BuildKind::s2_ternary_full | S2BuildKind::s2_ternary_nodistill
        ) && step > 5_000
        {
            cfg.lambda_zero
        } else {
            0.0
        },
        lambda_shape: 0.0,
        lambda_overflow: 0.0,
    })
    .expect("lambdas")
}

fn phase_events(step: GlobalStep, build_kind: S2BuildKind) -> Vec<PhaseEvent> {
    if build_kind == S2BuildKind::s2_ablation {
        return Vec::new();
    }
    match step {
        4_001 => vec![
            PhaseEvent::PhaseTransition {
                from: gbf_experiments::s2::schema::PhaseKindS2::PhaseA,
                to: gbf_experiments::s2::schema::PhaseKindS2::PhaseB,
            },
            PhaseEvent::TeacherFreeze {
                teacher_checkpoint_sha: hash(9),
            },
        ],
        5_001 => vec![PhaseEvent::PhaseTransition {
            from: gbf_experiments::s2::schema::PhaseKindS2::PhaseB,
            to: gbf_experiments::s2::schema::PhaseKindS2::PhaseC,
        }],
        8_001 => vec![PhaseEvent::PhaseTransition {
            from: gbf_experiments::s2::schema::PhaseKindS2::PhaseC,
            to: gbf_experiments::s2::schema::PhaseKindS2::PhaseD,
        }],
        _ => Vec::new(),
    }
}

fn first_jsonl_lines(entries: &[PhaseEntry], count: usize) -> String {
    entries
        .iter()
        .take(count)
        .map(|entry| {
            String::from_utf8(gbf_experiments::s1::schema::S1CanonicalJson::to_vec(entry).unwrap())
                .unwrap()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn hash(fill: u8) -> Hash256 {
    Hash256::from_bytes([fill; 32])
}
