#![cfg(feature = "s3")]

mod common;
mod common_s3;
mod report_s3_support;

use std::fs;

use common_s3::helpers::tracing_capture_s3::{capture_events, events_to_ndjson};
use gbf_experiments::s3::report::{
    EVENT_NAME_EMISSION_COMPLETE, EVENT_NAME_EMISSION_STARTED, EVENT_NAME_R_VALIDATOR_PASSED,
    emit_report,
};
use serde_json::json;

#[test]
fn report_emitter_logs_emission_and_validator_events() {
    let report = report_s3_support::pass_clean_report();
    let (bytes, events) = capture_events(|| emit_report(&report).expect("report emits"));
    write_capture_if_requested(&events);

    assert!(
        String::from_utf8(bytes)
            .expect("markdown")
            .contains("s3_report.v1")
    );
    assert!(events.iter().any(|event| {
        event.name == EVENT_NAME_EMISSION_STARTED
            && event.fields.get("s3_outcome") == Some(&json!("Pass-clean"))
    }));
    let validators = events
        .iter()
        .filter(|event| event.name == EVENT_NAME_R_VALIDATOR_PASSED)
        .collect::<Vec<_>>();
    assert_eq!(validators.len(), 7);
    let complete = events
        .iter()
        .find(|event| event.name == EVENT_NAME_EMISSION_COMPLETE)
        .expect("emission_complete emitted");
    assert_eq!(
        complete.fields.get("report_self_hash"),
        Some(&json!(report.front_matter.report_self_hash.to_string()))
    );
}

fn write_capture_if_requested(events: &[common::tracing_capture::TracingEvent]) {
    let Ok(path) = std::env::var("S3_REPORT_CAPTURE_EVENTS") else {
        return;
    };
    fs::write(path, events_to_ndjson(events)).expect("writes S3 report event capture");
}
