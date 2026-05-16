#![cfg(feature = "s3-schemas")]

#[path = "v0_success_s3_support/mod.rs"]
mod v0_success_s3_support;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use gbf_workload::{
    V0_SUCCESS_WORKLOAD_LOADED_EVENT, V0_SUCCESS_WORKLOAD_LOG_TARGET,
    read_v0_success_workload_manifest,
};
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::prelude::*;
use tracing_subscriber::registry::LookupSpan;

#[test]
fn workload_logging_s3() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::registry().with(CaptureLayer {
        events: Arc::clone(&events),
    });
    let mut manifest = None;

    tracing::subscriber::with_default(subscriber, || {
        manifest = Some(
            read_v0_success_workload_manifest(v0_success_s3_support::fixture_path())
                .expect("v0_success workload loads"),
        );
    });

    let manifest = manifest.expect("manifest captured");
    let events = events.lock().expect("events lock").clone();
    assert_eq!(events.len(), 1);

    let event = &events[0];
    assert_eq!(event.target, V0_SUCCESS_WORKLOAD_LOG_TARGET);
    assert_eq!(event.level, "INFO");
    assert_eq!(
        event.fields.get("event_name").map(String::as_str),
        Some(V0_SUCCESS_WORKLOAD_LOADED_EVENT)
    );
    assert_eq!(
        event.fields.get("id").map(String::as_str),
        Some("v0_success")
    );
    assert_eq!(
        event.fields.get("prompt_count").map(String::as_str),
        Some("8")
    );
    assert_eq!(
        event
            .fields
            .get("agreement_subset_size")
            .map(String::as_str),
        Some("3")
    );
    assert!(event.fields.contains_key("observation_policy_hash"));
    let expected_workload_self_hash = manifest.workload_self_hash.to_string();
    assert_eq!(
        event.fields.get("workload_self_hash").map(String::as_str),
        Some(expected_workload_self_hash.as_str())
    );
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CapturedEvent {
    target: &'static str,
    level: String,
    fields: BTreeMap<String, String>,
}

struct CaptureLayer {
    events: Arc<Mutex<Vec<CapturedEvent>>>,
}

impl<S> tracing_subscriber::Layer<S> for CaptureLayer
where
    S: Subscriber,
    S: for<'lookup> LookupSpan<'lookup>,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        if event.metadata().target() != V0_SUCCESS_WORKLOAD_LOG_TARGET {
            return;
        }

        let mut visitor = FieldVisitor::default();
        event.record(&mut visitor);
        self.events
            .lock()
            .expect("events lock")
            .push(CapturedEvent {
                target: event.metadata().target(),
                level: event.metadata().level().to_string(),
                fields: visitor.fields,
            });
    }
}

#[derive(Default)]
struct FieldVisitor {
    fields: BTreeMap<String, String>,
}

impl Visit for FieldVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.fields
            .insert(field.name().to_owned(), value.to_owned());
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.fields
            .insert(field.name().to_owned(), value.to_string());
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.fields.insert(
            field.name().to_owned(),
            trim_debug_string(format!("{value:?}")),
        );
    }
}

fn trim_debug_string(value: String) -> String {
    value
        .strip_prefix('"')
        .and_then(|stripped| stripped.strip_suffix('"'))
        .unwrap_or(&value)
        .to_owned()
}
