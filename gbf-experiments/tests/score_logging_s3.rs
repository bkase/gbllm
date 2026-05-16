#![cfg(feature = "s3")]

mod common;
mod score_s3_support;

use common::tracing_capture::{TraceCapture, captured_events, with_trace_capture};
use gbf_experiments::s3::score::{S3_SCORE_CHUNK_SIZE, s3_score_bpc_char};
use score_s3_support::{UniformEvaluator, repeated_a};

#[test]
fn score_logging_emits_started_chunk_and_complete_events() {
    let capture = TraceCapture::default();
    let val = repeated_a(129);
    let product = with_trace_capture(&capture, || {
        s3_score_bpc_char(UniformEvaluator, &val, S3_SCORE_CHUNK_SIZE)
    });
    let events = captured_events(&capture);

    assert_event(&events, "s3::score::started");
    assert_event(&events, "s3::score::chunk_complete");
    assert_event(&events, "s3::score::complete");

    let chunk_events = events
        .iter()
        .filter(|event| event.name == "s3::score::chunk_complete")
        .collect::<Vec<_>>();
    assert_eq!(chunk_events.len(), 2);

    let complete = events
        .iter()
        .find(|event| event.name == "s3::score::complete")
        .expect("complete event emitted");
    assert_eq!(
        complete.fields.get("score_self_hash"),
        Some(&serde_json::json!(product.score_self_hash.to_string()))
    );
}

fn assert_event(events: &[common::tracing_capture::TracingEvent], name: &str) {
    assert!(
        events.iter().any(|event| event.name == name),
        "missing event {name}; saw {:?}",
        events.iter().map(|event| &event.name).collect::<Vec<_>>()
    );
}
