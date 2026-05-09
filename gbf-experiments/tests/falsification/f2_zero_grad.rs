use gbf_experiments::s1::report::Hypothesis;
use gbf_experiments::s1::run::{
    RunProduct, RunTestOptions, s1_train_run_with_environment_and_options,
};
use gbf_experiments::s1::schema::{S1Completion, S1Outcome};

#[test]
fn f2_zero_grad_refutes_h1_and_fails_substrate() {
    let product = s1_train_run_with_environment_and_options(
        crate::integration_inputs(0),
        crate::canonical_env(),
        RunTestOptions {
            zero_gradients: true,
            ..RunTestOptions::default()
        },
    )
    .expect("zero-gradient substitute produces completed run product");

    let RunProduct::Completed(product) = product else {
        panic!("F2 zero-gradient substitute should complete with flat gradients");
    };
    assert_eq!(product.completion, S1Completion::Completed);
    assert!(
        product
            .grad_log
            .iter()
            .all(|point| point.grad_norm_l2 == 0.0),
        "zero-gradient seam must zero every recorded gradient norm"
    );

    assert_eq!(
        product
            .weight_stats
            .first()
            .map(|point| point.tensor_payload_hash),
        product
            .weight_stats
            .last()
            .map(|point| point.tensor_payload_hash),
        "zero-gradient seam must leave fixture trainable weights unchanged"
    );

    crate::assert_falsification_outcome(
        "F2",
        crate::refute(crate::confirmed_input(), Hypothesis::H1),
        S1Outcome::FailSubstrate,
        crate::fail_substrate_decision(),
    );
}
