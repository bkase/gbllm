#![cfg(feature = "s3")]

mod common;

use std::path::{Path, PathBuf};

use common::tracing_capture::{TraceCapture, captured_events, with_trace_capture};
use gbf_artifact::TextCharSeq;
use gbf_data::charset_v1::normalize_raw;
use gbf_experiments::s3::baseline::{KnBaselineInputs, s3_fit_kn5};

#[test]
fn baseline_logging_emits_happy_path_events() {
    let capture = TraceCapture::default();
    let product = with_trace_capture(&capture, || {
        let (train, val) = oracle_sequences();
        s3_fit_kn5(KnBaselineInputs {
            train_post: train,
            val_post: val,
        })
        .expect("oracle baseline fits")
    });
    let events = captured_events(&capture);

    assert_event(&events, "s3::baseline::fit_started");
    assert_event(&events, "s3::baseline::counts_computed");
    assert_event(&events, "s3::baseline::discounts_fit");
    assert_event(&events, "s3::baseline::scoring_complete");
    let discounts = events
        .iter()
        .find(|event| event.name == "s3::baseline::discounts_fit")
        .expect("discounts_fit event emitted");
    assert!(
        discounts
            .fields
            .get("d_1_order_2")
            .and_then(serde_json::Value::as_f64)
            .is_some(),
        "discounts_fit should expose structured per-order numeric discount fields"
    );
    assert!(
        discounts
            .fields
            .get("y_order_5")
            .and_then(serde_json::Value::as_f64)
            .is_some(),
        "discounts_fit should expose structured per-order numeric y fields"
    );
    assert!(
        events
            .iter()
            .any(|event| event.fields.get("baseline_self_hash")
                == Some(&serde_json::json!(product.baseline_self_hash.to_string())))
    );
}

#[test]
fn baseline_logging_emits_aborted_event_for_fail_baseline() {
    let capture = TraceCapture::default();
    with_trace_capture(&capture, || {
        let train = TextCharSeq::new(vec![
            3, 1, 3, 1, 4, 4, 1, 5, 4, 1, 4, 1, 4, 1, 0, 4, 4, 1, 4, 4,
        ])
        .expect("failure fixture uses valid text ids");
        let val = TextCharSeq::new(vec![1]).expect("validation fixture uses valid text ids");
        let _ = s3_fit_kn5(KnBaselineInputs {
            train_post: train,
            val_post: val,
        })
        .expect_err("fail-baseline fixture aborts");
    });
    let events = captured_events(&capture);

    assert_event(&events, "s3::baseline::fit_started");
    assert_event(&events, "s3::baseline::counts_computed");
    assert_event(&events, "s3::baseline::aborted");
    let aborted = events
        .iter()
        .find(|event| event.name == "s3::baseline::aborted")
        .expect("aborted event emitted");
    assert_eq!(aborted.fields.get("order"), Some(&serde_json::json!(4)));
}

fn assert_event(events: &[common::tracing_capture::TracingEvent], name: &str) {
    assert!(
        events.iter().any(|event| event.name == name),
        "missing event {name}; saw {:?}",
        events.iter().map(|event| &event.name).collect::<Vec<_>>()
    );
}

fn oracle_sequences() -> (TextCharSeq, TextCharSeq) {
    let root = workspace_root().join("fixtures/baselines/kn_oracle");
    (
        normalize_file(&root.join("train.bytes")),
        normalize_file(&root.join("eval.bytes")),
    )
}

fn normalize_file(path: &Path) -> TextCharSeq {
    let bytes = std::fs::read(path).expect("fixture bytes read");
    let normalized = normalize_raw(&bytes).expect("fixture normalizes");
    assert!(!normalized.dropped);
    normalized.tokens
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf()
}
