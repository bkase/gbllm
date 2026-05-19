use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use gbf_artifact::{BOS_ID, RESERVED_ID, TextCharSeq};
use tracing_subscriber::layer::Context;
use tracing_subscriber::prelude::*;

#[test]
fn text_char_seq_rejections_emit_subscriber_captured_warn_events() {
    for (ids, expected_kind, expected_id, expected_position) in [
        (vec![RESERVED_ID], "ReservedId76", RESERVED_ID, 0_u64),
        (vec![0, BOS_ID], "ControlIdInTextStream", BOS_ID, 1_u64),
        (vec![0, 1, 80], "OutOfRange", 80, 2_u64),
    ] {
        let events = capture_events(|| {
            let _ = TextCharSeq::new(ids);
        });

        assert_eq!(events.len(), 1);
        let event = &events[0];
        assert_eq!(event.target, "gbf_artifact::lexical");
        assert_eq!(event.level, "WARN");
        assert_eq!(
            event.fields.get("event_name").map(String::as_str),
            Some("s3::lexical::reject_construction")
        );
        assert_eq!(
            event.fields.get("error_kind").map(String::as_str),
            Some(expected_kind)
        );
        assert_eq!(
            event.fields.get("id").map(String::as_str),
            Some(expected_id.to_string().as_str())
        );
        assert_eq!(
            event.fields.get("position").map(String::as_str),
            Some(expected_position.to_string().as_str())
        );
        assert_eq!(
            event.fields.get("callsite_module").map(String::as_str),
            Some("gbf_artifact::lexical")
        );
    }
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
