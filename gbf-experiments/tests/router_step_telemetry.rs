#![cfg(feature = "s7")]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use gbf_experiments::S7_LOG_TARGET;
use gbf_experiments::s7::schema::{
    ConfidenceDist, ROUTER_STEP_TELEMETRY_EPSILON, ROUTER_STEP_TELEMETRY_EVENT,
    ROUTER_STEP_TELEMETRY_SCHEMA_VERSION, RouterStepTelemetry, RouterTelemetryError,
    entropy_bits_from_counts,
};
use gbf_foundation::{Hash256, SemVer};
use serde_json::{Value, json};
use tracing_subscriber::layer::Context;
use tracing_subscriber::prelude::*;

const OLD_ENTROPY_FIELD_PARTS: [&str; 3] = ["expert", "usage", "entropy"];

#[test]
fn router_step_telemetry_schema_uses_entropy_bits_field() {
    let telemetry = sample_telemetry(vec![2, 2, 0, 0]).expect("telemetry");

    assert_approx_eq(telemetry.expert_usage_entropy_bits, 1.0);
    telemetry
        .validate_with_n_blocks(4)
        .expect("valid telemetry");
    telemetry.verify_self_hash().expect("self hash verifies");
    assert_ne!(telemetry.telemetry_self_hash, Hash256::ZERO);

    let value = serde_json::to_value(&telemetry).expect("json");
    let object = value.as_object().expect("telemetry json object");
    assert!(object.contains_key("expert_usage_entropy_bits"));
    assert!(!object.contains_key(&old_entropy_field()));
    assert_eq!(
        value["schema_version"],
        json!(ROUTER_STEP_TELEMETRY_SCHEMA_VERSION)
    );
    assert_json_f32(&value["router_confidence_distribution"]["mean"], 0.70);
    assert_json_f32(&value["router_confidence_distribution"]["p10"], 0.55);
    assert_json_f32(&value["router_confidence_distribution"]["p50"], 0.72);
    assert_json_f32(&value["router_confidence_distribution"]["p90"], 0.91);
}

#[test]
fn entropy_bits_from_counts_uses_zero_probability_convention() {
    assert_approx_eq(
        entropy_bits_from_counts(&[10, 0, 0, 0]).expect("single expert entropy"),
        0.0,
    );
    assert_approx_eq(
        entropy_bits_from_counts(&[1, 1, 1, 1]).expect("uniform entropy"),
        2.0,
    );
}

#[test]
fn router_step_telemetry_rejects_entropy_outside_bits_range() {
    let err = RouterStepTelemetry::from_computed_entropy_bits(
        7,
        11,
        2,
        3.0,
        0.25,
        sample_confidence().expect("confidence"),
        vec![1, 1, 1, 1],
        0.5,
        4,
    )
    .expect_err("entropy above log2(4) must fail");

    assert!(matches!(
        err,
        RouterTelemetryError::EntropyBitsOutOfRange { value, max }
            if approx_eq(value, 3.0) && approx_eq(max, 2.0)
    ));

    let mut value = serde_json::to_value(sample_telemetry(vec![1, 1, 1, 1]).expect("telemetry"))
        .expect("telemetry json");
    value["expert_usage_entropy_bits"] = json!(3.0);
    let err = serde_json::from_value::<RouterStepTelemetry>(value)
        .expect_err("deserialization rejects entropy above log2(n_experts)");
    assert!(
        err.to_string().contains("expert_usage_entropy_bits"),
        "{err}"
    );
}

#[test]
fn router_step_telemetry_rejects_old_schema_version_and_bad_confidence_order() {
    let mut telemetry = sample_telemetry(vec![1, 1, 1, 1]).expect("telemetry");
    telemetry.schema_version = SemVer::new(0, 9, 0);
    assert!(matches!(
        telemetry.validate(),
        Err(RouterTelemetryError::UnexpectedSchemaVersion { .. })
    ));

    let err = ConfidenceDist::new(0.6, 0.7, 0.5, 0.8).expect_err("quantiles out of order");
    assert!(matches!(
        err,
        RouterTelemetryError::ConfidenceQuantilesOutOfOrder { .. }
    ));

    let err = serde_json::from_value::<ConfidenceDist>(json!({
        "mean": 0.6,
        "p10": 0.7,
        "p50": 0.5,
        "p90": 0.8,
    }))
    .expect_err("deserialization rejects confidence quantiles out of order");
    assert!(err.to_string().contains("quantiles"), "{err}");
}

#[test]
fn emitted_router_step_telemetry_entropy_bits_is_in_range() {
    let samples = [
        sample_telemetry(vec![4, 0, 0, 0]).expect("collapsed telemetry"),
        sample_telemetry(vec![2, 2, 0, 0]).expect("two expert telemetry"),
        sample_telemetry(vec![1, 1, 1, 1]).expect("uniform telemetry"),
    ];
    let events = capture_events(|| {
        for telemetry in &samples {
            telemetry.emit_trace().expect("telemetry emits");
        }
    });

    let router_events = events
        .iter()
        .filter(|event| event.fields.get("event_name") == Some(&ROUTER_STEP_TELEMETRY_EVENT.into()))
        .collect::<Vec<_>>();
    assert_eq!(router_events.len(), samples.len());

    for event in router_events {
        assert_eq!(event.target, S7_LOG_TARGET);
        let entropy_bits = event
            .fields
            .get("expert_usage_entropy_bits")
            .and_then(Value::as_f64)
            .expect("entropy bits field");
        let n_experts = event
            .fields
            .get("tokens_per_expert")
            .and_then(Value::as_array)
            .expect("tokens per expert field")
            .len();
        let max_bits = (n_experts as f64).log2();
        let epsilon = f64::from(ROUTER_STEP_TELEMETRY_EPSILON);
        assert!(
            entropy_bits >= -epsilon && entropy_bits <= max_bits + epsilon,
            "entropy {entropy_bits} must be within [{}, {}]",
            -epsilon,
            max_bits + epsilon
        );
        assert!(!event.fields.contains_key(&old_entropy_field()));
    }
}

#[test]
fn o12_router_step_telemetry_subscriber_captures_ten_step_segment() {
    const N_BLOCKS: u32 = 4;
    const N_STEPS: u64 = 10;

    let samples = (0..N_STEPS)
        .flat_map(|train_step| {
            (0..N_BLOCKS).map(move |layer_id| {
                segment_telemetry(train_step, layer_id, N_BLOCKS).expect("segment telemetry")
            })
        })
        .collect::<Vec<_>>();

    let events = capture_events(|| {
        for telemetry in &samples {
            telemetry.emit_trace().expect("telemetry emits");
        }
    });

    let router_events = router_step_events(&events);
    assert_eq!(
        router_events.len(),
        (N_STEPS * u64::from(N_BLOCKS)) as usize
    );

    let mut events_per_step = BTreeMap::<u64, usize>::new();
    let mut observed_pairs = BTreeSet::<(u64, u64)>::new();

    for event in router_events {
        assert_eq!(event.level, "INFO");
        assert_eq!(event.target, S7_LOG_TARGET);
        assert_eq!(event_str(event, "event_name"), ROUTER_STEP_TELEMETRY_EVENT);
        assert_eq!(
            event_u64(event, "schema_version_major"),
            ROUTER_STEP_TELEMETRY_SCHEMA_VERSION.major
        );
        assert_eq!(
            event_u64(event, "schema_version_minor"),
            ROUTER_STEP_TELEMETRY_SCHEMA_VERSION.minor
        );
        assert_eq!(
            event_u64(event, "schema_version_patch"),
            ROUTER_STEP_TELEMETRY_SCHEMA_VERSION.patch
        );

        assert_d19_event_fields_present_and_nonzero(event);

        let train_step = event_u64(event, "train_step");
        let layer_id = event_u64(event, "layer_id");
        assert!(observed_pairs.insert((train_step, layer_id)));
        *events_per_step.entry(train_step).or_default() += 1;

        let payload = event_str(event, "telemetry_canonical_json");
        let decoded: RouterStepTelemetry =
            serde_json::from_str(payload).expect("event payload deserializes");
        decoded
            .validate_with_n_blocks(N_BLOCKS)
            .expect("decoded event satisfies RST invariants");
        decoded
            .verify_self_hash()
            .expect("decoded event self-hash verifies");
        assert_eq!(
            decoded.canonical_json_string().expect("canonical payload"),
            payload
        );
        assert_eq!(decoded.train_step, train_step);
        assert_eq!(u64::from(decoded.layer_id), layer_id);
        assert_eq!(
            event_str(event, "telemetry_self_hash"),
            decoded.telemetry_self_hash.to_string()
        );
    }

    for train_step in 0..N_STEPS {
        assert_eq!(
            events_per_step.get(&train_step).copied(),
            Some(N_BLOCKS as usize),
            "expected one event per layer for train_step={train_step}"
        );
    }
    assert_eq!(observed_pairs.len(), samples.len());
}

fn sample_telemetry(
    tokens_per_expert: Vec<u32>,
) -> Result<RouterStepTelemetry, RouterTelemetryError> {
    RouterStepTelemetry::new(
        7,
        11,
        2,
        0.25,
        sample_confidence()?,
        tokens_per_expert,
        0.5,
        4,
    )
}

fn segment_telemetry(
    train_step: u64,
    layer_id: u32,
    n_blocks: u32,
) -> Result<RouterStepTelemetry, RouterTelemetryError> {
    let count_jitter = ((train_step + u64::from(layer_id)) % 3) as u32;
    RouterStepTelemetry::new(
        7,
        train_step,
        layer_id,
        0.25 + layer_id as f32 * 0.05,
        sample_confidence()?,
        vec![1 + count_jitter, 2, 3, 4],
        0.5 + layer_id as f32 * 0.25,
        n_blocks,
    )
}

fn sample_confidence() -> Result<ConfidenceDist, RouterTelemetryError> {
    ConfidenceDist::new(0.70, 0.55, 0.72, 0.91)
}

fn router_step_events(events: &[CapturedEvent]) -> Vec<&CapturedEvent> {
    events
        .iter()
        .filter(|event| event.fields.get("event_name") == Some(&ROUTER_STEP_TELEMETRY_EVENT.into()))
        .collect()
}

fn assert_d19_event_fields_present_and_nonzero(event: &CapturedEvent) {
    for field in [
        "expert_usage_entropy_bits",
        "same_expert_rate",
        "router_confidence_distribution",
        "tokens_per_expert",
        "bank_switches_per_token",
    ] {
        assert!(event.fields.contains_key(field), "missing {field}");
    }

    assert!(event_f64(event, "expert_usage_entropy_bits") > 0.0);
    assert!(event_f64(event, "same_expert_rate") > 0.0);
    assert!(event_f64(event, "router_confidence_mean") > 0.0);
    assert!(event_f64(event, "router_confidence_p10") > 0.0);
    assert!(event_f64(event, "router_confidence_p50") > 0.0);
    assert!(event_f64(event, "router_confidence_p90") > 0.0);
    assert!(event_f64(event, "bank_switches_per_token") > 0.0);

    let tokens = event
        .fields
        .get("tokens_per_expert")
        .and_then(Value::as_array)
        .expect("tokens_per_expert array");
    assert!(!tokens.is_empty());
    assert!(
        tokens
            .iter()
            .all(|token| token.as_u64().expect("token count") > 0),
        "fixture must not use sentinel zero expert counts"
    );

    assert_ne!(
        event_str(event, "telemetry_self_hash"),
        Hash256::ZERO.to_string()
    );
    assert!(!event_str(event, "router_confidence_distribution").is_empty());
    assert!(!event_str(event, "telemetry_canonical_json").is_empty());
}

fn event_str<'a>(event: &'a CapturedEvent, field: &str) -> &'a str {
    event
        .fields
        .get(field)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{field} string field"))
}

fn event_u64(event: &CapturedEvent, field: &str) -> u64 {
    event
        .fields
        .get(field)
        .and_then(Value::as_u64)
        .unwrap_or_else(|| panic!("{field} u64 field"))
}

fn event_f64(event: &CapturedEvent, field: &str) -> f64 {
    event
        .fields
        .get(field)
        .and_then(Value::as_f64)
        .unwrap_or_else(|| panic!("{field} f64 field"))
}

#[derive(Clone, Debug, Default)]
struct TraceCapture {
    events: Arc<Mutex<Vec<CapturedEvent>>>,
}

#[derive(Clone, Debug)]
struct CapturedEvent {
    level: String,
    target: String,
    fields: BTreeMap<String, Value>,
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
                level: event.metadata().level().to_string(),
                target: event.metadata().target().to_owned(),
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
    fields: BTreeMap<String, Value>,
}

impl FieldVisitor {
    fn insert(&mut self, field: &tracing::field::Field, value: Value) {
        self.fields.insert(field.name().to_owned(), value);
    }
}

impl tracing::field::Visit for FieldVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        let text = format!("{value:?}");
        if let Ok(value) = serde_json::from_str::<Value>(&text) {
            self.insert(field, value);
        } else if let Ok(value) = text.parse::<f64>() {
            self.insert(field, json!(value));
        } else {
            self.insert(field, json!(trim_debug_string(&text)));
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.insert(field, json!(value));
    }

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.insert(field, json!(value));
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.insert(field, json!(value));
    }

    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.insert(field, json!(value));
    }

    fn record_f64(&mut self, field: &tracing::field::Field, value: f64) {
        self.insert(field, json!(value));
    }
}

fn trim_debug_string(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|stripped| stripped.strip_suffix('"'))
        .unwrap_or(value)
}

fn approx_eq(left: f32, right: f32) -> bool {
    (left - right).abs() <= 1.0e-6
}

fn assert_approx_eq(left: f32, right: f32) {
    assert!(approx_eq(left, right), "left={left}, right={right}");
}

fn assert_json_f32(value: &Value, expected: f32) {
    let actual = value.as_f64().expect("json number") as f32;
    assert_approx_eq(actual, expected);
}

fn old_entropy_field() -> String {
    OLD_ENTROPY_FIELD_PARTS.join("_")
}
