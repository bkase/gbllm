#![cfg(feature = "falsify")]

mod common;

#[path = "falsification_s2/f1.rs"]
mod f1;
#[path = "falsification_s2/f2.rs"]
mod f2;
#[path = "falsification_s2/f3.rs"]
mod f3;
#[path = "falsification_s2/f4.rs"]
mod f4;
#[path = "falsification_s2/f5.rs"]
mod f5;
#[path = "falsification_s2/f6.rs"]
mod f6;

use common::tracing_capture::{TraceCapture, captured_events, with_trace_capture};
use gbf_experiments::s2::falsify::{
    BrokenKind, FalsificationCaseResult, install_broken_impl, log_test_done, log_test_start,
};
use serde_json::json;

fn run_logged(
    kind: BrokenKind,
    f: impl FnOnce() -> FalsificationCaseResult,
) -> FalsificationCaseResult {
    log_test_start(kind);
    let result = {
        let _guard = install_broken_impl(kind);
        f()
    };
    log_test_done(&result);
    result
}

fn assert_case(result: FalsificationCaseResult) {
    assert!(
        result.passed,
        "{} did not produce {}: observed {}",
        result.broken_kind, result.expected_verdict, result.observed_verdict
    );
}

#[test]
fn falsification_s2_all_six_emit_done_events_and_pass() {
    let capture = TraceCapture::default();
    let results = with_trace_capture(&capture, || {
        vec![
            f1::run(),
            f2::run(),
            f3::run(),
            f4::run(),
            f5::run(),
            f6::run(),
        ]
    });

    assert_eq!(results.len(), 6);
    assert!(
        results.iter().all(|result| result.passed),
        "all falsification cases must pass: {results:#?}"
    );
    let events = captured_events(&capture);
    let done = events
        .iter()
        .filter(|event| event.name == "falsify_test_done")
        .collect::<Vec<_>>();
    assert_eq!(done.len(), 6, "done events: {done:#?}");
    assert!(
        done.iter()
            .all(|event| event.fields.get("passed") == Some(&json!(true)))
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.name == "falsify_flag_active")
            .count(),
        6
    );
    assert!(
        events
            .iter()
            .all(|event| event.name != "falsify_verifier_insensitive"),
        "passing falsification suite must not emit verifier-insensitive events: {events:#?}"
    );
}
