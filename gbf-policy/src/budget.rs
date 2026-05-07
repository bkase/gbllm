//! Runtime chrome budget schema.

use std::collections::BTreeSet;

use gbf_foundation::{BudgetSlotId, CompileProfileId, Hash256, TargetProfileId};
use serde::{Deserialize, Serialize};

use crate::compile::PlacementProfile;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeChromeBudget {
    pub target: TargetProfileId,
    pub profile: CompileProfileId,
    pub runtime_nucleus_hash: Hash256,
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

    fn budget_fixture() -> RuntimeChromeBudget {
        RuntimeChromeBudget {
            target: TargetProfileId::from("dmg-mbc5"),
            profile: CompileProfileId::from("Bringup"),
            runtime_nucleus_hash: Hash256::from_bytes([1; 32]),
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
            sram_usable_bytes: 131_072,
            hram_usable_bytes: 300,
            source_target_profile_hash: Hash256::from_bytes([3; 32]),
        };
        let expected = serde_json::json!({
            "wram_usable_bytes": 70000,
            "sram_usable_bytes": 131072,
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
    fn budget_rejects_unknown_field() {
        let mut value = serde_json::to_value(budget_fixture()).expect("budget serializes");
        value["unexpected"] = serde_json::json!("nope");

        assert!(serde_json::from_value::<RuntimeChromeBudget>(value).is_err());
    }
}
