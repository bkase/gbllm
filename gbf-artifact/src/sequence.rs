//! Target-independent sequence-state semantics.

use std::error::Error;
use std::fmt;
use std::num::NonZeroU16;

use serde::{Deserialize, Deserializer, Serialize};

use crate::tensor::CanonicalTensorId;

pub const DEFAULT_LINEAR_STATE_DECAY: f32 = 0.5;
pub const LINEAR_STATE_SLOT_BYTES: u16 = core::mem::size_of::<f32>() as u16;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SequenceSemanticsSpec {
    LinearState(LinearStateSemantics),
    BoundedKv(BoundedKvSemantics),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SequenceSemanticsKind {
    LinearState,
    BoundedKv,
}

impl SequenceSemanticsSpec {
    pub fn linear_state(state_bytes_per_layer: u16) -> Result<Self, SequenceSemanticsError> {
        Self::linear_state_with_decay(state_bytes_per_layer, DecayPolicy::default())
    }

    pub fn linear_state_with_decay(
        state_bytes_per_layer: u16,
        decay_policy: DecayPolicy,
    ) -> Result<Self, SequenceSemanticsError> {
        Ok(Self::LinearState(LinearStateSemantics::new(
            state_bytes_per_layer,
            decay_policy,
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

    pub fn state_size(&self) -> SequenceStateSize {
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

    pub const fn kind(&self) -> SequenceSemanticsKind {
        match self {
            Self::LinearState(_) => SequenceSemanticsKind::LinearState,
            Self::BoundedKv(_) => SequenceSemanticsKind::BoundedKv,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct LinearStateSemantics {
    state_bytes_per_layer: NonZeroU16,
    decay_policy: DecayPolicy,
}

impl<'de> Deserialize<'de> for LinearStateSemantics {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct LinearStateSemanticsSerde {
            state_bytes_per_layer: u16,
            #[serde(default)]
            decay_policy: DecayPolicy,
        }

        let raw = LinearStateSemanticsSerde::deserialize(deserializer)?;
        Self::new(raw.state_bytes_per_layer, raw.decay_policy).map_err(serde::de::Error::custom)
    }
}

impl LinearStateSemantics {
    pub fn new(
        state_bytes_per_layer: u16,
        decay_policy: DecayPolicy,
    ) -> Result<Self, SequenceSemanticsError> {
        decay_policy.validate_for_state_bytes(state_bytes_per_layer)?;
        Ok(Self {
            state_bytes_per_layer: nonzero_u16("state_bytes_per_layer", state_bytes_per_layer)?,
            decay_policy,
        })
    }

    pub fn state_bytes_per_layer(&self) -> u16 {
        self.state_bytes_per_layer.get()
    }

    pub fn decay_policy(&self) -> &DecayPolicy {
        &self.decay_policy
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

    pub fn max_context(&self) -> u16 {
        self.max_context.get()
    }

    pub fn kv_bytes_per_token(&self) -> u16 {
        self.kv_bytes_per_token.get()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DecayRate(u32);

impl DecayRate {
    pub fn new(value: f32) -> Result<Self, SequenceSemanticsError> {
        if !value.is_finite() || value <= 0.0 || value >= 1.0 {
            return Err(SequenceSemanticsError::InvalidDecay {
                value_bits: value.to_bits(),
            });
        }
        Ok(Self(value.to_bits()))
    }

    pub fn value(self) -> f32 {
        f32::from_bits(self.0)
    }
}

impl Serialize for DecayRate {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_f32(self.value())
    }
}

impl<'de> Deserialize<'de> for DecayRate {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = f32::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub enum DecayPolicy {
    Fixed(DecayRate),
    MultiTimescale(Vec<DecayRate>),
    Learned(DecayRate),
}

impl Default for DecayPolicy {
    fn default() -> Self {
        Self::Fixed(
            DecayRate::new(DEFAULT_LINEAR_STATE_DECAY)
                .expect("default linear-state decay is finite and in range"),
        )
    }
}

impl<'de> Deserialize<'de> for DecayPolicy {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        enum RawDecayPolicy {
            Fixed(DecayRate),
            MultiTimescale(Vec<DecayRate>),
            Learned(DecayRate),
        }

        match RawDecayPolicy::deserialize(deserializer)? {
            RawDecayPolicy::Fixed(rate) => Ok(Self::Fixed(rate)),
            RawDecayPolicy::MultiTimescale(rates) => {
                if rates.is_empty() {
                    return Err(serde::de::Error::custom(
                        SequenceSemanticsError::EmptyDecayPolicy {
                            field: "multi_timescale",
                        },
                    ));
                }
                Ok(Self::MultiTimescale(rates))
            }
            RawDecayPolicy::Learned(rate) => Ok(Self::Learned(rate)),
        }
    }
}

impl DecayPolicy {
    pub fn fixed(value: f32) -> Result<Self, SequenceSemanticsError> {
        Ok(Self::Fixed(DecayRate::new(value)?))
    }

    pub fn multi_timescale(values: Vec<f32>) -> Result<Self, SequenceSemanticsError> {
        if values.is_empty() {
            return Err(SequenceSemanticsError::EmptyDecayPolicy {
                field: "multi_timescale",
            });
        }
        values
            .into_iter()
            .map(DecayRate::new)
            .collect::<Result<Vec<_>, _>>()
            .map(Self::MultiTimescale)
    }

    pub fn learned(initial_value: f32) -> Result<Self, SequenceSemanticsError> {
        Ok(Self::Learned(DecayRate::new(initial_value)?))
    }

    pub fn validate_for_state_bytes(
        &self,
        state_bytes_per_layer: u16,
    ) -> Result<(), SequenceSemanticsError> {
        if let Self::MultiTimescale(rates) = self {
            if rates.is_empty() {
                return Err(SequenceSemanticsError::EmptyDecayPolicy {
                    field: "multi_timescale",
                });
            }
            if !state_bytes_per_layer.is_multiple_of(LINEAR_STATE_SLOT_BYTES) {
                return Err(SequenceSemanticsError::UnalignedStateBytes {
                    state_bytes_per_layer,
                    slot_bytes: LINEAR_STATE_SLOT_BYTES,
                });
            }
            let state_slots = usize::from(state_bytes_per_layer / LINEAR_STATE_SLOT_BYTES);
            if !state_slots.is_multiple_of(rates.len()) {
                return Err(SequenceSemanticsError::MultiTimescaleDoesNotDivideState {
                    state_slots,
                    decay_count: rates.len(),
                });
            }
        }

        Ok(())
    }

    pub fn decay_for_slot(&self, slot: usize, state_slots: usize) -> f32 {
        match self {
            Self::Fixed(rate) | Self::Learned(rate) => rate.value(),
            Self::MultiTimescale(rates) => {
                let slots_per_rate = state_slots / rates.len();
                rates[slot / slots_per_rate].value()
            }
        }
    }

    pub fn rates(&self) -> Vec<f32> {
        match self {
            Self::Fixed(rate) | Self::Learned(rate) => vec![rate.value()],
            Self::MultiTimescale(rates) => rates.iter().map(|rate| rate.value()).collect(),
        }
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
        let measured_state_size = spec.state_size();
        Self {
            spec,
            measured_state_size,
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
        self.spec.clone()
    }

    pub fn measured_state_size(&self) -> SequenceStateSize {
        self.measured_state_size
    }

    pub fn canonical_tensor_handles(&self) -> &[CanonicalTensorId] {
        &self.canonical_tensor_handles
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SequenceSemanticsError {
    ZeroField {
        field: &'static str,
    },
    StateSizeMismatch {
        expected: SequenceStateSize,
        actual: SequenceStateSize,
    },
    InvalidDecay {
        value_bits: u32,
    },
    EmptyDecayPolicy {
        field: &'static str,
    },
    UnalignedStateBytes {
        state_bytes_per_layer: u16,
        slot_bytes: u16,
    },
    MultiTimescaleDoesNotDivideState {
        state_slots: usize,
        decay_count: usize,
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
            Self::InvalidDecay { value_bits } => {
                let value = f32::from_bits(*value_bits);
                write!(f, "decay must be finite and in (0, 1), got {value}")
            }
            Self::EmptyDecayPolicy { field } => write!(f, "{field} decay policy must be nonempty"),
            Self::UnalignedStateBytes {
                state_bytes_per_layer,
                slot_bytes,
            } => write!(
                f,
                "linear-state byte budget {state_bytes_per_layer} must be divisible by {slot_bytes}"
            ),
            Self::MultiTimescaleDoesNotDivideState {
                state_slots,
                decay_count,
            } => write!(
                f,
                "multi-timescale decay count {decay_count} must divide {state_slots} state slots"
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
        let semantics = SequenceSemanticsSpec::linear_state_with_decay(
            32,
            DecayPolicy::multi_timescale(vec![0.5, 0.75]).unwrap(),
        )
        .unwrap();

        let encoded = serde_json::to_string(&semantics).unwrap();
        let decoded: SequenceSemanticsSpec = serde_json::from_str(&encoded).unwrap();

        assert_eq!(decoded, semantics);
        assert_eq!(
            decoded.state_size(),
            SequenceStateSize {
                bytes_per_layer: 32,
                bytes_per_token: 0,
                fixed_overhead: 0,
            }
        );
    }

    #[test]
    fn linear_state_decay_policy_validates_scalar_guards_and_state_partition() {
        assert!(matches!(
            DecayPolicy::fixed(0.0),
            Err(SequenceSemanticsError::InvalidDecay { .. })
        ));
        assert!(matches!(
            DecayPolicy::fixed(1.0),
            Err(SequenceSemanticsError::InvalidDecay { .. })
        ));
        assert!(matches!(
            DecayPolicy::fixed(f32::NAN),
            Err(SequenceSemanticsError::InvalidDecay { .. })
        ));
        assert!(matches!(
            DecayPolicy::multi_timescale(vec![0.5, 1.0]),
            Err(SequenceSemanticsError::InvalidDecay { .. })
        ));
        assert!(matches!(
            DecayPolicy::learned(1.0),
            Err(SequenceSemanticsError::InvalidDecay { .. })
        ));
        assert_eq!(
            DecayPolicy::multi_timescale(Vec::new()),
            Err(SequenceSemanticsError::EmptyDecayPolicy {
                field: "multi_timescale"
            })
        );

        assert_eq!(
            SequenceSemanticsSpec::linear_state_with_decay(
                12,
                DecayPolicy::multi_timescale(vec![0.5, 0.75]).unwrap(),
            ),
            Err(SequenceSemanticsError::MultiTimescaleDoesNotDivideState {
                state_slots: 3,
                decay_count: 2,
            })
        );
        assert_eq!(
            SequenceSemanticsSpec::linear_state_with_decay(
                10,
                DecayPolicy::multi_timescale(vec![0.5, 0.75]).unwrap(),
            ),
            Err(SequenceSemanticsError::UnalignedStateBytes {
                state_bytes_per_layer: 10,
                slot_bytes: LINEAR_STATE_SLOT_BYTES,
            })
        );
    }

    #[test]
    fn linear_state_semantics_deserialize_back_compat_default_decay() {
        let decoded: SequenceSemanticsSpec =
            serde_json::from_str(r#"{"LinearState":{"state_bytes_per_layer":64}}"#).unwrap();

        assert_eq!(decoded, SequenceSemanticsSpec::linear_state(64).unwrap());
    }

    #[test]
    fn linear_state_semantics_deserialize_rejects_invalid_decay_policy() {
        let err = serde_json::from_str::<SequenceSemanticsSpec>(
            r#"{"LinearState":{"state_bytes_per_layer":12,"decay_policy":{"MultiTimescale":[0.5,0.75]}}}"#,
        )
        .unwrap_err()
        .to_string();

        assert!(err.contains("multi-timescale decay count 2 must divide 3 state slots"));
    }

    #[test]
    fn linear_state_decay_policy_is_part_of_semantic_identity() {
        let default = SequenceSemanticsSpec::linear_state(32).unwrap();
        let explicit_fixed =
            SequenceSemanticsSpec::linear_state_with_decay(32, DecayPolicy::fixed(0.5).unwrap())
                .unwrap();
        let multi = SequenceSemanticsSpec::linear_state_with_decay(
            32,
            DecayPolicy::multi_timescale(vec![0.5, 0.75]).unwrap(),
        )
        .unwrap();
        let learned =
            SequenceSemanticsSpec::linear_state_with_decay(32, DecayPolicy::learned(0.5).unwrap())
                .unwrap();

        assert_eq!(default, explicit_fixed);
        assert_ne!(default, multi);
        assert_ne!(default, learned);
        assert_eq!(
            multi.clone().state_size(),
            SequenceStateSize {
                bytes_per_layer: 32,
                bytes_per_token: 0,
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
            SequenceExportFacts::new(spec.clone(), actual, Vec::new()),
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
