#![cfg(feature = "s4")]

mod common;

use std::fs;

use common::tracing_capture::{TraceCapture, captured_events, with_trace_capture};
use gbf_experiments::s4::corpus_progression::{
    S4_CORPUS_PROGRESSION_EMIT_EVENT, S4_CORPUS_PROGRESSION_GATE_TS_TO_GUTENBERG,
    S4_CORPUS_PROGRESSION_SCHEDULE_VERSION, S4_CORPUS_PROGRESSION_SCHEMA, S4CorpusProgressionError,
    S4CorpusProgressionReport, write_s4_corpus_progression_report,
};
use gbf_experiments::s4::promote::{
    PromotionGateArtifactRef, PromotionGateInputBindings, PromotionGateOutcome,
    PromotionGateProduct,
};
use gbf_experiments::s4::schema::S4_OPTIMIZER_STEPS_GUTENBERG;
use gbf_foundation::Hash256;
use serde_json::{Value, json};

#[test]
fn corpus_progression_report_round_trips_canonical_schedule() {
    let temp = tempfile::tempdir().expect("tempdir");
    let report = S4CorpusProgressionReport::new(h(1), h(2), None).expect("report");

    let bytes = report.canonical_bytes().expect("canonical bytes");
    let decoded: S4CorpusProgressionReport = serde_json::from_slice(&bytes).expect("decode report");
    assert_eq!(
        decoded.canonical_bytes().expect("decoded canonical bytes"),
        bytes
    );

    let value: Value = serde_json::from_slice(&bytes).expect("json value");
    assert_eq!(value["schema"], json!(S4_CORPUS_PROGRESSION_SCHEMA));
    assert_eq!(
        value["schedule"]["schedule_version"],
        json!(S4_CORPUS_PROGRESSION_SCHEDULE_VERSION)
    );
    assert_eq!(
        value["schedule"]["ordered_corpora"],
        json!([
            {"corpus": "TinyStories", "corpus_self_hash": h(1).to_string()},
            {"corpus": "Gutenberg", "corpus_self_hash": h(2).to_string()}
        ])
    );
    assert_eq!(
        value["schedule"]["edges"],
        json!([{
            "from": "TinyStories",
            "to": "Gutenberg",
            "gate": S4_CORPUS_PROGRESSION_GATE_TS_TO_GUTENBERG
        }])
    );
    assert_eq!(value["schedule"]["seed_list"], json!([0, 1, 2, 3, 4]));
    assert_eq!(
        value["schedule"]["phase_boundaries"],
        json!([
            {
                "active_corpus": "TinyStories",
                "train_phase": "full_numeric_qat",
                "start_progression_step": 0,
                "end_progression_step_exclusive": 1
            },
            {
                "active_corpus": "Gutenberg",
                "train_phase": "full_numeric_qat",
                "start_progression_step": 1,
                "end_progression_step_exclusive": S4_OPTIMIZER_STEPS_GUTENBERG + 1
            }
        ])
    );
    assert_eq!(
        value["corpus_progression_self_hash"],
        json!(report.corpus_progression_self_hash.to_string())
    );

    let path = temp
        .path()
        .join("experiments/S4/corpus_progression/schedule.json");
    write_s4_corpus_progression_report(&path, &report).expect("write report");
    assert_eq!(fs::read(path).expect("written report"), bytes);
}

#[test]
fn corpus_progression_emit_event_pins_subscriber_captured_fields() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp
        .path()
        .join("experiments/S4/corpus_progression/schedule.json");
    let report = S4CorpusProgressionReport::new(h(1), h(2), None).expect("report");
    let capture = TraceCapture::default();

    with_trace_capture(&capture, || {
        write_s4_corpus_progression_report(&path, &report).expect("write report")
    });

    let events = captured_events(&capture);
    let emitted = events
        .iter()
        .find(|event| event.name == S4_CORPUS_PROGRESSION_EMIT_EVENT)
        .expect("corpus progression emit event");
    assert_eq!(
        emitted.fields.get("schema"),
        Some(&json!(S4_CORPUS_PROGRESSION_SCHEMA))
    );
    assert_eq!(
        emitted.fields.get("schedule_self_hash"),
        Some(&json!(report.schedule.schedule_self_hash.to_string()))
    );
    assert_eq!(
        emitted.fields.get("corpus_progression_self_hash"),
        Some(&json!(report.corpus_progression_self_hash.to_string()))
    );
    assert_eq!(
        emitted.fields.get("corpora"),
        Some(&json!("[TinyStories, Gutenberg]"))
    );
    assert_eq!(
        emitted.fields.get("path"),
        Some(&json!(path.display().to_string()))
    );
}

#[test]
fn corpus_progression_schedule_self_hash_changes_when_ordered_corpora_change() {
    let report = S4CorpusProgressionReport::new(h(1), h(2), None).expect("report");
    let mut changed_schedule = report.schedule.clone();
    changed_schedule.ordered_corpora[1].corpus_self_hash = h(3);
    let changed_schedule = changed_schedule
        .with_computed_self_hash()
        .expect("changed schedule rehashes");

    assert_ne!(
        report.schedule.schedule_self_hash,
        changed_schedule.schedule_self_hash
    );
}

#[test]
fn corpus_progression_rejects_overlapping_phase_boundaries_and_seed_drift() {
    let mut overlapping = S4CorpusProgressionReport::new(h(1), h(2), None).expect("report");
    overlapping.schedule.phase_boundaries[1].start_progression_step = 0;
    let err = overlapping
        .with_computed_self_hash()
        .expect_err("overlap must fail");
    match err {
        S4CorpusProgressionError::InvalidPhaseBoundaryRange {
            index,
            expected_start,
            observed_start,
            ..
        } => {
            assert_eq!(index, 1);
            assert_eq!(expected_start, 1);
            assert_eq!(observed_start, 0);
        }
        other => panic!("unexpected error: {other:?}"),
    }

    let mut seed_drift = S4CorpusProgressionReport::new(h(1), h(2), None).expect("report");
    seed_drift.schedule.seed_list = vec![0, 1, 2, 3, 99];
    assert!(
        matches!(
            seed_drift.with_computed_self_hash(),
            Err(S4CorpusProgressionError::Schema(_))
        ),
        "seed-list drift must use S4 seed-list validation"
    );
}

#[test]
fn corpus_progression_mutual_binding_tracks_promotion_gate_self_hash() {
    let report = S4CorpusProgressionReport::new(h(1), h(2), None).expect("report");
    let promotion = fixture_promotion_gate(None)
        .with_corpus_progression_self_hash(report.corpus_progression_self_hash)
        .expect("bound promotion");
    let bound_report = report
        .clone()
        .with_bound_promotion_gate(promotion.promotion_gate_self_hash)
        .expect("bound report");
    bound_report
        .validate_promotion_gate_binding(&promotion)
        .expect("mutual binding");

    let mut changed = S4CorpusProgressionReport::new(h(1), h(3), None).expect("changed report");
    assert_ne!(
        report.corpus_progression_self_hash,
        changed.corpus_progression_self_hash
    );
    changed = changed
        .with_bound_promotion_gate(promotion.promotion_gate_self_hash)
        .expect("changed report can carry same promotion hash");
    let err = changed
        .validate_promotion_gate_binding(&promotion)
        .expect_err("mutating corpus identity breaks promotion binding");
    match err {
        S4CorpusProgressionError::PromotionGateBindingMismatch {
            field,
            expected,
            observed,
        } => {
            assert_eq!(field, "corpus_progression_self_hash");
            assert_eq!(expected, changed.corpus_progression_self_hash);
            assert_eq!(observed, report.corpus_progression_self_hash);
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn corpus_progression_self_hash_mismatch_is_rejected() {
    let mut report = S4CorpusProgressionReport::new(h(1), h(2), None).expect("report");
    report.corpus_progression_self_hash = h(99);
    let err = report
        .validate_canonical_write()
        .expect_err("bad self hash");
    match err {
        S4CorpusProgressionError::SelfHashMismatch {
            field, observed, ..
        } => {
            assert_eq!(field, "corpus_progression_self_hash");
            assert_eq!(observed, h(99));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

fn fixture_promotion_gate(corpus_progression_self_hash: Option<Hash256>) -> PromotionGateProduct {
    let artifact_ref = |path: &str, byte: u8| PromotionGateArtifactRef {
        artifact_path: path.to_owned(),
        artifact_self_hash: h(byte),
    };
    let input_artifacts = PromotionGateInputBindings {
        c_ts: artifact_ref("experiments/S3/checkpoints/c_ts/checkpoint.json", 10),
        c_ts_v0success: Some(artifact_ref(
            "experiments/S3/v0_success/c_ts_v0success.json",
            11,
        )),
        c_ts_oracle_agreement: Some(artifact_ref(
            "experiments/S3/oracle/c_ts_oracle_agreement.json",
            12,
        )),
        gb_manifest: artifact_ref("experiments/S4/gutenberg_manifest/manifest.json", 2),
        contamination_report: artifact_ref("experiments/S4/contamination/contamination.json", 13),
        baseline_gutenberg: Some(artifact_ref(
            "experiments/S4/baseline_gutenberg/baseline.json",
            14,
        )),
        repetition_collapse_check: artifact_ref("experiments/S3/repetition/repetition.json", 15),
    };
    let mut product = PromotionGateProduct {
        schema: "s4_promotion_gate.v1".to_owned(),
        input_artifacts,
        tinystories_manifest_self_hash: h(1),
        gutenberg_manifest_self_hash: h(2),
        c_ts_checkpoint_self_hash: h(10),
        c_ts_v0success_self_hash: Some(h(11)),
        c_ts_oracle_agreement_self_hash: Some(h(12)),
        contamination_self_hash: h(13),
        baseline_gutenberg_self_hash: Some(h(14)),
        repetition_collapse_check_self_hash: h(15),
        corpus_progression_self_hash,
        outcome: PromotionGateOutcome::Promoted {
            c_ts_checkpoint_sha: h(10),
            gutenberg_manifest_sha: h(2),
        },
        promotion_gate_self_hash: Hash256::ZERO,
    };
    product.promotion_gate_self_hash = product.compute_self_hash().expect("promotion self hash");
    product
}

fn h(byte: u8) -> Hash256 {
    Hash256::from_bytes([byte; 32])
}
