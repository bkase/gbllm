#![cfg(feature = "s3")]

mod common;

use common::tracing_capture::{TraceCapture, captured_events, with_trace_capture};
use gbf_experiments::s3::schema::{S3BuildKind, S3Completion, S3Decision, S3VerifierBundle};

#[test]
fn b7_schema_constructors_emit_no_producer_events_without_phase_d_runtime() {
    if cfg!(feature = "s3-phase-d") {
        return;
    }

    let capture = TraceCapture::default();
    with_trace_capture(&capture, || {
        let _kind = S3BuildKind::s3_v0_success_real_oracle;
        let _decision = S3Decision::ProceedToS4;
        let _completion = S3Completion::Completed;
        let _bundle = S3VerifierBundle::closure_candidate();
    });

    assert!(
        captured_events(&capture).is_empty(),
        "B7 schema constructors must be inert"
    );
}
