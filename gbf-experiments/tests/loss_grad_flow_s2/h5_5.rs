use gbf_experiments::s2::loss_grad_flow::{
    H5_5_DISTILL_TEMPERATURE, H5_5_LAMBDA_DISTILL, run_h5_5_distill_fixture,
};
use serde_json::json;

use crate::common::tracing_capture::{TraceCapture, captured_events, with_trace_capture};

#[test]
fn h5_5_distillation_fixture_flows_grad_only_to_student_logits() {
    let capture = TraceCapture::default();
    let fixture = with_trace_capture(&capture, run_h5_5_distill_fixture).expect("h5.5 fixture");

    assert_eq!(fixture.sub_hypothesis, "H5.5");
    assert_eq!(fixture.loss_term, "lambda_distill");
    assert!(fixture.non_default_value_used);
    assert!(fixture.numerical_stability_passed);
    assert!(
        fixture
            .in_scope_grad_norms
            .get("student_logits")
            .copied()
            .expect("student grad")
            > 0.0
    );
    assert_eq!(
        fixture.stop_gradient_grad_norms.get("teacher_logits"),
        Some(&0.0)
    );
    assert_eq!(
        fixture.detached_grad_absence.get("teacher_logits"),
        Some(&true)
    );
    let student_grad_norm = fixture
        .in_scope_grad_norms
        .get("student_logits")
        .copied()
        .expect("student grad");
    let direct_weighted_norm = direct_h5_5_student_grad_norm(H5_5_LAMBDA_DISTILL);
    let direct_unweighted_norm = direct_h5_5_student_grad_norm(1.0);
    assert_close(student_grad_norm, direct_weighted_norm, 1.0e-7);
    assert_close(direct_weighted_norm * 2.0, direct_unweighted_norm, 1.0e-6);
    fixture.validate().expect("h5.5 fixture validates");
    insta::with_settings!({prepend_module_to_snapshot => false, snapshot_path => "../snapshots"}, {
        insta::assert_snapshot!(
            "h5_5__teacher_zero_grad_or_absence",
            serde_json::to_string_pretty(&fixture).expect("snapshot JSON")
        );
    });

    let events = captured_events(&capture);
    let event = events
        .iter()
        .find(|event| event.name == "h5_5_fixture_run")
        .expect("h5.5 event");
    assert_eq!(event.fields.get("batch"), Some(&json!(2)));
    assert_eq!(event.fields.get("vocab"), Some(&json!(4)));
    assert_eq!(
        event.fields.get("lambda_distill"),
        Some(&json!(H5_5_LAMBDA_DISTILL))
    );
    assert_eq!(
        event.fields.get("distill_temperature"),
        Some(&json!(H5_5_DISTILL_TEMPERATURE))
    );
    assert!(
        !event.fields.contains_key("teacher_grad_norm"),
        "tracing omits teacher_grad_norm when teacher_grad_absence=true"
    );
    assert_eq!(event.fields.get("teacher_grad_absence"), Some(&json!(true)));
    assert_eq!(
        event.fields.get("non_default_value_used"),
        Some(&json!(true))
    );
    assert_eq!(event.fields.get("sub_passed"), Some(&json!(true)));
}

#[test]
fn h5_5_teacher_gradient_leak_fails_lgf4() {
    let mut fixture = run_h5_5_distill_fixture().expect("h5.5 fixture");
    fixture
        .stop_gradient_grad_norms
        .insert("teacher_logits".to_owned(), 1.0e-5);
    fixture
        .detached_grad_absence
        .insert("teacher_logits".to_owned(), false);
    fixture.sub_passed = true;

    assert!(fixture.validate().is_err());
}

fn direct_h5_5_student_grad_norm(lambda_distill: f32) -> f32 {
    use gbf_train::adapter::burn::{
        BurnDevice, BurnNdArrayAutodiffBackend, float_tensor_from_vec, float_tensor_into_vec,
    };
    use gbf_train::loss::distillation::{burn_distillation_loss, burn_weighted_distillation_loss};

    type B = BurnNdArrayAutodiffBackend;

    let device = BurnDevice::<B>::default();
    let student_logits = float_tensor_from_vec::<B, 2>(
        vec![0.2, -0.1, 0.4, -0.3, 0.0, 0.3, -0.2, 0.5],
        [2, 4],
        &device,
    )
    .expect("student logits")
    .require_grad();
    let teacher_logits = float_tensor_from_vec::<B, 2>(
        vec![0.7, -0.4, 0.6, -0.7, -0.1, 0.7, -0.8, 0.8],
        [2, 4],
        &device,
    )
    .expect("teacher logits");
    let raw_loss = burn_distillation_loss(
        student_logits.clone(),
        teacher_logits,
        1,
        H5_5_DISTILL_TEMPERATURE,
    )
    .expect("raw distillation loss");
    let weighted = burn_weighted_distillation_loss(raw_loss, lambda_distill)
        .expect("weighted distillation loss");
    let gradients = weighted.backward();
    let grad = student_logits
        .grad(&gradients)
        .expect("student receives gradient");
    l2_norm(&float_tensor_into_vec(grad).expect("student grad values"))
}

fn l2_norm(values: &[f32]) -> f32 {
    values.iter().map(|value| value * value).sum::<f32>().sqrt()
}

fn assert_close(actual: f32, expected: f32, epsilon: f32) {
    assert!(
        (actual - expected).abs() <= epsilon,
        "expected {actual} to be within {epsilon} of {expected}"
    );
}
