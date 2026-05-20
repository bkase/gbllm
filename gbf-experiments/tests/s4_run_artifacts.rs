#![cfg(feature = "s4")]

use std::fs;

use gbf_experiments::s4::run::progress_eval_steps;
use gbf_experiments::s4::run_artifacts::{
    S4_FP_REFERENCE_KIND_QAT_SHADOW_AFTER_GUTENBERG, S4_FP_REFERENCE_SCHEMA,
    S4_GUTENBERG_CHECKPOINT_SCHEMA, S4_GUTENBERG_RUN_LOG_SCHEMA, S4DivergenceObserved,
    S4FpReferenceArtifact, S4GradNormSummary, S4GutenbergCheckpointMetadata, S4GutenbergRunLog,
    S4LossSpikeSurpriseConfig, S4RunArtifactError, S4RunSurpriseObserved, S4StepDiagnostics,
    d13_fail_closed_outcome, first_d13_divergence_event, first_loss_spike_surprise_event,
    write_s4_fp_reference, write_s4_gutenberg_checkpoint_metadata, write_s4_gutenberg_run_log,
};
use gbf_experiments::s4::schema::{
    S4_OPTIMIZER_STEPS_GUTENBERG, S4BuildKind, S4Completion, S4InitialWeightSource, S4Outcome,
    S4TrainConfig,
};
use gbf_foundation::{CanonicalJson, Hash256, SemVer};
use serde_json::{Value, json};

#[test]
fn s4_run_artifacts_emit_canonical_self_hashed_json() {
    let temp = tempfile::tempdir().expect("tempdir");
    let run_log = fixture_run_log()
        .with_computed_self_hash()
        .expect("run log");
    assert_ne!(run_log.run_log_self_hash, Hash256::ZERO);

    let run_log_bytes = run_log.canonical_bytes().expect("run log bytes");
    let run_log_json: Value = serde_json::from_slice(&run_log_bytes).expect("run log json");
    assert_eq!(run_log_json["schema"], json!(S4_GUTENBERG_RUN_LOG_SCHEMA));
    assert_eq!(
        run_log_json["c_TS_checkpoint_self_hash"],
        json!(run_log.c_ts_checkpoint_self_hash.to_string())
    );
    assert!(run_log_json.get("c_ts_checkpoint_self_hash").is_none());
    assert_eq!(run_log_json["initial_weight_source"], json!("c_TS_ref"));
    assert_eq!(
        run_log_json["init_rng_draw_count_before_first_step"],
        json!(0)
    );
    assert_eq!(run_log_json["shuffle_rng_draw_count_total"], json!(0));
    assert_eq!(
        run_log_json["losses"].as_array().expect("loss array").len(),
        S4_OPTIMIZER_STEPS_GUTENBERG as usize
    );
    assert_eq!(
        run_log_json["eval_points"]
            .as_array()
            .expect("eval point array")
            .len(),
        11
    );

    let run_log_path = temp.path().join("artifacts/run_log.json");
    write_s4_gutenberg_run_log(&run_log_path, &run_log).expect("write run log");
    assert_eq!(
        fs::read(&run_log_path).expect("written run log"),
        run_log_bytes
    );

    let checkpoint = fixture_checkpoint(&run_log)
        .with_computed_self_hash()
        .expect("checkpoint");
    let checkpoint_bytes = checkpoint.canonical_bytes().expect("checkpoint bytes");
    let checkpoint_json: Value =
        serde_json::from_slice(&checkpoint_bytes).expect("checkpoint json");
    assert_eq!(
        checkpoint_json["schema"],
        json!(S4_GUTENBERG_CHECKPOINT_SCHEMA)
    );
    assert_eq!(
        checkpoint_json["c_TS_checkpoint_self_hash"],
        json!(checkpoint.c_ts_checkpoint_self_hash.to_string())
    );
    assert!(checkpoint_json.get("c_ts_checkpoint_self_hash").is_none());
    assert_eq!(checkpoint_json["build_kind"], json!("phase_d_continuation"));
    assert_eq!(checkpoint_json["completion"], json!({"kind": "Completed"}));
    assert_eq!(
        checkpoint_json["checkpoint_self_hash"],
        json!(checkpoint.checkpoint_self_hash.to_string())
    );

    let checkpoint_path = temp.path().join("artifacts/checkpoint.json");
    write_s4_gutenberg_checkpoint_metadata(&checkpoint_path, &checkpoint)
        .expect("write checkpoint");
    assert_eq!(
        fs::read(&checkpoint_path).expect("written checkpoint"),
        checkpoint_bytes
    );

    let fp_reference = fixture_fp_reference(&checkpoint)
        .with_computed_self_hash()
        .expect("fp reference");
    fp_reference
        .validate_against_checkpoint(&checkpoint)
        .expect("fp lineage");
    fp_reference
        .validate_against_checkpoint_and_gutenberg_val_sha(&checkpoint, checkpoint.corpus_val_sha)
        .expect("fp manifest val sha lineage");
    let fp_bytes = fp_reference.canonical_bytes().expect("fp bytes");
    let fp_json: Value = serde_json::from_slice(&fp_bytes).expect("fp json");
    assert_eq!(fp_json["schema"], json!(S4_FP_REFERENCE_SCHEMA));
    assert_eq!(
        fp_json["fp_reference_kind"],
        json!(S4_FP_REFERENCE_KIND_QAT_SHADOW_AFTER_GUTENBERG)
    );
    assert_eq!(
        fp_json["source_checkpoint_self_hash"],
        json!(checkpoint.checkpoint_self_hash.to_string())
    );

    let fp_path = temp.path().join("artifacts/fp_reference.json");
    write_s4_fp_reference(&fp_path, &fp_reference).expect("write fp reference");
    assert_eq!(fs::read(&fp_path).expect("written fp reference"), fp_bytes);
}

#[test]
fn s4_run_log_rejects_nonfinite_metrics_and_bad_cardinality() {
    let mut nonfinite = fixture_run_log();
    nonfinite.losses[16].1 = f64::NAN;
    let err = nonfinite
        .with_computed_self_hash()
        .expect_err("NaN loss must fail before hashing");
    match err {
        S4RunArtifactError::NonFiniteMetric {
            field,
            step: Some(17),
            value,
        } => {
            assert_eq!(field, "losses.loss_nats_per_token");
            assert!(value.is_nan());
        }
        other => panic!("unexpected error: {other:?}"),
    }

    let mut short = fixture_run_log();
    short.losses.pop();
    let err = short
        .with_computed_self_hash()
        .expect_err("missing loss must fail");
    match err {
        S4RunArtifactError::LossCount { expected, observed } => {
            assert_eq!(expected, S4_OPTIMIZER_STEPS_GUTENBERG as usize);
            assert_eq!(observed, expected - 1);
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn s4_d13_divergence_event_records_first_nonfinite_without_nan_payload() {
    let diagnostics = [
        S4StepDiagnostics {
            step: 1,
            loss_nats_per_token: 0.91,
            moving_average_loss_nats_per_token: Some(0.91),
            grad_global_l2: 1.5,
        },
        S4StepDiagnostics {
            step: 2,
            loss_nats_per_token: 0.87,
            moving_average_loss_nats_per_token: Some(0.89),
            grad_global_l2: f64::INFINITY,
        },
        S4StepDiagnostics {
            step: 3,
            loss_nats_per_token: f64::NAN,
            moving_average_loss_nats_per_token: Some(0.88),
            grad_global_l2: 1.0,
        },
    ];

    let event = first_d13_divergence_event(&diagnostics)
        .expect("divergence scan")
        .expect("divergence event");
    assert_eq!(event.step, 2);
    assert_eq!(event.observed, S4DivergenceObserved::NonFiniteGradNorm);
    assert_eq!(event.last_finite_loss, Some(0.87));

    let event_bytes = CanonicalJson::to_vec(&event).expect("canonical event");
    let event_text = String::from_utf8(event_bytes.clone()).expect("utf8");
    let lower = event_text.to_ascii_lowercase();
    assert!(!lower.contains("nan"));
    assert!(!lower.contains("inf"));
    let event_json: Value = serde_json::from_slice(&event_bytes).expect("event json");
    assert_eq!(event_json["observed"], json!("non_finite_grad_norm"));
    assert_eq!(event_json["last_finite_loss"], json!(0.87));

    assert_eq!(
        d13_fail_closed_outcome(&[
            S4Completion::Completed,
            S4Completion::DivergedAt { step: 2 }
        ]),
        Some(S4Outcome::FailSubstrate)
    );
}

#[test]
fn s4_loss_spike_surprise_is_one_sided_and_not_d13_divergence() {
    let diagnostics = [
        S4StepDiagnostics {
            step: 1,
            loss_nats_per_token: 1.00,
            moving_average_loss_nats_per_token: Some(1.00),
            grad_global_l2: 1.0,
        },
        S4StepDiagnostics {
            step: 2,
            loss_nats_per_token: 0.20,
            moving_average_loss_nats_per_token: Some(1.00),
            grad_global_l2: 1.0,
        },
        S4StepDiagnostics {
            step: 3,
            loss_nats_per_token: 1.80,
            moving_average_loss_nats_per_token: Some(1.00),
            grad_global_l2: 1.0,
        },
        S4StepDiagnostics {
            step: 4,
            loss_nats_per_token: 1.70,
            moving_average_loss_nats_per_token: Some(1.00),
            grad_global_l2: 1.0,
        },
    ];

    assert!(
        first_d13_divergence_event(&diagnostics)
            .expect("d13 scan")
            .is_none(),
        "finite loss spikes are non-gating surprise evidence, not D13 divergence"
    );

    let event = first_loss_spike_surprise_event(
        &diagnostics,
        S4LossSpikeSurpriseConfig {
            threshold_nats_per_token: 0.5,
            consecutive_steps: 2,
        },
    )
    .expect("surprise scan")
    .expect("loss spike surprise");

    assert_eq!(event.step, 4);
    assert_eq!(
        event.observed,
        S4RunSurpriseObserved::FiniteLossSpikeSurprise
    );
    assert_eq!(event.loss_nats_per_token, 1.70);
    assert_eq!(event.moving_average_loss_nats_per_token, 1.00);
    assert_eq!(event.threshold_nats_per_token, 0.5);
    assert_eq!(event.consecutive_steps, 2);
    assert_eq!(d13_fail_closed_outcome(&[S4Completion::Completed]), None);

    let event_json: Value =
        serde_json::from_slice(&CanonicalJson::to_vec(&event).expect("canonical event"))
            .expect("event json");
    assert_eq!(event_json["observed"], json!("finite_loss_spike_surprise"));
    assert_eq!(event_json["threshold_nats_per_token"], json!(0.5));
    assert_eq!(event_json["consecutive_steps"], json!(2));
}

#[test]
fn s4_checkpoint_and_fp_reference_reject_divergent_or_mismatched_lineage() {
    let run_log = fixture_run_log()
        .with_computed_self_hash()
        .expect("run log");
    let checkpoint = fixture_checkpoint(&run_log)
        .with_computed_self_hash()
        .expect("checkpoint");

    let mut diverged_checkpoint = fixture_checkpoint(&run_log);
    diverged_checkpoint.completion = S4Completion::DivergedAt { step: 9 };
    let err = diverged_checkpoint
        .with_computed_self_hash()
        .expect_err("diverged run cannot emit checkpoint");
    match err {
        S4RunArtifactError::DivergedRunCannotCheckpoint { observed } => {
            assert_eq!(observed, S4Completion::DivergedAt { step: 9 });
        }
        other => panic!("unexpected error: {other:?}"),
    }

    let mut mismatched = fixture_fp_reference(&checkpoint);
    mismatched.source_checkpoint_self_hash = h(77);
    let mismatched = mismatched
        .with_computed_self_hash()
        .expect("self-hashed mismatch");
    let err = mismatched
        .validate_against_checkpoint(&checkpoint)
        .expect_err("mismatched checkpoint lineage");
    match err {
        S4RunArtifactError::LineageMismatch {
            field,
            expected,
            observed,
        } => {
            assert_eq!(field, "source_checkpoint_self_hash");
            assert_eq!(expected, checkpoint.checkpoint_self_hash);
            assert_eq!(observed, h(77));
        }
        other => panic!("unexpected error: {other:?}"),
    }

    let err = fp_reference_manifest_val_mismatch(&checkpoint)
        .validate_against_checkpoint_and_gutenberg_val_sha(&checkpoint, h(78))
        .expect_err("manifest val sha mismatch");
    match err {
        S4RunArtifactError::LineageMismatch {
            field,
            expected,
            observed,
        } => {
            assert_eq!(field, "corpus_val_sha");
            assert_eq!(expected, h(78));
            assert_eq!(observed, checkpoint.corpus_val_sha);
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn s4_run_artifact_self_hash_mismatches_are_rejected_directly() {
    let run_log = fixture_run_log()
        .with_computed_self_hash()
        .expect("run log");
    let checkpoint = fixture_checkpoint(&run_log)
        .with_computed_self_hash()
        .expect("checkpoint");
    let fp_reference = fixture_fp_reference(&checkpoint)
        .with_computed_self_hash()
        .expect("fp reference");

    let mut bad_run_log = run_log.clone();
    bad_run_log.run_log_self_hash = h(81);
    assert_self_hash_mismatch(
        bad_run_log.validate_canonical_write(),
        "run_log_self_hash",
        h(81),
    );

    let mut bad_checkpoint = checkpoint.clone();
    bad_checkpoint.checkpoint_self_hash = h(82);
    assert_self_hash_mismatch(
        bad_checkpoint.validate_canonical_write(),
        "checkpoint_self_hash",
        h(82),
    );

    let mut bad_fp_reference = fp_reference;
    bad_fp_reference.fp_reference_self_hash = h(83);
    assert_self_hash_mismatch(
        bad_fp_reference.validate_canonical_write(),
        "fp_reference_self_hash",
        h(83),
    );
}

fn fixture_run_log() -> S4GutenbergRunLog {
    S4GutenbergRunLog {
        schema: S4_GUTENBERG_RUN_LOG_SCHEMA.to_owned(),
        tinystories_manifest_self_hash: h(1),
        gutenberg_manifest_self_hash: h(2),
        seed: 0,
        train_config_hash: h(3),
        promotion_gate_self_hash: h(4),
        c_ts_checkpoint_self_hash: h(5),
        initial_checkpoint_payload_sha: h(6),
        initial_weight_source: S4InitialWeightSource::CTsRef,
        initial_fp_shadow_payload_sha: h(7),
        init_rng_draw_count_before_first_step: 0,
        shuffle_rng_draw_count_total: 0,
        losses: (1..=S4_OPTIMIZER_STEPS_GUTENBERG)
            .map(|step| (step, 0.8 + (step as f64 / 1_000_000.0)))
            .collect(),
        eval_points: progress_eval_steps(&S4TrainConfig::pinned())
            .expect("eval steps")
            .into_iter()
            .map(|step| (step, 1.1 + (step as f64 / 1_000_000.0)))
            .collect(),
        final_grad_norms: S4GradNormSummary {
            global_l2: 2.5,
            max_l2: 1.5,
            mean_l2: 0.25,
        },
        run_log_self_hash: Hash256::ZERO,
    }
}

fn fixture_checkpoint(run_log: &S4GutenbergRunLog) -> S4GutenbergCheckpointMetadata {
    S4GutenbergCheckpointMetadata {
        schema: S4_GUTENBERG_CHECKPOINT_SCHEMA.to_owned(),
        seed: run_log.seed,
        c_ts_checkpoint_self_hash: run_log.c_ts_checkpoint_self_hash,
        promotion_gate_self_hash: run_log.promotion_gate_self_hash,
        deployed_tensor_payload_sha: h(8),
        fp_shadow_tensor_payload_sha: run_log.initial_fp_shadow_payload_sha,
        corpus_train_sha: h(9),
        corpus_val_sha: h(10),
        gutenberg_manifest_self_hash: run_log.gutenberg_manifest_self_hash,
        tinystories_manifest_self_hash: run_log.tinystories_manifest_self_hash,
        model_config_hash: h(11),
        train_config_hash: run_log.train_config_hash,
        build_kind: S4BuildKind::phase_d_continuation,
        build_config_hash: h(12),
        dependency_lockfile_sha: h(13),
        rust_toolchain_hash: h(14),
        device_profile_hash: h(15),
        pass_version: SemVer::new(1, 0, 0),
        final_step: S4_OPTIMIZER_STEPS_GUTENBERG,
        final_train_loss: run_log.losses.last().expect("last loss").1,
        completion: S4Completion::Completed,
        checkpoint_self_hash: Hash256::ZERO,
    }
}

fn fixture_fp_reference(checkpoint: &S4GutenbergCheckpointMetadata) -> S4FpReferenceArtifact {
    S4FpReferenceArtifact {
        schema: S4_FP_REFERENCE_SCHEMA.to_owned(),
        seed: checkpoint.seed,
        source_checkpoint_self_hash: checkpoint.checkpoint_self_hash,
        fp_reference_kind: S4_FP_REFERENCE_KIND_QAT_SHADOW_AFTER_GUTENBERG.to_owned(),
        fp_shadow_payload_sha: checkpoint.fp_shadow_tensor_payload_sha,
        tinystories_manifest_self_hash: checkpoint.tinystories_manifest_self_hash,
        gutenberg_manifest_self_hash: checkpoint.gutenberg_manifest_self_hash,
        corpus_val_sha: checkpoint.corpus_val_sha,
        fp_reference_self_hash: Hash256::ZERO,
    }
}

fn fp_reference_manifest_val_mismatch(
    checkpoint: &S4GutenbergCheckpointMetadata,
) -> S4FpReferenceArtifact {
    fixture_fp_reference(checkpoint)
        .with_computed_self_hash()
        .expect("fp reference")
}

fn assert_self_hash_mismatch(
    result: Result<(), S4RunArtifactError>,
    expected_field: &'static str,
    expected_observed: Hash256,
) {
    match result.expect_err("self-hash mismatch") {
        S4RunArtifactError::SelfHashMismatch {
            field, observed, ..
        } => {
            assert_eq!(field, expected_field);
            assert_eq!(observed, expected_observed);
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

fn h(byte: u8) -> Hash256 {
    Hash256::from_bytes([byte; 32])
}
