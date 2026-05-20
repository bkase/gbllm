#![cfg(feature = "s4")]

mod common;

use common::tracing_capture::{TraceCapture, captured_events, with_trace_capture};
use gbf_artifact::{BOS_ID, EOS_ID};
use gbf_experiments::s4::contamination::{
    ContaminationDirection, ContaminationOutcome, CrossCorpusInputs, CrossCorpusSplit,
    DiagnosticOverlap, S4_CONTAMINATION_COLLISION_DISAMBIGUATION, S4_CONTAMINATION_DIRECTION_EVENT,
    S4_CONTAMINATION_FINGERPRINT_KIND, S4_CONTAMINATION_NGRAM_N, S4_CONTAMINATION_OUTCOME_EVENT,
    S4_CONTAMINATION_REPORT_SCHEMA, S4_CONTAMINATION_STARTED_EVENT,
    S4_CONTAMINATION_THRESHOLD_ESTIMATE_TAG, S4_CONTAMINATION_THRESHOLD_PROVENANCE,
    s4_cross_corpus_contamination, verify_h2_contamination_report,
};
use gbf_experiments::s4::schema::HypothesisStatus;
use gbf_foundation::Hash256;
use serde_json::{Value, json};

#[test]
fn clean_report_pins_schema_shape_self_hash_and_h2_verdict() {
    let report = s4_cross_corpus_contamination(inputs(
        vec![window_doc(1)],
        vec![window_doc(2)],
        vec![window_doc(3)],
        vec![window_doc(4)],
    ))
    .expect("clean report builds");

    assert_eq!(report.schema, S4_CONTAMINATION_REPORT_SCHEMA);
    assert_eq!(report.n, S4_CONTAMINATION_NGRAM_N as u64);
    assert_eq!(report.fingerprint_kind, S4_CONTAMINATION_FINGERPRINT_KIND);
    assert_eq!(
        report.collision_disambiguation,
        S4_CONTAMINATION_COLLISION_DISAMBIGUATION
    );
    assert_eq!(
        report.threshold_provenance.provenance,
        S4_CONTAMINATION_THRESHOLD_PROVENANCE
    );
    assert_eq!(
        report.threshold_provenance.estimate_tag,
        S4_CONTAMINATION_THRESHOLD_ESTIMATE_TAG
    );
    assert_eq!(report.fingerprint_count_ts_val_ngrams, 1);
    assert_eq!(report.fingerprint_count_gb_val_ngrams, 1);
    assert_eq!(report.fingerprint_count_ts_train_ngrams, 1);
    assert_eq!(report.fingerprint_count_gb_train_ngrams, 1);
    assert_eq!(report.overlap_ts_train_to_gb_val, 0.0);
    assert_eq!(report.overlap_gb_train_to_ts_val, 0.0);
    assert!(report.warnings.is_empty());
    assert!(report.hard_failures.is_empty());
    assert_eq!(report.outcome, ContaminationOutcome::Clean);
    assert_eq!(
        report.contamination_self_hash,
        report.compute_self_hash().expect("self-hash recomputes")
    );
    report
        .validate_canonical_write()
        .expect("canonical report validates");

    let verdict = verify_h2_contamination_report(&report);
    assert_eq!(verdict.status, HypothesisStatus::Confirmed);
    assert!(!verdict.contamination_warning);

    let json: Value =
        serde_json::from_slice(&report.canonical_bytes().expect("canonical bytes")).unwrap();
    assert_eq!(json["schema"], json!("s4_contamination_report.v1"));
    assert_eq!(json["fingerprint_kind"], json!("sha256_high_u64"));
    assert_eq!(
        json["collision_disambiguation"],
        json!("exact_13_token_bytes_on_hit")
    );
    assert_eq!(
        json["denominator_policy"]["gated_directions"],
        json!("full validation split against full opposite train split")
    );
    assert_eq!(
        json["denominator_policy"]["diagnostic_directions"],
        json!(
            "train/train diagnostic directions use deterministic per-document stratified samples if cap applies; validation/validation diagnostic directions reuse full validation sets"
        )
    );
    assert_eq!(
        json["threshold_provenance"],
        json!({
            "estimate_tag": "[ESTIMATE]",
            "hard_fail_threshold": 0.001,
            "provenance": "D6 [ESTIMATE for review]",
            "warn_threshold": 0.0005,
        })
    );
    assert_eq!(json["outcome"], json!({ "kind": "Clean" }));
}

#[test]
fn overlap_math_uses_exact_unique_denominator_fraction() {
    let report = s4_cross_corpus_contamination(inputs(
        vec![window_doc(10)],
        vec![window_doc(20)],
        vec![window_doc(30)],
        vec![window_doc(10), window_doc(11), window_doc(12)],
    ))
    .expect("report builds");

    assert_eq!(report.fingerprint_count_gb_val_ngrams, 3);
    assert_eq!(report.overlap_ts_train_to_gb_val, 1.0 / 3.0);
    assert_eq!(
        report.hard_failures[0].direction,
        ContaminationDirection::TsTrainContainsGbVal
    );
    assert_eq!(report.hard_failures[0].overlap_count, 1);
    assert_eq!(report.hard_failures[0].denominator_count, 3);
}

#[test]
fn contamination_logging_emits_started_direction_and_outcome_events() {
    let capture = TraceCapture::default();

    let report = with_trace_capture(&capture, || {
        s4_cross_corpus_contamination(inputs(
            vec![window_doc(1)],
            vec![window_doc(2)],
            vec![window_doc(3)],
            vec![window_doc(4)],
        ))
        .expect("clean report builds")
    });

    let events = captured_events(&capture);
    let started = events
        .iter()
        .find(|event| event.name == S4_CONTAMINATION_STARTED_EVENT)
        .expect("started event emitted");
    assert_eq!(
        started.fields.get("threshold_estimate_tag"),
        Some(&json!(S4_CONTAMINATION_THRESHOLD_ESTIMATE_TAG))
    );
    assert_eq!(
        started.fields.get("threshold_provenance"),
        Some(&json!(S4_CONTAMINATION_THRESHOLD_PROVENANCE))
    );

    let direction_events = events
        .iter()
        .filter(|event| event.name == S4_CONTAMINATION_DIRECTION_EVENT)
        .collect::<Vec<_>>();
    assert_eq!(direction_events.len(), 6);
    let gated = direction_events
        .iter()
        .find(|event| {
            event.fields.get("direction")
                == Some(&json!(
                    ContaminationDirection::TsTrainContainsGbVal.as_str()
                ))
        })
        .expect("gated direction event emitted");
    assert_eq!(gated.fields.get("direction_kind"), Some(&json!("gated")));
    assert_eq!(
        gated.fields.get("overlap_status"),
        Some(&json!("available"))
    );
    assert_eq!(gated.fields.get("thresholds_apply"), Some(&json!(true)));
    let validation_diagnostic = direction_events
        .iter()
        .find(|event| {
            event.fields.get("direction")
                == Some(&json!(ContaminationDirection::TsValOverlapsGbVal.as_str()))
        })
        .expect("validation diagnostic direction event emitted");
    assert_eq!(
        validation_diagnostic.fields.get("direction_kind"),
        Some(&json!("diagnostic"))
    );
    assert_eq!(
        validation_diagnostic.fields.get("denominator_policy"),
        Some(&json!(
            "full validation split against full validation split"
        ))
    );
    assert_eq!(
        validation_diagnostic.fields.get("thresholds_apply"),
        Some(&json!(false))
    );

    let outcome = events
        .iter()
        .find(|event| event.name == S4_CONTAMINATION_OUTCOME_EVENT)
        .expect("outcome event emitted");
    assert_eq!(outcome.fields.get("outcome"), Some(&json!("Clean")));
    assert_eq!(
        outcome.fields.get("contamination_self_hash"),
        Some(&json!(report.contamination_self_hash.to_string()))
    );
}

#[test]
fn gated_warning_and_hard_fail_map_to_h2_verdicts() {
    let warn_report = s4_cross_corpus_contamination(inputs(
        vec![window_doc(7)],
        vec![window_doc(5_000)],
        vec![window_doc(6_000)],
        (0..2_000).map(window_doc).collect(),
    ))
    .expect("warning report builds");

    assert_eq!(warn_report.overlap_ts_train_to_gb_val, 1.0 / 2_000.0);
    assert_eq!(
        warn_report.outcome,
        ContaminationOutcome::Warn {
            findings: vec![ContaminationDirection::TsTrainContainsGbVal],
        }
    );
    let warn_verdict = verify_h2_contamination_report(&warn_report);
    assert_eq!(warn_verdict.status, HypothesisStatus::Confirmed);
    assert!(warn_verdict.contamination_warning);

    let hard_report = s4_cross_corpus_contamination(inputs(
        vec![window_doc(7)],
        vec![window_doc(8)],
        vec![window_doc(9)],
        (0..999).map(window_doc).collect(),
    ))
    .expect("hard-fail report builds");

    assert!(matches!(
        hard_report.outcome,
        ContaminationOutcome::HardFail { .. }
    ));
    assert_eq!(
        hard_report.hard_failures[0].direction,
        ContaminationDirection::TsTrainContainsGbVal
    );
    assert_eq!(
        hard_report.warnings[0].direction,
        ContaminationDirection::TsTrainContainsGbVal
    );
    assert_eq!(
        hard_report.outcome,
        ContaminationOutcome::HardFail {
            failures: vec![ContaminationDirection::TsTrainContainsGbVal],
            warnings: vec![ContaminationDirection::TsTrainContainsGbVal],
        }
    );
    let hard_verdict = verify_h2_contamination_report(&hard_report);
    assert_eq!(hard_verdict.status, HypothesisStatus::Refuted);
    assert!(!hard_verdict.contamination_warning);
}

#[test]
fn bos_eos_control_tokens_are_excluded_from_overlap_windows() {
    let body_short_of_window = vec![9_u8; S4_CONTAMINATION_NGRAM_N - 1];
    let control_only_match = bos_body_eos_doc(&body_short_of_window);
    let report = s4_cross_corpus_contamination(inputs(
        vec![window_doc(1), control_only_match.clone()],
        vec![window_doc(2)],
        vec![window_doc(3)],
        vec![window_doc(4), control_only_match],
    ))
    .expect("report builds");

    assert_eq!(report.fingerprint_count_ts_train_ngrams, 1);
    assert_eq!(report.fingerprint_count_gb_val_ngrams, 1);
    assert_eq!(report.overlap_ts_train_to_gb_val, 0.0);
    assert_eq!(report.outcome, ContaminationOutcome::Clean);
}

#[test]
fn document_boundary_windows_never_cross_documents() {
    let target = window_doc(88);

    for split_at in 1..S4_CONTAMINATION_NGRAM_N {
        let report = s4_cross_corpus_contamination(inputs(
            vec![
                window_doc(1),
                target[..split_at].to_vec(),
                target[split_at..].to_vec(),
            ],
            vec![window_doc(2)],
            vec![window_doc(3)],
            vec![target.clone()],
        ))
        .expect("report builds");

        assert_eq!(
            report.overlap_ts_train_to_gb_val, 0.0,
            "split_at={split_at} must not produce a cross-document 13-gram"
        );
    }
}

#[test]
fn diagnostic_train_train_overlap_is_reported_but_not_gating() {
    let shared = window_doc(42);
    let report = s4_cross_corpus_contamination(inputs(
        vec![shared.clone()],
        vec![window_doc(43)],
        vec![shared],
        vec![window_doc(44)],
    ))
    .expect("diagnostic-only overlap report builds");

    assert_eq!(
        report.overlap_ts_train_contains_gb_train,
        DiagnosticOverlap::Fraction(1.0)
    );
    assert_eq!(
        report.overlap_gb_train_contains_ts_train,
        DiagnosticOverlap::Fraction(1.0)
    );
    assert_eq!(report.outcome, ContaminationOutcome::Clean);
    assert!(report.warnings.is_empty());
    assert!(report.hard_failures.is_empty());
    assert_eq!(
        verify_h2_contamination_report(&report).status,
        HypothesisStatus::Confirmed
    );
}

#[test]
fn diagnostic_not_available_is_serialized_as_marker() {
    let mut report = s4_cross_corpus_contamination(inputs(
        vec![window_doc(1)],
        vec![window_doc(2)],
        vec![window_doc(3)],
        vec![window_doc(4)],
    ))
    .expect("report builds");
    report.overlap_ts_train_contains_gb_train = DiagnosticOverlap::DiagnosticNotAvailable;
    report.contamination_self_hash = report.compute_self_hash().expect("self hash recomputes");

    report
        .validate_canonical_write()
        .expect("diagnostic marker validates");
    let json: Value =
        serde_json::from_slice(&report.canonical_bytes().expect("canonical bytes")).unwrap();

    assert_eq!(
        json["overlap_ts_train_contains_gb_train"],
        json!("diagnostic_not_available")
    );
}

#[test]
fn invalid_self_hash_refutes_h2_verifier() {
    let mut report = s4_cross_corpus_contamination(inputs(
        vec![window_doc(1)],
        vec![window_doc(2)],
        vec![window_doc(3)],
        vec![window_doc(4)],
    ))
    .expect("report builds");
    report.contamination_self_hash = hash(99);

    let verdict = verify_h2_contamination_report(&report);

    assert_eq!(verdict.status, HypothesisStatus::Refuted);
    assert!(!verdict.contamination_warning);
}

fn inputs(
    ts_train: Vec<Vec<u8>>,
    ts_val: Vec<Vec<u8>>,
    gb_train: Vec<Vec<u8>>,
    gb_val: Vec<Vec<u8>>,
) -> CrossCorpusInputs {
    CrossCorpusInputs {
        tinystories_manifest_self_hash: hash(1),
        gutenberg_manifest_self_hash: hash(2),
        ts_train: CrossCorpusSplit::from_fixture_documents(ts_train),
        ts_val: CrossCorpusSplit::from_fixture_documents(ts_val),
        gb_train: CrossCorpusSplit::from_fixture_documents(gb_train),
        gb_val: CrossCorpusSplit::from_fixture_documents(gb_val),
    }
}

fn window_doc(value: u64) -> Vec<u8> {
    let mut remaining = value;
    let mut doc = Vec::with_capacity(S4_CONTAMINATION_NGRAM_N);
    for _ in 0..S4_CONTAMINATION_NGRAM_N {
        doc.push((remaining % 76) as u8);
        remaining /= 76;
    }
    doc
}

fn bos_body_eos_doc(body: &[u8]) -> Vec<u8> {
    let mut doc = Vec::with_capacity(body.len() + 2);
    doc.push(BOS_ID);
    doc.extend_from_slice(body);
    doc.push(EOS_ID);
    doc
}

fn hash(fill: u8) -> Hash256 {
    Hash256::from_bytes([fill; 32])
}
