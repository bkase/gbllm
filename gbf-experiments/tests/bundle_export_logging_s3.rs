#![cfg(feature = "s3")]

mod bundle_s3_support;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use bundle_s3_support::{export_product_from, frozen_toy_teacher};
use gbf_experiments::s3::bundle::{
    BUNDLE_EXPORT_LOG_TARGET, EVENT_NAME_BUNDLE_EXPORT_COMPLETE,
    EVENT_NAME_BUNDLE_EXPORT_PROGRAM_EMITTED, EVENT_NAME_BUNDLE_EXPORT_PROGRAM_VALIDATED,
    EVENT_NAME_BUNDLE_EXPORT_STARTED, EVENT_NAME_BUNDLE_EXPORT_TENSOR_EMITTED,
};
use gbf_experiments::s3::schema::S3_BUNDLE_SCHEMA;
use serde::Serialize;
use tracing_subscriber::layer::Context;
use tracing_subscriber::prelude::*;

#[test]
fn bundle_export_logging_captures_required_event_shape() {
    let frozen = frozen_toy_teacher(0);
    let (product, events) = capture_events(|| export_product_from(&frozen));
    write_capture_if_requested(&events, &product);

    assert_event(&events, EVENT_NAME_BUNDLE_EXPORT_STARTED, "INFO");
    assert_event(&events, EVENT_NAME_BUNDLE_EXPORT_PROGRAM_EMITTED, "INFO");
    assert_event(&events, EVENT_NAME_BUNDLE_EXPORT_PROGRAM_VALIDATED, "INFO");
    assert_event(&events, EVENT_NAME_BUNDLE_EXPORT_COMPLETE, "INFO");

    let tensor_events = events
        .iter()
        .filter(|event| {
            event.fields.get("event_name").map(String::as_str)
                == Some(EVENT_NAME_BUNDLE_EXPORT_TENSOR_EMITTED)
        })
        .collect::<Vec<_>>();
    assert_eq!(tensor_events.len(), product.bundle.tensors.len());
    assert!(tensor_events.iter().all(|event| event.level == "TRACE"));
    assert!(
        tensor_events.iter().any(|event| event
            .fields
            .get("alias_target_present")
            .map(String::as_str)
            == Some("true"))
    );

    let complete = event_by_name(&events, EVENT_NAME_BUNDLE_EXPORT_COMPLETE);
    assert_eq!(
        complete.fields.get("canonical_bundle_payload_sha"),
        Some(&product.canonical_bundle_payload_sha.to_string())
    );
    assert_eq!(
        complete.fields.get("bundle_self_hash"),
        Some(&product.bundle_self_hash.to_string())
    );
    assert_eq!(
        product.metadata.schema, S3_BUNDLE_SCHEMA,
        "export product must carry s3_bundle.v1 metadata"
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

fn assert_event(events: &[CapturedEvent], name: &str, level: &str) {
    let event = event_by_name(events, name);
    assert_eq!(event.target, BUNDLE_EXPORT_LOG_TARGET);
    assert_eq!(event.level, level);
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

fn write_capture_if_requested(
    events: &[CapturedEvent],
    product: &gbf_experiments::s3::bundle::BundleExportProduct,
) {
    if let Ok(path) = std::env::var("S3_BUNDLE_CAPTURE_EVENTS") {
        let mut lines = String::new();
        for event in events {
            lines.push_str(&serde_json::to_string(event).expect("event serializes"));
            lines.push('\n');
        }
        std::fs::write(path, lines).expect("writes captured bundle events");
    }
    if let Ok(path) = std::env::var("S3_BUNDLE_CAPTURE_METADATA") {
        std::fs::write(
            path,
            product
                .metadata
                .canonical_json_bytes()
                .expect("metadata canonicalizes"),
        )
        .expect("writes captured bundle metadata");
    }
}
