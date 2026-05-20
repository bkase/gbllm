#![cfg(feature = "s4")]

mod common;

use gbf_artifact::{
    GutenbergCompressionKind, GutenbergDedupPolicy, GutenbergFetchNamespaceKind, GutenbergManifest,
    GutenbergSourceRecord, GutenbergSplit,
};
use gbf_experiments::s4::promote::{
    PromotionGateArtifactRef, PromotionGateBoundArtifact, PromotionGateInputs,
    PromotionGateOutcome, PromotionGateProduct, PromotionGateRejectionReason,
    S3CheckpointPromotionArtifact, S3OracleAgreementOutcome, S3OracleAgreementPromotionArtifact,
    S3RepetitionCollapseOutcome, S3RepetitionCollapsePromotionArtifact,
    S3V0SuccessPromotionArtifact, S4_PROMOTION_GATE_CHECK_EVENT, S4_PROMOTION_GATE_OUTCOME_EVENT,
    S4_PROMOTION_GATE_STARTED_EVENT, S4BaselineGutenbergPromotionArtifact, S4ContaminationOutcome,
    S4ContaminationPromotionArtifact, V0SuccessAcceptanceBits, V0SuccessGateOutcome,
    promotion_gate,
};
use gbf_foundation::Hash256;
use serde_json::json;

use common::tracing_capture::{TraceCapture, captured_events, with_trace_capture};

#[test]
fn promotion_gate_promotes_hash_bound_positive_bundle() {
    let inputs = positive_inputs();
    let product = promotion_gate(inputs.clone()).expect("promotion gate evaluates");
    product
        .validate_canonical_write()
        .expect("promotion gate self-hash round trips");

    match &product.outcome {
        PromotionGateOutcome::Promoted {
            c_ts_checkpoint_sha,
            gutenberg_manifest_sha,
        } => {
            assert_eq!(
                *c_ts_checkpoint_sha,
                inputs.c_ts.artifact_ref.artifact_self_hash
            );
            assert_eq!(
                *gutenberg_manifest_sha,
                inputs.gb_manifest.artifact_ref.artifact_self_hash
            );
        }
        PromotionGateOutcome::Rejected { reasons } => {
            panic!("positive bundle was rejected: {reasons:?}");
        }
    }

    assert_eq!(
        product.input_artifacts.c_ts_v0success.as_ref(),
        inputs
            .c_ts_v0success
            .as_ref()
            .map(|bound| &bound.artifact_ref)
    );
    assert_eq!(
        product.c_ts_oracle_agreement_self_hash,
        inputs
            .c_ts_oracle_agreement
            .as_ref()
            .map(|bound| bound.artifact_ref.artifact_self_hash)
    );
    assert_eq!(
        product.repetition_collapse_check_self_hash,
        inputs
            .repetition_collapse_check
            .artifact_ref
            .artifact_self_hash
    );

    let second = promotion_gate(inputs).expect("promotion gate is deterministic");
    assert_eq!(product.outcome, second.outcome);
    assert_eq!(
        product.canonical_bytes().expect("canonical bytes"),
        second.canonical_bytes().expect("canonical bytes")
    );
}

#[test]
fn promotion_gate_promotes_warn_contamination_bundle() {
    let product = promotion_gate(mutate(positive_inputs(), |inputs| {
        inputs.contamination_report.artifact.outcome = S4ContaminationOutcome::Warn {
            findings: vec!["TS_val_overlaps_GB_val_diagnostic".to_owned()],
        };
        refresh_contamination(&mut inputs.contamination_report);
    }))
    .expect("promotion gate evaluates");

    assert!(
        matches!(product.outcome, PromotionGateOutcome::Promoted { .. }),
        "Warn contamination should promote, got {:?}",
        product.outcome
    );
}

#[test]
fn promotion_gate_emits_subscriber_captured_events() {
    let capture = TraceCapture::default();

    let product = with_trace_capture(&capture, || {
        promotion_gate(positive_inputs()).expect("promotion gate evaluates")
    });

    let events = captured_events(&capture);
    let started = events
        .iter()
        .find(|event| event.name == S4_PROMOTION_GATE_STARTED_EVENT)
        .expect("promotion gate started event");
    assert_eq!(
        started.fields.get("schema"),
        Some(&json!("s4_promotion_gate.v1"))
    );
    assert_eq!(started.fields.get("has_v0success"), Some(&json!(true)));
    assert_eq!(
        started.fields.get("has_oracle_agreement"),
        Some(&json!(true))
    );
    assert_eq!(
        started.fields.get("has_baseline_gutenberg"),
        Some(&json!(true))
    );

    let checks = events
        .iter()
        .filter(|event| event.name == S4_PROMOTION_GATE_CHECK_EVENT)
        .collect::<Vec<_>>();
    assert_eq!(checks.len(), 9);
    let p5 = checks
        .iter()
        .find(|event| event.fields.get("predicate") == Some(&json!("P-5")))
        .expect("P-5 check event");
    assert_eq!(p5.fields.get("passed"), Some(&json!(true)));
    assert_eq!(p5.fields.get("reason_count"), Some(&json!(0)));

    let outcome = events
        .iter()
        .find(|event| event.name == S4_PROMOTION_GATE_OUTCOME_EVENT)
        .expect("promotion gate outcome event");
    assert_eq!(outcome.fields.get("outcome"), Some(&json!("Promoted")));
    assert_eq!(outcome.fields.get("promoted"), Some(&json!(true)));
    assert_eq!(outcome.fields.get("reason_count"), Some(&json!(0)));
    assert_eq!(
        outcome.fields.get("promotion_gate_self_hash"),
        Some(&json!(product.promotion_gate_self_hash.to_string()))
    );
}

#[test]
fn promotion_gate_resumable_reason_serializes_canonical_spelling_and_reads_legacy_alias() {
    let canonical =
        serde_json::to_value(PromotionGateRejectionReason::P1CheckpointNotPhaseDResumable)
            .expect("reason serializes");
    assert_eq!(canonical, json!("P1_checkpoint_not_phase_d_resumable"));

    let legacy: PromotionGateRejectionReason =
        serde_json::from_value(json!("P1_checkpoint_not_phase_d_resumeable"))
            .expect("legacy misspelling remains readable");
    assert_eq!(
        legacy,
        PromotionGateRejectionReason::P1CheckpointNotPhaseDResumable
    );
}

#[test]
fn promotion_gate_rejects_each_p_predicate_family() {
    assert_exact_reasons(
        mutate(positive_inputs(), |inputs| inputs.c_ts_v0success = None),
        &[PromotionGateRejectionReason::P1V0SuccessMissing],
    );

    assert_exact_reasons(
        mutate(positive_inputs(), |inputs| {
            inputs.c_ts.artifact.checkpoint_self_hash = hash(250);
        }),
        &[PromotionGateRejectionReason::P1CheckpointSelfHashInvalid],
    );

    assert_exact_reasons(
        mutate(positive_inputs(), |inputs| {
            let v0 = inputs.c_ts_v0success.as_mut().expect("v0 present");
            v0.artifact.outcome = V0SuccessGateOutcome::Fail;
            refresh_v0_success(v0);
        }),
        &[PromotionGateRejectionReason::P1V0SuccessNotPassing],
    );

    assert_exact_reasons(
        mutate(positive_inputs(), |inputs| {
            let oracle = inputs
                .c_ts_oracle_agreement
                .as_mut()
                .expect("oracle present");
            oracle.artifact.outcome = S3OracleAgreementOutcome::Disagree;
            refresh_oracle(oracle);
        }),
        &[PromotionGateRejectionReason::P2OracleDisagreement],
    );

    assert_exact_reasons(
        mutate(positive_inputs(), |inputs| {
            let v0 = inputs.c_ts_v0success.as_mut().expect("v0 present");
            v0.artifact.ternary_gap_ts = 0.500_001;
            refresh_v0_success(v0);
        }),
        &[PromotionGateRejectionReason::P3TernaryGapTooLarge],
    );

    assert_exact_reasons(
        mutate(positive_inputs(), |inputs| {
            inputs.gb_manifest.artifact.book_ids.reverse();
            refresh_manifest(&mut inputs.gb_manifest);
            rebind_gutenberg_manifest_consumers(inputs);
        }),
        &[PromotionGateRejectionReason::P4GutenbergManifestInvalid],
    );

    assert_exact_reasons(
        mutate(positive_inputs(), |inputs| {
            inputs.contamination_report.artifact.outcome = S4ContaminationOutcome::HardFail {
                failures: vec!["TS_train_contains_GB_val".to_owned()],
                warnings: vec![],
            };
            refresh_contamination(&mut inputs.contamination_report);
        }),
        &[PromotionGateRejectionReason::P5ContaminationDirty],
    );

    assert_exact_reasons(
        mutate(positive_inputs(), |inputs| {
            inputs.gb_manifest.artifact.unmappable_rate_corpus = 0.005_001;
            refresh_manifest(&mut inputs.gb_manifest);
            rebind_gutenberg_manifest_consumers(inputs);
        }),
        &[PromotionGateRejectionReason::P6UnmappableRateTooHigh],
    );
    assert_exact_reasons(
        mutate(positive_inputs(), |inputs| {
            inputs.gb_manifest.artifact.unmappable_rate_corpus = -0.001;
            refresh_manifest(&mut inputs.gb_manifest);
            rebind_gutenberg_manifest_consumers(inputs);
        }),
        &[PromotionGateRejectionReason::P6UnmappableRateTooHigh],
    );

    assert_exact_reasons(
        mutate(positive_inputs(), |inputs| {
            let baseline = inputs
                .baseline_gutenberg
                .as_mut()
                .expect("baseline present");
            baseline.artifact.bpc_kn5 = f64::INFINITY;
            refresh_baseline(baseline);
        }),
        &[PromotionGateRejectionReason::P7BaselineNonfinite],
    );

    assert_exact_reasons(
        mutate(positive_inputs(), |inputs| {
            inputs.repetition_collapse_check.artifact.outcome = S3RepetitionCollapseOutcome::Fail;
            refresh_repetition(&mut inputs.repetition_collapse_check);
        }),
        &[PromotionGateRejectionReason::P8RepetitionCollapse],
    );

    assert_exact_reasons(
        mutate(positive_inputs(), |inputs| {
            inputs
                .repetition_collapse_check
                .artifact
                .tinystories_manifest_self_hash = hash(252);
            refresh_repetition(&mut inputs.repetition_collapse_check);
        }),
        &[PromotionGateRejectionReason::P8RepetitionManifestMismatch],
    );

    assert_exact_reasons(
        mutate(positive_inputs(), |inputs| {
            inputs.c_ts.artifact_ref.artifact_path =
                "experiments/S3/checkpoints/latest/checkpoint.json".to_owned();
        }),
        &[PromotionGateRejectionReason::P9NonExplicitArtifactSelector],
    );
}

#[test]
fn promotion_gate_rejects_lineage_missing_and_self_hash_failures() {
    assert_exact_reasons(
        mutate(positive_inputs(), |inputs| {
            let v0 = inputs.c_ts_v0success.as_mut().expect("v0 present");
            v0.artifact.checkpoint_self_hash = hash(40);
            refresh_v0_success(v0);
        }),
        &[PromotionGateRejectionReason::P1V0SuccessCheckpointMismatch],
    );

    assert_exact_reasons(
        mutate(positive_inputs(), |inputs| {
            let v0 = inputs.c_ts_v0success.as_mut().expect("v0 present");
            v0.artifact.tinystories_manifest_self_hash = hash(41);
            refresh_v0_success(v0);
        }),
        &[PromotionGateRejectionReason::P1V0SuccessManifestMismatch],
    );

    assert_exact_reasons(
        mutate(positive_inputs(), |inputs| {
            inputs.c_ts.artifact.contains_qat_shadow_weights = false;
            refresh_checkpoint(&mut inputs.c_ts);
            rebind_checkpoint_consumers(inputs);
        }),
        &[PromotionGateRejectionReason::P1CheckpointNotPhaseDResumable],
    );

    assert_exact_reasons(
        mutate(positive_inputs(), |inputs| {
            inputs.c_ts_oracle_agreement = None;
        }),
        &[PromotionGateRejectionReason::P2OracleMissing],
    );

    assert_exact_reasons(
        mutate(positive_inputs(), |inputs| {
            let oracle = inputs
                .c_ts_oracle_agreement
                .as_mut()
                .expect("oracle present");
            oracle.artifact_ref.artifact_self_hash = hash(42);
        }),
        &[PromotionGateRejectionReason::P2OracleSelfHashInvalid],
    );

    assert_exact_reasons(
        mutate(positive_inputs(), |inputs| {
            let oracle = inputs
                .c_ts_oracle_agreement
                .as_mut()
                .expect("oracle present");
            oracle.artifact.checkpoint_self_hash = hash(43);
            refresh_oracle(oracle);
        }),
        &[PromotionGateRejectionReason::P2OracleCheckpointMismatch],
    );

    assert_exact_reasons(
        mutate(positive_inputs(), |inputs| {
            let oracle = inputs
                .c_ts_oracle_agreement
                .as_mut()
                .expect("oracle present");
            oracle.artifact.tinystories_manifest_self_hash = hash(44);
            refresh_oracle(oracle);
        }),
        &[PromotionGateRejectionReason::P2OracleManifestMismatch],
    );

    assert_exact_reasons(
        mutate(positive_inputs(), |inputs| {
            inputs.contamination_report.artifact_ref.artifact_self_hash = hash(45);
        }),
        &[PromotionGateRejectionReason::P5ContaminationSelfHashInvalid],
    );

    assert_exact_reasons(
        mutate(positive_inputs(), |inputs| {
            inputs
                .contamination_report
                .artifact
                .gutenberg_manifest_self_hash = hash(46);
            refresh_contamination(&mut inputs.contamination_report);
        }),
        &[PromotionGateRejectionReason::P5ContaminationManifestMismatch],
    );

    assert_exact_reasons(
        mutate(positive_inputs(), |inputs| {
            inputs.baseline_gutenberg = None;
        }),
        &[PromotionGateRejectionReason::P7BaselineMissing],
    );

    assert_exact_reasons(
        mutate(positive_inputs(), |inputs| {
            let baseline = inputs
                .baseline_gutenberg
                .as_mut()
                .expect("baseline present");
            baseline.artifact_ref.artifact_self_hash = hash(47);
        }),
        &[PromotionGateRejectionReason::P7BaselineSelfHashInvalid],
    );

    assert_exact_reasons(
        mutate(positive_inputs(), |inputs| {
            let baseline = inputs
                .baseline_gutenberg
                .as_mut()
                .expect("baseline present");
            baseline.artifact.corpus_val_sha = hash(48);
            refresh_baseline(baseline);
        }),
        &[PromotionGateRejectionReason::P7BaselineManifestMismatch],
    );

    assert_exact_reasons(
        mutate(positive_inputs(), |inputs| {
            inputs
                .repetition_collapse_check
                .artifact_ref
                .artifact_self_hash = hash(49);
        }),
        &[PromotionGateRejectionReason::P8RepetitionSelfHashInvalid],
    );

    assert_exact_reasons(
        mutate(positive_inputs(), |inputs| {
            inputs
                .repetition_collapse_check
                .artifact
                .checkpoint_self_hash = hash(50);
            refresh_repetition(&mut inputs.repetition_collapse_check);
        }),
        &[PromotionGateRejectionReason::P8RepetitionCheckpointMismatch],
    );
}

#[test]
fn promotion_gate_suppresses_semantics_from_invalid_self_hash_artifact() {
    assert_exact_reasons(
        mutate(positive_inputs(), |inputs| {
            let v0 = inputs.c_ts_v0success.as_mut().expect("v0 present");
            v0.artifact.ternary_gap_ts = 10.0;
            v0.artifact_ref.artifact_self_hash = hash(251);
        }),
        &[PromotionGateRejectionReason::P1V0SuccessSelfHashInvalid],
    );
}

#[test]
fn promotion_gate_suppresses_semantics_for_invalid_self_hash_inputs() {
    assert_exact_reasons(
        mutate(positive_inputs(), |inputs| {
            inputs.c_ts.artifact.contains_qat_shadow_weights = false;
            inputs.c_ts.artifact.checkpoint_self_hash = hash(59);
        }),
        &[PromotionGateRejectionReason::P1CheckpointSelfHashInvalid],
    );

    assert_exact_reasons(
        mutate(positive_inputs(), |inputs| {
            let oracle = inputs
                .c_ts_oracle_agreement
                .as_mut()
                .expect("oracle present");
            oracle.artifact.outcome = S3OracleAgreementOutcome::Disagree;
            oracle.artifact_ref.artifact_self_hash = hash(60);
        }),
        &[PromotionGateRejectionReason::P2OracleSelfHashInvalid],
    );

    assert_exact_reasons(
        mutate(positive_inputs(), |inputs| {
            inputs.contamination_report.artifact.outcome = S4ContaminationOutcome::HardFail {
                failures: vec!["TS_train_contains_GB_val".to_owned()],
                warnings: vec![],
            };
            inputs.contamination_report.artifact_ref.artifact_self_hash = hash(61);
        }),
        &[PromotionGateRejectionReason::P5ContaminationSelfHashInvalid],
    );

    assert_exact_reasons(
        mutate(positive_inputs(), |inputs| {
            let baseline = inputs
                .baseline_gutenberg
                .as_mut()
                .expect("baseline present");
            baseline.artifact.corpus_val_sha = hash(62);
            baseline.artifact_ref.artifact_self_hash = hash(63);
        }),
        &[PromotionGateRejectionReason::P7BaselineSelfHashInvalid],
    );

    assert_exact_reasons(
        mutate(positive_inputs(), |inputs| {
            inputs.repetition_collapse_check.artifact.outcome = S3RepetitionCollapseOutcome::Fail;
            inputs
                .repetition_collapse_check
                .artifact_ref
                .artifact_self_hash = hash(64);
        }),
        &[PromotionGateRejectionReason::P8RepetitionSelfHashInvalid],
    );

    assert_exact_reasons(
        mutate(positive_inputs(), |inputs| {
            inputs.gb_manifest.artifact.manifest_self_hash = hash(65);
            inputs.gb_manifest.artifact.unmappable_rate_corpus = 0.10;
        }),
        &[PromotionGateRejectionReason::P4GutenbergManifestInvalid],
    );
}

#[test]
fn promotion_gate_rejects_wrong_schema_inputs() {
    assert_exact_reasons(
        mutate(positive_inputs(), |inputs| {
            inputs.c_ts.artifact.schema = "wrong_checkpoint_schema.v1".to_owned();
            refresh_checkpoint(&mut inputs.c_ts);
            rebind_checkpoint_consumers(inputs);
        }),
        &[PromotionGateRejectionReason::P1CheckpointSelfHashInvalid],
    );

    assert_exact_reasons(
        mutate(positive_inputs(), |inputs| {
            let v0 = inputs.c_ts_v0success.as_mut().expect("v0 present");
            v0.artifact.schema = "wrong_v0_success_schema.v1".to_owned();
            refresh_v0_success(v0);
        }),
        &[PromotionGateRejectionReason::P1V0SuccessSelfHashInvalid],
    );

    assert_exact_reasons(
        mutate(positive_inputs(), |inputs| {
            let oracle = inputs
                .c_ts_oracle_agreement
                .as_mut()
                .expect("oracle present");
            oracle.artifact.schema = "wrong_oracle_schema.v1".to_owned();
            refresh_oracle(oracle);
        }),
        &[PromotionGateRejectionReason::P2OracleSelfHashInvalid],
    );

    assert_exact_reasons(
        mutate(positive_inputs(), |inputs| {
            inputs.gb_manifest.artifact.schema = "wrong_gutenberg_manifest.v1".to_owned();
            refresh_manifest(&mut inputs.gb_manifest);
            rebind_gutenberg_manifest_consumers(inputs);
        }),
        &[PromotionGateRejectionReason::P4GutenbergManifestInvalid],
    );

    assert_exact_reasons(
        mutate(positive_inputs(), |inputs| {
            inputs.contamination_report.artifact.schema =
                "wrong_contamination_schema.v1".to_owned();
            refresh_contamination(&mut inputs.contamination_report);
        }),
        &[PromotionGateRejectionReason::P5ContaminationSelfHashInvalid],
    );

    assert_exact_reasons(
        mutate(positive_inputs(), |inputs| {
            let baseline = inputs
                .baseline_gutenberg
                .as_mut()
                .expect("baseline present");
            baseline.artifact.schema = "wrong_baseline_schema.v1".to_owned();
            refresh_baseline(baseline);
        }),
        &[PromotionGateRejectionReason::P7BaselineSelfHashInvalid],
    );

    assert_exact_reasons(
        mutate(positive_inputs(), |inputs| {
            inputs.repetition_collapse_check.artifact.schema =
                "wrong_repetition_schema.v1".to_owned();
            refresh_repetition(&mut inputs.repetition_collapse_check);
        }),
        &[PromotionGateRejectionReason::P8RepetitionSelfHashInvalid],
    );
}

#[test]
fn promotion_gate_reports_combined_evaluable_failures() {
    assert_exact_reasons(
        mutate(positive_inputs(), |inputs| {
            inputs.c_ts_v0success = None;
            inputs.c_ts.artifact.contains_qat_shadow_weights = false;
            refresh_checkpoint(&mut inputs.c_ts);
            rebind_checkpoint_consumers(inputs);
        }),
        &[
            PromotionGateRejectionReason::P1V0SuccessMissing,
            PromotionGateRejectionReason::P1CheckpointNotPhaseDResumable,
        ],
    );

    assert_exact_reasons(
        mutate(positive_inputs(), |inputs| {
            let baseline = inputs
                .baseline_gutenberg
                .as_mut()
                .expect("baseline present");
            baseline.artifact.corpus_val_sha = hash(66);
            baseline.artifact.bpc_kn5 = f64::INFINITY;
            refresh_baseline(baseline);
        }),
        &[
            PromotionGateRejectionReason::P7BaselineManifestMismatch,
            PromotionGateRejectionReason::P7BaselineNonfinite,
        ],
    );
}

#[test]
fn promotion_gate_rejection_reasons_are_canonical_ordered() {
    let product = promotion_gate(mutate(positive_inputs(), |inputs| {
        inputs.c_ts.artifact_ref.artifact_path = "latest".to_owned();
        let oracle = inputs
            .c_ts_oracle_agreement
            .as_mut()
            .expect("oracle present");
        oracle.artifact.outcome = S3OracleAgreementOutcome::Disagree;
        refresh_oracle(oracle);
        inputs.repetition_collapse_check.artifact.outcome = S3RepetitionCollapseOutcome::Fail;
        refresh_repetition(&mut inputs.repetition_collapse_check);
    }))
    .expect("promotion gate evaluates");

    assert_eq!(
        rejected_reasons(&product),
        &[
            PromotionGateRejectionReason::P2OracleDisagreement,
            PromotionGateRejectionReason::P8RepetitionCollapse,
            PromotionGateRejectionReason::P9NonExplicitArtifactSelector,
        ]
    );
}

fn positive_inputs() -> PromotionGateInputs {
    let tinystories_manifest_self_hash = hash(9);
    let checkpoint = S3CheckpointPromotionArtifact::new("phase_d", true, true)
        .expect("checkpoint summary builds");
    let manifest = valid_manifest(0.001);
    let v0_success = S3V0SuccessPromotionArtifact::new(
        checkpoint.checkpoint_self_hash,
        tinystories_manifest_self_hash,
        V0SuccessAcceptanceBits::all_pass(),
        V0SuccessGateOutcome::Pass,
        0.25,
    )
    .expect("v0_success summary builds");
    let oracle = S3OracleAgreementPromotionArtifact::new(
        checkpoint.checkpoint_self_hash,
        tinystories_manifest_self_hash,
        S3OracleAgreementOutcome::Agree,
        true,
    )
    .expect("oracle summary builds");
    let contamination = S4ContaminationPromotionArtifact::new(
        tinystories_manifest_self_hash,
        manifest.manifest_self_hash,
        S4ContaminationOutcome::Clean,
    )
    .expect("contamination summary builds");
    let baseline = S4BaselineGutenbergPromotionArtifact::new(
        manifest.manifest_self_hash,
        manifest.train_sha256,
        manifest.val_sha256,
        2.75,
    )
    .expect("baseline summary builds");
    let repetition = S3RepetitionCollapsePromotionArtifact::new(
        checkpoint.checkpoint_self_hash,
        tinystories_manifest_self_hash,
        S3RepetitionCollapseOutcome::Pass,
    )
    .expect("repetition summary builds");

    PromotionGateInputs {
        tinystories_manifest_self_hash,
        c_ts: bound(
            "experiments/S3/checkpoints/seed-0/checkpoint.json",
            checkpoint.checkpoint_self_hash,
            checkpoint,
        ),
        c_ts_v0success: Some(bound(
            "experiments/S3/v0_success/seed-0.json",
            v0_success.v0_success_self_hash,
            v0_success,
        )),
        c_ts_oracle_agreement: Some(bound(
            "experiments/S3/oracle_agreement/seed-0.json",
            oracle.oracle_agreement_self_hash,
            oracle,
        )),
        gb_manifest: bound(
            "experiments/S4/gutenberg_manifest.json",
            manifest.manifest_self_hash,
            manifest,
        ),
        contamination_report: bound(
            "experiments/S4/contamination/cross_corpus.json",
            contamination.contamination_self_hash,
            contamination,
        ),
        baseline_gutenberg: Some(bound(
            "experiments/S4/baseline/baseline_gutenberg.json",
            baseline.baseline_self_hash,
            baseline,
        )),
        repetition_collapse_check: bound(
            "experiments/S3/repetition/seed-0.json",
            repetition.repetition_self_hash,
            repetition,
        ),
    }
}

fn valid_manifest(unmappable_rate_corpus: f64) -> GutenbergManifest {
    let mut manifest = GutenbergManifest {
        schema: GutenbergManifest::schema_id(),
        source_name: GutenbergManifest::source_name_literal(),
        catalog_snapshot_url: "file://fixtures/gutenberg-rdf.tar.bz2".to_owned(),
        catalog_snapshot_sha256: hash(1),
        catalog_snapshot_observed_at_utc: "2026-05-19T00:00:00Z".to_owned(),
        catalog_snapshot_last_modified_utc: None,
        selection_filter_canonical_json: "{}".to_owned(),
        selection_filter_sha256: hash(2),
        book_ids: vec![1001, 1002],
        sources: vec![
            source_record(1001, GutenbergSplit::Train, 20),
            source_record(1002, GutenbergSplit::Val, 30),
        ],
        header_regex_pattern: "START".to_owned(),
        footer_regex_pattern: "END".to_owned(),
        normalization_spec_self_hash: hash(3),
        dedup_policy: GutenbergDedupPolicy::exact_post_strip_charset_body_sha(),
        split_seed_u128: "00000000000000000000000000000001".to_owned(),
        split_train_fraction: 0.90,
        split_val_fraction: 0.10,
        train_path: "experiments/S4/corpus/gutenberg_train.bin".to_owned(),
        val_path: "experiments/S4/corpus/gutenberg_val.bin".to_owned(),
        train_sha256: hash(4),
        val_sha256: hash(5),
        train_byte_length: 128,
        val_byte_length: 128,
        train_book_count: 1,
        val_book_count: 1,
        drop_count_total: 0,
        drop_count_no_supported_plaintext_format: 0,
        drop_count_no_plaintext_archive_member: 0,
        drop_count_source_decode_failed: 0,
        drop_count_ambiguous_plaintext_archive: 0,
        drop_count_invalid_utf8: 0,
        drop_count_empty_after_strip: 0,
        drop_count_marker_missing: 0,
        drop_count_unmappable_density: 0,
        drop_count_dedup_collision: 0,
        unmappable_rate_corpus,
        raw_byte_policy: GutenbergManifest::raw_byte_policy_literal(),
        retained_book_count_min: 2,
        manifest_self_hash: Hash256::ZERO,
    };
    manifest.manifest_self_hash = manifest.compute_self_hash().expect("manifest self-hash");
    manifest
}

fn source_record(book_id: u32, split: GutenbergSplit, salt: u8) -> GutenbergSourceRecord {
    GutenbergSourceRecord {
        book_id,
        title: format!("Book {book_id}"),
        author: "Fixture Author".to_owned(),
        source_landing_url: format!("https://www.gutenberg.org/ebooks/{book_id}"),
        mirror_fetch_url: None,
        mirror_snapshot_id: None,
        selected_format: Some("text/plain\nutf-8\nnone\n\nfile://fixture".to_owned()),
        source_blob_sha256: Some(hash(salt)),
        pre_strip_utf8_sha256: Some(hash(salt + 1)),
        license: GutenbergSourceRecord::public_domain_in_usa_license(),
        fetch_namespace_kind: Some(GutenbergFetchNamespaceKind::ContentAddressedCache),
        fetch_namespace_id: Some("fixture-cache".to_owned()),
        compression_kind: Some(GutenbergCompressionKind::None),
        archive_member_path: None,
        pre_strip_byte_length: Some(160),
        drop_reason: None,
        duplicate_of_book_id: None,
        post_strip_byte_length: Some(128),
        post_strip_sha256: Some(hash(salt + 2)),
        post_charset_body_sha256: Some(hash(salt + 3)),
        post_charset_token_length: Some(128),
        unmappable_count: Some(0),
        unmappable_density: Some(0.0),
        split: Some(split),
    }
}

fn bound<T>(path: &str, self_hash: Hash256, artifact: T) -> PromotionGateBoundArtifact<T> {
    PromotionGateBoundArtifact::new(PromotionGateArtifactRef::new(path, self_hash), artifact)
}

fn mutate(
    mut inputs: PromotionGateInputs,
    mutation: impl FnOnce(&mut PromotionGateInputs),
) -> PromotionGateInputs {
    mutation(&mut inputs);
    inputs
}

fn assert_exact_reasons(inputs: PromotionGateInputs, expected: &[PromotionGateRejectionReason]) {
    let product = promotion_gate(inputs).expect("promotion gate evaluates");
    assert_eq!(rejected_reasons(&product), expected);
}

fn rejected_reasons(product: &PromotionGateProduct) -> &[PromotionGateRejectionReason] {
    match &product.outcome {
        PromotionGateOutcome::Promoted { .. } => panic!("expected rejection"),
        PromotionGateOutcome::Rejected { reasons } => reasons,
    }
}

fn refresh_manifest(bound: &mut PromotionGateBoundArtifact<GutenbergManifest>) {
    bound.artifact.manifest_self_hash = bound
        .artifact
        .compute_self_hash()
        .expect("manifest self-hash recomputes");
    bound.artifact_ref.artifact_self_hash = bound.artifact.manifest_self_hash;
}

fn rebind_gutenberg_manifest_consumers(inputs: &mut PromotionGateInputs) {
    let manifest_self_hash = inputs.gb_manifest.artifact_ref.artifact_self_hash;
    let train_sha = inputs.gb_manifest.artifact.train_sha256;
    let val_sha = inputs.gb_manifest.artifact.val_sha256;

    inputs
        .contamination_report
        .artifact
        .gutenberg_manifest_self_hash = manifest_self_hash;
    refresh_contamination(&mut inputs.contamination_report);

    if let Some(baseline) = inputs.baseline_gutenberg.as_mut() {
        baseline.artifact.gutenberg_manifest_self_hash = manifest_self_hash;
        baseline.artifact.corpus_train_sha = train_sha;
        baseline.artifact.corpus_val_sha = val_sha;
        refresh_baseline(baseline);
    }
}

fn refresh_checkpoint(bound: &mut PromotionGateBoundArtifact<S3CheckpointPromotionArtifact>) {
    bound.artifact.checkpoint_self_hash = bound
        .artifact
        .compute_self_hash()
        .expect("checkpoint self-hash recomputes");
    bound.artifact_ref.artifact_self_hash = bound.artifact.checkpoint_self_hash;
}

fn rebind_checkpoint_consumers(inputs: &mut PromotionGateInputs) {
    let checkpoint_self_hash = inputs.c_ts.artifact_ref.artifact_self_hash;

    if let Some(v0_success) = inputs.c_ts_v0success.as_mut() {
        v0_success.artifact.checkpoint_self_hash = checkpoint_self_hash;
        refresh_v0_success(v0_success);
    }

    if let Some(oracle) = inputs.c_ts_oracle_agreement.as_mut() {
        oracle.artifact.checkpoint_self_hash = checkpoint_self_hash;
        refresh_oracle(oracle);
    }

    inputs
        .repetition_collapse_check
        .artifact
        .checkpoint_self_hash = checkpoint_self_hash;
    refresh_repetition(&mut inputs.repetition_collapse_check);
}

fn refresh_v0_success(bound: &mut PromotionGateBoundArtifact<S3V0SuccessPromotionArtifact>) {
    bound.artifact.v0_success_self_hash = bound
        .artifact
        .compute_self_hash()
        .expect("v0_success self-hash recomputes");
    bound.artifact_ref.artifact_self_hash = bound.artifact.v0_success_self_hash;
}

fn refresh_baseline(bound: &mut PromotionGateBoundArtifact<S4BaselineGutenbergPromotionArtifact>) {
    bound.artifact.baseline_self_hash = bound
        .artifact
        .compute_self_hash()
        .expect("baseline self-hash recomputes");
    bound.artifact_ref.artifact_self_hash = bound.artifact.baseline_self_hash;
}

fn refresh_oracle(bound: &mut PromotionGateBoundArtifact<S3OracleAgreementPromotionArtifact>) {
    bound.artifact.oracle_agreement_self_hash = bound
        .artifact
        .compute_self_hash()
        .expect("oracle self-hash recomputes");
    bound.artifact_ref.artifact_self_hash = bound.artifact.oracle_agreement_self_hash;
}

fn refresh_contamination(bound: &mut PromotionGateBoundArtifact<S4ContaminationPromotionArtifact>) {
    bound.artifact.contamination_self_hash = bound
        .artifact
        .compute_self_hash()
        .expect("contamination self-hash recomputes");
    bound.artifact_ref.artifact_self_hash = bound.artifact.contamination_self_hash;
}

fn refresh_repetition(
    bound: &mut PromotionGateBoundArtifact<S3RepetitionCollapsePromotionArtifact>,
) {
    bound.artifact.repetition_self_hash = bound
        .artifact
        .compute_self_hash()
        .expect("repetition self-hash recomputes");
    bound.artifact_ref.artifact_self_hash = bound.artifact.repetition_self_hash;
}

fn hash(byte: u8) -> Hash256 {
    Hash256::from_bytes([byte; 32])
}
