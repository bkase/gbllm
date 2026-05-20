#![cfg(feature = "s4")]

mod common;

use std::collections::BTreeMap;

use common::tracing_capture::{TraceCapture, captured_events, with_trace_capture};
use gbf_artifact::{BOS_ID, EOS_ID, GutenbergManifest, GutenbergSplit, UNK_ID};
use gbf_experiments::s4::contamination::{S4_CONTAMINATION_FINGERPRINT_KIND, sha256_high_u64};
use gbf_experiments::s4::corpus_oracle::{
    S4_CORPUS_ORACLE_CHECK_EVENT, S4_CORPUS_ORACLE_FALLBACK_USED_EVENT,
    S4_CORPUS_ORACLE_FIXTURE_FALLBACK, S4_CORPUS_ORACLE_OUTCOME_EVENT,
    S4_CORPUS_ORACLE_PRODUCTION_EVALUATOR, S4_CORPUS_ORACLE_PRODUCTION_STARTED_EVENT,
    S4ContaminationMathFixture, S4CorpusOracleCheckId, S4CorpusOracleInputs,
    S4ForcedIndexCollision, S4SplitDeterminismFixture, S4StripperOracleCase,
    contamination_math_fixture, fixture_post_strip_sha256, run_fixture_local_corpus_oracle,
    run_production_corpus_oracle,
};
use gbf_experiments::s4::schema::{HypothesisStatus, S4Hypothesis, S4Outcome};
use serde_json::json;

#[test]
fn fixture_local_corpus_oracle_passes_all_cor_checks_and_logs_named_fallback() {
    let inputs = clean_inputs();
    let capture = TraceCapture::default();

    let report = with_trace_capture(&capture, || run_fixture_local_corpus_oracle(&inputs));

    assert!(report.passed());
    assert_eq!(report.evaluator_name, S4_CORPUS_ORACLE_FIXTURE_FALLBACK);
    assert_eq!(
        report.fallback_name,
        Some(S4_CORPUS_ORACLE_FIXTURE_FALLBACK)
    );
    assert!(report.used_fallback());
    assert_eq!(report.checks.len(), 6);
    assert_eq!(
        report.hypothesis_status(S4Hypothesis::H1),
        HypothesisStatus::Confirmed
    );
    assert_eq!(
        report.hypothesis_status(S4Hypothesis::H2),
        HypothesisStatus::Confirmed
    );
    assert_eq!(
        S4CorpusOracleCheckId::ALL
            .iter()
            .map(|check| (
                check.as_str(),
                check.refuted_hypothesis(),
                check.refuted_outcome()
            ))
            .collect::<Vec<_>>(),
        vec![
            ("COr-1", S4Hypothesis::H1, S4Outcome::FailCorpusIntegrity),
            ("COr-2", S4Hypothesis::H1, S4Outcome::FailCorpusIntegrity),
            ("COr-3", S4Hypothesis::H1, S4Outcome::FailCorpusIntegrity),
            ("COr-4", S4Hypothesis::H1, S4Outcome::FailCorpusIntegrity),
            ("COr-5", S4Hypothesis::H1, S4Outcome::FailCorpusIntegrity),
            ("COr-6", S4Hypothesis::H2, S4Outcome::FailContamination),
        ]
    );

    let events = captured_events(&capture);
    let fallback = events
        .iter()
        .find(|event| event.name == S4_CORPUS_ORACLE_FALLBACK_USED_EVENT)
        .expect("fallback event emitted");
    assert_eq!(
        fallback.fields.get("fallback_name"),
        Some(&json!(S4_CORPUS_ORACLE_FIXTURE_FALLBACK))
    );
    assert_eq!(fallback.fields.get("check_count"), Some(&json!(6)));
    assert!(
        events
            .iter()
            .all(|event| event.name != S4_CORPUS_ORACLE_PRODUCTION_STARTED_EVENT),
        "fallback path must not emit production-start event"
    );

    let check_events = events
        .iter()
        .filter(|event| event.name == S4_CORPUS_ORACLE_CHECK_EVENT)
        .collect::<Vec<_>>();
    assert_eq!(check_events.len(), 6);
    assert_eq!(
        check_events
            .iter()
            .map(|event| event.fields.get("check_id").cloned().expect("check id"))
            .collect::<Vec<_>>(),
        vec![
            json!("COr-1"),
            json!("COr-2"),
            json!("COr-3"),
            json!("COr-4"),
            json!("COr-5"),
            json!("COr-6"),
        ]
    );
    assert!(
        check_events
            .iter()
            .all(|event| event.fields.get("passed") == Some(&json!(true)))
    );
    assert!(
        check_events
            .iter()
            .all(|event| event.fields.get("fallback_used") == Some(&json!(true)))
    );

    let outcome = events
        .iter()
        .find(|event| event.name == S4_CORPUS_ORACLE_OUTCOME_EVENT)
        .expect("outcome event emitted");
    assert_eq!(outcome.fields.get("passed"), Some(&json!(true)));
    assert_eq!(outcome.fields.get("failed_check_count"), Some(&json!(0)));
    assert_eq!(outcome.fields.get("fallback_used"), Some(&json!(true)));
}

#[test]
fn production_corpus_oracle_path_does_not_emit_fallback_used_event() {
    let inputs = clean_inputs();
    let capture = TraceCapture::default();

    let report = with_trace_capture(&capture, || run_production_corpus_oracle(&inputs));

    assert!(report.passed());
    assert_eq!(report.evaluator_name, S4_CORPUS_ORACLE_PRODUCTION_EVALUATOR);
    assert_eq!(report.fallback_name, None);
    assert!(!report.used_fallback());

    let events = captured_events(&capture);
    assert_eq!(
        events
            .iter()
            .filter(|event| event.name == S4_CORPUS_ORACLE_FALLBACK_USED_EVENT)
            .count(),
        0,
        "production COr path must not silently select the fixture fallback"
    );
    let production_started = events
        .iter()
        .find(|event| event.name == S4_CORPUS_ORACLE_PRODUCTION_STARTED_EVENT)
        .expect("production-start event emitted");
    assert_eq!(
        production_started.fields.get("evaluator_name"),
        Some(&json!(S4_CORPUS_ORACLE_PRODUCTION_EVALUATOR))
    );
    assert_eq!(
        production_started.fields.get("fallback_used"),
        Some(&json!(false))
    );

    let check_events = events
        .iter()
        .filter(|event| event.name == S4_CORPUS_ORACLE_CHECK_EVENT)
        .collect::<Vec<_>>();
    assert_eq!(check_events.len(), 6);
    assert!(
        check_events
            .iter()
            .all(|event| event.fields.get("evaluator_name")
                == Some(&json!(S4_CORPUS_ORACLE_PRODUCTION_EVALUATOR)))
    );
    assert!(
        check_events
            .iter()
            .all(|event| event.fields.get("fallback_used") == Some(&json!(false)))
    );

    let outcome = events
        .iter()
        .find(|event| event.name == S4_CORPUS_ORACLE_OUTCOME_EVENT)
        .expect("outcome event emitted");
    assert_eq!(
        outcome.fields.get("evaluator_name"),
        Some(&json!(S4_CORPUS_ORACLE_PRODUCTION_EVALUATOR))
    );
    assert_eq!(outcome.fields.get("fallback_used"), Some(&json!(false)));
    assert_eq!(outcome.fields.get("passed"), Some(&json!(true)));
}

#[test]
fn contamination_math_oracle_rejects_window_too_small_contract() {
    let mut inputs = clean_inputs();
    inputs.contamination_math.n = 3;

    let report = run_fixture_local_corpus_oracle(&inputs);

    assert!(!report.passed());
    assert!(
        report
            .failed_checks()
            .contains(&S4CorpusOracleCheckId::ContaminationOverlapMath)
    );
    assert_eq!(
        report.hypothesis_status(S4Hypothesis::H2),
        HypothesisStatus::Refuted
    );
}

#[test]
fn contamination_math_oracle_uses_sha256_high_u64_and_exact_collision_disambiguation() {
    let inputs = clean_inputs();
    let collision = inputs
        .contamination_math
        .forced_index_collisions
        .first()
        .expect("forced collision fixture");

    assert_ne!(collision.left_window, collision.right_window);
    assert_ne!(
        sha256_high_u64(&collision.left_window),
        sha256_high_u64(&collision.right_window)
    );
    assert_eq!(
        inputs.contamination_math.fingerprint_kind,
        S4_CONTAMINATION_FINGERPRINT_KIND
    );

    let report = run_fixture_local_corpus_oracle(&inputs);
    assert!(report.passed(), "{report:?}");
}

fn clean_inputs() -> S4CorpusOracleInputs {
    let manifest_json =
        include_str!("../../fixtures/schemas/s4/gutenberg_manifest_v1_minimal.json")
            .trim_end()
            .as_bytes()
            .to_vec();
    let manifest: GutenbergManifest =
        serde_json::from_slice(&manifest_json).expect("minimal manifest fixture parses");
    let raw_utf8 = gutenberg_raw("Café body with ASCII punctuation!\n");
    let split_map = split_map(&manifest);
    let train_bytes = b"train-bytes-replayed".to_vec();
    let val_bytes = b"val-bytes-replayed".to_vec();

    S4CorpusOracleInputs {
        manifest: manifest.clone(),
        manifest_canonical_json: manifest_json,
        stripper_cases: vec![S4StripperOracleCase {
            expected_post_strip_sha256: fixture_post_strip_sha256(&raw_utf8)
                .expect("fixture strips"),
            raw_utf8,
        }],
        charset_roundtrip_prefix: vec![BOS_ID, 0, 26, 75, UNK_ID, EOS_ID],
        split_replay: S4SplitDeterminismFixture {
            expected_split_map: split_map.clone(),
            replayed_split_map: split_map,
            expected_train_bytes: train_bytes.clone(),
            replayed_train_bytes: train_bytes,
            expected_val_bytes: val_bytes.clone(),
            replayed_val_bytes: val_bytes,
        },
        unmappable_manifest: manifest,
        contamination_math: clean_contamination_fixture(),
    }
}

fn clean_contamination_fixture() -> S4ContaminationMathFixture {
    let shared = window(1);
    let left_only = window(2);
    let right_only = window(3);
    let collision_left = window(20);
    let collision_right = window(21);
    contamination_math_fixture(
        vec![shared, left_only, collision_left],
        vec![shared, right_only, collision_right],
        1,
        vec![S4ForcedIndexCollision {
            left_window: collision_left,
            right_window: collision_right,
            forced_index: 7,
        }],
    )
}

fn split_map(manifest: &GutenbergManifest) -> BTreeMap<u32, GutenbergSplit> {
    manifest
        .sources
        .iter()
        .filter_map(|source| source.split.map(|split| (source.book_id, split)))
        .collect()
}

fn gutenberg_raw(body: &str) -> Vec<u8> {
    format!(
        "Header\n*** START OF THE PROJECT GUTENBERG EBOOK FIXTURE ***\n{body}\n*** END OF THE PROJECT GUTENBERG EBOOK FIXTURE ***\nFooter\n"
    )
    .into_bytes()
}

fn window(seed: u8) -> [u8; 13] {
    let mut out = [0_u8; 13];
    for (idx, byte) in out.iter_mut().enumerate() {
        *byte = seed.wrapping_add(idx as u8);
    }
    out
}
