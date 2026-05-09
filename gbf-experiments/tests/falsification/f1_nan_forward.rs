use serde_json::json;

use gbf_experiments::s1::logging::event;
use gbf_experiments::s1::report::Hypothesis;
use gbf_experiments::s1::run::{
    DivergenceObserved, RunProduct, RunTestOptions, s1_train_run_with_environment_and_options,
};
use gbf_experiments::s1::schema::{S1Completion, S1Outcome};

#[test]
fn f1_nan_forward_refutes_h1_and_fails_substrate() {
    let capture = crate::TraceCapture::default();
    let product = crate::with_trace_capture(&capture, || {
        s1_train_run_with_environment_and_options(
            crate::integration_inputs(0),
            crate::canonical_env(),
            RunTestOptions {
                inject_non_finite_loss_at_step: Some(42),
                inject_non_finite_grad_norm_at_step: None,
                ..RunTestOptions::default()
            },
        )
    })
    .expect("NaN substitute produces diverged run product");

    let RunProduct::Diverged(product) = product else {
        panic!("F1 NaN substitute must diverge");
    };
    assert_eq!(product.completion, S1Completion::DivergedAt { step: 42 });
    assert_eq!(product.divergence_event.step, 42);
    assert_eq!(
        product.divergence_event.observed,
        DivergenceObserved::NonFiniteLoss
    );
    let divergence_events = crate::captured_events(&capture)
        .into_iter()
        .filter(|event| event.name == event::RUN_DIVERGENCE)
        .collect::<Vec<_>>();
    assert_eq!(divergence_events.len(), 1, "{divergence_events:?}");
    assert_eq!(
        divergence_events[0].fields.get("event_name"),
        Some(&json!(event::RUN_DIVERGENCE))
    );
    assert_eq!(divergence_events[0].fields.get("seed"), Some(&json!(0)));
    assert_eq!(divergence_events[0].fields.get("step"), Some(&json!(42)));
    assert_eq!(
        divergence_events[0].fields.get("observed"),
        Some(&json!("non_finite_loss"))
    );

    let mut input = crate::refute(crate::confirmed_input(), Hypothesis::H1);
    input.any_seed_diverged = true;
    crate::assert_falsification_outcome(
        "F1",
        input,
        S1Outcome::FailSubstrate,
        crate::fail_substrate_decision(),
    );
}
