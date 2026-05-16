#![cfg(all(feature = "s3", feature = "s3-oracle-real"))]

mod conformance_s3_support;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use conformance_s3_support::{
    canonical_bytes, fixture_agreement_product, fixture_envelope_with_product, fixture_workload,
};
use gbf_artifact::AggregationKind;
use gbf_experiments::s3::conformance::{
    CONFORMANCE_LOG_TARGET, EVENT_NAME_AGGREGATION_REJECTED, EVENT_NAME_BUILD_COMPLETE,
    EVENT_NAME_BUILD_STARTED, EVENT_NAME_SEED_ENVELOPE_BUILT, build_conformance_envelope,
};
use serde::Serialize;
use tracing_subscriber::layer::Context;
use tracing_subscriber::prelude::*;

#[test]
fn conformance_logging_s3() {
    let workload = fixture_workload();
    let agreement = fixture_agreement_product();
    let (envelope, events) = capture_events(|| {
        build_conformance_envelope(&workload, vec![agreement]).expect("conformance builds")
    });

    let started = event_by_name(&events, EVENT_NAME_BUILD_STARTED);
    assert_eq!(started.target, CONFORMANCE_LOG_TARGET);
    assert_eq!(
        started.fields.get("real_owner_bead").map(String::as_str),
        Some("bd-35l3")
    );
    assert_eq!(
        started
            .fields
            .get("agreement_product_count")
            .map(String::as_str),
        Some("1")
    );

    let seed_events = events_by_name(&events, EVENT_NAME_SEED_ENVELOPE_BUILT);
    assert_eq!(seed_events.len(), 5);
    assert!(seed_events.iter().all(|event| event.level == "TRACE"));

    let complete = event_by_name(&events, EVENT_NAME_BUILD_COMPLETE);
    assert_eq!(
        complete.fields.get("conformance_self_hash"),
        Some(&envelope.conformance_self_hash.to_string())
    );
    assert_eq!(
        complete.fields.get("overall_passed").map(String::as_str),
        Some("true")
    );

    let mut f8_agreement = fixture_agreement_product();
    f8_agreement.records[0].aggregation_kind = AggregationKind::PromptWideSoftmaxForbidden;
    let (_result, f8_events) = capture_events(|| {
        let envelope =
            fixture_envelope_with_product(f8_agreement).expect("F8 marker envelope builds");
        canonical_bytes(&envelope).expect_err("canonical write rejects F8 marker");
    });
    let rejected = event_by_name(&f8_events, EVENT_NAME_AGGREGATION_REJECTED);
    assert_eq!(rejected.target, CONFORMANCE_LOG_TARGET);
    assert_eq!(
        rejected.fields.get("aggregation_kind").map(String::as_str),
        Some("PromptWideSoftmaxForbidden")
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
}

fn trim_debug_string(value: String) -> String {
    value
        .strip_prefix('"')
        .and_then(|stripped| stripped.strip_suffix('"'))
        .unwrap_or(&value)
        .to_owned()
}
