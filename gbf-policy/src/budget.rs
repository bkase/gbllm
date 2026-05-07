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
    pub memory_caps: RuntimeMemoryCaps,
    pub wram_reserved: u16,
    pub sram_reserved: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeMemoryCaps {
    pub wram_usable_bytes: u16,
    pub sram_usable_bytes: u32,
    pub hram_usable_bytes: u8,
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
            memory_caps: RuntimeMemoryCaps {
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

        let encoded = serde_json::to_string(&budget).expect("budget serializes");
        let decoded: RuntimeChromeBudget =
            serde_json::from_str(&encoded).expect("budget deserializes");

        assert_eq!(decoded, budget);
    }

    #[test]
    fn budget_rejects_unknown_field() {
        let mut value = serde_json::to_value(budget_fixture()).expect("budget serializes");
        value["unexpected"] = serde_json::json!("nope");

        assert!(serde_json::from_value::<RuntimeChromeBudget>(value).is_err());
    }
}
