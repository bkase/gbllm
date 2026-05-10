mod common;

use common::tracing_capture::{TraceCapture, captured_events, with_trace_capture};
use gbf_artifact::ids::ArtifactPath;
use gbf_artifact::tensor::{
    CanonicalTensor, CanonicalTensorKind, CanonicalTensorLayout, CanonicalTensorPayload,
    CanonicalTensorShape, TensorElementType, canonical_tensor_payload_hash,
};
use gbf_experiments::s1::ablation::{AblationCheckpoint, AblationError, compare};
use gbf_experiments::s1::logging::{event, field};
use gbf_experiments::s1::rng::rng_stream_def_hash;
use gbf_experiments::s1::schema::{
    CheckpointMetadata, S1BuildKind, S1CanonicalJson, S1Completion, TensorMismatch,
};
use gbf_foundation::{Hash256, SemVer};
use proptest::prelude::*;
use serde_json::json;

#[test]
fn equal_payloads_emit_self_hashed_match_report() {
    let phase_a_metadata = checkpoint_metadata(S1BuildKind::PhaseA);
    let ablation_metadata = checkpoint_metadata(S1BuildKind::Ablation);
    let phase_a_tensors = vec![f32_tensor("toy0.weight", vec![1.0, 2.0])];
    let ablation_tensors = phase_a_tensors.clone();

    let report = compare(
        checkpoint(&phase_a_metadata, hash(10), &phase_a_tensors),
        checkpoint(&ablation_metadata, hash(11), &ablation_tensors),
    )
    .expect("ablation report");

    assert_eq!(report.schema, "s1_ablation.v1");
    assert_eq!(report.seed, 0);
    assert_eq!(report.phase_a_checkpoint_sha, hash(10));
    assert_eq!(report.ablation_checkpoint_sha, hash(11));
    assert_eq!(
        report.phase_a_tensor_payload_sha,
        canonical_tensor_payload_hash(&phase_a_tensors)
    );
    assert_eq!(
        report.phase_a_tensor_payload_sha,
        report.ablation_tensor_payload_sha
    );
    assert!(report.phase_a_eq_ablation);
    assert_eq!(report.first_mismatch, None);
    assert_eq!(
        report.ablation_self_hash,
        report.computed_self_hash().expect("self hash")
    );

    let bytes = report.canonical_json_bytes().expect("canonical JSON");
    let decoded: serde_json::Value = serde_json::from_slice(&bytes).expect("JSON");
    assert_eq!(decoded["phase_a_eq_ablation"], true);
}

#[test]
fn one_byte_payload_mismatch_reports_exact_tensor_and_offset() {
    let phase_a_metadata = checkpoint_metadata(S1BuildKind::PhaseA);
    let ablation_metadata = checkpoint_metadata(S1BuildKind::Ablation);
    let phase_a_tensors = vec![i8_tensor("toy0.router.weight", vec![0, 1, -1])];
    let ablation_tensors = vec![i8_tensor("toy0.router.weight", vec![0, 1, 0])];

    let report = compare(
        checkpoint(&phase_a_metadata, hash(10), &phase_a_tensors),
        checkpoint(&ablation_metadata, hash(11), &ablation_tensors),
    )
    .expect("ablation report");

    assert!(!report.phase_a_eq_ablation);
    assert_ne!(
        report.phase_a_tensor_payload_sha,
        report.ablation_tensor_payload_sha
    );
    assert_eq!(
        report.first_mismatch,
        Some(TensorMismatch {
            tensor: "toy0.router.weight".to_owned(),
            byte_offset: 2,
        })
    );
    assert_eq!(
        report.ablation_self_hash,
        report.computed_self_hash().expect("self hash")
    );
}

#[test]
fn same_name_payload_length_mismatch_reports_first_missing_byte() {
    let phase_a_metadata = checkpoint_metadata(S1BuildKind::PhaseA);
    let ablation_metadata = checkpoint_metadata(S1BuildKind::Ablation);
    let phase_a_tensors = vec![u16_tensor_unchecked_layout("toy0.scale", vec![1, 2], &[3])];
    let ablation_tensors = vec![u16_tensor("toy0.scale", vec![1, 2, 3])];

    let report = compare(
        checkpoint(&phase_a_metadata, hash(10), &phase_a_tensors),
        checkpoint(&ablation_metadata, hash(11), &ablation_tensors),
    )
    .expect("ablation report");

    assert_eq!(
        report.first_mismatch,
        Some(TensorMismatch {
            tensor: "toy0.scale".to_owned(),
            byte_offset: 4,
        })
    );
}

#[test]
fn dtype_layout_and_kind_divergence_report_zero_offset() {
    let phase_a_metadata = checkpoint_metadata(S1BuildKind::PhaseA);
    let ablation_metadata = checkpoint_metadata(S1BuildKind::Ablation);

    for (phase_a_tensors, ablation_tensors) in [
        (
            vec![f32_tensor("toy0.weight", vec![1.0, 2.0])],
            vec![i8_tensor("toy0.weight", vec![-1, 1])],
        ),
        (
            vec![f32_tensor_with_dims("toy0.weight", vec![1.0, 2.0], &[2])],
            vec![f32_tensor_with_dims("toy0.weight", vec![1.0, 2.0], &[1, 2])],
        ),
        (
            vec![i8_tensor_with_kind(
                "toy0.weight",
                vec![-1, 1],
                CanonicalTensorKind::TernaryWeight,
            )],
            vec![i8_tensor_with_kind(
                "toy0.weight",
                vec![-1, 1],
                CanonicalTensorKind::RouterWeight,
            )],
        ),
    ] {
        let report = compare(
            checkpoint(&phase_a_metadata, hash(10), &phase_a_tensors),
            checkpoint(&ablation_metadata, hash(11), &ablation_tensors),
        )
        .expect("ablation report");

        assert_eq!(
            report.first_mismatch,
            Some(TensorMismatch {
                tensor: "toy0.weight".to_owned(),
                byte_offset: 0,
            })
        );
    }
}

#[test]
fn missing_or_extra_tensor_reports_tensor_name_at_zero_offset() {
    let phase_a_metadata = checkpoint_metadata(S1BuildKind::PhaseA);
    let ablation_metadata = checkpoint_metadata(S1BuildKind::Ablation);
    let phase_a_tensors = vec![
        f32_tensor("toy0.a.weight", vec![1.0]),
        f32_tensor("toy0.b.weight", vec![2.0]),
    ];
    let ablation_tensors = vec![f32_tensor("toy0.a.weight", vec![1.0])];

    let missing = compare(
        checkpoint(&phase_a_metadata, hash(10), &phase_a_tensors),
        checkpoint(&ablation_metadata, hash(11), &ablation_tensors),
    )
    .expect("missing tensor report");
    assert_eq!(
        missing.first_mismatch,
        Some(TensorMismatch {
            tensor: "toy0.b.weight".to_owned(),
            byte_offset: 0,
        })
    );

    let extra = compare(
        checkpoint(&phase_a_metadata, hash(10), &ablation_tensors),
        checkpoint(&ablation_metadata, hash(11), &phase_a_tensors),
    )
    .expect("extra tensor report");
    assert_eq!(
        extra.first_mismatch,
        Some(TensorMismatch {
            tensor: "toy0.b.weight".to_owned(),
            byte_offset: 0,
        })
    );
}

#[test]
fn normative_ablation_seed_must_be_zero_even_when_both_sides_match() {
    let phase_a_metadata = checkpoint_metadata(S1BuildKind::PhaseA);
    let mut ablation_metadata = checkpoint_metadata(S1BuildKind::Ablation);
    let mut phase_a_metadata = phase_a_metadata;
    phase_a_metadata.seed = 1;
    ablation_metadata.seed = 1;
    let phase_a_tensors = vec![f32_tensor("toy0.weight", vec![1.0])];
    let ablation_tensors = vec![f32_tensor("toy0.weight", vec![1.0])];

    let error = compare(
        checkpoint(&phase_a_metadata, hash(10), &phase_a_tensors),
        checkpoint(&ablation_metadata, hash(11), &ablation_tensors),
    )
    .expect_err("nonzero seed must fail before comparison");

    assert!(matches!(
        error,
        AblationError::InvalidSeed {
            side: gbf_experiments::s1::ablation::AblationSide::PhaseA,
            observed: 1,
        }
    ));
}

#[test]
fn metadata_mismatch_matrix_is_typed_error_before_payload_compare() {
    let tensors = vec![f32_tensor("toy0.weight", vec![1.0])];

    let cases: [(&str, fn(&mut CheckpointMetadata)); 6] = [
        ("corpus_train_sha", |metadata: &mut CheckpointMetadata| {
            metadata.corpus_train_sha = hash(90)
        }),
        ("corpus_val_sha", |metadata: &mut CheckpointMetadata| {
            metadata.corpus_val_sha = hash(91)
        }),
        ("model_config_hash", |metadata: &mut CheckpointMetadata| {
            metadata.model_config_hash = hash(92)
        }),
        ("train_config_hash", |metadata: &mut CheckpointMetadata| {
            metadata.train_config_hash = hash(93)
        }),
        (
            "device_profile_hash",
            |metadata: &mut CheckpointMetadata| metadata.device_profile_hash = hash(94),
        ),
        (
            "rng_stream_def_hash",
            |metadata: &mut CheckpointMetadata| metadata.rng_stream_def_hash = hash(95),
        ),
    ];

    for (field_name, mutate) in cases {
        let phase_a_metadata = checkpoint_metadata(S1BuildKind::PhaseA);
        let mut ablation_metadata = checkpoint_metadata(S1BuildKind::Ablation);
        mutate(&mut ablation_metadata);

        let error = compare(
            checkpoint(&phase_a_metadata, hash(10), &tensors),
            checkpoint(&ablation_metadata, hash(11), &tensors),
        )
        .expect_err("metadata mismatch must fail");

        assert!(matches!(
            error,
            AblationError::MetadataMismatch {
                field,
                ..
            } if field == field_name
        ));
    }
}

#[test]
fn build_kind_must_match_comparison_side() {
    let phase_a_metadata = checkpoint_metadata(S1BuildKind::Ablation);
    let ablation_metadata = checkpoint_metadata(S1BuildKind::Ablation);
    let tensors = vec![f32_tensor("toy0.weight", vec![1.0])];

    let error = compare(
        checkpoint(&phase_a_metadata, hash(10), &tensors),
        checkpoint(&ablation_metadata, hash(11), &tensors),
    )
    .expect_err("phase A side must have phase_a build kind");

    assert!(matches!(error, AblationError::InvalidBuildKind { .. }));
}

#[test]
fn duplicate_tensor_name_is_typed_error() {
    let phase_a_metadata = checkpoint_metadata(S1BuildKind::PhaseA);
    let ablation_metadata = checkpoint_metadata(S1BuildKind::Ablation);
    let duplicate_tensors = vec![
        f32_tensor("toy0.weight", vec![1.0]),
        f32_tensor("toy0.weight", vec![1.0]),
    ];
    let ablation_tensors = vec![f32_tensor("toy0.weight", vec![1.0])];

    let error = compare(
        checkpoint(&phase_a_metadata, hash(10), &duplicate_tensors),
        checkpoint(&ablation_metadata, hash(11), &ablation_tensors),
    )
    .expect_err("duplicate tensor names are invalid");

    assert!(matches!(
        error,
        AblationError::DuplicateTensorName {
            tensor,
            ..
        } if tensor == "toy0.weight"
    ));
}

#[test]
fn report_round_trips_through_canonical_json() {
    let phase_a_metadata = checkpoint_metadata(S1BuildKind::PhaseA);
    let ablation_metadata = checkpoint_metadata(S1BuildKind::Ablation);
    let phase_a_tensors = vec![f32_tensor("toy0.weight", vec![1.0])];
    let ablation_tensors = phase_a_tensors.clone();
    let report = compare(
        checkpoint(&phase_a_metadata, hash(10), &phase_a_tensors),
        checkpoint(&ablation_metadata, hash(11), &ablation_tensors),
    )
    .expect("ablation report");

    let bytes = S1CanonicalJson::to_vec(&report).expect("canonical JSON");
    let decoded: gbf_experiments::s1::schema::AblationReport =
        serde_json::from_slice(&bytes).expect("round trip");

    assert_eq!(decoded, report);
    assert_eq!(
        decoded.ablation_self_hash,
        decoded.computed_self_hash().expect("self hash")
    );
}

#[test]
fn compare_emits_subscriber_captured_match_events() {
    let capture = TraceCapture::default();
    let phase_a_metadata = checkpoint_metadata(S1BuildKind::PhaseA);
    let ablation_metadata = checkpoint_metadata(S1BuildKind::Ablation);
    let tensors = vec![f32_tensor("toy0.weight", vec![1.0])];

    let report = with_trace_capture(&capture, || {
        compare(
            checkpoint(&phase_a_metadata, hash(10), &tensors),
            checkpoint(&ablation_metadata, hash(11), &tensors),
        )
        .expect("ablation report")
    });

    let ablation_events = ablation_events(&capture);
    assert_eq!(
        ablation_events
            .iter()
            .map(|event| event.name.as_str())
            .collect::<Vec<_>>(),
        vec![
            event::ABLATION_COMPARE_START,
            event::ABLATION_TENSOR_COMPARE,
            event::ABLATION_COMPLETE,
        ]
    );
    assert_eq!(ablation_events[0].level, "INFO");
    assert_eq!(ablation_events[0].fields.get(field::SEED), Some(&json!(0)));
    assert_eq!(ablation_events[1].level, "TRACE");
    assert_eq!(
        ablation_events[1].fields.get(field::TENSOR_NAME),
        Some(&json!("toy0.weight"))
    );
    assert_eq!(ablation_events[2].level, "INFO");
    assert_eq!(
        ablation_events[2].fields.get(field::PHASE_A_EQ_ABLATION),
        Some(&json!(true))
    );
    assert_eq!(
        ablation_events[2].fields.get(field::ABLATION_SELF_HASH),
        Some(&json!(report.ablation_self_hash.to_string()))
    );
}

#[test]
fn compare_emits_subscriber_captured_mismatch_event() {
    let capture = TraceCapture::default();
    let phase_a_metadata = checkpoint_metadata(S1BuildKind::PhaseA);
    let ablation_metadata = checkpoint_metadata(S1BuildKind::Ablation);
    let phase_a_tensors = vec![i8_tensor("toy0.router.weight", vec![0, 1, -1])];
    let ablation_tensors = vec![i8_tensor("toy0.router.weight", vec![0, 1, 0])];

    let report = with_trace_capture(&capture, || {
        compare(
            checkpoint(&phase_a_metadata, hash(10), &phase_a_tensors),
            checkpoint(&ablation_metadata, hash(11), &ablation_tensors),
        )
        .expect("ablation report")
    });

    assert_eq!(
        report.first_mismatch,
        Some(TensorMismatch {
            tensor: "toy0.router.weight".to_owned(),
            byte_offset: 2,
        })
    );
    let ablation_events = ablation_events(&capture);
    let mismatch = ablation_events
        .iter()
        .find(|event| event.name == event::ABLATION_MISMATCH)
        .expect("mismatch event");
    assert_eq!(mismatch.level, "ERROR");
    assert_eq!(
        mismatch.fields.get(field::TENSOR_NAME),
        Some(&json!("toy0.router.weight"))
    );
    assert_eq!(mismatch.fields.get(field::BYTE_OFFSET), Some(&json!(2)));
}

#[test]
fn metadata_failure_emits_subscriber_captured_error_without_completion() {
    let capture = TraceCapture::default();
    let phase_a_metadata = checkpoint_metadata(S1BuildKind::PhaseA);
    let mut ablation_metadata = checkpoint_metadata(S1BuildKind::Ablation);
    ablation_metadata.corpus_val_sha = hash(90);
    let tensors = vec![f32_tensor("toy0.weight", vec![1.0])];

    let error = with_trace_capture(&capture, || {
        compare(
            checkpoint(&phase_a_metadata, hash(10), &tensors),
            checkpoint(&ablation_metadata, hash(11), &tensors),
        )
        .expect_err("metadata mismatch")
    });
    assert!(matches!(
        error,
        AblationError::MetadataMismatch {
            field: "corpus_val_sha",
            ..
        }
    ));

    let ablation_events = ablation_events(&capture);
    assert_eq!(
        ablation_events
            .iter()
            .map(|event| event.name.as_str())
            .collect::<Vec<_>>(),
        vec![
            event::ABLATION_COMPARE_START,
            event::ABLATION_METADATA_CHECK_FAIL,
        ]
    );
    assert_eq!(ablation_events[1].level, "ERROR");
    assert_eq!(
        ablation_events[1].fields.get(field::REASON),
        Some(&json!(
            "ablation metadata mismatch on corpus_val_sha: phase_a=sha256:0202020202020202020202020202020202020202020202020202020202020202, ablation=sha256:5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a"
        ))
    );
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        failure_persistence: None,
        .. ProptestConfig::default()
    })]

    #[test]
    fn byte_equal_tensor_sets_report_equal(values in proptest::collection::vec(any::<u16>(), 1..=64)) {
        let phase_a_metadata = checkpoint_metadata(S1BuildKind::PhaseA);
        let ablation_metadata = checkpoint_metadata(S1BuildKind::Ablation);
        let phase_a_tensors = vec![u16_tensor("toy0.scale", values.clone())];
        let ablation_tensors = phase_a_tensors.clone();

        let report = compare(
            checkpoint(&phase_a_metadata, hash(10), &phase_a_tensors),
            checkpoint(&ablation_metadata, hash(11), &ablation_tensors),
        )
        .expect("ablation report");

        prop_assert!(report.phase_a_eq_ablation);
        prop_assert_eq!(report.first_mismatch, None);
    }

    #[test]
    fn mutated_byte_pairs_report_mutated_offset(
        mut values in proptest::collection::vec(any::<u16>(), 1..=64),
        index in 0usize..64,
    ) {
        let offset = index % values.len();
        let mut mutated = values.clone();
        mutated[offset] = mutated[offset].wrapping_add(1);
        prop_assume!(mutated[offset] != values[offset]);

        let phase_a_metadata = checkpoint_metadata(S1BuildKind::PhaseA);
        let ablation_metadata = checkpoint_metadata(S1BuildKind::Ablation);
        let phase_a_tensors = vec![u16_tensor("toy0.scale", std::mem::take(&mut values))];
        let ablation_tensors = vec![u16_tensor("toy0.scale", mutated)];

        let report = compare(
            checkpoint(&phase_a_metadata, hash(10), &phase_a_tensors),
            checkpoint(&ablation_metadata, hash(11), &ablation_tensors),
        )
        .expect("ablation report");

        prop_assert!(!report.phase_a_eq_ablation);
        prop_assert_eq!(
            report.first_mismatch,
            Some(TensorMismatch {
                tensor: "toy0.scale".to_owned(),
                byte_offset: (offset * 2) as u64,
            })
        );
    }
}

fn checkpoint<'a>(
    metadata: &'a CheckpointMetadata,
    checkpoint_sha: Hash256,
    tensors: &'a [CanonicalTensor],
) -> AblationCheckpoint<'a> {
    AblationCheckpoint {
        metadata,
        checkpoint_sha,
        tensors,
    }
}

fn checkpoint_metadata(build_kind: S1BuildKind) -> CheckpointMetadata {
    CheckpointMetadata {
        schema: "s1_checkpoint.v1".to_owned(),
        seed: 0,
        corpus_train_sha: hash(1),
        corpus_val_sha: hash(2),
        model_config_hash: hash(3),
        train_config_hash: hash(4),
        build_kind,
        build_config_hash: hash(5),
        dependency_lockfile_sha: hash(6),
        rust_toolchain_hash: hash(7),
        device_profile_hash: hash(8),
        rng_stream_def_hash: rng_stream_def_hash(),
        pass_version: SemVer::new(0, 1, 0),
        budget_profile: "integration_fixture".to_owned(),
        final_step: 100,
        final_train_loss: 1.25,
        completion: S1Completion::Completed,
        checkpoint_safetensors_sha256: Hash256::ZERO,
        checkpoint_self_hash: Hash256::ZERO,
    }
    .with_computed_self_hash()
    .expect("checkpoint self hash")
}

fn f32_tensor(name: &str, values: Vec<f32>) -> CanonicalTensor {
    f32_tensor_with_dims(name, values.clone(), &[values.len()])
}

fn f32_tensor_with_dims(name: &str, values: Vec<f32>, dims: &[usize]) -> CanonicalTensor {
    CanonicalTensor::new(
        ArtifactPath::new(name).expect("artifact path"),
        CanonicalTensorKind::DenseWeight,
        CanonicalTensorLayout::new(
            CanonicalTensorShape::from_usize_dims(dims).expect("shape"),
            TensorElementType::Float32,
        ),
        CanonicalTensorPayload::F32(values),
    )
    .expect("canonical tensor")
}

fn i8_tensor(name: &str, values: Vec<i8>) -> CanonicalTensor {
    i8_tensor_with_kind(name, values, CanonicalTensorKind::TernaryWeight)
}

fn i8_tensor_with_kind(name: &str, values: Vec<i8>, kind: CanonicalTensorKind) -> CanonicalTensor {
    CanonicalTensor::new(
        ArtifactPath::new(name).expect("artifact path"),
        kind,
        CanonicalTensorLayout::new(
            CanonicalTensorShape::from_usize_dims(&[values.len()]).expect("shape"),
            TensorElementType::TernaryI2,
        ),
        CanonicalTensorPayload::I8(values),
    )
    .expect("canonical tensor")
}

fn u16_tensor(name: &str, values: Vec<u16>) -> CanonicalTensor {
    CanonicalTensor::new(
        ArtifactPath::new(name).expect("artifact path"),
        CanonicalTensorKind::TernaryScale,
        CanonicalTensorLayout::new(
            CanonicalTensorShape::from_usize_dims(&[values.len()]).expect("shape"),
            TensorElementType::Q8_8,
        ),
        CanonicalTensorPayload::U16(values),
    )
    .expect("canonical tensor")
}

fn u16_tensor_unchecked_layout(name: &str, values: Vec<u16>, dims: &[usize]) -> CanonicalTensor {
    CanonicalTensor {
        id: ArtifactPath::new(name).expect("artifact path"),
        kind: CanonicalTensorKind::TernaryScale,
        layout: CanonicalTensorLayout::new(
            CanonicalTensorShape::from_usize_dims(dims).expect("shape"),
            TensorElementType::Q8_8,
        ),
        payload: CanonicalTensorPayload::U16(values),
        content_hash: hash(0),
    }
}

fn ablation_events(capture: &TraceCapture) -> Vec<common::tracing_capture::TracingEvent> {
    captured_events(capture)
        .into_iter()
        .filter(|event| event.name.starts_with("s1.ablation."))
        .collect()
}

fn hash(byte: u8) -> Hash256 {
    Hash256::from_bytes([byte; 32])
}
