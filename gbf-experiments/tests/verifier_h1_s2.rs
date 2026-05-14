mod common;

use common::tracing_capture::{TraceCapture, captured_events, with_trace_capture};
use gbf_experiments::s2::run::hardness::hardness_for_global_step;
use gbf_experiments::s2::run::lambdas::phase_effective_lambdas;
use gbf_experiments::s2::run::scheduler::{PhasePlan, phase_for_global_step};
use gbf_experiments::s2::schema::{
    GlobalStep, HardnessTriple, HypothesisStatus, PhaseEntry, PhaseEvent, PhaseKindS2, PhaseLog,
    QuantHardness, RouterTrainMode, S2_PHASE_B_END_STEP, S2_TEACHER_FREEZE_STEP, S2BuildKind,
    TrainConfigS2Full, quant_hardness_override_for_build_kind,
};
use gbf_experiments::s2::verifiers::{DiagnosticHit, verify_h1};
use gbf_foundation::Hash256;
use proptest::prelude::*;
use serde_json::Value;

type H1MutationCase = (&'static str, u64, fn(&mut [PhaseEntry]));

#[test]
fn h1_clean_full_phase_log_confirms() {
    let (phase_log, entries) = fixture_phase_log(S2BuildKind::s2_ternary_full);

    let verdict = verify_h1(&phase_log, &entries, S2BuildKind::s2_ternary_full);

    assert_eq!(verdict.status, HypothesisStatus::Confirmed);
    assert!(verdict.hits.is_empty());
}

#[test]
fn h1_clean_ablation_phase_log_confirms_phase_a_only_contract() {
    let (phase_log, entries) = fixture_phase_log(S2BuildKind::s2_ablation);

    let verdict = verify_h1(&phase_log, &entries, S2BuildKind::s2_ablation);

    assert_eq!(entries.len(), S2_TEACHER_FREEZE_STEP as usize);
    assert!(
        entries
            .iter()
            .all(|entry| entry.phase == PhaseKindS2::PhaseA)
    );
    assert!(entries.iter().all(|entry| entry.events.is_empty()));
    assert!(entries.iter().all(|entry| !entry.teacher_frozen));
    assert!(
        entries
            .iter()
            .all(|entry| entry.hardness == HardnessTriple::all_off())
    );
    assert!(entries.iter().all(|entry| {
        entry.lambda_effective.lambda_distill == 0.0
            && entry.lambda_effective.lambda_range == 0.0
            && entry.lambda_effective.lambda_zero == 0.0
    }));
    assert_eq!(verdict.status, HypothesisStatus::Confirmed);
    assert!(verdict.hits.is_empty());
}

#[test]
fn h1_targeted_falsifications_report_offending_step() {
    let cases: [H1MutationCase; 7] = [
        ("contiguous_steps", 11, |entries| {
            entries[9].step = 11;
        }),
        ("finite_train_loss", 42, |entries| {
            entries[41].train_loss = f32::NAN;
        }),
        ("finite_grad_norm", 77, |entries| {
            entries[76].grad_norm = f32::INFINITY;
        }),
        ("phase_transition_events", 5_001, |entries| {
            entries[5_000].events.clear();
        }),
        ("phase_schedule", 5_001, |entries| {
            entries[5_000].phase = PhaseKindS2::PhaseD;
        }),
        ("teacher_freeze", S2_TEACHER_FREEZE_STEP + 1, |entries| {
            entries[S2_TEACHER_FREEZE_STEP as usize].teacher_frozen = false;
        }),
        ("d2_hardness_sequence", 7_001, |entries| {
            entries[7_000].hardness = HardnessTriple::all_off();
        }),
    ];

    for (check_name, step, mutate) in cases {
        let (phase_log, mut entries) = fixture_phase_log(S2BuildKind::s2_ternary_full);
        mutate(&mut entries);

        let verdict = verify_h1(&phase_log, &entries, S2BuildKind::s2_ternary_full);

        assert_eq!(verdict.status, HypothesisStatus::Refuted);
        assert_hit(&verdict.hits, check_name, step);
    }
}

#[test]
fn h1_phase_transition_events_refute_extra_transition_at_non_boundary() {
    let (phase_log, mut entries) = fixture_phase_log(S2BuildKind::s2_ternary_full);
    entries[199].events.push(PhaseEvent::PhaseTransition {
        from: PhaseKindS2::PhaseA,
        to: PhaseKindS2::PhaseB,
    });

    let verdict = verify_h1(&phase_log, &entries, S2BuildKind::s2_ternary_full);

    assert_eq!(verdict.status, HypothesisStatus::Refuted);
    assert_hit(&verdict.hits, "phase_transition_events", 200);
}

#[test]
fn h1_phase_transition_events_refute_wrong_boundary_pair_without_teacher_freeze_coupling() {
    let (phase_log, mut entries) = fixture_phase_log(S2BuildKind::s2_ternary_full);
    entries[4_000].events = vec![
        PhaseEvent::PhaseTransition {
            from: PhaseKindS2::PhaseB,
            to: PhaseKindS2::PhaseC,
        },
        PhaseEvent::TeacherFreeze {
            teacher_checkpoint_sha: hash(9),
        },
    ];

    let verdict = verify_h1(&phase_log, &entries, S2BuildKind::s2_ternary_full);

    assert_eq!(verdict.status, HypothesisStatus::Refuted);
    assert_hit(&verdict.hits, "phase_transition_events", 4_001);
    assert!(
        !verdict
            .hits
            .iter()
            .any(|hit| hit.check_name == "teacher_freeze" && hit.step == Some(4_001)),
        "wrong transition pair test should not also trip teacher freeze: {:#?}",
        verdict.hits
    );
}

#[test]
fn h1_header_falsification_reports_header_hit() {
    let (mut phase_log, entries) = fixture_phase_log(S2BuildKind::s2_ternary_full);
    phase_log.optimizer_steps = 9_999;

    let verdict = verify_h1(&phase_log, &entries, S2BuildKind::s2_ternary_full);

    assert_eq!(verdict.status, HypothesisStatus::Refuted);
    assert!(
        verdict
            .hits
            .iter()
            .any(|hit| hit.check_name == "phase_log_header" && hit.step.is_none())
    );
}

#[test]
fn h1_lambda_mismatch_reports_offending_step() {
    let (phase_log, mut entries) = fixture_phase_log(S2BuildKind::s2_ternary_full);
    entries[9_000].lambda_effective.lambda_range = 0.0;

    let verdict = verify_h1(&phase_log, &entries, S2BuildKind::s2_ternary_full);

    assert_eq!(verdict.status, HypothesisStatus::Refuted);
    assert_hit(&verdict.hits, "phase_effective_lambdas", 9_001);
}

#[test]
fn h1_phase_schedule_hit_snapshot_pins_shape() {
    let (phase_log, mut entries) = fixture_phase_log(S2BuildKind::s2_ternary_full);
    entries[5_000].phase = PhaseKindS2::PhaseD;

    let verdict = verify_h1(&phase_log, &entries, S2BuildKind::s2_ternary_full);

    insta::with_settings!({prepend_module_to_snapshot => false}, {
        insta::assert_snapshot!(
            "h1_diagnostic_hit__phase_b_skips_ternary",
            pretty_json(&serde_json::to_value(first_hit(&verdict.hits, "phase_schedule")).unwrap())
        );
    });
}

#[test]
fn h1_grad_norm_surprise_warns_without_refuting() {
    let (phase_log, mut entries) = fixture_phase_log(S2BuildKind::s2_ternary_full);
    entries[199].grad_norm = 1_500.0;
    let capture = TraceCapture::default();

    let verdict = with_trace_capture(&capture, || {
        verify_h1(&phase_log, &entries, S2BuildKind::s2_ternary_full)
    });

    assert_eq!(verdict.status, HypothesisStatus::Confirmed);
    assert!(verdict.hits.is_empty());
    assert!(captured_events(&capture).iter().any(|event| {
        event.name == "h1_surprise"
            && event.fields.get("check").and_then(Value::as_str) == Some("grad_norm_spike")
            && event.fields.get("step").and_then(Value::as_u64) == Some(200)
    }));
}

#[test]
fn h1_mean_loss_surprise_warns_without_refuting() {
    let (phase_log, mut entries) = fixture_phase_log(S2BuildKind::s2_ternary_full);
    for entry in entries.iter_mut().take(10) {
        entry.train_loss = 7.0;
    }
    let capture = TraceCapture::default();

    let verdict = with_trace_capture(&capture, || {
        verify_h1(&phase_log, &entries, S2BuildKind::s2_ternary_full)
    });

    assert_eq!(verdict.status, HypothesisStatus::Confirmed);
    assert!(captured_events(&capture).iter().any(|event| {
        event.name == "h1_surprise"
            && event.fields.get("check").and_then(Value::as_str) == Some("mean_train_loss")
    }));
}

#[test]
fn h1_surprise_events_are_emitted_before_diagnostic_hits() {
    let (phase_log, mut entries) = fixture_phase_log(S2BuildKind::s2_ternary_full);
    entries[199].grad_norm = 1_500.0;
    entries[5_000].phase = PhaseKindS2::PhaseD;
    let capture = TraceCapture::default();

    let verdict = with_trace_capture(&capture, || {
        verify_h1(&phase_log, &entries, S2BuildKind::s2_ternary_full)
    });

    assert_eq!(verdict.status, HypothesisStatus::Refuted);
    let events = captured_events(&capture);
    let surprise_index = events
        .iter()
        .position(|event| event.name == "h1_surprise")
        .expect("surprise event");
    let hit_index = events
        .iter()
        .position(|event| event.name == "diagnostic_hit")
        .expect("diagnostic hit");
    assert!(
        surprise_index < hit_index,
        "surprise should precede diagnostic hit: {events:#?}"
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(8))]

    #[test]
    fn h1_known_violation_reports_generated_location(kind in 0_u8..=8, step in 1_u64..=10_000) {
        let (mut phase_log, mut entries) = fixture_phase_log(S2BuildKind::s2_ternary_full);
        let (check_name, expected_step) = apply_known_violation(kind, step, &mut phase_log, &mut entries);

        let verdict = verify_h1(&phase_log, &entries, S2BuildKind::s2_ternary_full);

        prop_assert_eq!(verdict.status, HypothesisStatus::Refuted);
        prop_assert!(
            verdict
                .hits
                .iter()
                .any(|hit| hit.check_name == check_name && hit.step == expected_step)
        );
    }
}

fn apply_known_violation(
    kind: u8,
    step: u64,
    phase_log: &mut PhaseLog,
    entries: &mut [PhaseEntry],
) -> (&'static str, Option<u64>) {
    let index = (step - 1) as usize;
    match kind {
        0 => {
            phase_log.optimizer_steps = 9_999;
            ("phase_log_header", None)
        }
        1 => {
            entries[index].step = step + 1;
            ("contiguous_steps", Some(step + 1))
        }
        2 => {
            entries[index].train_loss = f32::INFINITY;
            ("finite_train_loss", Some(step))
        }
        3 => {
            entries[index].grad_norm = f32::INFINITY;
            ("finite_grad_norm", Some(step))
        }
        4 => {
            entries[index].phase = if entries[index].phase == PhaseKindS2::PhaseA {
                PhaseKindS2::PhaseD
            } else {
                PhaseKindS2::PhaseA
            };
            ("phase_schedule", Some(step))
        }
        5 => {
            let boundary_step = match step % 3 {
                0 => 4_001,
                1 => 5_001,
                _ => 8_001,
            };
            entries[(boundary_step - 1) as usize]
                .events
                .retain(|event| matches!(event, PhaseEvent::TeacherFreeze { .. }));
            ("phase_transition_events", Some(boundary_step))
        }
        6 => {
            entries[S2_TEACHER_FREEZE_STEP as usize]
                .events
                .push(PhaseEvent::TeacherFreeze {
                    teacher_checkpoint_sha: hash(8),
                });
            ("teacher_freeze", Some(S2_TEACHER_FREEZE_STEP + 1))
        }
        7 => {
            entries[index].hardness = if entries[index].hardness == HardnessTriple::all_off() {
                HardnessTriple::new(
                    QuantHardness::Hard,
                    QuantHardness::Hard,
                    QuantHardness::Hard,
                )
            } else {
                HardnessTriple::all_off()
            };
            ("d2_hardness_sequence", Some(step))
        }
        _ => {
            entries[index].lambda_effective.lambda_distill = 0.25;
            ("phase_effective_lambdas", Some(step))
        }
    }
}

fn fixture_phase_log(build_kind: S2BuildKind) -> (PhaseLog, Vec<PhaseEntry>) {
    let entries = phase_entries(build_kind);
    let phase_log = PhaseLog::new(
        0,
        build_kind,
        hash(1),
        checkpoint_steps(build_kind),
        &entries,
    )
    .expect("phase log");
    (phase_log, entries)
}

fn phase_entries(build_kind: S2BuildKind) -> Vec<PhaseEntry> {
    let cfg = TrainConfigS2Full::pinned();
    let plan = if build_kind == S2BuildKind::s2_ablation {
        PhasePlan::phase_a_only()
    } else {
        PhasePlan::full_s2()
    };
    let steps = if build_kind == S2BuildKind::s2_ablation {
        S2_TEACHER_FREEZE_STEP
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
        lambda_effective: phase_effective_lambdas(step, build_kind, cfg).expect("lambdas"),
        teacher_frozen: build_kind != S2BuildKind::s2_ablation && step > S2_TEACHER_FREEZE_STEP,
        train_loss: 5.0,
        grad_norm: 0.5,
        distill_loss: (build_kind != S2BuildKind::s2_ablation && step > S2_PHASE_B_END_STEP)
            .then_some(0.125),
        events: phase_events(step, build_kind),
    }
}

fn phase_events(step: GlobalStep, build_kind: S2BuildKind) -> Vec<PhaseEvent> {
    if build_kind == S2BuildKind::s2_ablation {
        return Vec::new();
    }
    match step {
        4_001 => vec![
            PhaseEvent::PhaseTransition {
                from: PhaseKindS2::PhaseA,
                to: PhaseKindS2::PhaseB,
            },
            PhaseEvent::TeacherFreeze {
                teacher_checkpoint_sha: hash(9),
            },
        ],
        5_001 => vec![PhaseEvent::PhaseTransition {
            from: PhaseKindS2::PhaseB,
            to: PhaseKindS2::PhaseC,
        }],
        8_001 => vec![PhaseEvent::PhaseTransition {
            from: PhaseKindS2::PhaseC,
            to: PhaseKindS2::PhaseD,
        }],
        _ => Vec::new(),
    }
}

fn checkpoint_steps(build_kind: S2BuildKind) -> Vec<u64> {
    if build_kind == S2BuildKind::s2_ablation {
        vec![4_000]
    } else {
        vec![4_000, 5_000, 8_000, 10_000]
    }
}

fn assert_hit(hits: &[DiagnosticHit], check_name: &str, step: u64) {
    assert!(
        hits.iter().any(|hit| {
            hit.check_name == check_name && hit.step == Some(step) && hit.expected != hit.observed
        }),
        "missing hit {check_name} at step {step}: {hits:#?}"
    );
}

fn first_hit<'a>(hits: &'a [DiagnosticHit], check_name: &str) -> &'a DiagnosticHit {
    hits.iter()
        .find(|hit| hit.check_name == check_name)
        .unwrap_or_else(|| panic!("missing hit {check_name}: {hits:#?}"))
}

fn pretty_json(value: &Value) -> String {
    serde_json::to_string_pretty(value).expect("snapshot value serializes")
}

fn hash(fill: u8) -> Hash256 {
    Hash256::from_bytes([fill; 32])
}
