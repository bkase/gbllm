#![cfg(all(feature = "s7", feature = "falsify"))]

mod common;

#[path = "falsification/f1_router_top_k_ge_2.rs"]
mod f1_router_top_k_ge_2;
#[path = "falsification/f2_bytes_unscaled.rs"]
mod f2_bytes_unscaled;
#[path = "falsification/f3_pareto_unequal_bytes.rs"]
mod f3_pareto_unequal_bytes;
#[path = "falsification/f4_switch_grad_router_only.rs"]
mod f4_switch_grad_router_only;
#[path = "falsification/f5_z_uncentered.rs"]
mod f5_z_uncentered;
#[path = "falsification/f6_balance_no_stop_grad.rs"]
mod f6_balance_no_stop_grad;
#[path = "falsification/f7_window_one.rs"]
mod f7_window_one;
#[path = "falsification/f8_sweep_constant_lambda.rs"]
mod f8_sweep_constant_lambda;
#[path = "falsification/f9_expert_block_qat_grad_dead.rs"]
mod f9_expert_block_qat_grad_dead;

use common::tracing_capture::{TraceCapture, captured_events, with_trace_capture};
use gbf_experiments::s7::falsify::{
    S7_FALSIFICATION_CASE_COUNT, S7_FALSIFICATION_CASE_EVENT, S7FalsificationCase,
    S7FalsificationObservation,
};
use gbf_experiments::s7::outcome::{S7Decision, S7Outcome};
use serde_json::json;

pub(crate) fn assert_s7_case(
    case: S7FalsificationCase,
    expected_outcome: S7Outcome,
    expected_decision: S7Decision,
    run_case: impl FnOnce() -> S7FalsificationObservation,
) -> S7FalsificationObservation {
    let capture = TraceCapture::default();
    let observation = with_trace_capture(&capture, run_case);

    assert_eq!(observation.case(), case);
    assert_eq!(observation.evidence().case(), case);
    assert!(
        observation.evidence().refutes_expected(),
        "broken substitute evidence did not refute expected clause: {observation:#?}"
    );
    assert_eq!(observation.observed_verdict(), "Refuted");
    assert_eq!(observation.outcome(), expected_outcome);
    assert_eq!(observation.decision(), expected_decision);
    assert!(
        observation.matches_expected(),
        "{} did not produce the expected O5 mapping: {observation:#?}",
        case.substitute_id()
    );

    let events = captured_events(&capture)
        .into_iter()
        .filter(|event| event.name == S7_FALSIFICATION_CASE_EVENT)
        .collect::<Vec<_>>();
    assert_eq!(events.len(), 1, "case event should fire once: {events:#?}");

    let event = &events[0];
    assert_eq!(event.level, "INFO");
    assert_eq!(event.fields.get("case"), Some(&json!(case.case())));
    assert_eq!(
        event.fields.get("substitute_id"),
        Some(&json!(case.substitute_id()))
    );
    assert_eq!(
        event.fields.get("hypothesis"),
        Some(&json!(case.hypothesis()))
    );
    assert_eq!(
        event.fields.get("expected_verdict"),
        Some(&json!(case.expected_verdict()))
    );
    assert_eq!(
        event.fields.get("observed_verdict"),
        Some(&json!("Refuted"))
    );
    assert_eq!(
        event.fields.get("falsification_clause"),
        Some(&json!(case.falsification_clause()))
    );
    assert_eq!(
        event.fields.get("diagnostic"),
        Some(&json!(case.diagnostic()))
    );
    assert!(
        event.fields.get("evidence").is_some(),
        "event should include the debug evidence payload: {event:#?}"
    );
    assert!(
        event.fields.get("diagnostic").is_some_and(|value| value
            .as_str()
            .is_some_and(|diagnostic| !diagnostic.is_empty())),
        "diagnostic should be present and non-empty: {event:#?}"
    );

    observation
}

#[test]
fn s7_falsification_catalog_matches_rfc_o5_order() {
    assert_eq!(S7FalsificationCase::ALL.len(), S7_FALSIFICATION_CASE_COUNT);
    assert_eq!(
        S7FalsificationCase::ALL
            .iter()
            .map(|case| case.substitute_id())
            .collect::<Vec<_>>(),
        vec![
            "F1-router-top-k-ge-2",
            "F2-bytes-unscaled",
            "F3-pareto-unequal-bytes",
            "F4-switch-grad-router-only",
            "F5-z-uncentered",
            "F6-balance-no-stop-grad",
            "F7-window-one",
            "F8-sweep-constant-lambda",
            "F9-expert-block-qat-grad-dead",
        ]
    );
}
