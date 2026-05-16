#![cfg(feature = "s3-schemas")]

#[path = "bundle_s3_support/mod.rs"]
mod bundle_s3_support;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use gbf_artifact::{TextCharSeq, evaluate_reference_program};
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::prelude::*;
use tracing_subscriber::registry::LookupSpan;

#[test]
fn evaluator_emits_node_boundary_trace_events() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::registry().with(CaptureLayer {
        events: Arc::clone(&events),
    });

    let bundle = bundle_s3_support::toy_bundle();
    let prompt = TextCharSeq::new(vec![0, 1, 2]).expect("prompt is valid charset_v1 text");
    let policy = bundle_s3_support::observation_policy();
    tracing::subscriber::with_default(subscriber, || {
        let _ = evaluate_reference_program(&bundle, &prompt, &policy);
    });

    let events = events.lock().expect("events lock").clone();
    assert_eq!(events.len(), bundle.program.graph.nodes.len());
    for (step, event) in events.iter().enumerate() {
        let expected_step = step.to_string();
        assert_eq!(event.target, "gbf_artifact::bundle_program_evaluator");
        assert_eq!(
            event.fields.get("event_name").map(String::as_str),
            Some("s3::bundle_eval::node_executed")
        );
        assert_eq!(
            event.fields.get("evaluation_step").map(String::as_str),
            Some(expected_step.as_str())
        );
        assert!(event.fields.contains_key("op_id"));
        assert!(event.fields.contains_key("op_kind"));
        assert!(event.fields.contains_key("input_count"));
        assert!(event.fields.contains_key("output_count"));
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CapturedEvent {
    target: &'static str,
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
        if event.metadata().target() != "gbf_artifact::bundle_program_evaluator" {
            return;
        }
        let mut visitor = FieldVisitor::default();
        event.record(&mut visitor);
        self.events
            .lock()
            .expect("events lock")
            .push(CapturedEvent {
                target: event.metadata().target(),
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

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.fields
            .insert(field.name().to_owned(), value.to_string());
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.fields
            .insert(field.name().to_owned(), value.to_string());
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.fields
            .insert(field.name().to_owned(), format!("{value:?}"));
    }
}
