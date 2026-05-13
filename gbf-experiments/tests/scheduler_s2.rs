mod common;

use common::tracing_capture::{TraceCapture, captured_events, with_trace_capture};
use gbf_experiments::s2::run::hardness::{hardness_for_global_step, hardness_for_phase_local};
use gbf_experiments::s2::run::scheduler::{
    PhaseEvent, PhasePlan, PhasePlanError, events_for_global_step, is_transition_step,
    phase_for_global_step,
};
use gbf_experiments::s2::schema::{
    HardnessTriple, PhaseKindS2, QuantHardness, QuantHardnessOverride, TrainConfigS2Phase,
};
use serde_json::{Value, json};

#[test]
fn scheduler_d2_table_and_counts_are_pinned() {
    let plan = PhasePlan::full_s2();
    let mut expert_hard = 0_u32;
    let mut activation_hard = 0_u32;
    let mut norm_hard = 0_u32;
    for step in 1..=10_000 {
        let hardness = hardness_for_global_step(step, &plan, QuantHardnessOverride::None).unwrap();
        expert_hard += u32::from(hardness.expert_qat == QuantHardness::Hard);
        activation_hard += u32::from(hardness.activation_qat == QuantHardness::Hard);
        norm_hard += u32::from(hardness.norm_qat == QuantHardness::Hard);
    }

    assert_eq!(expert_hard, 3_000);
    assert_eq!(activation_hard, 1_000);
    assert_eq!(norm_hard, 1_000);
    assert_eq!(
        hardness_for_global_step(8_001, &plan, QuantHardnessOverride::None).unwrap(),
        HardnessTriple::new(QuantHardness::Hard, QuantHardness::Off, QuantHardness::Off)
    );
    for (step, expected) in [
        (6_000, HardnessTriple::all_off()),
        (
            6_001,
            HardnessTriple::new(QuantHardness::Soft, QuantHardness::Off, QuantHardness::Off),
        ),
        (
            7_000,
            HardnessTriple::new(QuantHardness::Soft, QuantHardness::Off, QuantHardness::Off),
        ),
        (
            7_001,
            HardnessTriple::new(QuantHardness::Hard, QuantHardness::Off, QuantHardness::Off),
        ),
        (
            8_500,
            HardnessTriple::new(QuantHardness::Hard, QuantHardness::Off, QuantHardness::Off),
        ),
        (
            8_501,
            HardnessTriple::new(
                QuantHardness::Hard,
                QuantHardness::Soft,
                QuantHardness::Soft,
            ),
        ),
        (
            9_000,
            HardnessTriple::new(
                QuantHardness::Hard,
                QuantHardness::Soft,
                QuantHardness::Soft,
            ),
        ),
        (
            9_001,
            HardnessTriple::new(
                QuantHardness::Hard,
                QuantHardness::Hard,
                QuantHardness::Hard,
            ),
        ),
    ] {
        assert_eq!(
            hardness_for_global_step(step, &plan, QuantHardnessOverride::None).unwrap(),
            expected,
            "canonical D2 edge at global step {step}"
        );
        assert_eq!(
            hardness_for_global_step(step, &plan, QuantHardnessOverride::None).unwrap(),
            reference_d2_hardness(step),
            "independent D2 edge reference at global step {step}"
        );
    }

    insta::with_settings!({prepend_module_to_snapshot => false}, {
        insta::assert_snapshot!("scheduler_s2__d2_table", pretty_json(&json!({
            "ranges": [
                {"steps": "1..=5000", "expert_qat": "off", "activation_qat": "off", "norm_qat": "off"},
                {"steps": "5001..=6000", "expert_qat": "off", "activation_qat": "off", "norm_qat": "off"},
                {"steps": "6001..=7000", "expert_qat": "soft", "activation_qat": "off", "norm_qat": "off"},
                {"steps": "7001..=8000", "expert_qat": "hard", "activation_qat": "off", "norm_qat": "off"},
                {"steps": "8001..=8500", "expert_qat": "hard", "activation_qat": "off", "norm_qat": "off"},
                {"steps": "8501..=9000", "expert_qat": "hard", "activation_qat": "soft", "norm_qat": "soft"},
                {"steps": "9001..=10000", "expert_qat": "hard", "activation_qat": "hard", "norm_qat": "hard"}
            ],
            "hard_counts": {
                "expert_qat": expert_hard,
                "activation_qat": activation_hard,
                "norm_qat": norm_hard
            }
        })));
    });
}

#[test]
fn scheduler_boundaries_and_events_are_pinned() {
    let plan = PhasePlan::full_s2();
    let capture = TraceCapture::default();
    let events = with_trace_capture(&capture, || {
        [
            events_for_global_step(4_001, &plan, Some("sha256:teacher")),
            events_for_global_step(5_001, &plan, Some("sha256:teacher")),
            events_for_global_step(8_001, &plan, Some("sha256:teacher")),
        ]
        .concat()
    });

    assert!(is_transition_step(4_001, &plan));
    assert!(is_transition_step(5_001, &plan));
    assert!(is_transition_step(8_001, &plan));
    assert!(!is_transition_step(1, &plan));
    assert_eq!(
        phase_for_global_step(5_001, &plan).unwrap(),
        PhaseKindS2::PhaseC
    );
    assert_eq!(
        events,
        vec![
            PhaseEvent::PhaseTransition {
                from: PhaseKindS2::PhaseA,
                to: PhaseKindS2::PhaseB
            },
            PhaseEvent::TeacherFreeze {
                teacher_checkpoint_sha: "sha256:teacher".to_owned()
            },
            PhaseEvent::PhaseTransition {
                from: PhaseKindS2::PhaseB,
                to: PhaseKindS2::PhaseC
            },
            PhaseEvent::PhaseTransition {
                from: PhaseKindS2::PhaseC,
                to: PhaseKindS2::PhaseD
            }
        ]
    );
    assert_eq!(
        captured_events(&capture)
            .iter()
            .filter(|event| event.name == "phase_transition")
            .count(),
        3
    );
}

#[test]
fn teacher_freeze_event_follows_phase_b_start() {
    let plan = PhasePlan::new(vec![
        TrainConfigS2Phase::new(PhaseKindS2::PhaseA, 1, 2).unwrap(),
        TrainConfigS2Phase::new(PhaseKindS2::PhaseB, 3, 4).unwrap(),
    ])
    .unwrap();

    assert_eq!(
        events_for_global_step(3, &plan, Some("sha256:teacher")),
        vec![
            PhaseEvent::PhaseTransition {
                from: PhaseKindS2::PhaseA,
                to: PhaseKindS2::PhaseB
            },
            PhaseEvent::TeacherFreeze {
                teacher_checkpoint_sha: "sha256:teacher".to_owned()
            }
        ]
    );
    assert!(events_for_global_step(4_001, &plan, Some("sha256:teacher")).is_empty());
}

#[test]
fn scheduler_rejects_empty_and_overlapping_plans() {
    assert_eq!(
        PhasePlan::new(Vec::new()).unwrap_err(),
        PhasePlanError::Empty
    );
    assert_eq!(
        PhasePlan::new(vec![
            TrainConfigS2Phase::new(PhaseKindS2::PhaseA, 1, 10).unwrap(),
            TrainConfigS2Phase::new(PhaseKindS2::PhaseB, 10, 20).unwrap(),
        ])
        .unwrap_err(),
        PhasePlanError::Overlap {
            a: PhaseKindS2::PhaseA,
            b: PhaseKindS2::PhaseB
        }
    );
}

#[test]
fn fp_override_and_skip_phase_fixture_hardness_are_explicit() {
    let plan = PhasePlan::full_s2();
    for step in 1..=10_000 {
        assert_eq!(
            hardness_for_global_step(step, &plan, QuantHardnessOverride::AllOff).unwrap(),
            HardnessTriple::all_off()
        );
    }
    assert_eq!(
        hardness_for_phase_local(PhaseKindS2::PhaseC, 1),
        HardnessTriple::all_off()
    );
    let skip_phase_plan = PhasePlan::new(vec![
        TrainConfigS2Phase::new(PhaseKindS2::PhaseC, 1, 1_000).unwrap(),
    ])
    .unwrap();
    assert_eq!(
        phase_for_global_step(1, &skip_phase_plan).unwrap(),
        PhaseKindS2::PhaseC
    );
    assert_eq!(
        hardness_for_global_step(1, &skip_phase_plan, QuantHardnessOverride::None).unwrap(),
        HardnessTriple::all_off()
    );
    assert!(events_for_global_step(1, &skip_phase_plan, Some("sha256:teacher")).is_empty());
    assert_eq!(
        phase_for_global_step(1_001, &skip_phase_plan).unwrap_err(),
        PhasePlanError::StepOutOfRange { step: 1_001 }
    );
}

#[test]
fn ablation_plan_has_no_transition_or_teacher_freeze() {
    let plan = PhasePlan::phase_a_only();
    assert!(events_for_global_step(4_001, &plan, Some("sha256:teacher")).is_empty());
    assert!(!is_transition_step(4_000, &plan));
}

fn pretty_json(value: &Value) -> String {
    serde_json::to_string_pretty(value).expect("snapshot serializes")
}

fn reference_d2_hardness(step: u64) -> HardnessTriple {
    match step {
        1..=6_000 => HardnessTriple::all_off(),
        6_001..=7_000 => {
            HardnessTriple::new(QuantHardness::Soft, QuantHardness::Off, QuantHardness::Off)
        }
        7_001..=8_500 => {
            HardnessTriple::new(QuantHardness::Hard, QuantHardness::Off, QuantHardness::Off)
        }
        8_501..=9_000 => HardnessTriple::new(
            QuantHardness::Hard,
            QuantHardness::Soft,
            QuantHardness::Soft,
        ),
        9_001..=10_000 => HardnessTriple::new(
            QuantHardness::Hard,
            QuantHardness::Hard,
            QuantHardness::Hard,
        ),
        _ => panic!("step outside canonical S2 schedule: {step}"),
    }
}
