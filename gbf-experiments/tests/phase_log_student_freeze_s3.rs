#![cfg(feature = "s3")]

mod common;
mod common_s3;

use common::tracing_capture::{TraceCapture, captured_events, with_trace_capture};
use gbf_experiments::s3::schema::{
    EVENT_NAME_S3_PHASE_LOG, S3_PHASE_LOG_SCHEMA, S3_STUDENT_FREEZE_EVENT_STEP, S3PhaseLogError,
    S3PhaseLogEvent, emit_s3_phase_log_event, s3_phase_log_jsonl_bytes,
};
use serde_json::json;

#[test]
fn student_freeze_phase_log_serializes_canonical_jsonl_row() {
    let event =
        S3PhaseLogEvent::student_freeze("storage-abc", "weight-def").expect("student freeze event");

    let line = event.canonical_json_line().expect("canonical line");

    assert_eq!(
        std::str::from_utf8(&line).expect("utf8"),
        concat!(
            r#"{"event_kind":"student_freeze","schema":"s3_phase_log.v1","#,
            r#""step":10001,"student_storage_fingerprint":"storage-abc","#,
            r#""student_weight_fingerprint":"weight-def"}"#,
            "\n"
        )
    );
    assert_eq!(s3_phase_log_jsonl_bytes(&[event]).expect("jsonl"), line);
}

#[test]
fn student_freeze_phase_log_event_is_subscriber_captured() {
    let event =
        S3PhaseLogEvent::student_freeze("storage-abc", "weight-def").expect("student freeze event");
    let capture = TraceCapture::default();

    with_trace_capture(&capture, || {
        emit_s3_phase_log_event(&event).expect("phase-log event emits");
    });

    let events = captured_events(&capture);
    let phase_events = events
        .iter()
        .filter(|event| event.name == EVENT_NAME_S3_PHASE_LOG)
        .collect::<Vec<_>>();
    assert_eq!(phase_events.len(), 1);
    assert_eq!(
        phase_events[0].fields.get("schema"),
        Some(&json!(S3_PHASE_LOG_SCHEMA))
    );
    assert_eq!(
        phase_events[0].fields.get("event_kind"),
        Some(&json!("student_freeze"))
    );
    assert_eq!(
        phase_events[0].fields.get("step"),
        Some(&json!(S3_STUDENT_FREEZE_EVENT_STEP))
    );
    assert_eq!(
        phase_events[0].fields.get("student_storage_fingerprint"),
        Some(&json!("storage-abc"))
    );
    assert_eq!(
        phase_events[0].fields.get("student_weight_fingerprint"),
        Some(&json!("weight-def"))
    );
}

#[test]
fn student_freeze_phase_log_rejects_missing_fingerprints() {
    let event = S3PhaseLogEvent::StudentFreeze {
        schema: S3_PHASE_LOG_SCHEMA.to_owned(),
        step: S3_STUDENT_FREEZE_EVENT_STEP,
        student_storage_fingerprint: " ".to_owned(),
        student_weight_fingerprint: "weight-def".to_owned(),
    };

    assert!(matches!(
        event.validate().unwrap_err(),
        S3PhaseLogError::EmptyField {
            name: "student_storage_fingerprint"
        }
    ));
}
