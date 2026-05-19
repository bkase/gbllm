mod common;

use std::env;
use std::ffi::OsString;
use std::sync::{Mutex, MutexGuard};

use common::tracing_capture::{TraceCapture, captured_events, with_trace_capture};
use gbf_artifact::ids::ArtifactPath;
use gbf_artifact::tensor::{
    CanonicalTensor, CanonicalTensorKind, CanonicalTensorLayout, CanonicalTensorPayload,
    CanonicalTensorShape, TensorElementType,
};
use gbf_experiments::s1::run::{CheckpointMetadata, canonical_checkpoint_bytes};
use gbf_experiments::s1::schema::S1CanonicalJson;
use gbf_experiments::s2::run::{CompletedRunProductS2, RunInputs, RunProductS2, s2_train_run};
use gbf_experiments::s2::schema::S2BuildKind;
use gbf_experiments::s2::score::{S2ScoreError, ScoreInputs, s2_score, try_s2_score};
use gbf_foundation::sha256;
use serde_json::json;

#[test]
fn bd_1btw_same_inputs_and_checkpoint_emit_byte_equal_score_reports() {
    let _env = s2_env(&[]);
    let product = completed(
        s2_train_run(&RunInputs::tiny_fixture(0, S2BuildKind::s2_ternary_full))
            .expect("s2 train run"),
    );
    let inputs = ScoreInputs::new(b"canonical validation bytes".to_vec());

    let first = s2_score(inputs.clone(), &product);
    let second = s2_score(inputs, &product);

    assert_eq!(first.bpc, second.bpc);
    assert_eq!(first.score_self_hash, second.score_self_hash);
    let threshold_stats = first
        .threshold_stats
        .as_ref()
        .expect("ternary score threshold stats");
    let scale_stats = first
        .scale_stats
        .as_ref()
        .expect("ternary score scale stats");
    assert_eq!(
        serde_json::to_value(&first).expect("s2 score JSON shape"),
        json!({
            "schema": "s2_score.v1",
            "seed": first.seed,
            "build_kind": "s2-ternary-full",
            "checkpoint_sha": first.checkpoint_sha.to_string(),
            "corpus_val_sha": first.corpus_val_sha.to_string(),
            "chunk_size": 128,
            "token_count": first.token_count,
            "log2_sum": first.log2_sum,
            "bpc": first.bpc,
            "threshold_stats": {
                "matrices": threshold_stats.matrices,
                "threshold_min": threshold_stats.threshold_min,
                "threshold_max": threshold_stats.threshold_max,
                "threshold_mean": threshold_stats.threshold_mean,
                "threshold_count": threshold_stats.threshold_count,
            },
            "scale_stats": {
                "matrices": scale_stats.matrices,
                "scale_count": scale_stats.scale_count,
                "scale_min": scale_stats.scale_min,
                "scale_max": scale_stats.scale_max,
                "scale_mean_f32": scale_stats.scale_mean_f32,
            },
            "score_self_hash": first.score_self_hash.to_string(),
        })
    );
    assert_eq!(
        S1CanonicalJson::to_vec(&first).expect("first score JSON"),
        S1CanonicalJson::to_vec(&second).expect("second score JSON")
    );
}

#[test]
fn bd_1btw_ternary_final_checkpoint_emits_threshold_and_scale_stats() {
    let _env = s2_env(&[]);
    let product = completed(
        s2_train_run(&RunInputs::tiny_fixture(0, S2BuildKind::s2_ternary_full))
            .expect("s2 train run"),
    );
    let capture = TraceCapture::default();

    let report = with_trace_capture(&capture, || {
        s2_score(ScoreInputs::new(b"held-out eval".to_vec()), &product)
    });

    let threshold_stats = report
        .threshold_stats
        .as_ref()
        .expect("ternary threshold stats");
    let scale_stats = report.scale_stats.as_ref().expect("ternary scale stats");
    assert_eq!(threshold_stats.matrices, 1);
    assert_eq!(scale_stats.matrices, 1);
    assert_eq!(threshold_stats.threshold_count, 2);
    assert_eq!(scale_stats.scale_count, threshold_stats.threshold_count);
    assert_eq!(threshold_stats.threshold_min, 0.7);
    assert_eq!(threshold_stats.threshold_max, 0.8);
    assert_eq!(scale_stats.scale_min, 1.0);
    assert_eq!(scale_stats.scale_max, 1.5);
    assert!(captured_events(&capture).iter().any(|event| {
        event.name == "s2_score_computed"
            && event.fields.get("seed") == Some(&json!(0))
            && event.fields.get("threshold_stats_present") == Some(&json!(true))
            && event.fields.get("scale_stats_present") == Some(&json!(true))
    }));

    insta::assert_snapshot!(
        "score_s2__ternary_seed0_tiny_bd_1btw",
        String::from_utf8(report.canonical_json_bytes().expect("score canonical")).unwrap()
    );
}

#[test]
fn bd_1btw_fp_and_ablation_score_stats_are_null() {
    for build_kind in [S2BuildKind::s2_fp_full, S2BuildKind::s2_ablation] {
        let _env = s2_env(&[]);
        let product =
            completed(s2_train_run(&RunInputs::tiny_fixture(0, build_kind)).expect("s2 train run"));

        let report = s2_score(ScoreInputs::new(b"held-out eval".to_vec()), &product);

        assert_eq!(report.build_kind, build_kind);
        assert!(report.threshold_stats.is_none());
        assert!(report.scale_stats.is_none());
    }
}

#[test]
fn bd_1btw_repeated_wrapper_runs_produce_byte_equal_score_json() {
    let _env = s2_env(&[]);
    let product = completed(
        s2_train_run(&RunInputs::tiny_fixture(
            0,
            S2BuildKind::s2_ternary_nodistill,
        ))
        .expect("s2 train run"),
    );
    let inputs = ScoreInputs::new(b"same canonical val".to_vec());

    let left = s2_score(inputs.clone(), &product)
        .canonical_json_bytes()
        .expect("left canonical");
    let right = s2_score(inputs, &product)
        .canonical_json_bytes()
        .expect("right canonical");

    assert_eq!(left, right);
}

#[test]
fn bd_766b_qat_stat_extraction_uses_exact_names_and_sorted_order() {
    let _env = s2_env(&[]);
    let mut product = completed(
        s2_train_run(&RunInputs::tiny_fixture(0, S2BuildKind::s2_ternary_full))
            .expect("s2 train run"),
    );
    install_checkpoint(&mut product, order_sensitive_qat_checkpoint());

    let report = s2_score(ScoreInputs::new(b"held-out eval".to_vec()), &product);

    let threshold_stats = report
        .threshold_stats
        .as_ref()
        .expect("ternary threshold stats");
    let scale_stats = report.scale_stats.as_ref().expect("ternary scale stats");
    assert_eq!(threshold_stats.matrices, 3);
    assert_eq!(threshold_stats.threshold_count, 3);
    assert_eq!(
        threshold_stats.threshold_mean.to_bits(),
        (1.0_f32 / 3.0).to_bits()
    );
    assert_eq!(scale_stats.matrices, 3);
    assert_eq!(scale_stats.scale_count, 3);
}

#[test]
fn bd_766b_accidental_qat_substrings_do_not_satisfy_stats_contract() {
    let _env = s2_env(&[]);
    let mut product = completed(
        s2_train_run(&RunInputs::tiny_fixture(0, S2BuildKind::s2_ternary_full))
            .expect("s2 train run"),
    );
    install_checkpoint(&mut product, accidental_qat_name_checkpoint());

    let error = try_s2_score(ScoreInputs::new(b"held-out eval".to_vec()), &product)
        .expect_err("accidental tensor names must not satisfy QAT stats");

    assert!(matches!(
        error,
        S2ScoreError::MissingQatStats {
            build_kind: S2BuildKind::s2_ternary_full
        }
    ));
}

fn install_checkpoint(product: &mut CompletedRunProductS2, checkpoint: Vec<u8>) {
    product.final_checkpoint = checkpoint;
    product.final_checkpoint_sha = sha256(&product.final_checkpoint);
}

fn order_sensitive_qat_checkpoint() -> Vec<u8> {
    canonical_checkpoint_bytes(
        &[
            tensor(
                "toy0.block0.weight",
                CanonicalTensorKind::DenseWeight,
                TensorElementType::Float32,
                CanonicalTensorPayload::F32(vec![0.1, -0.2, 0.3, -0.4]),
                &[2, 2],
            ),
            tensor(
                "toy0.a.thresholds",
                CanonicalTensorKind::DenseBias,
                TensorElementType::Float32,
                CanonicalTensorPayload::F32(vec![-1.0e30]),
                &[1],
            ),
            tensor(
                "toy0.b.thresholds",
                CanonicalTensorKind::DenseBias,
                TensorElementType::Float32,
                CanonicalTensorPayload::F32(vec![1.0e30]),
                &[1],
            ),
            tensor(
                "toy0.c.thresholds",
                CanonicalTensorKind::DenseBias,
                TensorElementType::Float32,
                CanonicalTensorPayload::F32(vec![1.0]),
                &[1],
            ),
            tensor(
                "toy0.a.scales",
                CanonicalTensorKind::TernaryScale,
                TensorElementType::Q8_8,
                CanonicalTensorPayload::U16(vec![256]),
                &[1],
            ),
            tensor(
                "toy0.b.scales",
                CanonicalTensorKind::TernaryScale,
                TensorElementType::Q8_8,
                CanonicalTensorPayload::U16(vec![512]),
                &[1],
            ),
            tensor(
                "toy0.c.scales",
                CanonicalTensorKind::TernaryScale,
                TensorElementType::Q8_8,
                CanonicalTensorPayload::U16(vec![768]),
                &[1],
            ),
        ],
        &CheckpointMetadata::default(),
    )
    .expect("canonical checkpoint")
}

fn accidental_qat_name_checkpoint() -> Vec<u8> {
    canonical_checkpoint_bytes(
        &[
            tensor(
                "toy0.block0.weight",
                CanonicalTensorKind::DenseWeight,
                TensorElementType::Float32,
                CanonicalTensorPayload::F32(vec![0.1, -0.2, 0.3, -0.4]),
                &[2, 2],
            ),
            tensor(
                "toy0.block0.threshold_guess",
                CanonicalTensorKind::DenseBias,
                TensorElementType::Float32,
                CanonicalTensorPayload::F32(vec![0.1, 0.2]),
                &[2],
            ),
            tensor(
                "toy0.block0.upscale_factor",
                CanonicalTensorKind::DenseBias,
                TensorElementType::Q8_8,
                CanonicalTensorPayload::U16(vec![256, 384]),
                &[2],
            ),
        ],
        &CheckpointMetadata::default(),
    )
    .expect("canonical checkpoint")
}

fn tensor(
    id: &str,
    kind: CanonicalTensorKind,
    element_type: TensorElementType,
    payload: CanonicalTensorPayload,
    dims: &[usize],
) -> CanonicalTensor {
    CanonicalTensor::new(
        ArtifactPath::new(id).expect("artifact path"),
        kind,
        CanonicalTensorLayout::new(
            CanonicalTensorShape::from_usize_dims(dims).expect("shape"),
            element_type,
        ),
        payload,
    )
    .expect("canonical tensor")
}

fn completed(product: RunProductS2) -> Box<CompletedRunProductS2> {
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
