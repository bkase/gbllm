#![cfg(feature = "s3")]

mod common;

use common::tracing_capture::{TraceCapture, captured_events, with_trace_capture};
use gbf_experiments::s3;
use serde_json::json;

#[test]
fn s3_module_loaded_event_is_emitted_once_with_feature_state() {
    let capture = TraceCapture::default();

    with_trace_capture(&capture, || {
        s3::ensure_module_loaded();
        s3::ensure_module_loaded();
    });

    let events = captured_events(&capture);
    let module_events = events
        .iter()
        .filter(|event| event.name == "s3::module_loaded")
        .collect::<Vec<_>>();
    assert_eq!(module_events.len(), 1);
    let event = module_events[0];
    assert_eq!(event.fields.get("schema_count"), Some(&json!(3)));
    assert_eq!(event.fields.get("type_count"), Some(&json!(15)));
    assert_eq!(event.fields.get("s3_enabled"), Some(&json!(true)));
    assert_eq!(
        event.fields.get("s3_phase_d_enabled"),
        Some(&json!(cfg!(feature = "s3-phase-d")))
    );
    assert_eq!(
        event.fields.get("s3_oracle_real_enabled"),
        Some(&json!(cfg!(feature = "s3-oracle-real")))
    );
    assert_eq!(
        event.fields.get("s3_oracle_fallback_enabled"),
        Some(&json!(cfg!(feature = "s3-oracle-fallback")))
    );
}
