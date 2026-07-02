use gbf_experiments::s7::falsify::{
    S7FalsificationCase, S7FalsificationEvidence, f4_switch_grad_router_only,
};
use gbf_experiments::s7::outcome::{S7Decision, S7Outcome};
use gbf_model::loss::temporal_smoothness::SmoothnessWindow;
use gbf_train::adapter::burn::{
    BurnDevice, BurnNdArrayAutodiffBackend, float_tensor_from_vec, float_tensor_into_vec,
};
use gbf_train::loss::switch::burn_temporal_switch_loss;

type B = BurnNdArrayAutodiffBackend;

#[test]
fn f4_switch_grad_router_only_refutes_h7() {
    let device = BurnDevice::<B>::default();
    let routing_probs = float_tensor_from_vec::<B, 4>(
        vec![
            1.0, 0.0, //
            1.0, 0.0, //
            0.0, 1.0, //
            0.0, 1.0, //
        ],
        [1, 4, 1, 2],
        &device,
    )
    .expect("routing probabilities")
    .require_grad();
    let loss = burn_temporal_switch_loss(
        routing_probs.clone(),
        &[true; 4],
        None,
        SmoothnessWindow::new(2).expect("valid window"),
    )
    .expect("L_switch loss");
    let gradients = loss.backward();
    let routing_grad = float_tensor_into_vec(
        routing_probs
            .grad(&gradients)
            .expect("routing probabilities should receive L_switch gradient"),
    )
    .expect("routing probability gradients");
    assert!(
        routing_grad.iter().all(|value| value.is_finite())
            && routing_grad.iter().any(|value| value.abs() > 0.0),
        "production L_switch helper should reach routing probabilities: {routing_grad:?}"
    );

    let evidence = f4_switch_grad_router_only::broken_substitute();
    assert!(matches!(
        evidence,
        S7FalsificationEvidence::SwitchGradRouterOnly {
            routing_probs_grad_nonzero: true,
            low_rank_router_grad_nonzero: false,
        }
    ));
    assert!(evidence.refutes_expected());

    crate::assert_s7_case(
        S7FalsificationCase::SwitchGradRouterOnly,
        S7Outcome::FailGradProvenance,
        S7Decision::Halt {
            reason: "loss-math-dishonest",
        },
        f4_switch_grad_router_only::run,
    );
}
