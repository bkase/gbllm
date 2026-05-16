//! F-S3 canonical tensor schema records.

use std::error::Error;
use std::fmt;

use gbf_foundation::{Hash256, sha256};
use serde::{Deserialize, Serialize};

pub use crate::tensor::CanonicalTensorId;

/// Canonical tensor dtype for S3 artifact payload records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Dtype {
    Fp32,
    Ternary2,
    Q8_8,
    I32,
}

impl Dtype {
    #[must_use]
    pub const fn bits_per_element(self) -> u64 {
        match self {
            Self::Fp32 => 32,
            Self::Ternary2 => 2,
            Self::Q8_8 => 16,
            Self::I32 => 32,
        }
    }
}

/// Payload role used by B5 artifact schema checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PayloadRole {
    DeployableWeight,
    DeployableQuantParam,
    ReferenceFp32,
}

/// Canonical tensor metadata record stored by `ArtifactCore`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalTensor {
    pub id: CanonicalTensorId,
    pub dtype: Dtype,
    pub shape: Vec<u32>,
    pub payload_sha: Hash256,
    pub payload_role: PayloadRole,
}

impl CanonicalTensor {
    pub fn new(
        id: CanonicalTensorId,
        dtype: Dtype,
        shape: Vec<u32>,
        payload_sha: Hash256,
        payload_role: PayloadRole,
    ) -> Result<Self, CanonicalTensorSchemaError> {
        validate_shape(&shape)?;
        Ok(Self {
            id,
            dtype,
            shape,
            payload_sha,
            payload_role,
        })
    }

    /// Exact byte length for this tensor's canonical row-major payload.
    pub fn byte_length(&self) -> Result<u64, CanonicalTensorSchemaError> {
        byte_length(self.dtype, &self.shape)
    }
}

/// Hash canonical tensor payload bytes with the project SHA-256 wrapper.
#[must_use]
pub fn canonical_payload_sha(payload: impl AsRef<[u8]>) -> Hash256 {
    sha256(payload)
}

/// Compute the exact byte length for a dtype and shape.
pub fn byte_length(dtype: Dtype, shape: &[u32]) -> Result<u64, CanonicalTensorSchemaError> {
    validate_shape(shape)?;
    let elements = shape.iter().try_fold(1_u128, |product, dim| {
        product
            .checked_mul(u128::from(*dim))
            .ok_or(CanonicalTensorSchemaError::ShapeElementOverflow)
    })?;
    let bits = elements
        .checked_mul(u128::from(dtype.bits_per_element()))
        .ok_or(CanonicalTensorSchemaError::ByteLengthOverflow)?;
    let bytes = bits.div_ceil(8);
    u64::try_from(bytes).map_err(|_| CanonicalTensorSchemaError::ByteLengthOverflow)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanonicalTensorSchemaError {
    EmptyShape,
    ZeroDim { index: usize },
    ShapeElementOverflow,
    ByteLengthOverflow,
}

impl fmt::Display for CanonicalTensorSchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyShape => f.write_str("canonical tensor shape must not be empty"),
            Self::ZeroDim { index } => {
                write!(f, "canonical tensor dimension {index} must be nonzero")
            }
            Self::ShapeElementOverflow => {
                f.write_str("canonical tensor shape overflows element count")
            }
            Self::ByteLengthOverflow => f.write_str("canonical tensor byte length overflows u64"),
        }
    }
}

impl Error for CanonicalTensorSchemaError {}

fn validate_shape(shape: &[u32]) -> Result<(), CanonicalTensorSchemaError> {
    if shape.is_empty() {
        return Err(CanonicalTensorSchemaError::EmptyShape);
    }
    if let Some(index) = shape.iter().position(|dim| *dim == 0) {
        return Err(CanonicalTensorSchemaError::ZeroDim { index });
    }
    let _ = shape.iter().try_fold(1_u128, |product, dim| {
        product
            .checked_mul(u128::from(*dim))
            .ok_or(CanonicalTensorSchemaError::ShapeElementOverflow)
    })?;
    Ok(())
}
