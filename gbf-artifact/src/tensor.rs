//! Target-independent canonical tensor contracts.

use std::error::Error;
use std::fmt;

use gbf_foundation::Hash256;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::ids::ArtifactPath;

pub type CanonicalTensorId = ArtifactPath;
pub type CanonicalTensorPayloadHash = Hash256;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CanonicalTensor {
    pub id: CanonicalTensorId,
    pub kind: CanonicalTensorKind,
    pub layout: CanonicalTensorLayout,
    pub payload: CanonicalTensorPayload,
    pub content_hash: Hash256,
}

impl CanonicalTensor {
    pub fn new(
        id: CanonicalTensorId,
        kind: CanonicalTensorKind,
        layout: CanonicalTensorLayout,
        payload: CanonicalTensorPayload,
    ) -> Result<Self, CanonicalTensorError> {
        if layout.element_type != payload.element_type() {
            return Err(CanonicalTensorError::ElementTypeMismatch {
                expected: layout.element_type,
                actual: payload.element_type(),
            });
        }

        if layout.shape.element_count() != payload.len() {
            return Err(CanonicalTensorError::PayloadLenMismatch {
                expected: layout.shape.element_count(),
                actual: payload.len(),
            });
        }

        validate_payload_values(&payload)?;
        let content_hash = stable_digest(&canonical_tensor_content_bytes(&layout, &payload));

        Ok(Self {
            id,
            kind,
            layout,
            payload,
            content_hash,
        })
    }
}

/// Computes the RFC §1 canonical payload hash for a caller-supplied set of
/// trainable tensors.
///
/// Tensors are sorted by tensor name before hashing. The SHA-256 stream is
/// framed as tensor count, then for each tensor: tensor name byte length, tensor
/// name UTF-8 bytes, one dtype tag byte, shape rank, each shape dimension as a
/// little-endian `u64`, payload byte length, and the row-major raw payload
/// bytes. SafeTensors/container metadata is intentionally not part of this
/// contract.
#[must_use]
pub fn canonical_tensor_payload_hash(tensors: &[CanonicalTensor]) -> CanonicalTensorPayloadHash {
    tracing::debug!(
        target: "gbf_artifact",
        event = "s1.canonical_tensor.hash.start",
        n_tensors = tensors.len()
    );

    let mut ordered_tensors = tensors.iter().collect::<Vec<_>>();
    ordered_tensors.sort_by(|left, right| {
        left.id
            .as_str()
            .as_bytes()
            .cmp(right.id.as_str().as_bytes())
    });

    let mut hasher = Sha256::new();
    hash_u64(&mut hasher, ordered_tensors.len() as u64);
    for tensor in ordered_tensors {
        let name_bytes = tensor.id.as_str().as_bytes();
        hash_u64(&mut hasher, name_bytes.len() as u64);
        hasher.update(name_bytes);
        hasher.update([element_type_tag(tensor.layout.element_type)]);
        hash_u64(&mut hasher, tensor.layout.shape.dims().len() as u64);
        for &dim in tensor.layout.shape.dims() {
            hash_u64(&mut hasher, u64::from(dim));
        }
        hash_u64(&mut hasher, payload_byte_len(&tensor.payload) as u64);
        hash_payload_bytes(&mut hasher, &tensor.payload);
    }

    let payload_hash = Hash256::from_bytes(hasher.finalize().into());
    tracing::debug!(
        target: "gbf_artifact",
        event = "s1.canonical_tensor.hash.complete",
        %payload_hash
    );
    payload_hash
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum CanonicalTensorKind {
    TernaryWeight,
    TernaryScale,
    Bias,
    RouterWeight,
    RouterBias,
    DenseWeight,
    DenseBias,
    Embedding,
    Classifier,
    NormLut,
    SharedDenseAlpha,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CanonicalTensorLayout {
    pub shape: CanonicalTensorShape,
    pub element_type: TensorElementType,
}

impl CanonicalTensorLayout {
    pub fn new(shape: CanonicalTensorShape, element_type: TensorElementType) -> Self {
        Self {
            shape,
            element_type,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CanonicalTensorShape {
    dims: Vec<u32>,
    element_count: usize,
}

impl CanonicalTensorShape {
    pub fn new(dims: Vec<u32>) -> Result<Self, CanonicalTensorError> {
        if dims.is_empty() {
            return Err(CanonicalTensorError::EmptyShape);
        }

        if let Some(index) = dims.iter().position(|&dim| dim == 0) {
            return Err(CanonicalTensorError::ZeroDim { index });
        }

        let element_count = dims.iter().try_fold(1usize, |product, &dim| {
            product
                .checked_mul(dim as usize)
                .ok_or(CanonicalTensorError::ShapeElementOverflow)
        })?;

        Ok(Self {
            dims,
            element_count,
        })
    }

    pub fn from_usize_dims(dims: &[usize]) -> Result<Self, CanonicalTensorError> {
        let dims = dims
            .iter()
            .copied()
            .map(|dim| u32::try_from(dim).map_err(|_| CanonicalTensorError::DimTooLarge { dim }))
            .collect::<Result<Vec<_>, _>>()?;
        Self::new(dims)
    }

    pub fn dims(&self) -> &[u32] {
        &self.dims
    }

    pub fn element_count(&self) -> usize {
        self.element_count
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum TensorElementType {
    Float32,
    TernaryI2,
    Q8_8,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CanonicalTensorPayload {
    F32(Vec<f32>),
    I8(Vec<i8>),
    U16(Vec<u16>),
}

impl CanonicalTensorPayload {
    pub fn element_type(&self) -> TensorElementType {
        match self {
            Self::F32(_) => TensorElementType::Float32,
            Self::I8(_) => TensorElementType::TernaryI2,
            Self::U16(_) => TensorElementType::Q8_8,
        }
    }

    pub fn len(&self) -> usize {
        match self {
            Self::F32(values) => values.len(),
            Self::I8(values) => values.len(),
            Self::U16(values) => values.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn as_f32_slice(&self) -> Option<&[f32]> {
        match self {
            Self::F32(values) => Some(values),
            Self::I8(_) | Self::U16(_) => None,
        }
    }

    pub fn as_i8_slice(&self) -> Option<&[i8]> {
        match self {
            Self::I8(values) => Some(values),
            Self::F32(_) | Self::U16(_) => None,
        }
    }

    pub fn as_u16_slice(&self) -> Option<&[u16]> {
        match self {
            Self::U16(values) => Some(values),
            Self::F32(_) | Self::I8(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanonicalTensorError {
    EmptyShape,
    ZeroDim {
        index: usize,
    },
    DimTooLarge {
        dim: usize,
    },
    ShapeElementOverflow,
    PayloadLenMismatch {
        expected: usize,
        actual: usize,
    },
    ElementTypeMismatch {
        expected: TensorElementType,
        actual: TensorElementType,
    },
    NonFiniteFloat {
        index: usize,
    },
    InvalidTernaryValue {
        index: usize,
        value: i8,
    },
}

impl fmt::Display for CanonicalTensorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyShape => f.write_str("canonical tensor shape must not be empty"),
            Self::ZeroDim { index } => {
                write!(f, "canonical tensor dimension {index} must be nonzero")
            }
            Self::DimTooLarge { dim } => write!(f, "canonical tensor dimension {dim} exceeds u32"),
            Self::ShapeElementOverflow => {
                f.write_str("canonical tensor shape overflows addressable element count")
            }
            Self::PayloadLenMismatch { expected, actual } => write!(
                f,
                "canonical tensor payload length mismatch: expected {expected}, got {actual}"
            ),
            Self::ElementTypeMismatch { expected, actual } => write!(
                f,
                "canonical tensor payload type mismatch: expected {expected:?}, got {actual:?}"
            ),
            Self::NonFiniteFloat { index } => {
                write!(
                    f,
                    "canonical tensor float payload at index {index} is not finite"
                )
            }
            Self::InvalidTernaryValue { index, value } => {
                write!(
                    f,
                    "canonical ternary payload at index {index} must be -1, 0, or 1, got {value}"
                )
            }
        }
    }
}

impl Error for CanonicalTensorError {}

pub(crate) fn stable_digest(bytes: &[u8]) -> Hash256 {
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

    let mut lanes = [
        FNV_OFFSET,
        FNV_OFFSET ^ 0x9e37_79b9_7f4a_7c15,
        FNV_OFFSET ^ 0xc2b2_ae3d_27d4_eb4f,
        FNV_OFFSET ^ 0x1656_67b1_9e37_79f9,
    ];

    for &byte in bytes {
        for (lane_index, lane) in lanes.iter_mut().enumerate() {
            *lane ^= u64::from(byte).wrapping_add((lane_index as u64) << 8);
            *lane = lane.wrapping_mul(FNV_PRIME.wrapping_add(lane_index as u64));
            *lane ^= *lane >> 32;
        }
    }

    let mut digest = [0_u8; 32];
    for (index, lane) in lanes.into_iter().enumerate() {
        digest[index * 8..(index + 1) * 8].copy_from_slice(&lane.to_le_bytes());
    }

    Hash256::from_bytes(digest)
}

fn validate_payload_values(payload: &CanonicalTensorPayload) -> Result<(), CanonicalTensorError> {
    if let CanonicalTensorPayload::F32(values) = payload
        && let Some(index) = values.iter().position(|value| !value.is_finite())
    {
        return Err(CanonicalTensorError::NonFiniteFloat { index });
    }

    if let CanonicalTensorPayload::I8(values) = payload
        && let Some((index, &value)) = values
            .iter()
            .enumerate()
            .find(|(_, value)| !matches!(**value, -1..=1))
    {
        return Err(CanonicalTensorError::InvalidTernaryValue { index, value });
    }

    Ok(())
}

fn canonical_tensor_content_bytes(
    layout: &CanonicalTensorLayout,
    payload: &CanonicalTensorPayload,
) -> Vec<u8> {
    let mut bytes = Vec::new();
    push_u8(&mut bytes, element_type_tag(layout.element_type));
    push_u64(&mut bytes, layout.shape.dims().len() as u64);
    for &dim in layout.shape.dims() {
        push_u32(&mut bytes, dim);
    }

    push_u64(&mut bytes, payload.len() as u64);
    match payload {
        CanonicalTensorPayload::F32(values) => {
            for value in values {
                bytes.extend_from_slice(&value.to_bits().to_le_bytes());
            }
        }
        CanonicalTensorPayload::I8(values) => {
            for value in values {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        CanonicalTensorPayload::U16(values) => {
            for value in values {
                push_u16(&mut bytes, *value);
            }
        }
    }

    bytes
}

fn hash_payload_bytes(hasher: &mut Sha256, payload: &CanonicalTensorPayload) {
    match payload {
        CanonicalTensorPayload::F32(values) => {
            for value in values {
                hasher.update(value.to_bits().to_le_bytes());
            }
        }
        CanonicalTensorPayload::I8(values) => {
            for value in values {
                hasher.update(value.to_le_bytes());
            }
        }
        CanonicalTensorPayload::U16(values) => {
            for value in values {
                hasher.update(value.to_le_bytes());
            }
        }
    }
}

fn hash_u64(hasher: &mut Sha256, value: u64) {
    hasher.update(value.to_le_bytes());
}

fn payload_byte_len(payload: &CanonicalTensorPayload) -> usize {
    match payload {
        CanonicalTensorPayload::F32(values) => values.len() * size_of::<f32>(),
        CanonicalTensorPayload::I8(values) => values.len() * size_of::<i8>(),
        CanonicalTensorPayload::U16(values) => values.len() * size_of::<u16>(),
    }
}

fn element_type_tag(element_type: TensorElementType) -> u8 {
    match element_type {
        TensorElementType::Float32 => 0,
        TensorElementType::TernaryI2 => 1,
        TensorElementType::Q8_8 => 2,
    }
}

fn push_u8(bytes: &mut Vec<u8>, value: u8) {
    bytes.push(value);
}

fn push_u16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_u64(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::str::FromStr;

    #[test]
    fn canonical_tensor_validates_layout_and_payload() {
        let tensor = CanonicalTensor::new(
            CanonicalTensorId::new("layer.0.weight").unwrap(),
            CanonicalTensorKind::TernaryWeight,
            CanonicalTensorLayout::new(
                CanonicalTensorShape::from_usize_dims(&[1, 3]).unwrap(),
                TensorElementType::TernaryI2,
            ),
            CanonicalTensorPayload::I8(vec![-1, 0, 1]),
        )
        .unwrap();

        assert_eq!(tensor.layout.shape.dims(), &[1, 3]);
        assert_eq!(tensor.payload.as_i8_slice(), Some(&[-1, 0, 1][..]));
        assert_ne!(tensor.content_hash, Hash256::ZERO);
    }

    #[test]
    fn canonical_tensor_rejects_mismatched_payloads() {
        let err = CanonicalTensor::new(
            CanonicalTensorId::new("layer.0.weight").unwrap(),
            CanonicalTensorKind::TernaryWeight,
            CanonicalTensorLayout::new(
                CanonicalTensorShape::from_usize_dims(&[1, 2]).unwrap(),
                TensorElementType::TernaryI2,
            ),
            CanonicalTensorPayload::I8(vec![1]),
        )
        .unwrap_err();

        assert_eq!(
            err,
            CanonicalTensorError::PayloadLenMismatch {
                expected: 2,
                actual: 1
            }
        );
    }

    #[test]
    fn canonical_tensor_rejects_invalid_ternary_payloads() {
        let err = CanonicalTensor::new(
            CanonicalTensorId::new("layer.0.weight").unwrap(),
            CanonicalTensorKind::TernaryWeight,
            CanonicalTensorLayout::new(
                CanonicalTensorShape::from_usize_dims(&[1, 1]).unwrap(),
                TensorElementType::TernaryI2,
            ),
            CanonicalTensorPayload::I8(vec![2]),
        )
        .unwrap_err();

        assert_eq!(
            err,
            CanonicalTensorError::InvalidTernaryValue { index: 0, value: 2 }
        );
    }

    #[test]
    fn canonical_tensor_rejects_non_finite_float_payloads_for_all_float_kinds() {
        let kinds = [
            CanonicalTensorKind::Bias,
            CanonicalTensorKind::RouterWeight,
            CanonicalTensorKind::RouterBias,
            CanonicalTensorKind::DenseWeight,
            CanonicalTensorKind::DenseBias,
            CanonicalTensorKind::Embedding,
            CanonicalTensorKind::Classifier,
            CanonicalTensorKind::NormLut,
            CanonicalTensorKind::SharedDenseAlpha,
        ];

        for (kind_index, kind) in kinds.into_iter().enumerate() {
            for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
                let err = CanonicalTensor::new(
                    CanonicalTensorId::new(format!("tensor.{kind_index}")).unwrap(),
                    kind,
                    CanonicalTensorLayout::new(
                        CanonicalTensorShape::from_usize_dims(&[3]).unwrap(),
                        TensorElementType::Float32,
                    ),
                    CanonicalTensorPayload::F32(vec![1.0, value, 2.0]),
                )
                .unwrap_err();

                assert_eq!(err, CanonicalTensorError::NonFiniteFloat { index: 1 });
            }
        }
    }

    #[test]
    fn canonical_tensor_payload_hash_pins_dtype_tags_and_empty_hash() {
        assert_eq!(element_type_tag(TensorElementType::Float32), 0);
        assert_eq!(element_type_tag(TensorElementType::TernaryI2), 1);
        assert_eq!(element_type_tag(TensorElementType::Q8_8), 2);
        assert_eq!(
            canonical_tensor_payload_hash(&[]),
            Hash256::from_str(
                "sha256:af5570f5a1810b7af78caf4bc70a660f0df51e42baf91d4de5b2328de0e83dfc",
            )
            .unwrap()
        );
    }

    #[test]
    fn canonical_tensor_payload_hash_is_invariant_to_input_order() {
        let tensors = mixed_fixture_tensors();
        let reordered = vec![tensors[2].clone(), tensors[0].clone(), tensors[1].clone()];

        assert_eq!(
            canonical_tensor_payload_hash(&tensors),
            canonical_tensor_payload_hash(&reordered)
        );
    }

    #[test]
    fn canonical_tensor_payload_hash_excludes_container_metadata() {
        struct SafetensorsLikeContainer {
            metadata: Vec<(&'static str, &'static str)>,
            tensors: Vec<CanonicalTensor>,
        }

        let tensors = mixed_fixture_tensors();
        let first = SafetensorsLikeContainer {
            metadata: vec![("version", "trainer-a"), ("created_by", "phase-a")],
            tensors: tensors.clone(),
        };
        let second = SafetensorsLikeContainer {
            metadata: vec![("created_by", "ablation"), ("version", "trainer-b")],
            tensors,
        };

        assert_ne!(first.metadata, second.metadata);
        assert_eq!(
            canonical_tensor_payload_hash(&first.tensors),
            canonical_tensor_payload_hash(&second.tensors)
        );
    }

    #[test]
    fn canonical_tensor_payload_hash_changes_for_payload_shape_dtype_and_name() {
        let base = vec![float_tensor("block.0.linear", &[2], vec![1.0, -2.0])];

        let mut payload_changed = base.clone();
        payload_changed[0].payload = CanonicalTensorPayload::F32(vec![1.0, -2.000_000_2]);

        let shape_changed = vec![float_tensor("block.0.linear", &[2, 1], vec![1.0, -2.0])];
        let dtype_changed = vec![q8_8_tensor("block.0.linear", &[2], vec![256, 512])];
        let name_changed = vec![float_tensor("block.0.Linear", &[2], vec![1.0, -2.0])];

        let base_hash = canonical_tensor_payload_hash(&base);
        assert_ne!(base_hash, canonical_tensor_payload_hash(&payload_changed));
        assert_ne!(base_hash, canonical_tensor_payload_hash(&shape_changed));
        assert_ne!(base_hash, canonical_tensor_payload_hash(&dtype_changed));
        assert_ne!(base_hash, canonical_tensor_payload_hash(&name_changed));
    }

    #[test]
    fn canonical_tensor_payload_hash_frames_tensor_count_and_payload_length() {
        let first = ternary_tensor("a", &[1], vec![1]);
        let second = ternary_tensor("b", &[1], vec![-1]);
        let two_tensors = vec![first.clone(), second.clone()];
        let first_stream = legacy_unframed_tensor_stream(&[first.clone()]);
        let prefix_len = first_stream.len() - payload_byte_len(&first.payload);

        let mut absorbed_second_tensor = first;
        absorbed_second_tensor.payload = CanonicalTensorPayload::I8(
            legacy_unframed_tensor_stream(&two_tensors)[prefix_len..]
                .iter()
                .map(|byte| i8::from_ne_bytes([*byte]))
                .collect(),
        );

        assert_eq!(
            legacy_unframed_tensor_stream(&[absorbed_second_tensor.clone()]),
            legacy_unframed_tensor_stream(&two_tensors)
        );
        assert_ne!(
            canonical_tensor_payload_hash(&[absorbed_second_tensor]),
            canonical_tensor_payload_hash(&two_tensors)
        );
    }

    #[test]
    fn canonical_tensor_payload_hash_frames_shape_rank_before_payload() {
        let mut rank_one = ternary_tensor("rank.probe", &[1], vec![1]);
        rank_one.payload = CanonicalTensorPayload::I8(vec![2, 0, 0, 0, 0, 0, 0, 0, -1]);

        let mut rank_two = ternary_tensor("rank.probe", &[1, 1], vec![1]);
        rank_two.layout = CanonicalTensorLayout::new(
            CanonicalTensorShape::from_usize_dims(&[1, 2]).unwrap(),
            TensorElementType::TernaryI2,
        );
        rank_two.payload = CanonicalTensorPayload::I8(vec![-1]);

        assert_eq!(
            legacy_unframed_tensor_stream(&[rank_one.clone()]),
            legacy_unframed_tensor_stream(&[rank_two.clone()])
        );
        assert_ne!(
            canonical_tensor_payload_hash(&[rank_one]),
            canonical_tensor_payload_hash(&[rank_two])
        );
    }

    #[test]
    fn canonical_tensor_payload_hash_frames_tensor_name_lengths() {
        let split_after_first_byte = vec![
            ternary_tensor("a", &[1], vec![1]),
            ternary_tensor("bc", &[1], vec![-1]),
        ];
        let split_after_second_byte = vec![
            ternary_tensor("ab", &[1], vec![1]),
            ternary_tensor("c", &[1], vec![-1]),
        ];

        assert_eq!(
            legacy_unframed_tensor_name_bytes(&split_after_first_byte),
            legacy_unframed_tensor_name_bytes(&split_after_second_byte)
        );
        assert_ne!(
            canonical_tensor_name_frame_bytes(&split_after_first_byte),
            canonical_tensor_name_frame_bytes(&split_after_second_byte)
        );
        assert_ne!(
            canonical_tensor_payload_hash(&split_after_first_byte),
            canonical_tensor_payload_hash(&split_after_second_byte)
        );
    }

    #[test]
    fn canonical_tensor_payload_hash_golden_values_pin_stream_encoding() {
        let small = vec![ternary_tensor("a.weight", &[3], vec![-1, 0, 1])];
        let medium = vec![
            float_tensor("layer.1.bias", &[2], vec![1.5, -0.0]),
            q8_8_tensor("layer.0.scale", &[1, 2], vec![256, 511]),
        ];
        let mixed = mixed_fixture_tensors();

        assert_eq!(
            canonical_tensor_payload_hash(&small).to_string(),
            "sha256:c7c398255b7133e04549485a58247c0433d8cfc0a5dc3b14f0352c49a0db854e"
        );
        assert_eq!(
            canonical_tensor_payload_hash(&medium).to_string(),
            "sha256:af22e4f70ccb7f2ebf30dc9afe8afff7a13f502e6d266f96a7a44794f96e9b18"
        );
        assert_eq!(
            canonical_tensor_payload_hash(&mixed).to_string(),
            "sha256:bb3a684b965ef7d9d55385162576139a6eed76966723c0e25ab64ac27fea519c"
        );
    }

    proptest! {
        #[test]
        fn canonical_tensor_payload_hash_is_invariant_under_permutation(seed in any::<u8>()) {
            let tensors = mixed_fixture_tensors();
            let mut permuted = tensors.clone();
            let rotation = usize::from(seed) % permuted.len();
            permuted.rotate_left(rotation);

            prop_assert_eq!(
                canonical_tensor_payload_hash(&tensors),
                canonical_tensor_payload_hash(&permuted)
            );
        }

        #[test]
        fn canonical_tensor_payload_hash_changes_when_payload_byte_changes(byte in any::<u8>()) {
            let base = vec![q8_8_tensor("layer.0.scale", &[2], vec![256, 512])];
            let changed_value = u16::from(byte) + 1;
            let changed = vec![q8_8_tensor("layer.0.scale", &[2], vec![256, changed_value])];

            prop_assert_ne!(
                canonical_tensor_payload_hash(&base),
                canonical_tensor_payload_hash(&changed)
            );
        }
    }

    // Witness helper for adversarial tests of the post-amendment framing rule.
    // This is deliberately not a production encoding.
    fn legacy_unframed_tensor_stream(tensors: &[CanonicalTensor]) -> Vec<u8> {
        let mut ordered_tensors = tensors.iter().collect::<Vec<_>>();
        ordered_tensors.sort_by(|left, right| {
            left.id
                .as_str()
                .as_bytes()
                .cmp(right.id.as_str().as_bytes())
        });

        let mut bytes = Vec::new();
        for tensor in ordered_tensors {
            bytes.extend_from_slice(tensor.id.as_str().as_bytes());
            bytes.push(element_type_tag(tensor.layout.element_type));
            for &dim in tensor.layout.shape.dims() {
                bytes.extend_from_slice(&u64::from(dim).to_le_bytes());
            }
            append_payload_bytes(&tensor.payload, &mut bytes);
        }
        bytes
    }

    fn legacy_unframed_tensor_name_bytes(tensors: &[CanonicalTensor]) -> Vec<u8> {
        let mut ordered_tensors = tensors.iter().collect::<Vec<_>>();
        ordered_tensors.sort_by(|left, right| {
            left.id
                .as_str()
                .as_bytes()
                .cmp(right.id.as_str().as_bytes())
        });

        let mut bytes = Vec::new();
        for tensor in ordered_tensors {
            bytes.extend_from_slice(tensor.id.as_str().as_bytes());
        }
        bytes
    }

    fn canonical_tensor_name_frame_bytes(tensors: &[CanonicalTensor]) -> Vec<u8> {
        let mut ordered_tensors = tensors.iter().collect::<Vec<_>>();
        ordered_tensors.sort_by(|left, right| {
            left.id
                .as_str()
                .as_bytes()
                .cmp(right.id.as_str().as_bytes())
        });

        let mut bytes = Vec::new();
        for tensor in ordered_tensors {
            let name_bytes = tensor.id.as_str().as_bytes();
            bytes.extend_from_slice(&(name_bytes.len() as u64).to_le_bytes());
            bytes.extend_from_slice(name_bytes);
        }
        bytes
    }

    fn append_payload_bytes(payload: &CanonicalTensorPayload, bytes: &mut Vec<u8>) {
        match payload {
            CanonicalTensorPayload::F32(values) => {
                for value in values {
                    bytes.extend_from_slice(&value.to_bits().to_le_bytes());
                }
            }
            CanonicalTensorPayload::I8(values) => {
                for value in values {
                    bytes.extend_from_slice(&value.to_le_bytes());
                }
            }
            CanonicalTensorPayload::U16(values) => {
                for value in values {
                    bytes.extend_from_slice(&value.to_le_bytes());
                }
            }
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
            CanonicalTensorId::new(id).unwrap(),
            kind,
            CanonicalTensorLayout::new(
                CanonicalTensorShape::from_usize_dims(dims).unwrap(),
                element_type,
            ),
            payload,
        )
        .unwrap()
    }
}
