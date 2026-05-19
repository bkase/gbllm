#![cfg(all(feature = "s3", feature = "s3-oracle-real"))]

mod oracle_agreement_s3_support;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use gbf_experiments::s3::oracle::{
    AGREEMENT_LOG_TARGET, EVENT_NAME_LIVE_OBSERVATION_CAPTURED, EVENT_NAME_RECORD_EMITTED,
    EVENT_NAME_RUN_COMPLETE, EVENT_NAME_RUN_STARTED,
};
use gbf_oracle::phase_surface_agreement::S3_LIVE_OBSERVATION_REAL_OWNER_BEAD;
use oracle_agreement_s3_support::run_default_agreement;
use serde::Serialize;
use tracing_subscriber::layer::Context;
use tracing_subscriber::prelude::*;

#[test]
fn oracle_agreement_logging_s3() {
    let (product, events) = capture_events(run_default_agreement);
    write_capture_if_requested(&events);

    let started = event_by_name(&events, EVENT_NAME_RUN_STARTED);
    assert_eq!(started.target, AGREEMENT_LOG_TARGET);
    assert_eq!(started.level, "INFO");
    assert_eq!(
        started.fields.get("seed_count").map(String::as_str),
        Some("5")
    );
    assert_eq!(
        started.fields.get("prompt_subset_size").map(String::as_str),
        Some("3")
    );
    assert_eq!(
        started
            .fields
            .get("agreement_trace_steps")
            .map(String::as_str),
        Some("16")
    );
    assert_eq!(
        started.fields.get("stop_on_eos").map(String::as_str),
        Some("false")
    );
    assert_eq!(
        started
            .fields
            .get("live_observation_source")
            .map(String::as_str),
        Some("oracle_derived_fixture")
    );
    assert_eq!(
        started
            .fields
            .get("live_observation_real_owner_bead")
            .map(String::as_str),
        Some(S3_LIVE_OBSERVATION_REAL_OWNER_BEAD)
    );
    assert_eq!(
        started
            .fields
            .get("live_observation_count")
            .map(String::as_str),
        Some(product.records.len().to_string().as_str())
    );

    let live_events = events_by_name(&events, EVENT_NAME_LIVE_OBSERVATION_CAPTURED);
    assert_eq!(live_events.len(), product.records.len());
    assert!(live_events.iter().all(|event| event.level == "TRACE"));
    assert!(live_events.iter().all(|event| {
        event
            .fields
            .get("live_observation_source")
            .map(String::as_str)
            == Some("oracle_derived_fixture")
            && event
                .fields
                .get("live_observation_real_owner_bead")
                .map(String::as_str)
                == Some(S3_LIVE_OBSERVATION_REAL_OWNER_BEAD)
    }));

    let record_events = events_by_name(&events, EVENT_NAME_RECORD_EMITTED);
    assert_eq!(record_events.len(), product.records.len());
    assert!(record_events.iter().all(|event| event.level == "TRACE"));

    let complete = event_by_name(&events, EVENT_NAME_RUN_COMPLETE);
    assert_eq!(complete.target, AGREEMENT_LOG_TARGET);
    assert_eq!(complete.level, "INFO");
    assert_eq!(
        complete.fields.get("total_records").map(String::as_str),
        Some(product.records.len().to_string().as_str())
    );
    assert_eq!(
        complete.fields.get("overall_pass").map(String::as_str),
        Some("true")
    );
    assert_eq!(
        complete
            .fields
            .get("live_observation_source")
            .map(String::as_str),
        Some("oracle_derived_fixture")
    );
    assert_eq!(
        complete
            .fields
            .get("live_observation_real_owner_bead")
            .map(String::as_str),
        Some(S3_LIVE_OBSERVATION_REAL_OWNER_BEAD)
    );
    assert!(
        complete
            .fields
            .get("fallback_used")
            .map_or(false, |value| value.contains("S3LiveObservationFixture"))
    );
    assert_eq!(
        complete.fields.get("agreement_self_hash"),
        Some(&product.agreement_self_hash.to_string())
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

fn events_by_name<'a>(events: &'a [CapturedEvent], name: &str) -> Vec<&'a CapturedEvent> {
    events
        .iter()
        .filter(|event| event.fields.get("event_name").map(String::as_str) == Some(name))
        .collect()
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
    let Ok(path) = std::env::var("S3_ORACLE_AGREEMENT_CAPTURE_EVENTS") else {
        return;
    };
    let mut lines = String::new();
    for event in events {
        lines.push_str(&serde_json::to_string(event).expect("event serializes"));
        lines.push('\n');
    }
    std::fs::write(path, lines).expect("writes agreement events");
}
