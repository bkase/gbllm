#![cfg(feature = "s3")]

mod common;
mod score_s3_support;

use common::tracing_capture::{TraceCapture, captured_events, with_trace_capture};
use gbf_experiments::s3::score::{KnScorer, S3_SCORE_CHUNK_SIZE, s3_score_bpc_char};
use gbf_oracle::scorers::ReferenceScorer;
use score_s3_support::{kn_product_for_val, predictable_a_bundle, repeated_a};

#[test]
fn score_smoke_fixture_reference_beats_kn_and_writes_optional_ndjson() {
    let val = repeated_a(64);
    let bundle = predictable_a_bundle();
    let reference = ReferenceScorer::new(&bundle);
    let (train, kn_product) = kn_product_for_val(val.clone());
    let kn = KnScorer::from_product_and_train(&kn_product, &train).unwrap();

    let capture = TraceCapture::default();
    let (reference_score, kn_score) = with_trace_capture(&capture, || {
        (
            s3_score_bpc_char(reference, &val, S3_SCORE_CHUNK_SIZE),
            s3_score_bpc_char(kn, &val, S3_SCORE_CHUNK_SIZE),
        )
    });
    let events = captured_events(&capture);

    assert!(
        reference_score.bpc_char.get() < kn_score.bpc_char.get() - 0.05,
        "reference={} kn={}",
        reference_score.bpc_char.get(),
        kn_score.bpc_char.get()
    );
    assert!(
        events
            .iter()
            .any(|event| event.name == "s3::score::started"),
        "started event emitted"
    );
    assert!(
        events
            .iter()
            .any(|event| event.name == "s3::score::complete"),
        "complete event emitted"
    );

    if let Ok(path) = std::env::var("S3_SCORE_CAPTURE_NDJSON") {
        let mut bytes = Vec::new();
        for event in events {
            serde_json::to_writer(
                &mut bytes,
                &serde_json::json!({
                    "name": event.name,
                    "level": event.level,
                    "fields": event.fields,
                }),
            )
            .expect("event JSON encodes");
            bytes.push(b'\n');
        }
        std::fs::write(path, bytes).expect("score smoke NDJSON writes");
    }
}
