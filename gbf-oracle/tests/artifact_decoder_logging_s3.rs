#![cfg(feature = "s3-real")]

mod artifact_support;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use artifact_support::{fixture_artifact, fixture_prompt};
use gbf_oracle::artifact::ArtifactDecoder;
use gbf_oracle::artifact::decoder::{
    ARTIFACT_DECODER_LOG_TARGET, EVENT_NAME_DECODE_COMPLETE, EVENT_NAME_DECODE_STARTED,
    EVENT_NAME_DECODE_STEP,
};
use serde::Serialize;
use tracing_subscriber::layer::Context;
use tracing_subscriber::prelude::*;

#[test]
fn artifact_decoder_logging_emits_required_events() {
    let artifact = fixture_artifact();
    let prompt = fixture_prompt();
    let (result, events) =
        capture_events(|| ArtifactDecoder::new(&artifact).decode_argmax(&prompt, 4, false));
    write_capture_if_requested(&events);

    let started = event_by_name(&events, EVENT_NAME_DECODE_STARTED);
    let expected_prompt_len = prompt.len().to_string();
    assert_eq!(started.target, ARTIFACT_DECODER_LOG_TARGET);
    assert_eq!(started.level, "INFO");
    assert_eq!(
        started.fields.get("prompt_char_count").map(String::as_str),
        Some(expected_prompt_len.as_str())
    );
    assert_eq!(
        started.fields.get("max_chars").map(String::as_str),
        Some("4")
    );
    assert_eq!(
        started.fields.get("stop_on_eos").map(String::as_str),
        Some("false")
    );

    let steps = events
        .iter()
        .filter(|event| {
            event.fields.get("event_name").map(String::as_str) == Some(EVENT_NAME_DECODE_STEP)
        })
        .collect::<Vec<_>>();
    assert_eq!(steps.len(), result.decode_log.len());
    assert!(steps.iter().all(|event| event.level == "TRACE"));

    let complete = event_by_name(&events, EVENT_NAME_DECODE_COMPLETE);
    let expected_generated_len = result.generated.len().to_string();
    let expected_terminal = result.terminal_eos_seen.to_string();
    assert_eq!(complete.level, "INFO");
    assert_eq!(
        complete
            .fields
            .get("generated_char_count")
            .map(String::as_str),
        Some(expected_generated_len.as_str())
    );
    assert_eq!(
        complete.fields.get("terminal_eos_seen").map(String::as_str),
        Some(expected_terminal.as_str())
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

fn capture_events<R>(f: impl FnOnce() -> R) -> (R, Vec<CapturedEvent>) {
    let capture = TraceCapture::default();
    let subscriber = tracing_subscriber::registry()
        .with(tracing_subscriber::filter::LevelFilter::TRACE)
        .with(capture.clone());
    let result = tracing::subscriber::with_default(subscriber, f);
    let events = capture.events.lock().expect("trace capture mutex").clone();
    (result, events)
}

fn event_by_name<'a>(events: &'a [CapturedEvent], name: &str) -> &'a CapturedEvent {
    events
        .iter()
        .find(|event| event.fields.get("event_name").map(String::as_str) == Some(name))
        .unwrap_or_else(|| panic!("missing event {name:?}; saw {events:#?}"))
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

    fn record_f64(&mut self, field: &tracing::field::Field, value: f64) {
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
    let Ok(path) = std::env::var("S3_ARTIFACT_DECODER_CAPTURE_EVENTS") else {
        return;
    };
    let mut lines = String::new();
    for event in events {
        lines.push_str(&serde_json::to_string(event).expect("event serializes"));
        lines.push('\n');
    }
    std::fs::write(path, lines).expect("writes artifact decoder events");
}
