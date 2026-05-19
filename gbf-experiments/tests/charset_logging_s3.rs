use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use gbf_data::charset_v1::{CharsetInputs, s3_charset_v1};
use serde::Serialize;
use tracing_subscriber::layer::Context;
use tracing_subscriber::prelude::*;

#[test]
fn charset_pipeline_logging_emits_started_complete_and_per_example_events() {
    let events = capture_events(|| {
        let _ = s3_charset_v1(CharsetInputs {
            raw_train_examples: vec![b"Alpha".to_vec(), b"Beta".to_vec()],
            raw_val_examples: vec![b"Gamma".to_vec()],
            spec: gbf_artifact::LexicalSpec_v1::pinned(),
        })
        .expect("pipeline normalizes");
    });
    write_capture_if_requested(&events);

    let started = events
        .iter()
        .filter(|event| {
            event.fields.get("event_name").map(String::as_str)
                == Some("s3::charset::pipeline_started")
        })
        .collect::<Vec<_>>();
    assert_eq!(started.len(), 1);
    assert_eq!(started[0].target, "gbf_data::charset_v1");
    assert!(started[0].fields.contains_key("raw_train_byte_count"));
    assert!(started[0].fields.contains_key("raw_val_byte_count"));
    assert!(started[0].fields.contains_key("charset_v1_sha256"));

    let examples = events
        .iter()
        .filter(|event| {
            event.fields.get("event_name").map(String::as_str)
                == Some("s3::charset::example_normalized")
        })
        .collect::<Vec<_>>();
    assert_eq!(examples.len(), 3);

    let complete = events
        .iter()
        .filter(|event| {
            event.fields.get("event_name").map(String::as_str)
                == Some("s3::charset::pipeline_complete")
        })
        .collect::<Vec<_>>();
    assert_eq!(complete.len(), 1);
    assert!(complete[0].fields.contains_key("train_post_char_count"));
    assert!(complete[0].fields.contains_key("val_post_char_count"));
    assert!(complete[0].fields.contains_key("charset_self_hash"));
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
    let Ok(path) = std::env::var("S3_CHARSET_CAPTURE_EVENTS") else {
        return;
    };
    let mut lines = String::new();
    for event in events {
        lines.push_str(&serde_json::to_string(event).expect("event serializes"));
        lines.push('\n');
    }
    std::fs::write(path, lines).expect("writes captured charset events");
}
