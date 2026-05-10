mod common;

use std::fs;

use gbf_artifact::ids::ArtifactPath;
use gbf_artifact::tensor::{
    CanonicalTensor, CanonicalTensorKind, CanonicalTensorLayout, CanonicalTensorPayload,
    CanonicalTensorShape, TensorElementType, canonical_tensor_payload_hash,
};
use gbf_experiments::s1::run::{
    CheckpointMetadata, CheckpointWriteError, canonical_checkpoint_bytes,
    canonical_checkpoint_write,
};
use proptest::prelude::*;
use safetensors::{Dtype, SafeTensors};
use serde_json::json;

use common::tracing_capture::{TraceCapture, captured_events, with_trace_capture};

#[test]
fn canonical_checkpoint_write_is_byte_identical_for_same_inputs() {
    let tensors = mixed_fixture_tensors();
    let metadata = CheckpointMetadata::default();
    let tempdir = tempfile::tempdir().unwrap();
    let first_path = tempdir.path().join("first.safetensors");
    let second_path = tempdir.path().join("second.safetensors");
    let capture = TraceCapture::default();

    with_trace_capture(&capture, || {
        canonical_checkpoint_write(&first_path, &tensors, &metadata).unwrap();
        canonical_checkpoint_write(&second_path, &tensors, &metadata).unwrap();
    });

    assert_eq!(
        fs::read(first_path).unwrap(),
        fs::read(second_path).unwrap()
    );
}

#[test]
fn canonical_checkpoint_write_is_byte_identical_for_reordered_inputs() {
    let tensors = mixed_fixture_tensors();
    let reordered = vec![tensors[2].clone(), tensors[0].clone(), tensors[1].clone()];
    let metadata = CheckpointMetadata::default();

    assert_eq!(
        canonical_checkpoint_bytes(&tensors, &metadata).unwrap(),
        canonical_checkpoint_bytes(&reordered, &metadata).unwrap()
    );
}

#[test]
fn canonical_checkpoint_write_changes_when_payload_byte_changes() {
    let tensors = mixed_fixture_tensors();
    let mut changed = tensors.clone();
    changed[1].payload = CanonicalTensorPayload::I8(vec![-1, 0, 1, 0]);
    let metadata = CheckpointMetadata::default();

    assert_ne!(
        canonical_checkpoint_bytes(&tensors, &metadata).unwrap(),
        canonical_checkpoint_bytes(&changed, &metadata).unwrap()
    );
}

#[test]
fn canonical_checkpoint_write_serializes_tensors_in_name_order() {
    let tensors = mixed_fixture_tensors();
    let reordered = vec![tensors[2].clone(), tensors[0].clone(), tensors[1].clone()];
    let bytes = canonical_checkpoint_bytes(&reordered, &CheckpointMetadata::default()).unwrap();
    let (_, metadata) = SafeTensors::read_metadata(&bytes).unwrap();

    assert_eq!(
        metadata.offset_keys(),
        vec![
            "layer.0.scale".to_owned(),
            "layer.0.weight".to_owned(),
            "layer.1.bias".to_owned(),
        ]
    );
}

#[test]
fn canonical_checkpoint_write_rejects_duplicate_tensor_names() {
    let tensors = vec![
        float_tensor("layer.0.weight", &[2], vec![1.0, 2.0]),
        ternary_tensor("layer.0.weight", &[2], vec![-1, 1]),
    ];
    let tempdir = tempfile::tempdir().unwrap();
    let path = tempdir.path().join("duplicate.safetensors");

    let error = canonical_checkpoint_write(&path, &tensors, &CheckpointMetadata::default())
        .expect_err("duplicate tensor names should be rejected before writing checkpoint bytes");

    assert!(matches!(
        error,
        CheckpointWriteError::DuplicateTensorName { ref name }
            if name == "layer.0.weight"
    ));
    assert!(!path.exists());
}

#[test]
fn canonical_checkpoint_write_round_trips_via_safetensors_reader() {
    let tensors = mixed_fixture_tensors();
    let bytes = canonical_checkpoint_bytes(&tensors, &CheckpointMetadata::default()).unwrap();
    let safetensors = SafeTensors::deserialize(&bytes).unwrap();

    assert_safetensors_round_trip_identity(&tensors, &safetensors);
}

#[test]
fn canonical_checkpoint_write_pins_metadata_free_header_bytes() {
    let bytes =
        canonical_checkpoint_bytes(&mixed_fixture_tensors(), &CheckpointMetadata::default())
            .unwrap();
    let header_len = u64::from_le_bytes(bytes[0..8].try_into().unwrap()) as usize;
    let header_bytes = &bytes[..8 + header_len];
    let (_, metadata) = SafeTensors::read_metadata(&bytes).unwrap();

    assert!(metadata.metadata().is_none());
    insta::assert_snapshot!(hex_bytes(header_bytes), @r###"
c8 00 00 00 00 00 00 00 7b 22 6c 61 79 65 72 2e
30 2e 73 63 61 6c 65 22 3a 7b 22 64 74 79 70 65
22 3a 22 55 31 36 22 2c 22 73 68 61 70 65 22 3a
5b 32 5d 2c 22 64 61 74 61 5f 6f 66 66 73 65 74
73 22 3a 5b 30 2c 34 5d 7d 2c 22 6c 61 79 65 72
2e 30 2e 77 65 69 67 68 74 22 3a 7b 22 64 74 79
70 65 22 3a 22 49 38 22 2c 22 73 68 61 70 65 22
3a 5b 32 2c 32 5d 2c 22 64 61 74 61 5f 6f 66 66
73 65 74 73 22 3a 5b 34 2c 38 5d 7d 2c 22 6c 61
79 65 72 2e 31 2e 62 69 61 73 22 3a 7b 22 64 74
79 70 65 22 3a 22 46 33 32 22 2c 22 73 68 61 70
65 22 3a 5b 32 5d 2c 22 64 61 74 61 5f 6f 66 66
73 65 74 73 22 3a 5b 38 2c 31 36 5d 7d 7d 20 20
"###);
}

#[test]
fn canonical_checkpoint_write_supports_empty_tensor_set() {
    let bytes = canonical_checkpoint_bytes(&[], &CheckpointMetadata::default()).unwrap();
    let safetensors = SafeTensors::deserialize(&bytes).unwrap();

    assert_eq!(
        &bytes,
        &[
            8, 0, 0, 0, 0, 0, 0, 0, b'{', b'}', b' ', b' ', b' ', b' ', b' ', b' '
        ]
    );
    assert!(safetensors.is_empty());
}

#[test]
fn canonical_checkpoint_write_logs_tensor_payload_hash_not_checkpoint_self_hash() {
    let tensors = mixed_fixture_tensors();
    let tempdir = tempfile::tempdir().unwrap();
    let path = tempdir.path().join("checkpoint.safetensors");
    let capture = TraceCapture::default();

    with_trace_capture(&capture, || {
        canonical_checkpoint_write(&path, &tensors, &CheckpointMetadata::default()).unwrap();
    });

    let events = captured_events(&capture);
    let event = events
        .iter()
        .find(|event| {
            event.fields.get("event") == Some(&json!("s1.checkpoint_writer.write.complete"))
        })
        .unwrap_or_else(|| panic!("checkpoint writer completion event in {events:?}"));

    assert_eq!(
        event.fields.get("tensor_payload_hash"),
        Some(&json!(canonical_tensor_payload_hash(&tensors).to_string()))
    );
    assert!(!event.fields.contains_key("checkpoint_self_hash"));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn canonical_checkpoint_write_property_round_trips_generated_tensor_sets(
        tensors in arb_canonical_tensor_set(),
    ) {
        let bytes = canonical_checkpoint_bytes(&tensors, &CheckpointMetadata::default()).unwrap();
        let safetensors = SafeTensors::deserialize(&bytes).unwrap();

        assert_safetensors_round_trip_identity(&tensors, &safetensors);
    }
}

fn assert_safetensors_round_trip_identity(
    tensors: &[CanonicalTensor],
    safetensors: &SafeTensors<'_>,
) {
    let mut ordered_tensors = tensors.iter().collect::<Vec<_>>();
    ordered_tensors.sort_by(|left, right| left.id.as_str().cmp(right.id.as_str()));

    assert_eq!(safetensors.len(), ordered_tensors.len());

    for tensor in ordered_tensors {
        let view = safetensors.tensor(tensor.id.as_str()).unwrap();
        let expected_shape = tensor
            .layout
            .shape
            .dims()
            .iter()
            .map(|&dim| dim as usize)
            .collect::<Vec<_>>();
        assert_eq!(view.dtype(), safetensors_dtype(tensor.layout.element_type));
        assert_eq!(view.shape(), expected_shape.as_slice());
        assert_eq!(view.data(), tensor_payload_bytes(tensor));
    }
}

fn mixed_fixture_tensors() -> Vec<CanonicalTensor> {
    vec![
        float_tensor("layer.1.bias", &[2], vec![1.5, -0.0]),
        ternary_tensor("layer.0.weight", &[2, 2], vec![-1, 0, 1, -1]),
        q8_8_tensor("layer.0.scale", &[2], vec![256, 512]),
    ]
}

fn float_tensor(id: &str, dims: &[usize], values: Vec<f32>) -> CanonicalTensor {
    tensor(
        id,
        CanonicalTensorKind::DenseWeight,
        TensorElementType::Float32,
        CanonicalTensorPayload::F32(values),
        dims,
    )
}

fn ternary_tensor(id: &str, dims: &[usize], values: Vec<i8>) -> CanonicalTensor {
    tensor(
        id,
        CanonicalTensorKind::TernaryWeight,
        TensorElementType::TernaryI2,
        CanonicalTensorPayload::I8(values),
        dims,
    )
}

fn q8_8_tensor(id: &str, dims: &[usize], values: Vec<u16>) -> CanonicalTensor {
    tensor(
        id,
        CanonicalTensorKind::TernaryScale,
        TensorElementType::Q8_8,
        CanonicalTensorPayload::U16(values),
        dims,
    )
}

fn tensor(
    id: &str,
    kind: CanonicalTensorKind,
    element_type: TensorElementType,
    payload: CanonicalTensorPayload,
    dims: &[usize],
) -> CanonicalTensor {
    CanonicalTensor::new(
        ArtifactPath::new(id).unwrap(),
        kind,
        CanonicalTensorLayout::new(
            CanonicalTensorShape::from_usize_dims(dims).unwrap(),
            element_type,
        ),
        payload,
    )
    .unwrap()
}

fn arb_canonical_tensor_set() -> impl Strategy<Value = Vec<CanonicalTensor>> {
    prop::collection::btree_map(arb_tensor_name(), arb_tensor_body(), 0..=8).prop_map(|entries| {
        entries
            .into_iter()
            .map(|(name, body)| body.into_tensor(&name))
            .collect()
    })
}

fn arb_tensor_name() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9_]{0,8}(\\.[a-z][a-z0-9_]{0,8}){0,2}"
}

fn arb_tensor_body() -> impl Strategy<Value = TensorBody> {
    prop_oneof![
        arb_shape()
            .prop_flat_map(|shape| {
                let len = element_count(&shape);
                (Just(shape), prop::collection::vec(finite_f32(), len..=len))
            })
            .prop_map(|(shape, values)| TensorBody {
                kind: CanonicalTensorKind::DenseWeight,
                element_type: TensorElementType::Float32,
                payload: CanonicalTensorPayload::F32(values),
                shape,
            }),
        arb_shape()
            .prop_flat_map(|shape| {
                let len = element_count(&shape);
                (
                    Just(shape),
                    prop::collection::vec(
                        prop_oneof![Just(-1_i8), Just(0_i8), Just(1_i8)],
                        len..=len,
                    ),
                )
            })
            .prop_map(|(shape, values)| TensorBody {
                kind: CanonicalTensorKind::TernaryWeight,
                element_type: TensorElementType::TernaryI2,
                payload: CanonicalTensorPayload::I8(values),
                shape,
            }),
        arb_shape()
            .prop_flat_map(|shape| {
                let len = element_count(&shape);
                (Just(shape), prop::collection::vec(any::<u16>(), len..=len))
            })
            .prop_map(|(shape, values)| TensorBody {
                kind: CanonicalTensorKind::TernaryScale,
                element_type: TensorElementType::Q8_8,
                payload: CanonicalTensorPayload::U16(values),
                shape,
            }),
    ]
}

fn arb_shape() -> impl Strategy<Value = Vec<usize>> {
    prop_oneof![
        (1_usize..=4).prop_map(|a| vec![a]),
        (1_usize..=4, 1_usize..=4).prop_map(|(a, b)| vec![a, b]),
        (1_usize..=2, 1_usize..=3, 1_usize..=3).prop_map(|(a, b, c)| vec![a, b, c]),
    ]
}

fn finite_f32() -> impl Strategy<Value = f32> {
    any::<i16>().prop_map(|value| f32::from(value) / 16.0)
}

#[derive(Debug, Clone)]
struct TensorBody {
    kind: CanonicalTensorKind,
    element_type: TensorElementType,
    payload: CanonicalTensorPayload,
    shape: Vec<usize>,
}

impl TensorBody {
    fn into_tensor(self, name: &str) -> CanonicalTensor {
        tensor(
            name,
            self.kind,
            self.element_type,
            self.payload,
            &self.shape,
        )
    }
}

fn element_count(shape: &[usize]) -> usize {
    shape.iter().product()
}

fn safetensors_dtype(element_type: TensorElementType) -> Dtype {
    match element_type {
        TensorElementType::Float32 => Dtype::F32,
        TensorElementType::TernaryI2 => Dtype::I8,
        TensorElementType::Q8_8 => Dtype::U16,
    }
}

fn tensor_payload_bytes(tensor: &CanonicalTensor) -> Vec<u8> {
    match &tensor.payload {
        CanonicalTensorPayload::F32(values) => values
            .iter()
            .flat_map(|value| value.to_bits().to_le_bytes())
            .collect(),
        CanonicalTensorPayload::I8(values) => values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect(),
        CanonicalTensorPayload::U16(values) => values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect(),
    }
}

fn hex_bytes(bytes: &[u8]) -> String {
    bytes
        .chunks(16)
        .map(|chunk| {
            chunk
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>()
        .join("\n")
}
