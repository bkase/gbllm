use gbf_experiments::s7::falsify::{
    S7FalsificationCase, S7FalsificationEvidence, f6_balance_no_stop_grad,
};
use gbf_experiments::s7::outcome::{S7Decision, S7Outcome};
use gbf_train::adapter::burn::{
    BurnDevice, BurnNdArrayAutodiffBackend, float_tensor_from_vec, float_tensor_into_vec,
};
use gbf_train::loss::router::burn_load_balance_loss_with_stop_gradient_dispatch;

type B = BurnNdArrayAutodiffBackend;

#[test]
fn f6_balance_no_stop_grad_refutes_h7() {
    let device = BurnDevice::<B>::default();
    let routing_probs = float_tensor_from_vec::<B, 2>(vec![0.8, 0.2, 0.3, 0.7], [2, 2], &device)
        .expect("routing probs")
        .require_grad();
    let dispatch_indicator =
        float_tensor_from_vec::<B, 2>(vec![1.0, 0.0, 0.0, 1.0], [2, 2], &device)
            .expect("dispatch indicator")
            .require_grad();

    let loss = burn_load_balance_loss_with_stop_gradient_dispatch(
        routing_probs.clone(),
        dispatch_indicator.clone(),
    )
    .expect("load-balance loss")
        + dispatch_indicator.clone().sum() * 0.0;
    let gradients = loss.backward();
    let prob_grad = float_tensor_into_vec(
        routing_probs
            .grad(&gradients)
            .expect("routing_probs should receive gradient"),
    )
    .expect("routing gradient values");
    let dispatch_grad = float_tensor_into_vec(
        dispatch_indicator
            .grad(&gradients)
            .expect("dispatch_indicator should carry an explicit zero gradient"),
    )
    .expect("dispatch gradient values");

    assert!(prob_grad.iter().any(|value| value.abs() > 0.0));
    assert!(
        dispatch_grad.iter().all(|value| *value == 0.0),
        "dispatch provenance must be stop-gradient: {dispatch_grad:?}"
    );

    let evidence = f6_balance_no_stop_grad::broken_substitute();
    assert!(matches!(
        evidence,
        S7FalsificationEvidence::BalanceNoStopGrad {
            routing_probs_grad_nonzero: true,
            dispatch_indicator_grad_leaked: true,
        }
    ));
    assert!(evidence.refutes_expected());

    crate::assert_s7_case(
        S7FalsificationCase::BalanceNoStopGrad,
        S7Outcome::FailGradProvenance,
        S7Decision::Halt {
            reason: "loss-math-dishonest",
        },
        f6_balance_no_stop_grad::run,
    );
}
