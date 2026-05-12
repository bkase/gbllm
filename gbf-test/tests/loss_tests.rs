use gbf_train::adapter::burn::{
    BurnAutodiffBackend, BurnDevice, BurnFloatTensor, BurnNdArrayAutodiffBackend, burn_softmax,
    float_tensor_from_vec, float_tensor_into_vec,
};
use gbf_train::loss::composer::{
    BurnLossTerms, InertClassification, LossTermApplicability, PhaseEffectiveLossWeights,
    PhaseEffectiveLossWeightsValues, TrainingLossUnit, burn_compose,
};
use gbf_train::loss::range::burn_range_loss;
use gbf_train::loss::router::{
    burn_load_balance_loss, burn_router_z_loss, load_balance_loss, router_z_loss,
};
use gbf_train::loss::zero::burn_zero_loss;

type B = BurnNdArrayAutodiffBackend;

const GRAD_TOLERANCE: f32 = 1.0e-6;
// Host-read tensor values from the NdArray autodiff backend should match small
// analytic oracles tightly, but not bit-for-bit across all math intrinsics.
const VALUE_TOLERANCE: f32 = 1.0e-5;
// The extreme z-loss oracle below computes log-sum-exp in f64, while the Burn
// helper differentiates f32 tensor math. Use an absolute tolerance because the
// expected gradient vector deliberately mixes a large ~200 component with
// near-zero tails; a relative tolerance would either over-tighten the tail or
// under-check the dominant slot.
const EXTREME_Z_GRAD_TOLERANCE: f32 = 2.0e-4;

#[test]
fn loss_tests_router_terms_gradient_flow_is_router_only() {
    let device = BurnDevice::<B>::default();

    let router_logits =
        tensor2(vec![1.0, 0.0, -1.0, 0.5, 0.25, -0.25], [2, 3], &device).require_grad();
    let disconnected_expert_weights =
        tensor2(vec![0.1, -0.2, 0.3, -0.4], [2, 2], &device).require_grad();

    let z_loss = burn_router_z_loss(router_logits.clone()).unwrap();
    let z_gradients = z_loss.backward();
    let z_router_grad = gradient_values(router_logits.clone(), &z_gradients);

    assert_finite_nonzero(&z_router_grad, "z-router logits");
    assert!(
        z_router_grad[0] > 0.0 && z_router_grad[2] > 0.0,
        "z-router gradients should reach concrete logits in each row: {z_router_grad:?}"
    );
    // These disconnected require_grad tensors are sentinels: absence means the
    // loss graph never touched them, unlike connected stop-gradient inputs
    // below where the raw helper sees the tensor but should not backpropagate.
    assert!(
        disconnected_expert_weights.grad(&z_gradients).is_none(),
        "disconnected expert-weight sentinel must remain absent from z-router gradients"
    );

    let routing_probs = tensor2(vec![0.75, 0.25, 0.60, 0.40], [2, 2], &device).require_grad();
    let direct_balance = burn_load_balance_loss(routing_probs.clone(), &[0, 0], &device).unwrap();
    let direct_gradients = direct_balance.backward();
    let direct_prob_grad = gradient_values(routing_probs, &direct_gradients);

    assert_close_slice(&direct_prob_grad, &[1.0, 0.0, 1.0, 0.0], GRAD_TOLERANCE);

    let balance_logits = tensor2(vec![2.0, 0.0, 1.5, 0.0], [2, 2], &device).require_grad();
    let disconnected_expert_weights =
        tensor2(vec![0.7, -0.1, 0.2, -0.5], [2, 2], &device).require_grad();
    let routing_probs_from_logits = burn_softmax(balance_logits.clone(), 1);

    let balance_loss = burn_load_balance_loss(routing_probs_from_logits, &[0, 0], &device).unwrap();
    let balance_gradients = balance_loss.backward();
    let balance_router_grad = gradient_values(balance_logits.clone(), &balance_gradients);

    assert_close_slice(
        &balance_router_grad,
        &[0.104_993_63, -0.104_993_63, 0.149_146_47, -0.149_146_47],
        GRAD_TOLERANCE,
    );
    assert!(
        disconnected_expert_weights
            .grad(&balance_gradients)
            .is_none(),
        "disconnected expert-weight sentinel must remain absent from load-balance gradients"
    );
}

#[test]
fn loss_tests_range_and_zero_gradients_are_local_and_boundary_shaped() {
    let device = BurnDevice::<B>::default();

    let activations = tensor2(vec![-1.0, 1.0, -1.25, 1.25], [2, 2], &device).require_grad();
    let disconnected_router_logits =
        tensor2(vec![0.4, -0.4, 0.1, -0.1], [2, 2], &device).require_grad();

    let range_loss = burn_range_loss(activations.clone(), -1.0, 1.0).unwrap();
    let range_gradients = range_loss.backward();
    let activation_grad = gradient_values(activations.clone(), &range_gradients);

    assert_close_slice(&activation_grad, &[0.0, 0.0, -0.25, 0.25], GRAD_TOLERANCE);
    assert!(
        disconnected_router_logits.grad(&range_gradients).is_none(),
        "disconnected router-logit sentinel must remain absent from range gradients"
    );

    let weights =
        tensor2(vec![-0.5, -0.49, 0.49, 0.25, -0.24, 0.24], [2, 3], &device).require_grad();
    let thresholds = tensor1(vec![0.5, 0.25], [2], &device).require_grad();
    let disconnected_activations =
        tensor2(vec![-1.25, 0.0, 0.25, 1.25], [2, 2], &device).require_grad();

    let zero_loss = burn_zero_loss(weights.clone(), thresholds.clone()).unwrap();
    let zero_gradients = zero_loss.backward();
    let weight_grad = gradient_values(weights.clone(), &zero_gradients);

    assert_close_slice(
        &weight_grad,
        &[0.0, -1.0 / 6.0, 1.0 / 6.0, 0.0, -1.0 / 6.0, 1.0 / 6.0],
        GRAD_TOLERANCE,
    );
    // Thresholds are connected to the zero-loss mask, so this proves the
    // intended stop-gradient boundary rather than mere graph disconnection.
    assert!(
        thresholds.grad(&zero_gradients).is_none(),
        "zero loss thresholds define the mask and must remain stop-gradient"
    );
    assert!(
        disconnected_activations.grad(&zero_gradients).is_none(),
        "disconnected activation sentinel must remain absent from zero-loss gradients"
    );
}

#[test]
fn loss_tests_standard_terms_are_independent_and_linear() {
    let device = BurnDevice::<B>::default();

    assert_close(compose_total(0.25, 0.0, 0.0, 0.0, &device), 0.25);
    assert_close(compose_total(0.0, 0.5, 0.0, 0.0, &device), 0.5);
    assert_close(compose_total(0.0, 0.0, 0.75, 0.0, &device), 0.75);
    assert_close(compose_total(0.0, 0.0, 0.0, 1.25, &device), 1.25);
    assert_close(compose_total(0.25, 0.5, 0.75, 1.25, &device), 2.75);

    let lm_loss = scalar_tensor(0.0, &device).require_grad();
    let balance_loss = scalar_tensor(0.25, &device).require_grad();
    let zrouter_loss = scalar_tensor(0.5, &device).require_grad();
    let range_loss = scalar_tensor(0.75, &device).require_grad();
    let zero_loss = scalar_tensor(1.25, &device).require_grad();

    let composed = burn_compose(
        BurnLossTerms {
            lm_loss_next_byte_nats: lm_loss.clone(),
            distill_loss_raw_nats: None,
            balance_loss_raw: Some(balance_loss.clone()),
            zrouter_loss_raw: Some(zrouter_loss.clone()),
            switch_loss_raw: None,
            range_loss_raw: Some(range_loss.clone()),
            zero_loss_raw: Some(zero_loss.clone()),
            shape_loss_raw: None,
            overflow_loss_raw: None,
        },
        weighted_lambdas(0.2, 0.3, 0.4, 0.5),
        standard_term_applicability(),
        TrainingLossUnit::Nats,
    )
    .unwrap();

    assert_close(composed.scalar.total_loss, 1.125);

    let gradients = composed.total_loss.backward();
    assert_close(gradient_values(lm_loss, &gradients)[0], 1.0);
    assert_close(gradient_values(balance_loss, &gradients)[0], 0.2);
    assert_close(gradient_values(zrouter_loss, &gradients)[0], 0.3);
    assert_close(gradient_values(range_loss, &gradients)[0], 0.4);
    assert_close(gradient_values(zero_loss, &gradients)[0], 0.5);
}

#[test]
fn loss_tests_real_helpers_compose_and_backpropagate_to_sources() {
    let device = BurnDevice::<B>::default();

    let lm_loss = scalar_tensor(0.125, &device).require_grad();
    let router_logits =
        tensor2(vec![1.2, -0.4, 0.1, -0.2, 0.8, -0.6], [2, 3], &device).require_grad();
    let balance_logits = tensor2(vec![1.5, 0.25, 1.0, 0.0], [2, 2], &device).require_grad();
    let activations = tensor2(vec![-1.5, -0.25, 0.25, 1.5], [2, 2], &device).require_grad();
    let zero_weights = tensor2(vec![-0.2, 0.6, 0.1, -0.8], [2, 2], &device).require_grad();
    let zero_thresholds = tensor1(vec![0.5, 0.5], [2], &device).require_grad();

    let routing_probs = burn_softmax(balance_logits.clone(), 1);
    let composed = burn_compose(
        BurnLossTerms {
            lm_loss_next_byte_nats: lm_loss.clone(),
            distill_loss_raw_nats: None,
            balance_loss_raw: Some(
                burn_load_balance_loss(routing_probs, &[0, 0], &device).unwrap(),
            ),
            zrouter_loss_raw: Some(burn_router_z_loss(router_logits.clone()).unwrap()),
            switch_loss_raw: None,
            range_loss_raw: Some(burn_range_loss(activations.clone(), -1.0, 1.0).unwrap()),
            zero_loss_raw: Some(
                burn_zero_loss(zero_weights.clone(), zero_thresholds.clone()).unwrap(),
            ),
            shape_loss_raw: None,
            overflow_loss_raw: None,
        },
        weighted_lambdas(0.7, 0.11, 0.13, 0.17),
        standard_term_applicability(),
        TrainingLossUnit::Nats,
    )
    .unwrap();

    assert!(composed.scalar.total_loss > 0.125);

    let gradients = composed.total_loss.backward();
    assert_close(gradient_values(lm_loss, &gradients)[0], 1.0);

    let balance_logit_grad = gradient_values(balance_logits, &gradients);
    assert_finite_nonzero(&balance_logit_grad, "composed balance logits");
    assert_close(balance_logit_grad[0] + balance_logit_grad[1], 0.0);
    assert_close(balance_logit_grad[2] + balance_logit_grad[3], 0.0);

    let zrouter_logit_grad = gradient_values(router_logits, &gradients);
    assert_finite_nonzero(&zrouter_logit_grad, "composed z-router logits");

    let activation_grad = gradient_values(activations, &gradients);
    assert_close_slice(&activation_grad, &[-0.065, 0.0, 0.0, 0.065], GRAD_TOLERANCE);

    let zero_weight_grad = gradient_values(zero_weights, &gradients);
    assert_close_slice(
        &zero_weight_grad,
        &[-0.0425, 0.0, 0.0425, 0.0],
        GRAD_TOLERANCE,
    );
    assert!(
        zero_thresholds.grad(&gradients).is_none(),
        "zero thresholds must remain stop-gradient after composition"
    );
}

#[test]
fn loss_tests_computed_disabled_composer_keeps_raw_but_zeroes_connected_gradients() {
    let device = BurnDevice::<B>::default();

    let lm_loss = scalar_tensor(0.5, &device).require_grad();
    let disabled_activations = tensor2(vec![-1.5, 0.0, 0.0, 1.5], [2, 2], &device).require_grad();
    let inert_logits = tensor2(vec![2.0, 0.0, 0.0, 2.0], [2, 2], &device).require_grad();

    let mut applicability = standard_term_applicability();
    applicability.balance = false;
    applicability.zrouter = false;
    applicability.zero = false;
    let composed = burn_compose(
        BurnLossTerms {
            lm_loss_next_byte_nats: lm_loss.clone(),
            distill_loss_raw_nats: None,
            balance_loss_raw: None,
            zrouter_loss_raw: Some(burn_router_z_loss(inert_logits.clone()).unwrap()),
            switch_loss_raw: None,
            range_loss_raw: Some(burn_range_loss(disabled_activations.clone(), -1.0, 1.0).unwrap()),
            zero_loss_raw: None,
            shape_loss_raw: None,
            overflow_loss_raw: None,
        },
        weighted_lambdas(0.0, 0.5, 0.0, 0.0),
        applicability,
        TrainingLossUnit::Nats,
    )
    .unwrap();

    let InertClassification::ComputedDisabled { raw, weighted } =
        composed.scalar.inert_classification.range
    else {
        panic!(
            "range should be ComputedDisabled, got {:?}",
            composed.scalar.inert_classification.range
        );
    };
    assert_close(raw, 0.25);
    assert_close(weighted, 0.0);
    assert_eq!(
        composed.scalar.inert_classification.zrouter,
        InertClassification::StructurallyInert
    );
    assert_close(composed.scalar.total_loss, 0.5);

    let gradients = composed.total_loss.backward();
    assert_close(gradient_values(lm_loss, &gradients)[0], 1.0);
    let disabled_activation_grad = disabled_activations
        .grad(&gradients)
        .expect("ComputedDisabled terms stay graph-live even when weighted by zero");
    assert_close_slice(
        &float_tensor_into_vec(disabled_activation_grad).unwrap(),
        &[0.0, 0.0, 0.0, 0.0],
        GRAD_TOLERANCE,
    );
    assert!(
        inert_logits.grad(&gradients).is_none(),
        "structurally inert raw helper output is validated but not linked into total loss"
    );
}

#[test]
fn loss_tests_extreme_router_inputs_are_finite() {
    let device = BurnDevice::<B>::default();

    let extreme_logits = vec![200.0, 0.0, -200.0, 121.0, 120.0, 119.0];
    let scalar_z = router_z_loss(&extreme_logits, 3).unwrap();
    assert!(scalar_z.is_finite());

    let expected_z_grad = expected_router_z_gradients(&extreme_logits, 3);
    let burn_logits = tensor2(extreme_logits, [2, 3], &device).require_grad();
    let burn_z = burn_router_z_loss(burn_logits.clone()).unwrap();
    let burn_z_value = float_tensor_into_vec(burn_z.clone()).unwrap()[0];
    assert!(burn_z_value.is_finite());

    let z_gradients = burn_z.backward();
    let z_grad = gradient_values(burn_logits, &z_gradients);
    assert_finite_nonzero(&z_grad, "extreme z-router logits");
    assert_close_slice(&z_grad, &expected_z_grad, EXTREME_Z_GRAD_TOLERANCE);

    let skewed_probs = vec![0.99, 0.01, 0.99, 0.01, 0.99, 0.01, 0.99, 0.01];
    let scalar_balance = load_balance_loss(&skewed_probs, &[0, 0, 0, 0], 2).unwrap();
    assert!(scalar_balance.is_finite());
    assert!(
        scalar_balance > 1.9,
        "99% usage skew should produce a large balance penalty, got {scalar_balance}"
    );

    let burn_probs = tensor2(skewed_probs, [4, 2], &device).require_grad();
    let burn_balance = burn_load_balance_loss(burn_probs.clone(), &[0, 0, 0, 0], &device).unwrap();
    let burn_balance_value = float_tensor_into_vec(burn_balance.clone()).unwrap()[0];
    assert!(burn_balance_value.is_finite());

    let balance_gradients = burn_balance.backward();
    let balance_grad = gradient_values(burn_probs, &balance_gradients);
    assert_close_slice(
        &balance_grad,
        &[0.5, 0.0, 0.5, 0.0, 0.5, 0.0, 0.5, 0.0],
        GRAD_TOLERANCE,
    );
}

fn tensor1(values: Vec<f32>, shape: [usize; 1], device: &BurnDevice<B>) -> BurnFloatTensor<B, 1> {
    float_tensor_from_vec::<B, 1>(values, shape, device).unwrap()
}

fn tensor2(values: Vec<f32>, shape: [usize; 2], device: &BurnDevice<B>) -> BurnFloatTensor<B, 2> {
    float_tensor_from_vec::<B, 2>(values, shape, device).unwrap()
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

fn compose_total(balance: f32, zrouter: f32, range: f32, zero: f32, device: &BurnDevice<B>) -> f32 {
    let composed = burn_compose(
        BurnLossTerms {
            lm_loss_next_byte_nats: scalar_tensor(0.0, device),
            distill_loss_raw_nats: None,
            balance_loss_raw: Some(scalar_tensor(balance, device)),
            zrouter_loss_raw: Some(scalar_tensor(zrouter, device)),
            switch_loss_raw: None,
            range_loss_raw: Some(scalar_tensor(range, device)),
            zero_loss_raw: Some(scalar_tensor(zero, device)),
            shape_loss_raw: None,
            overflow_loss_raw: None,
        },
        weighted_lambdas(
            enabled_lambda(balance),
            enabled_lambda(zrouter),
            enabled_lambda(range),
            enabled_lambda(zero),
        ),
        standard_term_applicability(),
        TrainingLossUnit::Nats,
    )
    .unwrap();

    composed.scalar.total_loss
}

fn enabled_lambda(raw: f32) -> f32 {
    // Isolation checks set lambda=1 only for the raw term under test and 0 for
    // all absent terms. That keeps each single-term total equal to its raw
    // diagnostic while the multi-term row still proves additive composition.
    if raw == 0.0 { 0.0 } else { 1.0 }
}

fn weighted_lambdas(
    lambda_balance: f32,
    lambda_zrouter: f32,
    lambda_range: f32,
    lambda_zero: f32,
) -> PhaseEffectiveLossWeights {
    PhaseEffectiveLossWeights::new(PhaseEffectiveLossWeightsValues {
        lambda_distill: 0.0,
        lambda_balance,
        lambda_zrouter,
        lambda_switch: 0.0,
        lambda_range,
        lambda_zero,
        lambda_shape: 0.0,
        lambda_overflow: 0.0,
    })
    .unwrap()
}

fn standard_term_applicability() -> LossTermApplicability {
    LossTermApplicability {
        distill: false,
        balance: true,
        zrouter: true,
        switch: false,
        range: true,
        zero: true,
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

fn assert_close_slice(actual: &[f32], expected: &[f32], tolerance: f32) {
    assert_eq!(actual.len(), expected.len());
    for (index, (actual, expected)) in actual.iter().zip(expected).enumerate() {
        assert!(
            (*actual - *expected).abs() <= tolerance,
            "index {index}: expected {actual} to be within {tolerance} of {expected}"
        );
    }
}

fn expected_router_z_gradients(logits: &[f32], n_experts: usize) -> Vec<f32> {
    assert_eq!(logits.len() % n_experts, 0);
    let row_count = logits.len() / n_experts;
    let expert_ln = (n_experts as f64).ln();
    let row_scale = 2.0 / row_count as f64;
    let mut expected = Vec::with_capacity(logits.len());

    for row in logits.chunks_exact(n_experts) {
        let max = row.iter().copied().fold(f32::NEG_INFINITY, f32::max) as f64;
        let exp_shifted = row
            .iter()
            .map(|logit| (f64::from(*logit) - max).exp())
            .collect::<Vec<_>>();
        let sum_exp = exp_shifted.iter().sum::<f64>();
        let centered_z = max + sum_exp.ln() - expert_ln;
        expected.extend(
            exp_shifted
                .into_iter()
                .map(|value| (row_scale * centered_z * value / sum_exp) as f32),
        );
    }

    expected
}
