use gbf_model::loss::temporal_smoothness::SmoothnessWindow;
use gbf_train::adapter::burn::{
    BurnAutodiffBackend, BurnDevice, BurnFloatTensor, BurnNdArrayAutodiffBackend, burn_softmax,
    float_tensor_from_vec, float_tensor_into_vec,
};
use gbf_train::loss::composer::{
    BurnLossTerms, LossTermApplicability, PhaseEffectiveLossWeights,
    PhaseEffectiveLossWeightsValues, TrainingLossUnit, burn_compose,
};
use gbf_train::loss::distillation::{DEFAULT_DISTILLATION_TEMPERATURE, burn_distillation_loss};
use gbf_train::loss::router::{BurnRawRouterLogits, burn_router_z_loss};
use gbf_train::loss::switch::burn_temporal_switch_loss;

type B = BurnNdArrayAutodiffBackend;

const GRAD_TOLERANCE: f32 = 1.0e-6;
const VALUE_TOLERANCE: f32 = 1.0e-5;

#[test]
fn switch_loss_tests_production_helper_matches_window_oracle_and_backprops() {
    let device = BurnDevice::<B>::default();
    let routing_probs = tensor4(
        vec![
            1.0, 0.0, //
            1.0, 0.0, //
            0.0, 1.0, //
            0.0, 1.0, //
        ],
        [1, 4, 1, 2],
        &device,
    )
    .require_grad();

    let loss = burn_temporal_switch_loss(
        routing_probs.clone(),
        &[true; 4],
        None,
        SmoothnessWindow::new(2).unwrap(),
    )
    .unwrap();
    assert_close(float_tensor_into_vec(loss.clone()).unwrap()[0], 0.6);

    let gradients = loss.backward();
    let grad = routing_probs
        .grad(&gradients)
        .expect("L_switch should reach routing probabilities");
    assert_finite_nonzero(&float_tensor_into_vec(grad).unwrap(), "routing_probs");
}

#[test]
fn switch_loss_tests_all_same_expert_has_zero_value() {
    let device = BurnDevice::<B>::default();
    let routing_probs = tensor4(
        vec![
            1.0, 0.0, //
            1.0, 0.0, //
            1.0, 0.0, //
            1.0, 0.0, //
            1.0, 0.0, //
            1.0, 0.0, //
            1.0, 0.0, //
            1.0, 0.0, //
        ],
        [1, 8, 1, 2],
        &device,
    );

    let loss = burn_temporal_switch_loss(
        routing_probs,
        &[true; 8],
        None,
        SmoothnessWindow::new(4).unwrap(),
    )
    .unwrap();

    assert_close(float_tensor_into_vec(loss).unwrap()[0], 0.0);
}

#[test]
fn switch_loss_tests_alternating_eight_token_window_four_matches_full_window_oracle() {
    let device = BurnDevice::<B>::default();
    let routing_probs = tensor4(
        vec![
            1.0, 0.0, //
            0.0, 1.0, //
            1.0, 0.0, //
            0.0, 1.0, //
            1.0, 0.0, //
            0.0, 1.0, //
            1.0, 0.0, //
            0.0, 1.0, //
        ],
        [1, 8, 1, 2],
        &device,
    );

    let loss = burn_temporal_switch_loss(
        routing_probs,
        &[true; 8],
        None,
        SmoothnessWindow::new(4).unwrap(),
    )
    .unwrap();

    assert_close(float_tensor_into_vec(loss).unwrap()[0], 6.0 / 11.0);
}

#[test]
fn switch_loss_tests_sequence_boundary_removes_cross_sequence_penalty() {
    let device = BurnDevice::<B>::default();
    let routing_probs = tensor4(
        vec![
            1.0, 0.0, //
            1.0, 0.0, //
            0.0, 1.0, //
            0.0, 1.0, //
        ],
        [1, 4, 1, 2],
        &device,
    );

    let loss = burn_temporal_switch_loss(
        routing_probs,
        &[true; 4],
        Some(&[false, false, true, false]),
        SmoothnessWindow::new(3).unwrap(),
    )
    .unwrap();

    assert_close(float_tensor_into_vec(loss).unwrap()[0], 0.0);
}

#[test]
fn switch_loss_tests_packed_two_sequences_do_not_cross_window_four_boundary() {
    let device = BurnDevice::<B>::default();
    let routing_probs = tensor4(
        vec![
            1.0, 0.0, //
            1.0, 0.0, //
            1.0, 0.0, //
            1.0, 0.0, //
            0.0, 1.0, //
            0.0, 1.0, //
            0.0, 1.0, //
            0.0, 1.0, //
        ],
        [1, 8, 1, 2],
        &device,
    );

    let loss = burn_temporal_switch_loss(
        routing_probs,
        &[true; 8],
        Some(&[false, false, false, false, true, false, false, false]),
        SmoothnessWindow::new(4).unwrap(),
    )
    .unwrap();

    assert_close(float_tensor_into_vec(loss).unwrap()[0], 0.0);
}

#[test]
fn switch_loss_tests_composer_combines_switch_zrouter_and_distill_gradients() {
    let device = BurnDevice::<B>::default();
    let lm_loss = scalar_tensor(0.125, &device).require_grad();
    let router_logits = tensor2(
        vec![
            2.0, 0.0, //
            1.5, 0.0, //
            0.0, 1.5, //
            0.0, 2.0, //
        ],
        [4, 2],
        &device,
    )
    .require_grad();
    let routing_probs = burn_softmax(router_logits.clone(), 1).reshape([1, 4, 1, 2]);
    let switch_loss = burn_temporal_switch_loss(
        routing_probs,
        &[true; 4],
        None,
        SmoothnessWindow::new(2).unwrap(),
    )
    .unwrap();
    let z_loss = burn_router_z_loss(BurnRawRouterLogits::from_raw_router_logits(
        router_logits.clone(),
    ))
    .unwrap();

    let student_logits = tensor2(
        vec![
            0.2, -0.1, 0.0, //
            0.0, 0.3, -0.2, //
        ],
        [2, 3],
        &device,
    )
    .require_grad();
    let teacher_logits = tensor2(
        vec![
            0.0, 0.2, -0.1, //
            0.1, -0.2, 0.3, //
        ],
        [2, 3],
        &device,
    );
    let distill_loss = burn_distillation_loss(
        student_logits.clone(),
        teacher_logits,
        1,
        DEFAULT_DISTILLATION_TEMPERATURE,
    )
    .unwrap();

    let composed = burn_compose(
        BurnLossTerms {
            lm_loss_next_byte_nats: lm_loss.clone(),
            distill_loss_raw_nats: Some(distill_loss),
            balance_loss_raw: None,
            zrouter_loss_raw: Some(z_loss),
            switch_loss_raw: Some(switch_loss),
            range_loss_raw: None,
            zero_loss_raw: None,
            shape_loss_raw: None,
            overflow_loss_raw: None,
        },
        s7_switch_test_lambdas(),
        s7_switch_test_applicability(),
        TrainingLossUnit::Nats,
    )
    .unwrap();

    assert!(composed.weighted.distill.is_some());
    assert!(composed.weighted.zrouter.is_some());
    assert!(composed.weighted.switch.is_some());
    assert!(composed.scalar.total_loss > 0.125);

    let gradients = composed.total_loss.backward();
    let router_grad = gradient_values(router_logits, &gradients);
    let student_grad = gradient_values(student_logits, &gradients);

    assert_close(gradient_values(lm_loss, &gradients)[0], 1.0);
    assert_finite_nonzero(&router_grad, "router_logits");
    assert_finite_nonzero(&student_grad, "student_logits");
}

fn tensor1(values: Vec<f32>, shape: [usize; 1], device: &BurnDevice<B>) -> BurnFloatTensor<B, 1> {
    float_tensor_from_vec(values, shape, device).unwrap()
}

fn tensor2(values: Vec<f32>, shape: [usize; 2], device: &BurnDevice<B>) -> BurnFloatTensor<B, 2> {
    float_tensor_from_vec(values, shape, device).unwrap()
}

fn tensor4(values: Vec<f32>, shape: [usize; 4], device: &BurnDevice<B>) -> BurnFloatTensor<B, 4> {
    float_tensor_from_vec(values, shape, device).unwrap()
}

fn scalar_tensor(value: f32, device: &BurnDevice<B>) -> BurnFloatTensor<B, 1> {
    tensor1(vec![value], [1], device)
}

fn gradient_values<const D: usize>(
    tensor: BurnFloatTensor<B, D>,
    gradients: &<B as BurnAutodiffBackend>::Gradients,
) -> Vec<f32> {
    float_tensor_into_vec(
        tensor
            .grad(gradients)
            .expect("tensor should receive gradient"),
    )
    .unwrap()
}

fn s7_switch_test_lambdas() -> PhaseEffectiveLossWeights {
    PhaseEffectiveLossWeights::new(PhaseEffectiveLossWeightsValues {
        lambda_distill: 0.3,
        lambda_balance: 0.0,
        lambda_zrouter: 0.5,
        lambda_switch: 0.7,
        lambda_range: 0.0,
        lambda_zero: 0.0,
        lambda_shape: 0.0,
        lambda_overflow: 0.0,
    })
    .unwrap()
}

fn s7_switch_test_applicability() -> LossTermApplicability {
    LossTermApplicability {
        distill: true,
        balance: false,
        zrouter: true,
        switch: true,
        range: false,
        zero: false,
        shape: false,
        overflow: false,
    }
}

fn assert_finite_nonzero(values: &[f32], label: &str) {
    assert!(
        values.iter().all(|value| value.is_finite()),
        "{label} gradient must be finite: {values:?}"
    );
    assert!(
        values.iter().any(|value| value.abs() > GRAD_TOLERANCE),
        "{label} gradient must be non-zero: {values:?}"
    );
}

fn assert_close(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() <= VALUE_TOLERANCE,
        "expected {actual} to be within {VALUE_TOLERANCE} of {expected}"
    );
}
