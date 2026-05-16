#![cfg(feature = "s3")]

mod common;
mod common_s3;

use common_s3::fixtures::{build_kind_matrix, fixed_kn_fixture, mock_artifact, toy0_model_factory};
use common_s3::helpers::ndjson_capture::NdjsonCaptureSink;
use common_s3::helpers::tracing_capture_s3::{capture_events, events_to_ndjson};
use gbf_experiments::s3::schema::S3BuildKind;
use serde_json::{Value, json};

#[test]
fn s3_fixture_builders_are_repeatable() {
    let first = toy0_model_factory(7);
    let second = toy0_model_factory(7);
    assert_eq!(first, second);
    assert_eq!(first.vocab_size, 80);
    assert_eq!(first.logits().iter().sum::<f32>(), 0.0);

    let kn = fixed_kn_fixture();
    assert_eq!(kn.order, 5);
    assert!(kn.expected_bpc_char > 0.0);

    let artifact = mock_artifact(S3BuildKind::s3_v0_success_real_oracle, b"payload");
    assert_eq!(
        artifact,
        mock_artifact(S3BuildKind::s3_v0_success_real_oracle, b"payload")
    );
}

#[test]
fn s3_build_kind_matrix_iterates_canonical_variants() {
    assert_eq!(
        build_kind_matrix().collect::<Vec<_>>(),
        S3BuildKind::ALL.to_vec()
    );
}

#[test]
fn ndjson_capture_sink_writes_valid_canonical_lines() {
    let mut sink = NdjsonCaptureSink::new();
    sink.push(&json!({"z": 1, "a": true}));
    sink.push(&json!({"event": "s3-fixture"}));

    let bytes = sink.to_bytes();
    assert!(bytes.ends_with(b"\n"));
    assert_eq!(sink.parsed_lines().len(), 2);
    for line in bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        let _: Value = serde_json::from_slice(line).expect("NDJSON line parses");
    }
}

#[test]
fn tracing_capture_s3_records_events_and_serializes_ndjson() {
    let (_, events) = capture_events(|| {
        tracing::info!(
            event_name = "s3_fixture_event",
            fixture = "common_s3",
            "s3 common fixture event"
        );
    });

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].name, "s3_fixture_event");
    assert!(
        String::from_utf8(events_to_ndjson(&events))
            .expect("trace NDJSON is UTF-8")
            .contains("s3_fixture_event")
    );
}
