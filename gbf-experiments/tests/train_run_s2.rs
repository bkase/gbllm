mod common;

use std::env;
use std::ffi::OsString;
use std::sync::{Mutex, MutexGuard};

use common::tracing_capture::{TraceCapture, captured_events, with_trace_capture};
use gbf_experiments::s2::run::{
    DivergenceObservation, RunInputs, RunProductS2, S2PreconditionError, S2TrainRunError,
    S2TrainRunOptions, TrainConfigS2Run, s2_train_run, s2_train_run_with_options,
};
use gbf_experiments::s2::schema::{
    S2_OPTIMIZER_STEPS, S2_TEACHER_FREEZE_STEP, S2BuildKind, TrainConfigS2Full,
};
use gbf_foundation::Hash256;
use safetensors::SafeTensors;
use serde_json::json;

#[test]
fn tiny_ternary_seed0_run_completes_with_artifact_hashes_and_logs() {
    let _env = s2_env(&[]);
    let inputs = RunInputs::tiny_fixture(0, S2BuildKind::s2_ternary_full);
    let capture = TraceCapture::default();

    let product = completed(with_trace_capture(&capture, || {
        s2_train_run(&inputs).expect("s2 train run")
    }));

    assert_ne!(product.final_checkpoint_sha, Hash256::ZERO);
    assert_eq!(
        product.final_checkpoint_sha,
        gbf_foundation::sha256(&product.final_checkpoint)
    );
    assert!(
        SafeTensors::deserialize(&product.final_checkpoint).is_ok(),
        "final checkpoint must deserialize as canonical SafeTensors"
    );
    assert_eq!(
        product
            .phase_boundary_checkpoint_shas
            .keys()
            .copied()
            .collect::<Vec<_>>(),
        vec![4_000, 5_000, 8_000, 10_000]
    );
    assert_eq!(product.phase_entries.len(), S2_OPTIMIZER_STEPS as usize);
    assert_eq!(
        product.phase_log_self_hash,
        product
            .phase_log
            .computed_self_hash(&product.phase_entries)
            .expect("phase-log hash")
    );
    assert_eq!(
        product.distill_log_self_hash,
        product
            .distillation_log
            .computed_self_hash()
            .expect("distill-log hash")
    );
    assert_eq!(
        product.score_self_hash,
        product
            .score_report
            .computed_self_hash()
            .expect("score hash")
    );
    assert_ne!(product.teacher_storage_fingerprint, Hash256::ZERO);
    assert_ne!(product.teacher_weight_fingerprint, Hash256::ZERO);
    let final_loss_terms = product
        .distillation_log
        .loss_terms_per_eval_point
        .iter()
        .find(|point| point.eval_step == S2_OPTIMIZER_STEPS)
        .expect("final eval loss terms");
    let range_raw = final_loss_terms.raw_losses["range"].expect("range raw loss");
    let zero_raw = final_loss_terms.raw_losses["zero"].expect("zero raw loss");
    assert!(range_raw > 0.0, "{range_raw}");
    assert!(zero_raw > 0.0, "{zero_raw}");
    assert_ne!(range_raw, 0.05, "range loss must not be the old Toy0 stub");
    assert_ne!(zero_raw, 0.03, "zero loss must not be the old Toy0 stub");
    assert_eq!(
        final_loss_terms.weighted_losses["range"],
        Some(range_raw * final_loss_terms.lambda_effective.lambda_range)
    );
    assert_eq!(
        final_loss_terms.weighted_losses["zero"],
        Some(zero_raw * final_loss_terms.lambda_effective.lambda_zero)
    );

    let events = captured_events(&capture);
    assert_eq!(
        events
            .iter()
            .filter(|event| event.name == "train_step")
            .count(),
        S2_OPTIMIZER_STEPS as usize
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.name == "eval_step_summary")
            .count(),
        10
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.name == "phase_boundary_checkpoint_written")
            .count(),
        4
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.name == "teacher_freeze_complete")
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.name == "train_run_completed")
            .count(),
        1
    );

    let snapshot_name = if cfg!(feature = "s2-ablation") {
        "train_run_s2__tiny_ternary_seed0_s2_ablation_feature"
    } else {
        "train_run_s2__tiny_ternary_seed0"
    };
    insta::assert_snapshot!(
        snapshot_name,
        json!({
            "build_kind": "s2_ternary_full",
            "seed": 0,
            "optimizer_steps": product.phase_entries.len(),
            "phase_boundary_checkpoint_steps": product
                .phase_boundary_checkpoint_shas
                .keys()
                .copied()
                .collect::<Vec<_>>(),
            "final_checkpoint_sha": product.final_checkpoint_sha,
            "phase_log_self_hash": product.phase_log_self_hash,
            "distill_log_self_hash": product.distill_log_self_hash,
            "score_self_hash": product.score_self_hash,
            "teacher_weight_fingerprint": product.teacher_weight_fingerprint,
            "teacher_storage_fingerprint": product.teacher_storage_fingerprint,
        })
        .to_string()
    );
}

#[test]
fn synthetic_non_finite_channels_diverge_without_serializing_nan() {
    assert_diverges_at(
        S2TrainRunOptions {
            non_finite_loss_step: Some(200),
            ..S2TrainRunOptions::default()
        },
        200,
        DivergenceObservation::NonFiniteLoss,
    );
    assert_diverges_at(
        S2TrainRunOptions {
            non_finite_grad_norm_step: Some(300),
            ..S2TrainRunOptions::default()
        },
        300,
        DivergenceObservation::NonFiniteGradNorm,
    );
    assert_diverges_at(
        S2TrainRunOptions {
            non_finite_distill_loss_step: Some(5_500),
            ..S2TrainRunOptions::default()
        },
        5_500,
        DivergenceObservation::NonFiniteDistillLoss,
    );
}

#[test]
fn step_one_non_finite_loss_has_no_last_finite_step() {
    let diverged = diverged_at(
        S2TrainRunOptions {
            non_finite_loss_step: Some(1),
            ..S2TrainRunOptions::default()
        },
        1,
        DivergenceObservation::NonFiniteLoss,
    );

    assert_eq!(diverged.divergence_event.last_finite_step, None);
}

#[test]
fn pre_phase_c_null_distill_loss_does_not_trigger_d12() {
    let _env = s2_env(&[]);
    let inputs = RunInputs::tiny_fixture(0, S2BuildKind::s2_ternary_full);

    let product = s2_train_run_with_options(
        &inputs,
        &S2TrainRunOptions {
            non_finite_distill_loss_step: Some(4_500),
            ..S2TrainRunOptions::default()
        },
    )
    .expect("s2 train run");

    assert!(matches!(product, RunProductS2::Completed(_)));
}

#[test]
fn prechecks_fail_before_run_artifacts_are_allocated() {
    let _env = s2_env(&[]);
    let mut inputs = RunInputs::tiny_fixture(0, S2BuildKind::s2_ternary_full);
    inputs.corpus_train.expected_sha = Hash256::from_bytes([0x5a; 32]);

    let error = s2_train_run(&inputs).expect_err("corpus mismatch should fail");

    assert!(matches!(
        error,
        S2TrainRunError::Precondition(S2PreconditionError::CorpusShaMismatch {
            corpus: "corpus_train",
            ..
        })
    ));
}

#[test]
fn env_exact_violation_aborts_before_run_artifacts_are_allocated() {
    let _env = s2_env(&[("BURN_NDARRAY_NUM_THREADS", "4")]);
    let inputs = RunInputs::tiny_fixture(0, S2BuildKind::s2_ternary_full);

    let error = s2_train_run(&inputs).expect_err("env_exact mismatch should fail");

    assert!(matches!(
        error,
        S2TrainRunError::Precondition(S2PreconditionError::EnvExactViolation {
            var: "BURN_NDARRAY_NUM_THREADS",
            expected: "1",
            ..
        })
    ));
}

#[test]
fn build_config_mismatch_precondition_aborts_before_run_artifacts_are_allocated() {
    let _env = s2_env(&[]);
    let mut inputs = RunInputs::tiny_fixture(0, S2BuildKind::s2_ablation);
    inputs.train_config = TrainConfigS2Run::Full(TrainConfigS2Full::pinned());

    let error = s2_train_run(&inputs).expect_err("build/config mismatch should fail");

    assert!(matches!(
        error,
        S2TrainRunError::Precondition(S2PreconditionError::BuildConfigMismatch {
            build_kind: S2BuildKind::s2_ablation
        })
    ));
}

#[test]
fn unsupported_model_profile_precondition_aborts_before_run_artifacts_are_allocated() {
    let _env = s2_env(&[]);
    let mut inputs = RunInputs::tiny_fixture(0, S2BuildKind::s2_ternary_full);
    inputs.model_config.profile = "FutureModel".to_owned();

    let error = s2_train_run(&inputs).expect_err("unsupported model profile should fail");

    assert!(matches!(
        error,
        S2TrainRunError::Precondition(S2PreconditionError::UnsupportedModelProfile {
            profile
        }) if profile == "FutureModel"
    ));
}

#[test]
fn memory_high_warning_has_structured_shape_and_threshold() {
    let _env = s2_env(&[]);
    let inputs = RunInputs::tiny_fixture(0, S2BuildKind::s2_ablation);
    let capture = TraceCapture::default();

    let product = with_trace_capture(&capture, || {
        s2_train_run_with_options(
            &inputs,
            &S2TrainRunOptions {
                rss_mib_sample: Some(4_097),
                ..S2TrainRunOptions::default()
            },
        )
        .expect("s2 train run")
    });

    assert!(matches!(product, RunProductS2::Completed(_)));
    let events = captured_events(&capture);
    let memory_events = events
        .iter()
        .filter(|event| event.name == "train_run_memory_high")
        .collect::<Vec<_>>();
    assert_eq!(memory_events.len(), 1, "{memory_events:?}");
    assert_eq!(memory_events[0].level, "WARN");
    assert_eq!(memory_events[0].fields.get("rss_mib"), Some(&json!(4_097)));
    assert_eq!(
        memory_events[0].fields.get("threshold_mib"),
        Some(&json!(4_096))
    );
}

#[test]
fn all_build_paths_complete_and_ablation_does_not_freeze_teacher() {
    for build_kind in [
        S2BuildKind::s2_ternary_full,
        S2BuildKind::s2_fp_full,
        S2BuildKind::s2_ternary_nodistill,
        S2BuildKind::s2_ablation,
    ] {
        let _env = s2_env(&[]);
        let inputs = RunInputs::tiny_fixture(0, build_kind);
        let product = completed(s2_train_run(&inputs).expect("s2 train run"));

        if build_kind == S2BuildKind::s2_ablation {
            assert_eq!(product.phase_entries.len(), S2_TEACHER_FREEZE_STEP as usize);
            assert_eq!(
                product
                    .phase_boundary_checkpoint_shas
                    .keys()
                    .copied()
                    .collect::<Vec<_>>(),
                vec![S2_TEACHER_FREEZE_STEP]
            );
            assert_eq!(product.teacher_storage_fingerprint, Hash256::ZERO);
            assert_eq!(product.teacher_weight_fingerprint, Hash256::ZERO);
            assert!(
                product
                    .phase_entries
                    .iter()
                    .all(|entry| entry.events.is_empty())
            );
        } else {
            assert_eq!(product.phase_entries.len(), S2_OPTIMIZER_STEPS as usize);
            assert_ne!(product.teacher_storage_fingerprint, Hash256::ZERO);
            assert_ne!(product.teacher_weight_fingerprint, Hash256::ZERO);
        }
    }
}

fn assert_diverges_at(options: S2TrainRunOptions, step: u64, observed: DivergenceObservation) {
    let _ = diverged_at(options, step, observed);
}

fn diverged_at(
    options: S2TrainRunOptions,
    step: u64,
    observed: DivergenceObservation,
) -> gbf_experiments::s2::run::DivergedRunProductS2 {
    let _env = s2_env(&[]);
    let inputs = RunInputs::tiny_fixture(0, S2BuildKind::s2_ternary_full);
    let capture = TraceCapture::default();

    let product = with_trace_capture(&capture, || {
        s2_train_run_with_options(&inputs, &options).expect("s2 train run")
    });

    let RunProductS2::Diverged(diverged) = product else {
        panic!("expected divergence at step {step}");
    };
    assert_eq!(diverged.divergence_event.step, step);
    assert_eq!(diverged.divergence_event.observed, observed);
    assert_eq!(
        diverged.divergence_event.last_finite_step,
        (step > 1).then_some(step - 1)
    );
    assert!(diverged.divergence_event.no_nan_serialized);
    let events = captured_events(&capture);
    assert!(events.iter().any(|event| event.name == "train_run_diverged"
        && event.fields.get("step") == Some(&json!(step))
        && event.fields.get("no_nan_serialized") == Some(&json!(true))));
    assert!(
        !events
            .iter()
            .any(|event| event.name == "train_step"
                && event.fields.get("step") == Some(&json!(step))),
        "divergent computed diagnostics must not be serialized as train_step: {events:?}"
    );

    if observed == DivergenceObservation::NonFiniteDistillLoss {
        insta::assert_snapshot!(
            "train_run_s2__divergence_event_serialization",
            serde_json::to_string(&diverged.divergence_event).expect("event JSON")
        );
    }
    diverged
}

fn completed(product: RunProductS2) -> Box<gbf_experiments::s2::run::CompletedRunProductS2> {
    match product {
        RunProductS2::Completed(product) => product,
        RunProductS2::Diverged(diverged) => {
            panic!("unexpected divergence: {:?}", diverged.divergence_event)
        }
    }
}

static ENV_LOCK: Mutex<()> = Mutex::new(());

struct S2EnvGuard {
    original: Vec<(&'static str, Option<OsString>)>,
    _lock: MutexGuard<'static, ()>,
}

impl Drop for S2EnvGuard {
    fn drop(&mut self) {
        for (key, value) in &self.original {
            match value {
                Some(value) => {
                    // SAFETY: S2EnvGuard serializes mutation of these vars.
                    unsafe { env::set_var(key, value) };
                }
                None => {
                    // SAFETY: S2EnvGuard serializes mutation of these vars.
                    unsafe { env::remove_var(key) };
                }
            }
        }
    }
}

fn s2_env(overrides: &[(&'static str, &'static str)]) -> S2EnvGuard {
    let lock = ENV_LOCK.lock().expect("S2 env test lock poisoned");
    let keys = [
        "BURN_NDARRAY_NUM_THREADS",
        "BURN_DETERMINISTIC",
        "OMP_NUM_THREADS",
        "RAYON_NUM_THREADS",
    ];
    let original = keys
        .iter()
        .map(|key| (*key, env::var_os(key)))
        .collect::<Vec<_>>();
    for key in keys {
        // SAFETY: S2EnvGuard serializes mutation of these vars.
        unsafe { env::remove_var(key) };
    }
    for (key, value) in overrides {
        // SAFETY: S2EnvGuard serializes mutation of these vars.
        unsafe { env::set_var(key, value) };
    }
    S2EnvGuard {
        original,
        _lock: lock,
    }
}
