mod common;

use common::tracing_capture::{TraceCapture, captured_events, with_trace_capture};
use gbf_experiments::s2::run::lambdas::phase_effective_lambdas;
use gbf_experiments::s2::run::loss_applicability::{
    toy0_loss_applicability_for_build_phase, toy0_loss_applicability_for_s2_phase,
};
use gbf_experiments::s2::schema::{
    PhaseKindS2, S2_OPTIMIZER_STEPS, S2_PHASE_B_END_STEP, S2_PHASE_C_END_STEP, S2BuildKind,
    TrainConfigS2Full,
};
use serde_json::{Value, json};

#[test]
fn lambda_distill_flips_for_full_builds_only() {
    let cfg = TrainConfigS2Full::pinned();

    assert_eq!(
        phase_effective_lambdas(5_000, S2BuildKind::s2_ternary_full, &cfg)
            .unwrap()
            .lambda_distill,
        0.0
    );
    assert_eq!(
        phase_effective_lambdas(5_001, S2BuildKind::s2_ternary_full, &cfg)
            .unwrap()
            .lambda_distill,
        1.0
    );
    assert_eq!(
        phase_effective_lambdas(5_001, S2BuildKind::s2_ternary_nodistill, &cfg)
            .unwrap()
            .lambda_distill,
        0.0
    );
    assert_eq!(
        phase_effective_lambdas(5_001, S2BuildKind::s2_fp_full, &cfg)
            .unwrap()
            .lambda_distill,
        1.0
    );
}

#[test]
fn lambda_range_and_zero_follow_qat_schedule_and_build_kind() {
    let cfg = TrainConfigS2Full::pinned();

    assert_eq!(
        phase_effective_lambdas(8_500, S2BuildKind::s2_ternary_full, &cfg)
            .unwrap()
            .lambda_range,
        0.0
    );
    assert_eq!(
        phase_effective_lambdas(8_501, S2BuildKind::s2_ternary_full, &cfg)
            .unwrap()
            .lambda_range,
        cfg.lambda_range
    );
    assert_eq!(
        phase_effective_lambdas(5_001, S2BuildKind::s2_ternary_full, &cfg)
            .unwrap()
            .lambda_zero,
        cfg.lambda_zero
    );
    assert_eq!(
        phase_effective_lambdas(8_501, S2BuildKind::s2_fp_full, &cfg)
            .unwrap()
            .lambda_range,
        0.0
    );
    assert_eq!(
        phase_effective_lambdas(5_001, S2BuildKind::s2_fp_full, &cfg)
            .unwrap()
            .lambda_zero,
        0.0
    );
    assert_eq!(
        phase_effective_lambdas(4_000, S2BuildKind::s2_ablation, &cfg)
            .unwrap()
            .lambda_zero,
        0.0
    );
}

#[test]
fn toy0_loss_applicability_is_canonical_by_s2_phase() {
    let cases = [
        (
            PhaseKindS2::PhaseA,
            (false, false, false, false, false, false, false),
        ),
        (
            PhaseKindS2::PhaseB,
            (false, false, false, false, false, false, false),
        ),
        (
            PhaseKindS2::PhaseC,
            (true, false, false, true, false, false, false),
        ),
        (
            PhaseKindS2::PhaseD,
            (true, false, true, true, false, false, false),
        ),
    ];

    for (phase, (distill, router, range, zero, shape, overflow, switch)) in cases {
        let applicability = toy0_loss_applicability_for_s2_phase(phase);
        assert_eq!(applicability.distill, distill, "{phase:?} distill");
        assert_eq!(applicability.balance, router, "{phase:?} balance");
        assert_eq!(applicability.zrouter, router, "{phase:?} zrouter");
        assert_eq!(applicability.range, range, "{phase:?} range");
        assert_eq!(applicability.zero, zero, "{phase:?} zero");
        assert_eq!(applicability.shape, shape, "{phase:?} shape");
        assert_eq!(applicability.overflow, overflow, "{phase:?} overflow");
        assert_eq!(applicability.switch, switch, "{phase:?} switch");
    }
}

#[test]
fn phase_effective_lambdas_respect_toy0_loss_applicability() {
    let cfg = TrainConfigS2Full::pinned();
    let rows = [
        (PhaseKindS2::PhaseA, 1),
        (PhaseKindS2::PhaseA, 4_000),
        (PhaseKindS2::PhaseB, 4_001),
        (PhaseKindS2::PhaseB, 5_000),
        (PhaseKindS2::PhaseC, 5_001),
        (PhaseKindS2::PhaseC, 8_000),
        (PhaseKindS2::PhaseD, 8_001),
        (PhaseKindS2::PhaseD, 8_500),
        (PhaseKindS2::PhaseD, 8_501),
        (PhaseKindS2::PhaseD, 10_000),
    ];

    for (phase, step) in rows {
        let applicability = toy0_loss_applicability_for_s2_phase(phase);
        let lambdas = phase_effective_lambdas(step, S2BuildKind::s2_ternary_full, &cfg).unwrap();

        if !applicability.distill {
            assert_eq!(lambdas.lambda_distill, 0.0, "{phase:?} distill");
        }
        if !applicability.range {
            assert_eq!(lambdas.lambda_range, 0.0, "{phase:?} range");
        }
        if !applicability.zero {
            assert_eq!(lambdas.lambda_zero, 0.0, "{phase:?} zero");
        }
        assert_eq!(lambdas.lambda_balance, 0.0, "{phase:?} balance");
        assert_eq!(lambdas.lambda_zrouter, 0.0, "{phase:?} zrouter");
        assert_eq!(lambdas.lambda_switch, 0.0, "{phase:?} switch");
        assert_eq!(lambdas.lambda_shape, 0.0, "{phase:?} shape");
        assert_eq!(lambdas.lambda_overflow, 0.0, "{phase:?} overflow");
    }

    let nodistill_phase_c =
        phase_effective_lambdas(5_001, S2BuildKind::s2_ternary_nodistill, &cfg).unwrap();
    assert!(toy0_loss_applicability_for_s2_phase(PhaseKindS2::PhaseC).distill);
    assert_eq!(nodistill_phase_c.lambda_distill, 0.0);

    let fp_phase_d =
        toy0_loss_applicability_for_build_phase(S2BuildKind::s2_fp_full, PhaseKindS2::PhaseD);
    assert!(fp_phase_d.distill);
    assert!(!fp_phase_d.range);
    assert!(!fp_phase_d.zero);

    let ablation_phase_a =
        toy0_loss_applicability_for_build_phase(S2BuildKind::s2_ablation, PhaseKindS2::PhaseA);
    assert_eq!(
        ablation_phase_a,
        toy0_loss_applicability_for_s2_phase(PhaseKindS2::PhaseA)
    );
    assert!(
        phase_effective_lambdas(4_001, S2BuildKind::s2_ablation, &cfg).is_err(),
        "s2_ablation has a Phase-A-only plan"
    );
}

#[test]
fn phase_effective_lambdas_match_applicability_across_all_steps() {
    let cfg = TrainConfigS2Full::pinned();

    for step in 1..=S2_OPTIMIZER_STEPS {
        let phase = phase_for_step(step);
        let ternary = phase_effective_lambdas(step, S2BuildKind::s2_ternary_full, &cfg).unwrap();
        let ternary_app =
            toy0_loss_applicability_for_build_phase(S2BuildKind::s2_ternary_full, phase);
        assert_eq!(
            ternary.lambda_distill > 0.0,
            ternary_app.distill,
            "step {step} ternary distill"
        );
        assert_eq!(
            ternary.lambda_zero > 0.0,
            ternary_app.zero,
            "step {step} ternary zero"
        );
        assert!(
            ternary_app.range || ternary.lambda_range == 0.0,
            "step {step} range cannot be nonzero while inapplicable"
        );
        assert_common_inert_lambdas_are_zero(phase, ternary);

        let fp = phase_effective_lambdas(step, S2BuildKind::s2_fp_full, &cfg).unwrap();
        let fp_app = toy0_loss_applicability_for_build_phase(S2BuildKind::s2_fp_full, phase);
        assert_eq!(
            fp.lambda_distill > 0.0,
            fp_app.distill,
            "step {step} fp distill"
        );
        assert_eq!(fp.lambda_range, 0.0, "step {step} fp range");
        assert_eq!(fp.lambda_zero, 0.0, "step {step} fp zero");
        assert!(!fp_app.range, "step {step} fp range applicability");
        assert!(!fp_app.zero, "step {step} fp zero applicability");
        assert_common_inert_lambdas_are_zero(phase, fp);

        let nodistill =
            phase_effective_lambdas(step, S2BuildKind::s2_ternary_nodistill, &cfg).unwrap();
        let nodistill_app =
            toy0_loss_applicability_for_build_phase(S2BuildKind::s2_ternary_nodistill, phase);
        assert_eq!(nodistill.lambda_distill, 0.0, "step {step} nodistill");
        assert_eq!(
            nodistill.lambda_zero > 0.0,
            nodistill_app.zero,
            "step {step} nodistill zero"
        );
        assert_common_inert_lambdas_are_zero(phase, nodistill);
    }

    for step in 1..=4_000 {
        let ablation = phase_effective_lambdas(step, S2BuildKind::s2_ablation, &cfg).unwrap();
        assert_eq!(
            toy0_loss_applicability_for_build_phase(S2BuildKind::s2_ablation, PhaseKindS2::PhaseA),
            toy0_loss_applicability_for_s2_phase(PhaseKindS2::PhaseA)
        );
        assert_eq!(ablation.lambda_distill, 0.0, "step {step} ablation");
        assert_eq!(ablation.lambda_range, 0.0, "step {step} ablation");
        assert_eq!(ablation.lambda_zero, 0.0, "step {step} ablation");
        assert_common_inert_lambdas_are_zero(PhaseKindS2::PhaseA, ablation);
    }
}

#[test]
fn lambda_table_snapshot_and_phase_step_event_are_stable_without_dispatch_noise() {
    let cfg = TrainConfigS2Full::pinned();
    let capture = TraceCapture::default();
    let rows = with_trace_capture(&capture, || {
        [
            (S2BuildKind::s2_ternary_full, 5_000),
            (S2BuildKind::s2_ternary_full, 5_001),
            (S2BuildKind::s2_ternary_full, 8_500),
            (S2BuildKind::s2_ternary_full, 8_501),
            (S2BuildKind::s2_fp_full, 8_501),
            (S2BuildKind::s2_ternary_nodistill, 5_001),
            (S2BuildKind::s2_ablation, 4_000),
        ]
        .into_iter()
        .map(|(build_kind, step)| {
            let lambdas = phase_effective_lambdas(step, build_kind, &cfg).unwrap();
            json!({
                "build_kind": format!("{build_kind:?}"),
                "step": step,
                "lambda_distill": lambdas.lambda_distill,
                "lambda_range": lambdas.lambda_range,
                "lambda_zero": lambdas.lambda_zero,
            })
        })
        .collect::<Vec<_>>()
    });

    assert!(captured_events(&capture).iter().any(|event| {
        event.name == "phase_step"
            && event.fields.get("step").and_then(Value::as_u64) == Some(8_501)
    }));
    assert!(
        !captured_events(&capture)
            .iter()
            .any(|event| event.name == "buildkind_dispatch")
    );
    assert!(
        !captured_events(&capture)
            .iter()
            .any(|event| event.name == "hardness_override_active")
    );
    insta::with_settings!({prepend_module_to_snapshot => false}, {
        insta::assert_snapshot!("scheduler_s2__lambda_table", pretty_json(&json!(rows)));
    });
}

fn pretty_json(value: &Value) -> String {
    serde_json::to_string_pretty(value).expect("snapshot serializes")
}

fn phase_for_step(step: u64) -> PhaseKindS2 {
    if step <= 4_000 {
        PhaseKindS2::PhaseA
    } else if step <= S2_PHASE_B_END_STEP {
        PhaseKindS2::PhaseB
    } else if step <= S2_PHASE_C_END_STEP {
        PhaseKindS2::PhaseC
    } else {
        PhaseKindS2::PhaseD
    }
}

fn assert_common_inert_lambdas_are_zero(
    phase: PhaseKindS2,
    lambdas: gbf_experiments::s2::schema::PhaseEffectiveLambda,
) {
    assert_eq!(lambdas.lambda_balance, 0.0, "{phase:?} balance");
    assert_eq!(lambdas.lambda_zrouter, 0.0, "{phase:?} zrouter");
    assert_eq!(lambdas.lambda_switch, 0.0, "{phase:?} switch");
    assert_eq!(lambdas.lambda_shape, 0.0, "{phase:?} shape");
    assert_eq!(lambdas.lambda_overflow, 0.0, "{phase:?} overflow");
}
