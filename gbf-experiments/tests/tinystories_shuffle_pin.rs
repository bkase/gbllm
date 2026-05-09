use std::path::{Path, PathBuf};

mod common;

use common::tracing_capture::{TraceCapture, captured_events, with_trace_capture};
use gbf_experiments::s1::logging::{event, field};
use gbf_experiments::s1::manifest::{
    S1ShufflePinError, TINYSTORIES_VAL_SHUFFLE_PIN_PASS_VERSION, load_val_bytes,
    read_tinystories_manifest, verify_val_shuffle_pin,
};
use gbf_experiments::s1::neg_test::{NEGATIVE_TEST_SHUFFLE_SEED, fisher_yates};
use gbf_foundation::{Hash256, sha256};
use serde_json::json;

const EXPECTED_VAL_SHUFFLE_DEADEEF_SHA256: &str =
    "sha256:33ab115b5d230b6286fd39347e7e542bb7663ed148d80e16fc3de1a866f60388";

#[test]
fn tinystories_shuffle_pin_manifest_guard() {
    let manifest = read_tinystories_manifest(manifest_path()).expect("TinyStories manifest");
    let pinned_at_pass_version = manifest
        .val_shuffle_deadeef_pinned_at_pass_version
        .as_deref()
        .expect("TinyStories shuffle pin pass version");

    assert_eq!(
        pinned_at_pass_version,
        TINYSTORIES_VAL_SHUFFLE_PIN_PASS_VERSION
    );
    assert_eq!(
        manifest.val_shuffle_deadeef_sha256,
        Some(expected_shuffle_hash_for_pass_version(
            pinned_at_pass_version
        ))
    );
}

#[test]
#[ignore = "requires canonical TinyStories validation bytes at corpus/tinystories/raw/TinyStoriesV2-GPT4-valid.txt; run when re-pinning"]
fn tinystories_shuffle_pin_recomputes_from_canonical_validation_bytes() {
    let manifest_path = manifest_path();
    let manifest = read_tinystories_manifest(&manifest_path).expect("TinyStories manifest");
    let val_path = manifest.split_path(gbf_data::SplitRole::Validation);
    if !val_path.exists() {
        panic!(
            "canonical TinyStories validation bytes are absent at {}; download the manifest validation split before re-pinning",
            val_path.display()
        );
    }

    let val = load_val_bytes(&manifest).expect("canonical validation bytes load");
    let shuffled = fisher_yates(&val, NEGATIVE_TEST_SHUFFLE_SEED);
    assert_eq!(
        byte_multiset(&shuffled),
        byte_multiset(&val),
        "canonical validation shuffle must preserve the byte multiset"
    );
    assert_ne!(
        shuffled, val,
        "canonical validation shuffle must be non-identity"
    );
    let expected = manifest
        .val_shuffle_deadeef_sha256
        .expect("TinyStories shuffle pin");
    let observed = verify_val_shuffle_pin(expected, &val).expect("shuffle pin verifies");
    eprintln!("computed val_shuffle_deadeef_sha256={observed}");
    assert_eq!(observed, expected);
}

#[test]
fn shuffle_pin_verify_logs_compute_and_ok_events() {
    let capture = TraceCapture::default();
    let val = b"abcabc<|endoftext|>";
    let expected = sha256(fisher_yates(val, NEGATIVE_TEST_SHUFFLE_SEED));

    let observed = with_trace_capture(&capture, || {
        verify_val_shuffle_pin(expected, val).expect("shuffle pin verifies")
    });

    assert_eq!(observed, expected);
    let events = manifest_shuffle_events(&capture);
    assert_eq!(
        events
            .iter()
            .map(|event| event.name.as_str())
            .collect::<Vec<_>>(),
        vec![
            event::MANIFEST_SHUFFLE_PIN_COMPUTE,
            event::MANIFEST_SHUFFLE_PIN_VERIFY_OK,
        ]
    );
    assert_eq!(events[0].level, "INFO");
    assert_eq!(
        events[0].fields.get(field::SHUFFLE_SEED),
        Some(&json!(NEGATIVE_TEST_SHUFFLE_SEED))
    );
    assert_eq!(
        events[0].fields.get(field::TOKEN_COUNT),
        Some(&json!(val.len()))
    );
    assert_eq!(
        events[0].fields.get(field::SHUFFLED_VAL_SHA256),
        Some(&json!(expected.to_string()))
    );
    assert_eq!(events[1].level, "INFO");
    assert_eq!(
        events[1].fields.get(field::EXPECTED),
        Some(&json!(expected.to_string()))
    );
    assert_eq!(
        events[1].fields.get(field::OBSERVED),
        Some(&json!(observed.to_string()))
    );
}

#[test]
fn shuffle_pin_verify_logs_fail_event_before_typed_error() {
    let capture = TraceCapture::default();
    let val = b"abcabc<|endoftext|>";
    let expected = Hash256::ZERO;
    let observed = sha256(fisher_yates(val, NEGATIVE_TEST_SHUFFLE_SEED));

    let error = with_trace_capture(&capture, || {
        verify_val_shuffle_pin(expected, val).expect_err("shuffle pin mismatch")
    });

    assert!(matches!(
        error,
        S1ShufflePinError::Mismatch {
            expected: Hash256::ZERO,
            observed: actual,
        } if actual == observed
    ));
    let events = manifest_shuffle_events(&capture);
    assert_eq!(
        events
            .iter()
            .map(|event| event.name.as_str())
            .collect::<Vec<_>>(),
        vec![
            event::MANIFEST_SHUFFLE_PIN_COMPUTE,
            event::MANIFEST_SHUFFLE_PIN_VERIFY_FAIL,
        ]
    );
    assert_eq!(events[1].level, "ERROR");
    assert_eq!(
        events[1].fields.get(field::EXPECTED),
        Some(&json!(expected.to_string()))
    );
    assert_eq!(
        events[1].fields.get(field::OBSERVED),
        Some(&json!(observed.to_string()))
    );
}

fn manifest_path() -> PathBuf {
    workspace_root().join("fixtures/corpora/tinystories.toml")
}

fn workspace_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
}

fn expected_shuffle_hash_for_pass_version(pass_version: &str) -> Hash256 {
    match pass_version {
        TINYSTORIES_VAL_SHUFFLE_PIN_PASS_VERSION => EXPECTED_VAL_SHUFFLE_DEADEEF_SHA256
            .parse()
            .expect("expected shuffle hash"),
        other => panic!("unrecognized TinyStories shuffle pin pass version: {other}"),
    }
}

fn byte_multiset(bytes: &[u8]) -> [usize; 256] {
    let mut counts = [0usize; 256];
    for &byte in bytes {
        counts[usize::from(byte)] += 1;
    }
    counts
}

fn manifest_shuffle_events(capture: &TraceCapture) -> Vec<common::tracing_capture::TracingEvent> {
    captured_events(capture)
        .into_iter()
        .filter(|event| event.name.starts_with("s1.manifest.shuffle_pin."))
        .collect()
}
