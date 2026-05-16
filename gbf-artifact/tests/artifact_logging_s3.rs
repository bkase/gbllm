mod artifact_b5_support;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use gbf_artifact::{
    ArtifactCore, DecodeCapabilitySet, Dtype, LexicalSpec_v1, ModelSpec_S3, PayloadRole,
    QuantSpec_S3, ReferenceEvalGraph, ReferenceNode, ReferenceOp, SequenceSemanticsSpec,
};
use tracing_subscriber::layer::Context;
use tracing_subscriber::prelude::*;

use artifact_b5_support::{id, manifest, quant_for, tensor};

#[test]
fn artifact_logging_captures_constructed_and_quant_coverage_missing_events() {
    let events = capture_events(|| {
        let linear = tensor(
            "tensor.linear.weight",
            Dtype::Ternary2,
            vec![16, 16],
            PayloadRole::DeployableWeight,
        );
        let quant = quant_for(std::slice::from_ref(&linear.id));
        let _ = ArtifactCore::new(
            manifest(),
            LexicalSpec_v1::pinned(),
            ModelSpec_S3::tiny("toy0"),
            quant,
            SequenceSemanticsSpec::linear_state(4).expect("sequence spec"),
            vec![linear],
            vec![],
            DecodeCapabilitySet::argmax_only(),
            None,
        )
        .expect("artifact constructs");

        let graph = ReferenceEvalGraph::new(
            vec![ReferenceNode::new(
                id("op.linear"),
                ReferenceOp::Linear,
                vec![id("hidden"), id("tensor.missing")],
                vec![id("out")],
            )],
            vec![],
        )
        .expect("valid graph");
        let _ = QuantSpec_S3::new(BTreeMap::new()).verify_coverage(&graph);
    });

    let constructed = events
        .iter()
        .find(|event| {
            event.fields.get("event_name").map(String::as_str) == Some("s3::artifact::constructed")
        })
        .expect("constructed event captured");
    assert_eq!(constructed.target, "gbf_artifact::artifact");
    assert_eq!(constructed.level, "DEBUG");
    assert_eq!(
        constructed.fields.get("tensor_count").map(String::as_str),
        Some("1")
    );
    assert_eq!(
        constructed
            .fields
            .get("weight_quant_coverage_count")
            .map(String::as_str),
        Some("1")
    );
    assert_eq!(
        constructed
            .fields
            .get("tied_alias_present")
            .map(String::as_str),
        Some("false")
    );

    let missing = events
        .iter()
        .find(|event| {
            event.fields.get("event_name").map(String::as_str)
                == Some("s3::quant::coverage_missing")
        })
        .expect("coverage missing event captured");
    assert_eq!(missing.target, "gbf_artifact::quant");
    assert_eq!(missing.level, "ERROR");
    assert_eq!(
        missing.fields.get("tensor_id").map(String::as_str),
        Some("tensor.missing")
    );
    assert_eq!(
        missing.fields.get("op_kind").map(String::as_str),
        Some("linear")
    );
}

#[derive(Clone, Debug, Default)]
struct TraceCapture {
    events: Arc<Mutex<Vec<CapturedEvent>>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
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
}

fn trim_debug_string(value: String) -> String {
    value
        .strip_prefix('"')
        .and_then(|stripped| stripped.strip_suffix('"'))
        .unwrap_or(&value)
        .to_owned()
}
