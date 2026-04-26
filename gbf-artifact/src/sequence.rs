//! Target-independent sequence-state semantics.

use std::error::Error;
use std::fmt;
use std::num::NonZeroU16;

use serde::{Deserialize, Serialize};

use crate::tensor::CanonicalTensorId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SequenceSemanticsSpec {
    LinearState(LinearStateSemantics),
    BoundedKv(BoundedKvSemantics),
}

impl SequenceSemanticsSpec {
    pub fn linear_state(state_bytes_per_layer: u16) -> Result<Self, SequenceSemanticsError> {
        Ok(Self::LinearState(LinearStateSemantics::new(
            state_bytes_per_layer,
        )?))
    }

    pub fn bounded_kv(
        max_context: u16,
        kv_bytes_per_token: u16,
    ) -> Result<Self, SequenceSemanticsError> {
        Ok(Self::BoundedKv(BoundedKvSemantics::new(
            max_context,
            kv_bytes_per_token,
        )?))
    }

    pub fn state_size(self) -> SequenceStateSize {
        match self {
            Self::LinearState(semantics) => SequenceStateSize {
                bytes_per_layer: u32::from(semantics.state_bytes_per_layer()),
                bytes_per_token: 0,
                fixed_overhead: 0,
            },
            Self::BoundedKv(semantics) => SequenceStateSize {
                bytes_per_layer: u32::from(semantics.max_context())
                    * u32::from(semantics.kv_bytes_per_token()),
                bytes_per_token: semantics.kv_bytes_per_token(),
                fixed_overhead: 0,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LinearStateSemantics {
    state_bytes_per_layer: NonZeroU16,
}

impl LinearStateSemantics {
    pub fn new(state_bytes_per_layer: u16) -> Result<Self, SequenceSemanticsError> {
        Ok(Self {
            state_bytes_per_layer: nonzero_u16("state_bytes_per_layer", state_bytes_per_layer)?,
        })
    }

    pub fn state_bytes_per_layer(self) -> u16 {
        self.state_bytes_per_layer.get()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BoundedKvSemantics {
    max_context: NonZeroU16,
    kv_bytes_per_token: NonZeroU16,
}

impl BoundedKvSemantics {
    pub fn new(max_context: u16, kv_bytes_per_token: u16) -> Result<Self, SequenceSemanticsError> {
        Ok(Self {
            max_context: nonzero_u16("max_context", max_context)?,
            kv_bytes_per_token: nonzero_u16("kv_bytes_per_token", kv_bytes_per_token)?,
        })
    }

    pub fn max_context(self) -> u16 {
        self.max_context.get()
    }

    pub fn kv_bytes_per_token(self) -> u16 {
        self.kv_bytes_per_token.get()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SequenceStateSize {
    pub bytes_per_layer: u32,
    pub bytes_per_token: u16,
    pub fixed_overhead: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SequenceExportFacts {
    spec: SequenceSemanticsSpec,
    measured_state_size: SequenceStateSize,
    canonical_tensor_handles: Vec<CanonicalTensorId>,
}

impl<'de> Deserialize<'de> for SequenceExportFacts {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct SequenceExportFactsSerde {
            spec: SequenceSemanticsSpec,
            measured_state_size: SequenceStateSize,
            #[serde(default)]
            canonical_tensor_handles: Vec<CanonicalTensorId>,
        }

        let raw = SequenceExportFactsSerde::deserialize(deserializer)?;
        Self::new(
            raw.spec,
            raw.measured_state_size,
            raw.canonical_tensor_handles,
        )
        .map_err(serde::de::Error::custom)
    }
}

impl SequenceExportFacts {
    pub fn for_spec(spec: SequenceSemanticsSpec) -> Self {
        Self {
            spec,
            measured_state_size: spec.state_size(),
            canonical_tensor_handles: Vec::new(),
        }
    }

    pub fn new(
        spec: SequenceSemanticsSpec,
        measured_state_size: SequenceStateSize,
        canonical_tensor_handles: Vec<CanonicalTensorId>,
    ) -> Result<Self, SequenceSemanticsError> {
        let expected = spec.state_size();
        if measured_state_size != expected {
            return Err(SequenceSemanticsError::StateSizeMismatch {
                expected,
                actual: measured_state_size,
            });
        }

        Ok(Self {
            spec,
            measured_state_size,
            canonical_tensor_handles,
        })
    }

    pub fn spec(&self) -> SequenceSemanticsSpec {
        self.spec
    }

    pub fn measured_state_size(&self) -> SequenceStateSize {
        self.measured_state_size
    }

    pub fn canonical_tensor_handles(&self) -> &[CanonicalTensorId] {
        &self.canonical_tensor_handles
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SequenceSemanticsError {
    ZeroField {
        field: &'static str,
    },
    StateSizeMismatch {
        expected: SequenceStateSize,
        actual: SequenceStateSize,
    },
}

impl fmt::Display for SequenceSemanticsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroField { field } => write!(f, "{field} must be nonzero"),
            Self::StateSizeMismatch { expected, actual } => write!(
                f,
                "sequence state size mismatch: expected {expected:?}, got {actual:?}"
            ),
        }
    }
}

impl Error for SequenceSemanticsError {}

fn nonzero_u16(field: &'static str, value: u16) -> Result<NonZeroU16, SequenceSemanticsError> {
    NonZeroU16::new(value).ok_or(SequenceSemanticsError::ZeroField { field })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequence_semantics_reject_zero_sized_contracts() {
        assert_eq!(
            SequenceSemanticsSpec::linear_state(0),
            Err(SequenceSemanticsError::ZeroField {
                field: "state_bytes_per_layer"
            })
        );
        assert_eq!(
            SequenceSemanticsSpec::bounded_kv(0, 4),
            Err(SequenceSemanticsError::ZeroField {
                field: "max_context"
            })
        );
        assert_eq!(
            SequenceSemanticsSpec::bounded_kv(16, 0),
            Err(SequenceSemanticsError::ZeroField {
                field: "kv_bytes_per_token"
            })
        );
    }

    #[test]
    fn sequence_semantics_round_trips_through_serde() {
        let semantics = SequenceSemanticsSpec::bounded_kv(32, 12).unwrap();

        let encoded = serde_json::to_string(&semantics).unwrap();
        let decoded: SequenceSemanticsSpec = serde_json::from_str(&encoded).unwrap();

        assert_eq!(decoded, semantics);
        assert_eq!(
            decoded.state_size(),
            SequenceStateSize {
                bytes_per_layer: 384,
                bytes_per_token: 12,
                fixed_overhead: 0,
            }
        );
    }

    #[test]
    fn sequence_semantics_have_distinct_state_size_shapes() {
        assert_eq!(
            SequenceSemanticsSpec::linear_state(128)
                .unwrap()
                .state_size(),
            SequenceStateSize {
                bytes_per_layer: 128,
                bytes_per_token: 0,
                fixed_overhead: 0,
            }
        );
        assert_eq!(
            SequenceSemanticsSpec::bounded_kv(16, 8)
                .unwrap()
                .state_size(),
            SequenceStateSize {
                bytes_per_layer: 128,
                bytes_per_token: 8,
                fixed_overhead: 0,
            }
        );
    }

    #[test]
    fn sequence_export_facts_reject_mismatched_state_size() {
        let spec = SequenceSemanticsSpec::linear_state(128).unwrap();
        let actual = SequenceStateSize {
            bytes_per_layer: 128,
            bytes_per_token: 1,
            fixed_overhead: 0,
        };

        assert_eq!(
            SequenceExportFacts::new(spec, actual, Vec::new()),
            Err(SequenceSemanticsError::StateSizeMismatch {
                expected: spec.state_size(),
                actual,
            })
        );
    }

    #[test]
    fn sequence_export_facts_use_stable_json_shape() {
        let facts =
            SequenceExportFacts::for_spec(SequenceSemanticsSpec::bounded_kv(32, 12).unwrap());

        let value = serde_json::to_value(&facts).unwrap();

        assert_eq!(
            value,
            serde_json::json!({
                "spec": {
                    "BoundedKv": {
                        "max_context": 32,
                        "kv_bytes_per_token": 12
                    }
                },
                "measured_state_size": {
                    "bytes_per_layer": 384,
                    "bytes_per_token": 12,
                    "fixed_overhead": 0
                },
                "canonical_tensor_handles": []
            })
        );
    }
}
