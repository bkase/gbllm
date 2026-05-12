#![cfg(feature = "burn-adapter")]

use gbf_train::adapter::burn::{
    BurnDevice, BurnNdArrayAutodiffBackend, float_tensor_from_vec, float_tensor_into_vec,
};
use gbf_train::loss::composer::{
    BurnLossTerms, InertClassification, LossTermApplicability, PhaseEffectiveLossWeights,
    PhaseEffectiveLossWeightsValues, TrainingLossUnit, burn_compose,
};

#[test]
fn composer_burn_s2_total_loss_backward_is_finite() {
    type B = BurnNdArrayAutodiffBackend;

    let device = BurnDevice::<B>::default();
    let lm_loss = float_tensor_from_vec::<B, 1>(vec![0.5], [1], &device)
        .unwrap()
        .require_grad();
    let distill_loss = float_tensor_from_vec::<B, 1>(vec![2.0], [1], &device)
        .unwrap()
        .require_grad();
    let range_loss = float_tensor_from_vec::<B, 1>(vec![0.25], [1], &device)
        .unwrap()
        .require_grad();
    let zero_loss = float_tensor_from_vec::<B, 1>(vec![0.75], [1], &device)
        .unwrap()
        .require_grad();

    let composed = burn_compose(
        BurnLossTerms {
            lm_loss_next_byte_nats: lm_loss.clone(),
            distill_loss_raw_nats: Some(distill_loss.clone()),
            balance_loss_raw: None,
            zrouter_loss_raw: None,
            switch_loss_raw: None,
            range_loss_raw: Some(range_loss.clone()),
            zero_loss_raw: Some(zero_loss.clone()),
            shape_loss_raw: None,
            overflow_loss_raw: None,
        },
        PhaseEffectiveLossWeights::new(PhaseEffectiveLossWeightsValues {
            lambda_distill: 0.25,
            lambda_balance: 0.0,
            lambda_zrouter: 0.0,
            lambda_switch: 0.0,
            lambda_range: 0.5,
            lambda_zero: 0.125,
            lambda_shape: 0.0,
            lambda_overflow: 0.0,
        })
        .unwrap(),
        LossTermApplicability::toy0_phase_cd(),
        TrainingLossUnit::Nats,
    )
    .unwrap();

    assert_eq!(composed.scalar.total_loss, 1.21875);
    assert_eq!(
        composed.scalar.inert_classification.distill,
        InertClassification::Enabled {
            raw: 2.0,
            weighted: 0.5
        }
    );
    assert_eq!(
        composed.scalar.inert_classification.balance,
        InertClassification::StructurallyInert
    );
    assert_eq!(
        float_tensor_into_vec(composed.weighted.range.unwrap()).unwrap(),
        vec![0.125]
    );

    let gradients = composed.total_loss.sum().backward();
    let grad = float_tensor_into_vec(lm_loss.grad(&gradients).unwrap()).unwrap();
    let distill_grad = float_tensor_into_vec(distill_loss.grad(&gradients).unwrap()).unwrap();
    let range_grad = float_tensor_into_vec(range_loss.grad(&gradients).unwrap()).unwrap();
    let zero_grad = float_tensor_into_vec(zero_loss.grad(&gradients).unwrap()).unwrap();

    assert_eq!(grad, vec![1.0]);
    assert_eq!(distill_grad, vec![0.25]);
    assert_eq!(range_grad, vec![0.5]);
    assert_eq!(zero_grad, vec![0.125]);
    assert!(
        grad.iter()
            .chain(distill_grad.iter())
            .chain(range_grad.iter())
            .chain(zero_grad.iter())
            .all(|value| value.is_finite())
    );
}
