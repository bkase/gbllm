//! Target-independent canonical tensor contracts.

use std::error::Error;
use std::fmt;

use gbf_foundation::Hash256;
use serde::{Deserialize, Serialize};

use crate::ids::ArtifactPath;

pub type CanonicalTensorId = ArtifactPath;

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
}
