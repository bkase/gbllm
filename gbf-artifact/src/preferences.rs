//! Compiler preference hints exported with an artifact.

use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;

use gbf_foundation::{ExpertId, LayerId};
use serde::{Deserialize, Serialize};

use crate::export_facts::RateQ8_8;

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct CompilePreferences {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    expert_slot_affinity: Vec<ExpertSlotAffinity>,
}

impl<'de> Deserialize<'de> for CompilePreferences {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct CompilePreferencesSerde {
            #[serde(default)]
            expert_slot_affinity: Vec<ExpertSlotAffinity>,
        }

        let raw = CompilePreferencesSerde::deserialize(deserializer)?;
        Self::new(raw.expert_slot_affinity).map_err(serde::de::Error::custom)
    }
}

impl CompilePreferences {
    pub fn new(
        expert_slot_affinity: Vec<ExpertSlotAffinity>,
    ) -> Result<Self, CompilePreferencesError> {
        validate_unique_affinity_layers(&expert_slot_affinity)?;

        Ok(Self {
            expert_slot_affinity,
        })
    }

    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn expert_slot_affinity(&self) -> &[ExpertSlotAffinity] {
        &self.expert_slot_affinity
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExpertSlotAffinity {
    layer: LayerId,
    affinities: Vec<AffinityPair>,
}

impl<'de> Deserialize<'de> for ExpertSlotAffinity {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct ExpertSlotAffinitySerde {
            layer: LayerId,
            affinities: Vec<AffinityPair>,
        }

        let raw = ExpertSlotAffinitySerde::deserialize(deserializer)?;
        Self::new(raw.layer, raw.affinities).map_err(serde::de::Error::custom)
    }
}

impl ExpertSlotAffinity {
    pub fn new(
        layer: LayerId,
        affinities: Vec<AffinityPair>,
    ) -> Result<Self, CompilePreferencesError> {
        validate_unique_affinity_pairs(layer, &affinities)?;

        Ok(Self { layer, affinities })
    }

    #[must_use]
    pub const fn layer(&self) -> LayerId {
        self.layer
    }

    #[must_use]
    pub fn affinities(&self) -> &[AffinityPair] {
        &self.affinities
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct AffinityPair {
    expert_a: ExpertId,
    expert_b: ExpertId,
    transition_rate_q8_8: RateQ8_8,
}

impl<'de> Deserialize<'de> for AffinityPair {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct AffinityPairSerde {
            expert_a: ExpertId,
            expert_b: ExpertId,
            transition_rate_q8_8: RateQ8_8,
        }

        let raw = AffinityPairSerde::deserialize(deserializer)?;
        Self::new(raw.expert_a, raw.expert_b, raw.transition_rate_q8_8)
            .map_err(serde::de::Error::custom)
    }
}

impl AffinityPair {
    pub fn new(
        expert_a: ExpertId,
        expert_b: ExpertId,
        transition_rate_q8_8: RateQ8_8,
    ) -> Result<Self, CompilePreferencesError> {
        if expert_a == expert_b {
            return Err(CompilePreferencesError::SelfAffinity { expert: expert_a });
        }
        let (expert_a, expert_b) = canonical_expert_pair(expert_a, expert_b);

        Ok(Self {
            expert_a,
            expert_b,
            transition_rate_q8_8,
        })
    }

    #[must_use]
    pub const fn expert_a(self) -> ExpertId {
        self.expert_a
    }

    #[must_use]
    pub const fn expert_b(self) -> ExpertId {
        self.expert_b
    }

    #[must_use]
    pub const fn transition_rate_q8_8(self) -> RateQ8_8 {
        self.transition_rate_q8_8
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompilePreferencesError {
    SelfAffinity {
        expert: ExpertId,
    },
    DuplicateAffinityPair {
        layer: LayerId,
        expert_a: ExpertId,
        expert_b: ExpertId,
    },
    DuplicateExpertSlotAffinityLayer {
        layer: LayerId,
    },
}

impl fmt::Display for CompilePreferencesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SelfAffinity { expert } => {
                write!(f, "expert affinity pair must use two experts, got {expert}")
            }
            Self::DuplicateAffinityPair {
                layer,
                expert_a,
                expert_b,
            } => write!(
                f,
                "duplicate affinity hint for layer {layer} expert pair {expert_a}/{expert_b}"
            ),
            Self::DuplicateExpertSlotAffinityLayer { layer } => {
                write!(f, "duplicate expert slot affinity entry for layer {layer}")
            }
        }
    }
}

impl Error for CompilePreferencesError {}

fn validate_unique_affinity_layers(
    expert_slot_affinity: &[ExpertSlotAffinity],
) -> Result<(), CompilePreferencesError> {
    let mut seen = BTreeSet::new();

    for affinity in expert_slot_affinity {
        if !seen.insert(affinity.layer()) {
            return Err(CompilePreferencesError::DuplicateExpertSlotAffinityLayer {
                layer: affinity.layer(),
            });
        }
    }

    Ok(())
}

fn validate_unique_affinity_pairs(
    layer: LayerId,
    affinities: &[AffinityPair],
) -> Result<(), CompilePreferencesError> {
    let mut seen = BTreeSet::new();

    for affinity in affinities {
        let (expert_a, expert_b) = canonical_expert_pair(affinity.expert_a(), affinity.expert_b());
        if !seen.insert((expert_a, expert_b)) {
            return Err(CompilePreferencesError::DuplicateAffinityPair {
                layer,
                expert_a,
                expert_b,
            });
        }
    }

    Ok(())
}

fn canonical_expert_pair(expert_a: ExpertId, expert_b: ExpertId) -> (ExpertId, ExpertId) {
    if expert_a <= expert_b {
        (expert_a, expert_b)
    } else {
        (expert_b, expert_a)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn compile_preferences_carry_expert_slot_affinity_hints() {
        let preferences = CompilePreferences::new(vec![
            ExpertSlotAffinity::new(
                LayerId::new(2),
                vec![
                    AffinityPair::new(
                        ExpertId::new(0),
                        ExpertId::new(1),
                        RateQ8_8::new(192).unwrap(),
                    )
                    .unwrap(),
                ],
            )
            .unwrap(),
        ])
        .unwrap();

        assert_eq!(
            preferences.expert_slot_affinity()[0].layer(),
            LayerId::new(2)
        );
        assert_eq!(
            preferences.expert_slot_affinity()[0].affinities()[0].expert_b(),
            ExpertId::new(1)
        );
        assert_eq!(
            preferences.expert_slot_affinity()[0].affinities()[0]
                .transition_rate_q8_8()
                .raw(),
            192
        );

        let encoded = serde_json::to_string(&preferences).unwrap();
        let decoded: CompilePreferences = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, preferences);

        let value = serde_json::to_value(&preferences).unwrap();
        assert_eq!(
            value,
            json!({
                "expert_slot_affinity": [{
                    "layer": 2,
                    "affinities": [{
                        "expert_a": 0,
                        "expert_b": 1,
                        "transition_rate_q8_8": 192,
                    }],
                }],
            })
        );
    }

    #[test]
    fn compile_preferences_default_empty_affinity_for_old_payloads() {
        let preferences: CompilePreferences = serde_json::from_value(json!({})).unwrap();

        assert!(preferences.expert_slot_affinity().is_empty());
        assert_eq!(serde_json::to_value(&preferences).unwrap(), json!({}));
    }

    #[test]
    fn affinity_pairs_are_canonicalized_as_unordered_hints() {
        let pair = AffinityPair::new(
            ExpertId::new(7),
            ExpertId::new(2),
            RateQ8_8::new(96).unwrap(),
        )
        .unwrap();
        let canonical = AffinityPair::new(
            ExpertId::new(2),
            ExpertId::new(7),
            RateQ8_8::new(96).unwrap(),
        )
        .unwrap();

        assert_eq!(pair, canonical);
        assert_eq!(pair.expert_a(), ExpertId::new(2));
        assert_eq!(pair.expert_b(), ExpertId::new(7));

        let decoded: AffinityPair = serde_json::from_value(json!({
            "expert_a": 7,
            "expert_b": 2,
            "transition_rate_q8_8": 96,
        }))
        .unwrap();
        assert_eq!(decoded, canonical);
        assert_eq!(
            serde_json::to_value(decoded).unwrap(),
            json!({
                "expert_a": 2,
                "expert_b": 7,
                "transition_rate_q8_8": 96,
            })
        );
    }

    #[test]
    fn expert_slot_affinity_rejects_invalid_pairs_and_layers() {
        assert_eq!(
            AffinityPair::new(ExpertId::new(3), ExpertId::new(3), RateQ8_8::ONE),
            Err(CompilePreferencesError::SelfAffinity {
                expert: ExpertId::new(3),
            })
        );

        let invalid_pair_json: Result<AffinityPair, _> = serde_json::from_value(json!({
            "expert_a": 3,
            "expert_b": 3,
            "transition_rate_q8_8": 128,
        }));
        assert!(invalid_pair_json.is_err());

        let duplicate = ExpertSlotAffinity::new(
            LayerId::new(0),
            vec![
                AffinityPair::new(
                    ExpertId::new(0),
                    ExpertId::new(1),
                    RateQ8_8::new(64).unwrap(),
                )
                .unwrap(),
                AffinityPair::new(
                    ExpertId::new(1),
                    ExpertId::new(0),
                    RateQ8_8::new(32).unwrap(),
                )
                .unwrap(),
            ],
        );
        assert_eq!(
            duplicate,
            Err(CompilePreferencesError::DuplicateAffinityPair {
                layer: LayerId::new(0),
                expert_a: ExpertId::new(0),
                expert_b: ExpertId::new(1),
            })
        );

        let duplicate_layers = CompilePreferences::new(vec![
            ExpertSlotAffinity::new(LayerId::new(1), Vec::new()).unwrap(),
            ExpertSlotAffinity::new(LayerId::new(1), Vec::new()).unwrap(),
        ]);
        assert_eq!(
            duplicate_layers,
            Err(CompilePreferencesError::DuplicateExpertSlotAffinityLayer {
                layer: LayerId::new(1),
            })
        );

        let invalid_rate_json: Result<CompilePreferences, _> = serde_json::from_value(json!({
            "expert_slot_affinity": [{
                "layer": 0,
                "affinities": [{
                    "expert_a": 0,
                    "expert_b": 1,
                    "transition_rate_q8_8": 300,
                }],
            }],
        }));
        assert!(invalid_rate_json.is_err());
    }
}
