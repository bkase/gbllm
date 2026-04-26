//! Target-independent semantic artifact core.

use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;

use gbf_foundation::Hash256;
use serde::{Deserialize, Serialize};

use crate::quant::QuantSpec;
use crate::tensor::{CanonicalTensor, CanonicalTensorId, stable_digest};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtifactCore {
    pub tensors: Vec<CanonicalTensor>,
    pub quant: QuantSpec,
}

impl ArtifactCore {
    pub fn new(tensors: Vec<CanonicalTensor>, quant: QuantSpec) -> Result<Self, ArtifactCoreError> {
        let mut seen = BTreeSet::new();
        for tensor in &tensors {
            if !seen.insert(tensor.id.clone()) {
                return Err(ArtifactCoreError::DuplicateTensor {
                    id: tensor.id.clone(),
                });
            }
        }

        Ok(Self { tensors, quant })
    }

    pub fn semantic_hash(&self) -> Hash256 {
        let bytes = serde_json::to_vec(self).expect("artifact core serializes deterministically");
        stable_digest(&bytes)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactCoreError {
    DuplicateTensor { id: CanonicalTensorId },
}

impl fmt::Display for ArtifactCoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateTensor { id } => {
                write!(f, "artifact core contains duplicate tensor id {id}")
            }
        }
    }
}

impl Error for ArtifactCoreError {}

#[cfg(test)]
mod tests {
    use crate::tensor::{
        CanonicalTensor, CanonicalTensorId, CanonicalTensorKind, CanonicalTensorLayout,
        CanonicalTensorPayload, CanonicalTensorShape, TensorElementType,
    };

    use super::*;

    #[test]
    fn artifact_core_hash_is_deterministic_for_same_payload() {
        let core_a =
            ArtifactCore::new(vec![fixture_tensor("layer.0.weight")], QuantSpec::default())
                .unwrap();
        let core_b =
            ArtifactCore::new(vec![fixture_tensor("layer.0.weight")], QuantSpec::default())
                .unwrap();

        assert_eq!(core_a.semantic_hash(), core_b.semantic_hash());
    }

    #[test]
    fn artifact_core_rejects_duplicate_tensor_ids() {
        let err = ArtifactCore::new(
            vec![
                fixture_tensor("layer.0.weight"),
                fixture_tensor("layer.0.weight"),
            ],
            QuantSpec::default(),
        )
        .unwrap_err();

        assert_eq!(
            err,
            ArtifactCoreError::DuplicateTensor {
                id: CanonicalTensorId::new("layer.0.weight").unwrap()
            }
        );
    }

    fn fixture_tensor(id: &str) -> CanonicalTensor {
        CanonicalTensor::new(
            CanonicalTensorId::new(id).unwrap(),
            CanonicalTensorKind::TernaryWeight,
            CanonicalTensorLayout::new(
                CanonicalTensorShape::from_usize_dims(&[1, 1]).unwrap(),
                TensorElementType::TernaryI2,
            ),
            CanonicalTensorPayload::I8(vec![1]),
        )
        .unwrap()
    }
}
