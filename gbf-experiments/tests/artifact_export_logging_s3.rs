#![cfg(feature = "s3")]

mod artifact_s3_support;
mod common;

use artifact_s3_support::{export_product_from, frozen_student};
use common::tracing_capture::{TraceCapture, captured_events, with_trace_capture};
use gbf_experiments::s3::artifact::{
    ARTIFACT_EXPORT_LOG_TARGET, EVENT_NAME_ARTIFACT_EXPORT_COMPLETE,
    EVENT_NAME_ARTIFACT_EXPORT_QUANTSPEC_VALIDATED, EVENT_NAME_ARTIFACT_EXPORT_STARTED,
    EVENT_NAME_ARTIFACT_EXPORT_TENSOR_EMITTED, EVENT_NAME_ARTIFACT_EXPORT_TIED_ALIAS_RECORDED,
};
use serde_json::json;

#[test]
fn artifact_export_logging_s3() {
    let frozen = frozen_student(0);
    let capture = TraceCapture::default();
    let product = with_trace_capture(&capture, || export_product_from(&frozen));
    let events = captured_events(&capture);

    let started = event_by_name(&events, EVENT_NAME_ARTIFACT_EXPORT_STARTED);
    assert_eq!(started.fields.get("seed"), Some(&json!(0)));
    assert!(
        started
            .fields
            .get("frozen_student_storage_fingerprint")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|value| !value.is_empty())
    );

    let tensor_events = events
        .iter()
        .filter(|event| event.name == EVENT_NAME_ARTIFACT_EXPORT_TENSOR_EMITTED)
        .collect::<Vec<_>>();
    assert_eq!(tensor_events.len(), product.artifact.core.tensors.len());
    assert!(tensor_events.iter().all(|event| event.level == "TRACE"));
    assert!(
        tensor_events
            .iter()
            .any(|event| event.fields.get("alias_target_present") == Some(&json!(true)))
    );

    let quant = event_by_name(&events, EVENT_NAME_ARTIFACT_EXPORT_QUANTSPEC_VALIDATED);
    assert_eq!(
        quant.fields.get("tensors_resolved_via_naming"),
        Some(&json!(0))
    );
    assert_eq!(
        quant.fields.get("tensors_resolved_via_quant_spec"),
        quant.fields.get("total_tensors")
    );

    let tied = event_by_name(&events, EVENT_NAME_ARTIFACT_EXPORT_TIED_ALIAS_RECORDED);
    assert_eq!(tied.fields.get("shared"), Some(&json!(true)));

    let complete = event_by_name(&events, EVENT_NAME_ARTIFACT_EXPORT_COMPLETE);
    assert_eq!(
        complete.fields.get("artifact_self_hash"),
        Some(&json!(product.artifact_self_hash.to_string()))
    );
    assert_eq!(
        complete.fields.get("canonical_artifact_payload_sha"),
        Some(&json!(product.canonical_artifact_payload_sha.to_string()))
    );
    let _target_pin = ARTIFACT_EXPORT_LOG_TARGET;
}

fn event_by_name<'a>(
    events: &'a [common::tracing_capture::TracingEvent],
    name: &str,
) -> &'a common::tracing_capture::TracingEvent {
    events
        .iter()
        .find(|event| event.name == name)
        .unwrap_or_else(|| {
            panic!(
                "missing event {name}; saw {:?}",
                events.iter().map(|event| &event.name).collect::<Vec<_>>()
            )
        })
}
