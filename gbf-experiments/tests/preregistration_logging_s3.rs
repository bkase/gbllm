#![cfg(feature = "s3")]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use gbf_experiments::s3::preregistration::{
    S3_PREREGISTRATION_LOG_TARGET, S3_PREREGISTRATION_PIN_LOADED_EVENT, load_preregistration_pin,
};
use serde_json::{Value, json};
use tracing_subscriber::prelude::*;

#[test]
fn preregistration_loader_emits_pin_loaded_event() {
    let capture = TraceCapture::default();
    let subscriber = tracing_subscriber::registry().with(capture.clone());
    let pin_path = workspace_root().join("experiments/S3/preregistration.toml");

    let pin = tracing::subscriber::with_default(subscriber, || {
        load_preregistration_pin(&pin_path).expect("S3 preregistration pin loads")
    });

    assert_eq!(pin.schema, "s3_preregistration.v1");
    assert_eq!(pin.first_result_commit, "");

    let events = capture.events();
    let event = events
        .iter()
        .find(|event| event.name == S3_PREREGISTRATION_PIN_LOADED_EVENT)
        .expect("missing s3 preregistration pin-loaded event");
    assert_eq!(event.target, S3_PREREGISTRATION_LOG_TARGET);
    assert_eq!(
        event.fields.get("predictions_commit"),
        Some(&json!(pin.predictions_commit))
    );
    assert_eq!(
        event.fields.get("predictions_section_hash"),
        Some(&json!(pin.predictions_section_hash))
    );
    assert_eq!(
        event.fields.get("pass_version_S3"),
        Some(&json!(pin.pass_version_s3))
    );
    assert_eq!(
        event.fields.get("rfc_revision"),
        Some(&json!(pin.rfc_revision))
    );
}

#[derive(Clone, Debug, Default)]
struct TraceCapture {
    events: Arc<Mutex<Vec<TracingEvent>>>,
}

impl TraceCapture {
    fn events(&self) -> Vec<TracingEvent> {
        self.events
            .lock()
            .expect("trace capture mutex is not poisoned")
            .clone()
    }
}

impl<S> tracing_subscriber::layer::Layer<S> for TraceCapture
where
    S: tracing::Subscriber,
{
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut visitor = TraceFieldVisitor::default();
        event.record(&mut visitor);
        let name = visitor
            .fields
            .get("event_name")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| event.metadata().name().to_owned());
        self.events
            .lock()
            .expect("trace capture mutex is not poisoned")
            .push(TracingEvent {
                name,
                target: event.metadata().target().to_owned(),
                fields: visitor.fields,
            });
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TracingEvent {
    name: String,
    target: String,
    fields: BTreeMap<String, Value>,
}

#[derive(Debug, Default)]
struct TraceFieldVisitor {
    fields: BTreeMap<String, Value>,
}

impl tracing::field::Visit for TraceFieldVisitor {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.fields
            .insert(field.name().to_owned(), Value::String(value.to_owned()));
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.fields
            .insert(field.name().to_owned(), Value::String(format!("{value:?}")));
    }
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("gbf-experiments parent is workspace root")
        .to_path_buf()
}
