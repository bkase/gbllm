use gbf_experiments::s7::falsify::{
    S7FalsificationCase, S7FalsificationEvidence, f1_router_top_k_ge_2,
};
use gbf_experiments::s7::outcome::{S7Decision, S7Outcome};
use gbf_model::qat::{RouterForwardOptions, RouterShape, RouterTrainMode, Top1RouterQat};

#[test]
fn f1_router_top_k_ge_2_refutes_h1() {
    let shape = RouterShape::new(2, 3, 1).expect("router shape");
    let router = Top1RouterQat::new(shape, vec![1.0, -1.0], None, vec![1.0, 0.0, -1.0], None)
        .expect("top-1 router");
    let options = RouterForwardOptions::hard_top1(3);
    assert_eq!(options.mode(), RouterTrainMode::HardTop1);

    let output = router
        .forward_stateless(&[0.25, -0.5], None, &options)
        .expect("hard top-1 forward");
    assert_eq!(
        output
            .dispatch_indicator()
            .iter()
            .filter(|weight| **weight == 1.0)
            .count(),
        1,
        "public S7 router execution must remain top-1"
    );

    let evidence = f1_router_top_k_ge_2::broken_substitute();
    assert!(matches!(
        evidence,
        S7FalsificationEvidence::RouterTopKGe2 {
            requested_top_k: 2,
            constructed: true,
            dispatch_weight_count: 2,
        }
    ));
    assert!(evidence.refutes_expected());

    crate::assert_s7_case(
        S7FalsificationCase::RouterTopKGe2,
        S7Outcome::FailMoeTrain,
        S7Decision::Investigate {
            reason: "burn-or-loss-substrate",
        },
        f1_router_top_k_ge_2::run,
    );
}
