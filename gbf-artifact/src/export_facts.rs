//! Export-side measured facts.

use std::error::Error;
use std::fmt;

use gbf_foundation::{ExpertId, LayerId};
use serde::{Deserialize, Serialize};

use crate::ids::{ArtifactPath, ArtifactPathError};
use crate::quant::{ActivationEvalModeSpec, ActivationQuantFormatSpec, ActivationRangeSpec};
use crate::sequence::SequenceExportFacts;

const Q8_8_ONE: u16 = 256;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportFacts {
    pub activation_ranges: Vec<RangeDigest>,
    pub sequence: SequenceExportFacts,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub temporal_switch: Vec<TemporalSwitchDigest>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clip_saturation: Vec<ClipSaturationDigest>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub expert_payloads: Vec<ExpertPayloadDigest>,
}

impl ExportFacts {
    pub fn new(activation_ranges: Vec<RangeDigest>, sequence: SequenceExportFacts) -> Self {
        Self {
            activation_ranges,
            sequence,
            temporal_switch: Vec::new(),
            clip_saturation: Vec::new(),
            expert_payloads: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RangeDigest {
    pub activation: ArtifactPath,
    pub range: ActivationRangeSpec,
    pub quant_format: ActivationQuantFormatSpec,
    pub eval_mode: ActivationEvalModeSpec,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct RateQ8_8(u16);

impl RateQ8_8 {
    pub const ZERO: Self = Self(0);
    pub const ONE: Self = Self(Q8_8_ONE);

    pub fn new(raw: u16) -> Result<Self, ExportFactsError> {
        if raw > Q8_8_ONE {
            return Err(ExportFactsError::RateQ8_8OutOfRange { value: raw });
        }

        Ok(Self(raw))
    }

    pub fn from_ratio(numerator: u64, denominator: u64) -> Result<Self, ExportFactsError> {
        if denominator == 0 {
            return Err(ExportFactsError::ZeroRateDenominator);
        }

        if numerator > denominator {
            return Err(ExportFactsError::RateRatioOutOfRange {
                numerator,
                denominator,
            });
        }

        let raw = ((u128::from(numerator) * u128::from(Q8_8_ONE)) + u128::from(denominator / 2))
            / u128::from(denominator);
        let raw = u16::try_from(raw)
            .map_err(|_| ExportFactsError::RateQ8_8OutOfRange { value: u16::MAX })?;
        Self::new(raw)
    }

    #[must_use]
    pub const fn raw(self) -> u16 {
        self.0
    }

    #[must_use]
    pub fn as_f32(self) -> f32 {
        f32::from(self.0) / f32::from(Q8_8_ONE)
    }
}

impl<'de> Deserialize<'de> for RateQ8_8 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = u16::deserialize(deserializer)?;
        Self::new(raw).map_err(serde::de::Error::custom)
    }
}

impl TryFrom<u16> for RateQ8_8 {
    type Error = ExportFactsError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<RateQ8_8> for u16 {
    fn from(value: RateQ8_8) -> Self {
        value.raw()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct BoundaryId(ArtifactPath);

impl BoundaryId {
    pub fn new(value: impl Into<String>) -> Result<Self, ExportFactsError> {
        ArtifactPath::new(value)
            .map(Self)
            .map_err(ExportFactsError::InvalidBoundaryId)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    #[must_use]
    pub fn as_path(&self) -> &ArtifactPath {
        &self.0
    }

    #[must_use]
    pub fn into_path(self) -> ArtifactPath {
        self.0
    }
}

impl<'de> Deserialize<'de> for BoundaryId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Self::new(raw).map_err(serde::de::Error::custom)
    }
}

impl fmt::Display for BoundaryId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TemporalSwitchDigest {
    layer: LayerId,
    same_expert_rate_q8_8: RateQ8_8,
    transition_mass: Vec<ExpertTransitionDigest>,
}

impl<'de> Deserialize<'de> for TemporalSwitchDigest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct TemporalSwitchDigestSerde {
            layer: LayerId,
            same_expert_rate_q8_8: RateQ8_8,
            transition_mass: Vec<ExpertTransitionDigest>,
        }

        let raw = TemporalSwitchDigestSerde::deserialize(deserializer)?;
        Self::new(raw.layer, raw.same_expert_rate_q8_8, raw.transition_mass)
            .map_err(serde::de::Error::custom)
    }
}

impl TemporalSwitchDigest {
    pub fn new(
        layer: LayerId,
        same_expert_rate_q8_8: RateQ8_8,
        transition_mass: Vec<ExpertTransitionDigest>,
    ) -> Result<Self, ExportFactsError> {
        validate_transition_mass(&transition_mass)?;

        Ok(Self {
            layer,
            same_expert_rate_q8_8,
            transition_mass,
        })
    }

    #[must_use]
    pub const fn layer(&self) -> LayerId {
        self.layer
    }

    #[must_use]
    pub const fn same_expert_rate_q8_8(&self) -> RateQ8_8 {
        self.same_expert_rate_q8_8
    }

    #[must_use]
    pub fn transition_mass(&self) -> &[ExpertTransitionDigest] {
        &self.transition_mass
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpertTransitionDigest {
    from_expert: ExpertId,
    to_expert: ExpertId,
    rate_q8_8: RateQ8_8,
}

impl ExpertTransitionDigest {
    pub const fn new(from_expert: ExpertId, to_expert: ExpertId, rate_q8_8: RateQ8_8) -> Self {
        Self {
            from_expert,
            to_expert,
            rate_q8_8,
        }
    }

    #[must_use]
    pub const fn source_expert(self) -> ExpertId {
        self.from_expert
    }

    #[must_use]
    pub const fn target_expert(self) -> ExpertId {
        self.to_expert
    }

    #[must_use]
    pub const fn rate_q8_8(self) -> RateQ8_8 {
        self.rate_q8_8
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipSaturationDigest {
    layer: LayerId,
    boundary: BoundaryId,
    saturation_rate_q8_8: RateQ8_8,
}

impl ClipSaturationDigest {
    pub fn new(layer: LayerId, boundary: BoundaryId, saturation_rate_q8_8: RateQ8_8) -> Self {
        Self {
            layer,
            boundary,
            saturation_rate_q8_8,
        }
    }

    #[must_use]
    pub const fn layer(&self) -> LayerId {
        self.layer
    }

    #[must_use]
    pub fn boundary(&self) -> &BoundaryId {
        &self.boundary
    }

    #[must_use]
    pub const fn saturation_rate_q8_8(&self) -> RateQ8_8 {
        self.saturation_rate_q8_8
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExpertPayloadDigest {
    layer: LayerId,
    expert: ExpertId,
    total_bytes: u32,
    ternary_bytes: u32,
    scale_bytes: u32,
}

impl<'de> Deserialize<'de> for ExpertPayloadDigest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct ExpertPayloadDigestSerde {
            layer: LayerId,
            expert: ExpertId,
            total_bytes: u32,
            ternary_bytes: u32,
            scale_bytes: u32,
        }

        let raw = ExpertPayloadDigestSerde::deserialize(deserializer)?;
        Self::new(
            raw.layer,
            raw.expert,
            raw.total_bytes,
            raw.ternary_bytes,
            raw.scale_bytes,
        )
        .map_err(serde::de::Error::custom)
    }
}

impl ExpertPayloadDigest {
    pub fn new(
        layer: LayerId,
        expert: ExpertId,
        total_bytes: u32,
        ternary_bytes: u32,
        scale_bytes: u32,
    ) -> Result<Self, ExportFactsError> {
        let known_bytes = ternary_bytes.checked_add(scale_bytes).ok_or(
            ExportFactsError::ExpertPayloadByteOverflow {
                ternary_bytes,
                scale_bytes,
            },
        )?;

        if known_bytes > total_bytes {
            return Err(ExportFactsError::ExpertPayloadBreakdownExceedsTotal {
                total_bytes,
                ternary_bytes,
                scale_bytes,
            });
        }

        Ok(Self {
            layer,
            expert,
            total_bytes,
            ternary_bytes,
            scale_bytes,
        })
    }

    #[must_use]
    pub const fn layer(&self) -> LayerId {
        self.layer
    }

    #[must_use]
    pub const fn expert(&self) -> ExpertId {
        self.expert
    }

    #[must_use]
    pub const fn total_bytes(&self) -> u32 {
        self.total_bytes
    }

    #[must_use]
    pub const fn ternary_bytes(&self) -> u32 {
        self.ternary_bytes
    }

    #[must_use]
    pub const fn scale_bytes(&self) -> u32 {
        self.scale_bytes
    }

    #[must_use]
    pub const fn metadata_bytes(&self) -> u32 {
        self.total_bytes - self.ternary_bytes - self.scale_bytes
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportFactsError {
    RateQ8_8OutOfRange {
        value: u16,
    },
    RateRatioOutOfRange {
        numerator: u64,
        denominator: u64,
    },
    ZeroRateDenominator,
    TransitionMassExceedsOne {
        total_q8_8: u32,
    },
    InvalidBoundaryId(ArtifactPathError),
    ExpertPayloadByteOverflow {
        ternary_bytes: u32,
        scale_bytes: u32,
    },
    ExpertPayloadBreakdownExceedsTotal {
        total_bytes: u32,
        ternary_bytes: u32,
        scale_bytes: u32,
    },
}

impl fmt::Display for ExportFactsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RateQ8_8OutOfRange { value } => {
                write!(f, "q8_8 rate must be in 0..=256, got {value}")
            }
            Self::RateRatioOutOfRange {
                numerator,
                denominator,
            } => write!(
                f,
                "q8_8 rate ratio must be in 0..=1, got {numerator}/{denominator}"
            ),
            Self::ZeroRateDenominator => f.write_str("q8_8 rate denominator must not be zero"),
            Self::TransitionMassExceedsOne { total_q8_8 } => write!(
                f,
                "transition_mass q8_8 rates must sum to <= 256, got {total_q8_8}"
            ),
            Self::InvalidBoundaryId(error) => write!(f, "invalid boundary id: {error}"),
            Self::ExpertPayloadByteOverflow {
                ternary_bytes,
                scale_bytes,
            } => write!(
                f,
                "expert payload byte components overflow u32: ternary_bytes={ternary_bytes}, scale_bytes={scale_bytes}"
            ),
            Self::ExpertPayloadBreakdownExceedsTotal {
                total_bytes,
                ternary_bytes,
                scale_bytes,
            } => write!(
                f,
                "expert payload total_bytes={total_bytes} is smaller than ternary_bytes={ternary_bytes} + scale_bytes={scale_bytes}"
            ),
        }
    }
}

impl Error for ExportFactsError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidBoundaryId(error) => Some(error),
            Self::RateQ8_8OutOfRange { .. }
            | Self::RateRatioOutOfRange { .. }
            | Self::ZeroRateDenominator
            | Self::TransitionMassExceedsOne { .. }
            | Self::ExpertPayloadByteOverflow { .. }
            | Self::ExpertPayloadBreakdownExceedsTotal { .. } => None,
        }
    }
}

fn validate_transition_mass(
    transition_mass: &[ExpertTransitionDigest],
) -> Result<(), ExportFactsError> {
    let total_q8_8 = transition_mass
        .iter()
        .map(|entry| u32::from(entry.rate_q8_8().raw()))
        .sum::<u32>();

    if total_q8_8 > u32::from(Q8_8_ONE) {
        return Err(ExportFactsError::TransitionMassExceedsOne { total_q8_8 });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::sequence::{SequenceExportFacts, SequenceSemanticsSpec};

    #[test]
    fn export_facts_carry_router_observation_digests() {
        let mut facts = ExportFacts::new(
            vec![RangeDigest {
                activation: ArtifactPath::new("layer.0.ffn.activation").unwrap(),
                range: ActivationRangeSpec {
                    lo: -1.0,
                    hi: 1.0,
                    mode: crate::quant::ActivationRangeModeSpec::Ema,
                },
                quant_format: ActivationQuantFormatSpec::Int8,
                eval_mode: ActivationEvalModeSpec::Quantized,
            }],
            SequenceExportFacts::for_spec(SequenceSemanticsSpec::linear_state(64).unwrap()),
        );

        facts.temporal_switch.push(
            TemporalSwitchDigest::new(
                LayerId::new(2),
                RateQ8_8::new(192).unwrap(),
                vec![ExpertTransitionDigest::new(
                    ExpertId::new(0),
                    ExpertId::new(1),
                    RateQ8_8::new(64).unwrap(),
                )],
            )
            .unwrap(),
        );
        facts.clip_saturation.push(ClipSaturationDigest::new(
            LayerId::new(2),
            BoundaryId::new("layer.2.expert.input").unwrap(),
            RateQ8_8::new(16).unwrap(),
        ));
        facts.expert_payloads.push(
            ExpertPayloadDigest::new(LayerId::new(2), ExpertId::new(1), 1024, 768, 128).unwrap(),
        );

        assert_eq!(
            facts.temporal_switch[0].transition_mass()[0].target_expert(),
            ExpertId::new(1)
        );
        assert_eq!(facts.temporal_switch[0].same_expert_rate_q8_8().raw(), 192);
        assert_eq!(
            facts.clip_saturation[0].boundary().as_str(),
            "layer.2.expert.input"
        );
        assert_eq!(facts.expert_payloads[0].metadata_bytes(), 128);

        let encoded = serde_json::to_string(&facts).unwrap();
        let decoded: ExportFacts = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, facts);

        let value = serde_json::to_value(&facts).unwrap();
        assert_eq!(
            value["temporal_switch"],
            json!([{
                "layer": 2,
                "same_expert_rate_q8_8": 192,
                "transition_mass": [{
                    "from_expert": 0,
                    "to_expert": 1,
                    "rate_q8_8": 64,
                }],
            }])
        );
        assert_eq!(
            value["clip_saturation"],
            json!([{
                "layer": 2,
                "boundary": "layer.2.expert.input",
                "saturation_rate_q8_8": 16,
            }])
        );
        assert_eq!(
            value["expert_payloads"],
            json!([{
                "layer": 2,
                "expert": 1,
                "total_bytes": 1024,
                "ternary_bytes": 768,
                "scale_bytes": 128,
            }])
        );
    }

    #[test]
    fn export_facts_default_new_digests_to_empty_for_old_payloads() {
        let value = json!({
            "activation_ranges": [],
            "sequence": SequenceExportFacts::for_spec(
                SequenceSemanticsSpec::bounded_kv(32, 12).unwrap()
            ),
        });

        let facts: ExportFacts = serde_json::from_value(value).unwrap();

        assert!(facts.temporal_switch.is_empty());
        assert!(facts.clip_saturation.is_empty());
        assert!(facts.expert_payloads.is_empty());

        let encoded = serde_json::to_value(&facts).unwrap();
        assert!(encoded.get("temporal_switch").is_none());
        assert!(encoded.get("clip_saturation").is_none());
        assert!(encoded.get("expert_payloads").is_none());
    }

    #[test]
    fn export_fact_rates_are_valid_q8_8_fractions() {
        assert_eq!(RateQ8_8::ZERO.raw(), 0);
        assert_eq!(RateQ8_8::ONE.as_f32(), 1.0);
        assert_eq!(RateQ8_8::from_ratio(1, 4).unwrap().raw(), 64);
        assert_eq!(
            RateQ8_8::from_ratio(u64::MAX, u64::MAX).unwrap(),
            RateQ8_8::ONE
        );
        assert_eq!(
            RateQ8_8::new(257),
            Err(ExportFactsError::RateQ8_8OutOfRange { value: 257 })
        );
        assert_eq!(
            RateQ8_8::from_ratio(1001, 1000),
            Err(ExportFactsError::RateRatioOutOfRange {
                numerator: 1001,
                denominator: 1000,
            })
        );
        assert_eq!(
            RateQ8_8::from_ratio(1, 0),
            Err(ExportFactsError::ZeroRateDenominator)
        );

        let invalid: Result<RateQ8_8, _> = serde_json::from_value(json!(300));
        assert!(invalid.is_err());
    }

    #[test]
    fn export_fact_transitions_ids_and_payload_bytes_are_validated() {
        assert_eq!(
            TemporalSwitchDigest::new(
                LayerId::new(0),
                RateQ8_8::new(128).unwrap(),
                vec![
                    ExpertTransitionDigest::new(
                        ExpertId::new(0),
                        ExpertId::new(1),
                        RateQ8_8::new(200).unwrap(),
                    ),
                    ExpertTransitionDigest::new(
                        ExpertId::new(1),
                        ExpertId::new(0),
                        RateQ8_8::new(57).unwrap(),
                    ),
                ],
            ),
            Err(ExportFactsError::TransitionMassExceedsOne { total_q8_8: 257 })
        );

        let invalid_switch: Result<TemporalSwitchDigest, _> = serde_json::from_value(json!({
            "layer": 0,
            "same_expert_rate_q8_8": 128,
            "transition_mass": [
                { "from_expert": 0, "to_expert": 1, "rate_q8_8": 200 },
                { "from_expert": 1, "to_expert": 0, "rate_q8_8": 57 },
            ],
        }));
        assert!(invalid_switch.is_err());

        assert!(matches!(
            BoundaryId::new("layer/0"),
            Err(ExportFactsError::InvalidBoundaryId(_))
        ));
        assert_eq!(
            ExpertPayloadDigest::new(LayerId::new(0), ExpertId::new(0), 100, 80, 40),
            Err(ExportFactsError::ExpertPayloadBreakdownExceedsTotal {
                total_bytes: 100,
                ternary_bytes: 80,
                scale_bytes: 40,
            })
        );
        assert_eq!(
            ExpertPayloadDigest::new(LayerId::new(0), ExpertId::new(0), u32::MAX, u32::MAX, 1),
            Err(ExportFactsError::ExpertPayloadByteOverflow {
                ternary_bytes: u32::MAX,
                scale_bytes: 1,
            })
        );

        let invalid: Result<ExpertPayloadDigest, _> = serde_json::from_value(json!({
            "layer": 0,
            "expert": 0,
            "total_bytes": 100,
            "ternary_bytes": 80,
            "scale_bytes": 40,
        }));
        assert!(invalid.is_err());
    }
}
