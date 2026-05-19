#![cfg(feature = "s3")]

mod common;

use common::tracing_capture::{TraceCapture, captured_events, with_trace_capture};
use gbf_experiments::s3::report::dispatcher::{
    EVENT_NAME_DISPATCH_COMPLETE, EVENT_NAME_DISPATCH_STARTED, dispatch,
};
use gbf_experiments::s3::schema::{S3Decision, S3Outcome, S3VerifierBundle};
use serde_json::json;

#[test]
fn closure_candidate_dispatches_to_pass_clean_and_proceed() {
    let capture = TraceCapture::default();
    let bundle = S3VerifierBundle::closure_candidate();

    let (outcome, decision) = with_trace_capture(&capture, || dispatch(&bundle));

    assert_eq!(outcome, S3Outcome::PassClean);
    assert_eq!(decision, S3Decision::ProceedToS4);
    let events = captured_events(&capture);
    assert!(
        events
            .iter()
            .any(|event| event.name == EVENT_NAME_DISPATCH_STARTED)
    );
    let complete = events
        .iter()
        .find(|event| event.name == EVENT_NAME_DISPATCH_COMPLETE)
        .expect("dispatch_complete event emitted");
    assert_eq!(
        complete.fields.get("s3_outcome"),
        Some(&json!("Pass-clean"))
    );
    assert_eq!(
        complete.fields.get("s3_decision"),
        Some(&json!("ProceedToS4"))
    );
}
