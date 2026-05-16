#![cfg(all(feature = "s3", feature = "s3-phase-d", feature = "s3-oracle-real"))]

mod v0_success_s3_support;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use gbf_experiments::s3::contamination::{
    EVENT_NAME_CONTAMINATION_CHECKED, check_no_prompt_in_train_post,
};
use gbf_experiments::s3::workload::{
    EVENT_NAME_V0_SUCCESS_GENERATION_PER_PROMPT, EVENT_NAME_V0_SUCCESS_QUALITY_GATE,
    EVENT_NAME_V0_SUCCESS_RUN_COMPLETE, EVENT_NAME_V0_SUCCESS_RUN_STARTED,
    EVENT_NAME_V0_SUCCESS_SCORING_COMPLETE, EVENT_NAME_V0_SUCCESS_SEED_STARTED,
    S3_WORKLOAD_LOG_TARGET, s3_run_v0_success,
};
use serde::Serialize;
use serde_json::{Value, json};
use tracing_subscriber::layer::Context;
use tracing_subscriber::prelude::*;
use v0_success_s3_support::{
    budget, fixture_workload, high_baseline, runner_artifacts, runner_bundles, tiny_val_post,
};

#[test]
fn v0_success_logging_s3() {
    let workload = fixture_workload();
    let seed_count = workload.seeds.len() as u64;
    let prompt_count = workload.prompts.len() as u64;
    let expected_workload_hash = workload.workload_self_hash.to_string();

    let (product, events) = capture_events(|| {
        let product = s3_run_v0_success(
            runner_artifacts(),
            runner_bundles(),
            workload.clone(),
            tiny_val_post(),
            high_baseline(),
            budget(1_000_000),
        );
        let contamination = check_no_prompt_in_train_post(&workload, &tiny_val_post());
        assert!(!contamination.contamination_found);
        product
    });
    write_capture_artifacts_if_requested(&product, &events);

    let started = event_by_name(&events, EVENT_NAME_V0_SUCCESS_RUN_STARTED);
    assert_eq!(started.target, S3_WORKLOAD_LOG_TARGET);
    assert_eq!(started.fields.get("seed_count"), Some(&json!(seed_count)));
    assert_eq!(
        started.fields.get("prompt_count"),
        Some(&json!(prompt_count))
    );
    assert_eq!(
        started.fields.get("workload_self_hash"),
        Some(&json!(expected_workload_hash))
    );

    let seed_started = events_by_name(&events, EVENT_NAME_V0_SUCCESS_SEED_STARTED);
    assert_eq!(seed_started.len(), seed_count as usize);
    assert!(
        seed_started
            .iter()
            .all(|event| event.target == S3_WORKLOAD_LOG_TARGET)
    );
    assert!(seed_started.iter().all(|event| event.level == "INFO"));
    assert!(seed_started.iter().all(|event| {
        event
            .fields
            .get("build_kind")
            .and_then(Value::as_str)
            .is_some()
    }));

    let generation = events_by_name(&events, EVENT_NAME_V0_SUCCESS_GENERATION_PER_PROMPT);
    assert_eq!(generation.len(), (seed_count * prompt_count) as usize);
    assert!(
        generation
            .iter()
            .all(|event| event.target == S3_WORKLOAD_LOG_TARGET)
    );
    assert!(generation.iter().all(|event| event.level == "TRACE"));
    assert!(generation.iter().all(|event| {
        event.fields.contains_key("seed")
            && event.fields.contains_key("prompt_id")
            && event.fields.contains_key("generated_char_count")
            && event.fields.contains_key("terminal_eos_seen")
            && event.fields.contains_key("max_consecutive_same_token")
            && event.fields.contains_key("charset_validity_rate")
    }));

    let scoring = events_by_name(&events, EVENT_NAME_V0_SUCCESS_SCORING_COMPLETE);
    assert_eq!(scoring.len(), seed_count as usize);
    assert!(
        scoring
            .iter()
            .all(|event| event.target == S3_WORKLOAD_LOG_TARGET)
    );
    assert!(scoring.iter().all(|event| event.level == "INFO"));

    let quality = events_by_name(&events, EVENT_NAME_V0_SUCCESS_QUALITY_GATE);
    assert_eq!(quality.len(), seed_count as usize);
    assert!(
        quality
            .iter()
            .all(|event| event.target == S3_WORKLOAD_LOG_TARGET)
    );
    assert!(quality.iter().all(|event| {
        event.fields.contains_key("Q1_holds")
            && event.fields.contains_key("Q2_holds")
            && event.fields.contains_key("Q3_holds")
            && event.fields.contains_key("Q4_holds")
            && event.fields.contains_key("Q5_holds")
            && event.fields.contains_key("Q6_holds")
            && event.fields.contains_key("pass")
    }));

    let complete = event_by_name(&events, EVENT_NAME_V0_SUCCESS_RUN_COMPLETE);
    assert_eq!(complete.target, S3_WORKLOAD_LOG_TARGET);
    assert_eq!(complete.level, "INFO");
    assert_eq!(complete.fields.get("seed_count"), Some(&json!(seed_count)));
    assert_eq!(
        complete.fields.get("overall_pass"),
        Some(&json!(product.overall_pass))
    );
    assert_eq!(
        complete.fields.get("suspicious_low_bpc"),
        Some(&json!(product.suspicious_low_bpc))
    );
    assert_eq!(
        complete.fields.get("v0_success_self_hash"),
        Some(&json!(product.v0_success_self_hash.to_string()))
    );

    let contamination = event_by_name(&events, EVENT_NAME_CONTAMINATION_CHECKED);
    assert_eq!(contamination.target, S3_WORKLOAD_LOG_TARGET);
    assert_eq!(contamination.level, "INFO");
    assert_eq!(
        contamination.fields.get("prompt_count"),
        Some(&json!(8_u64))
    );
    assert_eq!(
        contamination.fields.get("contamination_found"),
        Some(&json!(false))
    );

    assert!(events.iter().all(|event| event.name.starts_with("s3::")));
}

#[derive(Clone, Debug, Default)]
struct TraceCapture {
    events: Arc<Mutex<Vec<CapturedEvent>>>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct CapturedEvent {
    target: String,
    level: String,
    name: String,
    fields: BTreeMap<String, Value>,
}

impl<S> tracing_subscriber::layer::Layer<S> for TraceCapture
where
    S: tracing::Subscriber,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = FieldVisitor::default();
        event.record(&mut visitor);
        let name = visitor
            .fields
            .get("event_name")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| event.metadata().name().to_owned());
        self.events
            .lock()
            .expect("trace capture mutex")
            .push(CapturedEvent {
                target: event.metadata().target().to_owned(),
                level: event.metadata().level().to_string(),
                name,
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

#[derive(Debug, Default)]
struct FieldVisitor {
    fields: BTreeMap<String, Value>,
}

impl FieldVisitor {
    fn insert(&mut self, field: &tracing::field::Field, value: Value) {
        self.fields.insert(field.name().to_owned(), value);
    }
}

impl tracing::field::Visit for FieldVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.insert(field, Value::String(format!("{value:?}")));
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.insert(field, Value::String(value.to_owned()));
    }

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.insert(field, Value::Bool(value));
    }

    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.insert(field, Value::Number(value.into()));
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.insert(field, Value::Number(value.into()));
    }

    fn record_f64(&mut self, field: &tracing::field::Field, value: f64) {
        let value = serde_json::Number::from_f64(value)
            .map(Value::Number)
            .unwrap_or_else(|| Value::String(value.to_string()));
        self.insert(field, value);
    }
}

fn event_by_name<'a>(events: &'a [CapturedEvent], name: &str) -> &'a CapturedEvent {
    events
        .iter()
        .find(|event| event.name == name)
        .unwrap_or_else(|| panic!("missing event {name:?}; saw {events:#?}"))
}

fn events_by_name<'a>(events: &'a [CapturedEvent], name: &str) -> Vec<&'a CapturedEvent> {
    events.iter().filter(|event| event.name == name).collect()
}

fn write_capture_artifacts_if_requested(
    product: &gbf_experiments::s3::workload::V0SuccessProduct,
    events: &[CapturedEvent],
) {
    if let Ok(path) = std::env::var("S3_V0_SUCCESS_PRODUCT_OUT") {
        std::fs::write(
            path,
            product
                .canonical_bytes()
                .expect("v0_success product canonicalizes"),
        )
        .expect("write v0_success product");
    }
    if let Ok(path) = std::env::var("S3_V0_SUCCESS_CAPTURE_EVENTS") {
        let mut lines = String::new();
        for event in events {
            lines.push_str(&serde_json::to_string(event).expect("event serializes"));
            lines.push('\n');
        }
        std::fs::write(path, lines).expect("write v0_success events");
    }
}

#[test]
fn v0_success_logging_target_constant_s3() {
    assert_eq!(S3_WORKLOAD_LOG_TARGET, "gbf_experiments::s3::workload");
}
