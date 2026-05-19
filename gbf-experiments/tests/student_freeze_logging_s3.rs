#![cfg(feature = "s3")]

mod common;
mod common_s3;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use common_s3::fixtures::ToyHardTernaryStudent;
use gbf_experiments::s3::schema::{S3PhaseLogEvent, emit_s3_phase_log_event};
use gbf_train::logging::{EVENT_NAME_STUDENT_FREEZE, TrainingLogEmitter};
use gbf_train::student::StudentFreezeGuard;
use serde::Serialize;
use tracing_subscriber::layer::Context;
use tracing_subscriber::prelude::*;

#[test]
fn student_freeze_logging_emits_train_and_phase_log_events_once() {
    let events = capture_events(|| {
        let student = ToyHardTernaryStudent::new(vec![1.0, 0.0, -1.0, 1.0], true);
        let mut guard = StudentFreezeGuard::new();
        let emitter = TrainingLogEmitter::new();

        let frozen = guard
            .freeze_with_logging(&student, &emitter)
            .expect("student freeze succeeds");
        let phase_event = S3PhaseLogEvent::student_freeze(
            frozen.storage_fingerprint().to_hex(),
            frozen.weight_fingerprint().to_hex(),
        )
        .expect("phase-log student_freeze event builds");
        emit_s3_phase_log_event(&phase_event).expect("phase-log event emits");
    });
    write_capture_if_requested(&events);

    let student_freeze = events
        .iter()
        .filter(|event| {
            event.fields.get("event_name").map(String::as_str) == Some(EVENT_NAME_STUDENT_FREEZE)
        })
        .collect::<Vec<_>>();
    assert_eq!(student_freeze.len(), 1);
    assert_eq!(student_freeze[0].target, "gbf_train::student");
    assert_eq!(
        student_freeze[0].fields.get("step").map(String::as_str),
        Some("10001")
    );
    assert!(
        student_freeze[0]
            .fields
            .get("student_storage_fingerprint")
            .is_some_and(|value| !value.is_empty())
    );
    assert!(
        student_freeze[0]
            .fields
            .get("student_weight_fingerprint")
            .is_some_and(|value| !value.is_empty())
    );
    assert!(
        student_freeze[0]
            .fields
            .get("source_storage_identity")
            .is_some_and(|value| !value.is_empty())
    );
    assert!(
        student_freeze[0]
            .fields
            .get("frozen_storage_identity")
            .is_some_and(|value| !value.is_empty())
    );

    let phase_log = events
        .iter()
        .filter(|event| {
            event.fields.get("event_name").map(String::as_str) == Some("s3_phase_log.v1")
        })
        .collect::<Vec<_>>();
    assert_eq!(phase_log.len(), 1);
    assert_eq!(phase_log[0].target, "gbf_experiments::s3");
    assert_eq!(
        phase_log[0].fields.get("schema").map(String::as_str),
        Some("s3_phase_log.v1")
    );
    assert_eq!(
        phase_log[0].fields.get("event_kind").map(String::as_str),
        Some("student_freeze")
    );
    assert_eq!(
        phase_log[0].fields.get("step").map(String::as_str),
        Some("10001")
    );
    assert_eq!(
        phase_log[0].fields.get("student_storage_fingerprint"),
        student_freeze[0].fields.get("student_storage_fingerprint")
    );
    assert_eq!(
        phase_log[0].fields.get("student_weight_fingerprint"),
        student_freeze[0].fields.get("student_weight_fingerprint")
    );
}

#[derive(Clone, Debug, Default)]
struct TraceCapture {
    events: Arc<Mutex<Vec<CapturedEvent>>>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct CapturedEvent {
    target: String,
    level: String,
    fields: BTreeMap<String, String>,
}

impl<S> tracing_subscriber::layer::Layer<S> for TraceCapture
where
    S: tracing::Subscriber,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = FieldVisitor::default();
        event.record(&mut visitor);
        self.events
            .lock()
            .expect("trace capture mutex")
            .push(CapturedEvent {
                target: event.metadata().target().to_owned(),
                level: event.metadata().level().to_string(),
                fields: visitor.fields,
            });
    }
}

fn capture_events(f: impl FnOnce()) -> Vec<CapturedEvent> {
    let capture = TraceCapture::default();
    let subscriber = tracing_subscriber::registry().with(capture.clone());
    tracing::subscriber::with_default(subscriber, f);
    capture.events.lock().expect("trace capture mutex").clone()
}

#[derive(Debug, Default)]
struct FieldVisitor {
    fields: BTreeMap<String, String>,
}

impl FieldVisitor {
    fn insert(&mut self, field: &tracing::field::Field, value: String) {
        self.fields.insert(field.name().to_owned(), value);
    }
}

impl tracing::field::Visit for FieldVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.insert(field, trim_debug_string(format!("{value:?}")));
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.insert(field, value.to_owned());
    }

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.insert(field, value.to_string());
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.insert(field, value.to_string());
    }

    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.insert(field, value.to_string());
    }
}

fn trim_debug_string(value: String) -> String {
    value
        .strip_prefix('"')
        .and_then(|stripped| stripped.strip_suffix('"'))
        .unwrap_or(&value)
        .to_owned()
}

fn write_capture_if_requested(events: &[CapturedEvent]) {
    let Ok(path) = std::env::var("S3_STUDENT_FREEZE_CAPTURE_EVENTS") else {
        return;
    };
    let mut lines = String::new();
    for event in events {
        lines.push_str(&serde_json::to_string(event).expect("event serializes"));
        lines.push('\n');
    }
    std::fs::write(path, lines).expect("writes captured student-freeze events");
}
