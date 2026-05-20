//! Runtime chrome budget schema.

use std::collections::BTreeSet;
use std::fmt;
use std::str::FromStr;

use gbf_foundation::{BudgetSlotId, CompileProfileId, Hash256, TargetProfileId};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::compile::PlacementProfile;

pub const SYNTHETIC_REFERENCE_PREFIX: &str = "SYNTHETIC_REFERENCE:";

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RuntimeNucleusHash {
    Real(Hash256),
    SyntheticReference(Hash256),
}

impl RuntimeNucleusHash {
    #[must_use]
    pub const fn real(hash: Hash256) -> Self {
        Self::Real(hash)
    }

    #[must_use]
    pub const fn synthetic_reference(hash: Hash256) -> Self {
        Self::SyntheticReference(hash)
    }

    #[must_use]
    pub const fn hash(self) -> Hash256 {
        match self {
            Self::Real(hash) | Self::SyntheticReference(hash) => hash,
        }
    }

    #[must_use]
    pub const fn is_synthetic_reference(self) -> bool {
        matches!(self, Self::SyntheticReference(_))
    }
}

impl From<Hash256> for RuntimeNucleusHash {
    fn from(hash: Hash256) -> Self {
        Self::real(hash)
    }
}

impl fmt::Display for RuntimeNucleusHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Real(hash) => fmt::Display::fmt(hash, f),
            Self::SyntheticReference(hash) => {
                f.write_str(SYNTHETIC_REFERENCE_PREFIX)?;
                fmt::Display::fmt(hash, f)
            }
        }
    }
}

impl fmt::Debug for RuntimeNucleusHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl FromStr for RuntimeNucleusHash {
    type Err = RuntimeNucleusHashParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if let Some(hash) = value.strip_prefix(SYNTHETIC_REFERENCE_PREFIX) {
            return Hash256::from_str(hash)
                .map(Self::synthetic_reference)
                .map_err(RuntimeNucleusHashParseError::Hash);
        }

        Hash256::from_str(value)
            .map(Self::real)
            .map_err(RuntimeNucleusHashParseError::Hash)
    }
}

impl Serialize for RuntimeNucleusHash {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for RuntimeNucleusHash {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::from_str(&value).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeNucleusHashParseError {
    Hash(gbf_foundation::Hash256ParseError),
}

impl fmt::Display for RuntimeNucleusHashParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Hash(error) => fmt::Display::fmt(error, f),
        }
    }
}

impl std::error::Error for RuntimeNucleusHashParseError {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeChromeBudget {
    pub target: TargetProfileId,
    pub profile: CompileProfileId,
    pub runtime_nucleus_hash: RuntimeNucleusHash,
    pub rom_slots: Vec<RomBudgetSlot>,
    pub memory_caps: RuntimeMemoryCapSection,
    pub wram_reserved: u16,
    pub sram_reserved: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeMemoryCapSection {
    pub wram_usable_bytes: u32,
    pub sram_usable_bytes: u32,
    pub hram_usable_bytes: u32,
    pub source_target_profile_hash: Hash256,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RomBudgetSlot {
    pub id: BudgetSlotId,
    pub class: BudgetSlotClass,
    pub usable_bytes: u32,
    pub reserved_slack: u16,
    pub placement_caps: BTreeSet<PlacementProfile>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum BudgetSlotClass {
    Bank0Free,
    CommonBank,
    ExpertBank,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash_json(byte: u8) -> serde_json::Value {
        serde_json::to_value(Hash256::from_bytes([byte; 32])).expect("hash serializes")
    }

    fn runtime_hash(byte: u8) -> RuntimeNucleusHash {
        RuntimeNucleusHash::real(Hash256::from_bytes([byte; 32]))
    }

    fn budget_fixture() -> RuntimeChromeBudget {
        RuntimeChromeBudget {
            target: TargetProfileId::from("dmg-mbc5"),
            profile: CompileProfileId::from("Bringup"),
            runtime_nucleus_hash: runtime_hash(1),
            rom_slots: vec![RomBudgetSlot {
                id: BudgetSlotId::new(7),
                class: BudgetSlotClass::ExpertBank,
                usable_bytes: 16_000,
                reserved_slack: 384,
                placement_caps: BTreeSet::from([
                    PlacementProfile::StrictOnePerBank,
                    PlacementProfile::Budgeted,
                ]),
            }],
            memory_caps: RuntimeMemoryCapSection {
                wram_usable_bytes: 8 * 1024,
                sram_usable_bytes: 32 * 1024,
                hram_usable_bytes: 127,
                source_target_profile_hash: Hash256::from_bytes([2; 32]),
            },
            wram_reserved: 128,
            sram_reserved: 512,
        }
    }

    #[test]
    fn budget_types_round_trip() {
        let budget = budget_fixture();
        let expected = serde_json::json!({
            "target": "dmg-mbc5",
            "profile": "Bringup",
            "runtime_nucleus_hash": hash_json(1),
            "rom_slots": [
                {
                    "id": 7,
                    "class": {"kind": "ExpertBank"},
                    "usable_bytes": 16000,
                    "reserved_slack": 384,
                    "placement_caps": [
                        {"kind": "StrictOnePerBank"},
                        {"kind": "Budgeted"}
                    ]
                }
            ],
            "memory_caps": {
                "wram_usable_bytes": 8192,
                "sram_usable_bytes": 32768,
                "hram_usable_bytes": 127,
                "source_target_profile_hash": hash_json(2)
            },
            "wram_reserved": 128,
            "sram_reserved": 512
        });

        let encoded = serde_json::to_string(&budget).expect("budget serializes");
        let decoded: RuntimeChromeBudget =
            serde_json::from_str(&encoded).expect("budget deserializes");

        assert_eq!(decoded, budget);
        assert_eq!(
            serde_json::to_value(&budget).expect("budget serializes"),
            expected
        );
    }

    #[test]
    fn memory_cap_section_preserves_u32_json_widths() {
        let memory_caps = RuntimeMemoryCapSection {
            wram_usable_bytes: 70_000,
            sram_usable_bytes: u32::MAX,
            hram_usable_bytes: 300,
            source_target_profile_hash: Hash256::from_bytes([3; 32]),
        };
        let expected = serde_json::json!({
            "wram_usable_bytes": 70000,
            "sram_usable_bytes": 4294967295u64,
            "hram_usable_bytes": 300,
            "source_target_profile_hash": hash_json(3)
        });

        let encoded = serde_json::to_value(memory_caps).expect("memory caps serialize");
        let decoded: RuntimeMemoryCapSection =
            serde_json::from_value(expected.clone()).expect("memory caps deserialize");

        assert_eq!(encoded, expected);
        assert_eq!(decoded, memory_caps);
    }

    #[test]
    fn runtime_chrome_budget_preserves_reserved_field_boundaries() {
        let budget = RuntimeChromeBudget {
            wram_reserved: u16::MAX,
            sram_reserved: u32::MAX,
            ..budget_fixture()
        };
        let expected = serde_json::json!({
            "target": "dmg-mbc5",
            "profile": "Bringup",
            "runtime_nucleus_hash": hash_json(1),
            "rom_slots": [
                {
                    "id": 7,
                    "class": {"kind": "ExpertBank"},
                    "usable_bytes": 16000,
                    "reserved_slack": 384,
                    "placement_caps": [
                        {"kind": "StrictOnePerBank"},
                        {"kind": "Budgeted"}
                    ]
                }
            ],
            "memory_caps": {
                "wram_usable_bytes": 8192,
                "sram_usable_bytes": 32768,
                "hram_usable_bytes": 127,
                "source_target_profile_hash": hash_json(2)
            },
            "wram_reserved": u16::MAX,
            "sram_reserved": u32::MAX
        });

        let encoded = serde_json::to_value(&budget).expect("budget serializes");
        let decoded: RuntimeChromeBudget =
            serde_json::from_value(expected.clone()).expect("budget deserializes");

        assert_eq!(encoded, expected);
        assert_eq!(decoded, budget);
    }

    #[test]
    fn budget_rejects_unknown_field() {
        let mut value = serde_json::to_value(budget_fixture()).expect("budget serializes");
        value["unexpected"] = serde_json::json!("nope");

        assert!(serde_json::from_value::<RuntimeChromeBudget>(value).is_err());
    }

    #[test]
    fn runtime_nucleus_hash_round_trips_real_and_synthetic() {
        let real = RuntimeNucleusHash::real(Hash256::from_bytes([0xab; 32]));
        let synthetic = RuntimeNucleusHash::synthetic_reference(Hash256::from_bytes([0xcd; 32]));

        assert_eq!(
            serde_json::to_value(real).expect("real hash serializes"),
            serde_json::json!(
                "sha256:abababababababababababababababababababababababababababababababab"
            )
        );
        assert_eq!(
            serde_json::from_value::<RuntimeNucleusHash>(serde_json::json!(
                "sha256:abababababababababababababababababababababababababababababababab"
            ))
            .expect("real hash deserializes"),
            real
        );
        assert_eq!(
            serde_json::to_value(synthetic).expect("synthetic hash serializes"),
            serde_json::json!(
                "SYNTHETIC_REFERENCE:sha256:cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd"
            )
        );
        assert_eq!(
            serde_json::from_value::<RuntimeNucleusHash>(serde_json::json!(
                "SYNTHETIC_REFERENCE:sha256:cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd"
            ))
            .expect("synthetic hash deserializes"),
            synthetic
        );
        assert!(synthetic.is_synthetic_reference());
        assert_eq!(
            format!("{synthetic:?}"),
            "SYNTHETIC_REFERENCE:sha256:cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd"
        );
    }

    #[test]
    fn runtime_nucleus_hash_rejects_malformed_strings() {
        for value in [
            "SYNTHETIC_REFERENCE:foo",
            "foo",
            "SYNTHETIC_REFERENCE:SYNTHETIC_REFERENCE:sha256:abababababababababababababababababababababababababababababababab",
            "SYNTHETIC_REFERENCE:sha256:ABABABABABABABABABABABABABABABABABABABABABABABABABABABABABABABAB",
        ] {
            assert!(
                serde_json::from_value::<RuntimeNucleusHash>(serde_json::json!(value)).is_err(),
                "{value} must be rejected"
            );
        }
    }

    #[test]
    fn runtime_chrome_budget_accepts_synthetic_reference_hash() {
        let mut value = serde_json::to_value(budget_fixture()).expect("budget serializes");
        value["runtime_nucleus_hash"] = serde_json::json!(
            "SYNTHETIC_REFERENCE:sha256:0101010101010101010101010101010101010101010101010101010101010101"
        );

        let decoded: RuntimeChromeBudget =
            serde_json::from_value(value).expect("synthetic budget deserializes");

        assert_eq!(
            decoded.runtime_nucleus_hash,
            RuntimeNucleusHash::synthetic_reference(Hash256::from_bytes([1; 32]))
        );
    }
}
