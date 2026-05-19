#![cfg(feature = "s3-real")]

mod denotational_support;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use denotational_support::{fixture_bundle, fixture_policy, fixture_workload};
use gbf_oracle::denotational::{
    DENOTATIONAL_ORACLE_LOG_TARGET, DenotationalOracle, DenotationalOracleInputs,
    EVENT_NAME_EVALUATION_COMPLETE, EVENT_NAME_EVALUATION_STARTED, EVENT_NAME_OBSERVATION_CAPTURED,
    RealDenotationalOracle,
};
use serde::Serialize;
use tracing_subscriber::layer::Context;
use tracing_subscriber::prelude::*;

#[test]
fn denotational_oracle_logging_emits_required_events() {
    let bundle = fixture_bundle();
    let workload = fixture_workload();
    let policy = fixture_policy();
    let (product, events) = capture_events(|| {
        RealDenotationalOracle
            .evaluate(DenotationalOracleInputs::new(&bundle, &workload, &policy))
            .expect("real denotational oracle evaluates")
    });
    write_capture_if_requested(&events);

    let started = event_by_name(&events, EVENT_NAME_EVALUATION_STARTED);
    assert_eq!(started.target, DENOTATIONAL_ORACLE_LOG_TARGET);
    assert_eq!(started.level, "INFO");
    assert_eq!(
        started.fields.get("backend_kind").map(String::as_str),
        Some("real")
    );
    assert_eq!(
        started.fields.get("prompt_count").map(String::as_str),
        Some("3")
    );

    let captured = events
        .iter()
        .filter(|event| {
            event.fields.get("event_name").map(String::as_str)
                == Some(EVENT_NAME_OBSERVATION_CAPTURED)
        })
        .collect::<Vec<_>>();
    assert_eq!(captured.len(), product.observations.len());
    assert!(captured.iter().all(|event| event.level == "TRACE"));

    let complete = event_by_name(&events, EVENT_NAME_EVALUATION_COMPLETE);
    assert_eq!(complete.level, "INFO");
    assert_eq!(
        complete.fields.get("observation_count").map(String::as_str),
        Some(product.observations.len().to_string().as_str())
    );
    assert_eq!(
        complete.fields.get("oracle_self_hash"),
        Some(&product.oracle_self_hash.to_string())
    );
    assert_eq!(
        complete.fields.get("determinism_class").map(String::as_str),
        Some("BitExact")
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
}

fn trim_debug_string(value: String) -> String {
    value
        .strip_prefix('"')
        .and_then(|stripped| stripped.strip_suffix('"'))
        .unwrap_or(&value)
        .to_owned()
}

fn write_capture_if_requested(events: &[CapturedEvent]) {
    let Ok(path) = std::env::var("S3_DENOTATIONAL_CAPTURE_EVENTS") else {
        return;
    };
    let mut lines = String::new();
    for event in events {
        lines.push_str(&serde_json::to_string(event).expect("event serializes"));
        lines.push('\n');
    }
    std::fs::write(path, lines).expect("writes denotational events");
}
