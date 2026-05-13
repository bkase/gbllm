mod common;

use gbf_artifact::ids::ArtifactPath;
use gbf_artifact::tensor::{
    CanonicalTensor, CanonicalTensorKind, CanonicalTensorLayout, CanonicalTensorPayload,
    CanonicalTensorShape, TensorElementType, canonical_tensor_payload_hash,
};
use gbf_experiments::s2::run::FrozenTeacherRunState;
use gbf_experiments::s2::schema::S2BuildKind;
use gbf_foundation::{Hash256, sha256};

#[test]
#[should_panic(expected = "FrozenTeacher detach_for_teacher may fire only once per S2 run")]
fn detach_for_teacher_panics_on_second_call_within_one_run() {
    let tensors = teacher_tensors();
    let checkpoint_bytes = checkpoint_bytes();
    let checkpoint_sha = sha256(&checkpoint_bytes);
    let mut state = FrozenTeacherRunState::default();

    let _ = state.detach_for_teacher(
        0,
        S2BuildKind::s2_ternary_full,
        checkpoint_sha,
        &tensors,
        &checkpoint_bytes,
    );
    let _ = state.detach_for_teacher(
        0,
        S2BuildKind::s2_ternary_full,
        checkpoint_sha,
        &tensors,
        &checkpoint_bytes,
    );
}

#[test]
fn teacher_storage_fingerprint_is_deterministic_across_replays() {
    let tensors = teacher_tensors();
    let checkpoint_bytes = checkpoint_bytes();
    let checkpoint_sha = sha256(&checkpoint_bytes);
    let mut first_state = FrozenTeacherRunState::default();
    let mut second_state = FrozenTeacherRunState::default();

    let first = first_state.detach_for_teacher(
        0,
        S2BuildKind::s2_ternary_full,
        checkpoint_sha,
        &tensors,
        &checkpoint_bytes,
    );
    let second = second_state.detach_for_teacher(
        0,
        S2BuildKind::s2_ternary_full,
        checkpoint_sha,
        &tensors,
        &checkpoint_bytes,
    );

    assert_eq!(
        first.teacher_storage_fingerprint,
        second.teacher_storage_fingerprint
    );
    assert_eq!(
        first.teacher_weight_fingerprint,
        second.teacher_weight_fingerprint
    );
    assert!(!first.requires_grad);
    assert_ne!(first.teacher_storage_fingerprint, Hash256::ZERO);
}

#[test]
fn teacher_weight_fingerprint_is_canonical_tensor_payload_hash_at_step_4000() {
    let tensors = teacher_tensors();
    let checkpoint_bytes = checkpoint_bytes();
    let checkpoint_sha = sha256(&checkpoint_bytes);
    let mut state = FrozenTeacherRunState::default();

    let frozen = state.detach_for_teacher(
        0,
        S2BuildKind::s2_ternary_full,
        checkpoint_sha,
        &tensors,
        &checkpoint_bytes,
    );

    assert_eq!(
        frozen.teacher_weight_fingerprint,
        canonical_tensor_payload_hash(&tensors)
    );
    assert_eq!(frozen.teacher_checkpoint_sha, checkpoint_sha);
    assert!(!frozen.requires_grad);
}

fn teacher_tensors() -> Vec<CanonicalTensor> {
    vec![
        tensor(
            "toy0.block0.weight",
            CanonicalTensorKind::DenseWeight,
            TensorElementType::Float32,
            CanonicalTensorPayload::F32(vec![0.5, -0.25, 0.125, -0.0625]),
            &[2, 2],
        ),
        tensor(
            "toy0.block0.bias",
            CanonicalTensorKind::DenseBias,
            TensorElementType::Float32,
            CanonicalTensorPayload::F32(vec![0.0, 0.25]),
            &[2],
        ),
    ]
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

fn checkpoint_bytes() -> Vec<u8> {
    b"s2-phase-a-step-4000-teacher-checkpoint".to_vec()
}
