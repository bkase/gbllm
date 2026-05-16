#![cfg(feature = "falsify")]

mod common;

#[path = "falsification_s3/f1.rs"]
mod f1;
#[path = "falsification_s3/f2.rs"]
mod f2;
#[path = "falsification_s3/f3.rs"]
mod f3;
#[path = "falsification_s3/f4.rs"]
mod f4;
#[path = "falsification_s3/f5.rs"]
mod f5;
#[path = "falsification_s3/f6.rs"]
mod f6;
#[path = "falsification_s3/f7.rs"]
mod f7;
#[path = "falsification_s3/f8.rs"]
mod f8;
#[path = "falsification_s3/f9.rs"]
mod f9;

use common::tracing_capture::{TraceCapture, captured_events, with_trace_capture};
use gbf_experiments::s3::falsify::{
    BrokenKind, EVENT_NAME_SUBSTITUTE_COMPLETE, EVENT_NAME_SUBSTITUTE_RUN,
    EVENT_NAME_SUITE_COMPLETE, EVENT_NAME_SUITE_STARTED, FALSIFICATION_S3_SUITE_HASH,
    FalsificationCaseResult, SUBSTITUTE_COUNT, install_broken_impl, log_substitute_complete,
    log_substitute_run, log_suite_complete, log_suite_started, pinned_suite_hash,
};
use serde_json::json;

fn run_logged(
    kind: BrokenKind,
    f: impl FnOnce() -> FalsificationCaseResult,
) -> FalsificationCaseResult {
    log_substitute_run(kind);
    let result = {
        let _guard = install_broken_impl(kind);
        f()
    };
    log_substitute_complete(&result);
    result
}

fn assert_case(result: FalsificationCaseResult) {
    assert!(
        result.matches_expected,
        "{} did not produce {}: observed {}",
        result.substitute_name, result.expected_verdict, result.observed_verdict
    );
}

#[test]
fn falsification_s3_all_nine_emit_events_and_refute_targets() {
    let capture = TraceCapture::default();
    let results = with_trace_capture(&capture, || {
        log_suite_started(pinned_suite_hash());
        let results = vec![
            f1::run(),
            f2::run(),
            f3::run(),
            f4::run(),
            f5::run(),
            f6::run(),
            f7::run(),
            f8::run(),
            f9::run(),
        ];
        let suite_passed = results.iter().all(|result| result.matches_expected);
        log_suite_complete(suite_passed, suite_passed);
        results
    });

    assert_eq!(results.len(), SUBSTITUTE_COUNT);
    assert!(
        results.iter().all(|result| result.matches_expected),
        "all falsification cases must pass: {results:#?}"
    );

    let events = captured_events(&capture);
    let starts = events
        .iter()
        .filter(|event| event.name == EVENT_NAME_SUITE_STARTED)
        .collect::<Vec<_>>();
    assert_eq!(starts.len(), 1, "suite_started events: {starts:#?}");
    assert_eq!(
        starts[0].fields.get("substitute_count"),
        Some(&json!(SUBSTITUTE_COUNT as u64))
    );
    assert_eq!(
        starts[0].fields.get("suite_hash"),
        Some(&json!(FALSIFICATION_S3_SUITE_HASH))
    );

    let runs = events
        .iter()
        .filter(|event| event.name == EVENT_NAME_SUBSTITUTE_RUN)
        .collect::<Vec<_>>();
    assert_eq!(runs.len(), SUBSTITUTE_COUNT, "run events: {runs:#?}");
    assert!(
        runs.iter()
            .all(|event| event.fields.get("expected_verdict").is_some())
    );

    let completions = events
        .iter()
        .filter(|event| event.name == EVENT_NAME_SUBSTITUTE_COMPLETE)
        .collect::<Vec<_>>();
    assert_eq!(
        completions.len(),
        SUBSTITUTE_COUNT,
        "completion events: {completions:#?}"
    );
    assert!(
        completions
            .iter()
            .all(|event| event.fields.get("matches_expected") == Some(&json!(true)))
    );

    let suite_complete = events
        .iter()
        .find(|event| event.name == EVENT_NAME_SUITE_COMPLETE)
        .expect("suite_complete event emitted");
    assert_eq!(
        suite_complete.fields.get("all_substitutes_refuted_target"),
        Some(&json!(true))
    );
    assert_eq!(
        suite_complete.fields.get("suite_passed"),
        Some(&json!(true))
    );
}
