//! Deterministic synthetic runtime-chrome budget for pre-runtime training.

use std::collections::BTreeSet;

use gbf_abi::RuntimeShellModule;
use gbf_foundation::{BudgetSlotId, CompileProfileId, Hash256, TargetProfileId};
use serde::de::Error as _;

use crate::budget::{
    BudgetSlotClass, RomBudgetSlot, RuntimeChromeBudget, RuntimeChromeBudgetValidationError,
    RuntimeMemoryCapSection, RuntimeNucleusHash,
};
use crate::compile::{BRINGUP_COMPILE_PROFILE_ID, PlacementProfile};
use crate::reference_shell::pinned_reference_shell;
use crate::wram::WramReserved;

pub const SYNTHETIC_REFERENCE_BUDGET_FIXTURE_PATH: &str =
    "fixtures/calibration/synthetic_reference_budget.json";
pub const SYNTHETIC_REFERENCE_BUDGET_JSON: &str =
    include_str!("../../fixtures/calibration/synthetic_reference_budget.json");

pub const SYNTHETIC_BANK0_FREE_USABLE_BYTES: u32 = 8 * 1024;
pub const SYNTHETIC_BANK0_RESERVED_SLACK_BYTES: u16 = 1_152;
pub const SYNTHETIC_COMMON_BANK_USABLE_BYTES: u32 = 16 * 1024;
pub const SYNTHETIC_COMMON_BANK_RESERVED_SLACK_BYTES: u16 = 512;
pub const SYNTHETIC_EXPERT_BANK_COUNT: u8 = 8;
pub const SYNTHETIC_EXPERT_BANK_USABLE_BYTES: u32 = 16 * 1024;
pub const SYNTHETIC_EXPERT_BANK_RESERVED_SLACK_BYTES: u16 = 384;
pub const SYNTHETIC_WRAM_OVERLAY_BYTES: u16 = 512;
pub const SYNTHETIC_WRAM_HOT_ARENA_FLOOR_BYTES: u16 = 4_096;
pub const SYNTHETIC_WRAM_RESERVED_TOTAL_BYTES: u16 = 1_536;
pub const SYNTHETIC_SRAM_RESERVED_BYTES: u32 = 1_024;

/// Stable sentinel digest for `gbf-policy:synthetic-reference-runtime-chrome-budget:v1`.
pub const SYNTHETIC_REFERENCE_RUNTIME_NUCLEUS_HASH_BYTES: [u8; 32] = [
    0x8e, 0xd6, 0xd5, 0xbd, 0x8a, 0x9a, 0x2b, 0x64, 0x16, 0xf9, 0xde, 0x96, 0x7a, 0x41, 0xf6, 0x11,
    0x5d, 0xa4, 0x1b, 0x53, 0x46, 0xf1, 0x16, 0x7a, 0x52, 0xc2, 0x3f, 0xeb, 0xf2, 0xea, 0x1d, 0xdc,
];

pub fn synthetic_reference_runtime_chrome_budget()
-> Result<RuntimeChromeBudget, RuntimeChromeBudgetValidationError> {
    let mut rom_slots = vec![
        RomBudgetSlot::new(
            BudgetSlotId::new(0),
            BudgetSlotClass::Bank0Free,
            SYNTHETIC_BANK0_FREE_USABLE_BYTES,
            SYNTHETIC_BANK0_RESERVED_SLACK_BYTES,
            BTreeSet::from([PlacementProfile::StrictOnePerBank]),
        )?,
        RomBudgetSlot::new(
            BudgetSlotId::new(1),
            BudgetSlotClass::CommonBank,
            SYNTHETIC_COMMON_BANK_USABLE_BYTES,
            SYNTHETIC_COMMON_BANK_RESERVED_SLACK_BYTES,
            BTreeSet::from([PlacementProfile::Budgeted]),
        )?,
    ];

    for slot_id in 2..(2 + SYNTHETIC_EXPERT_BANK_COUNT) {
        rom_slots.push(RomBudgetSlot::new(
            BudgetSlotId::new(u16::from(slot_id)),
            BudgetSlotClass::ExpertBank,
            SYNTHETIC_EXPERT_BANK_USABLE_BYTES,
            SYNTHETIC_EXPERT_BANK_RESERVED_SLACK_BYTES,
            BTreeSet::from([
                PlacementProfile::StrictOnePerBank,
                PlacementProfile::Budgeted,
            ]),
        )?);
    }

    RuntimeChromeBudget::new(
        TargetProfileId::from("dmg-mbc5-8mib-128kib"),
        CompileProfileId::from(BRINGUP_COMPILE_PROFILE_ID),
        RuntimeNucleusHash::synthetic_reference(Hash256::from_bytes(
            SYNTHETIC_REFERENCE_RUNTIME_NUCLEUS_HASH_BYTES,
        )),
        synthetic_reference_shell_modules(),
        rom_slots,
        RuntimeMemoryCapSection {
            wram_usable_bytes: gbf_hw::memory::WRAM_SIZE_BYTES,
            sram_usable_bytes: 32 * 1024,
            hram_usable_bytes: 127,
            source_target_profile_hash: Hash256::from_bytes([
                0x64, 0xa3, 0x47, 0x99, 0x18, 0x11, 0xc5, 0xdb, 0x12, 0xb7, 0xbc, 0x17, 0xdc, 0x28,
                0x02, 0xd6, 0x17, 0xb4, 0x61, 0xc6, 0x10, 0xcc, 0xde, 0x6e, 0xf8, 0x1a, 0x22, 0xa1,
                0xc2, 0x89, 0x47, 0xc7,
            ]),
        },
        WramReserved::new(
            SYNTHETIC_WRAM_OVERLAY_BYTES,
            SYNTHETIC_WRAM_HOT_ARENA_FLOOR_BYTES,
            SYNTHETIC_WRAM_RESERVED_TOTAL_BYTES,
        )
        .expect("synthetic WRAM reservation constants are valid"),
        SYNTHETIC_SRAM_RESERVED_BYTES,
    )
}

pub fn synthetic_reference_runtime_chrome_budget_from_fixture()
-> Result<RuntimeChromeBudget, serde_json::Error> {
    serde_json::from_str(SYNTHETIC_REFERENCE_BUDGET_JSON)
}

#[must_use]
pub fn synthetic_reference_shell_modules() -> BTreeSet<RuntimeShellModule> {
    RuntimeChromeBudget::pinned_reference_shell_modules()
}

#[must_use]
pub fn synthetic_reference_future_module_slack_bytes() -> u16 {
    pinned_reference_shell()
        .future_reservations
        .values()
        .map(|reservation| reservation.rom_bytes_per_bank0)
        .sum()
}

pub fn synthetic_reference_module_binding_hash() -> Result<Hash256, serde_json::Error> {
    let budget = synthetic_reference_runtime_chrome_budget().map_err(serde_json::Error::custom)?;
    RuntimeChromeBudget::runtime_nucleus_module_binding_hash(
        budget.runtime_nucleus_hash.hash(),
        &synthetic_reference_shell_modules(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_sha256_hex() -> &'static str {
        include_str!("../../fixtures/calibration/synthetic_reference_budget.sha256")
            .split_whitespace()
            .next()
            .expect("fixture sha line has digest")
    }

    #[test]
    fn synthetic_round_trip_matches_checked_in_fixture() {
        let helper = synthetic_reference_runtime_chrome_budget().expect("helper budget is valid");
        let fixture =
            synthetic_reference_runtime_chrome_budget_from_fixture().expect("fixture parses");

        assert_eq!(helper, fixture);
        assert_eq!(
            serde_json::from_value::<RuntimeChromeBudget>(
                serde_json::to_value(&helper).expect("budget serializes")
            )
            .expect("serialized budget deserializes"),
            helper
        );
    }

    #[test]
    fn synthetic_fixture_hash_is_pinned() {
        assert_eq!(
            gbf_foundation::sha256(SYNTHETIC_REFERENCE_BUDGET_JSON.as_bytes()).to_hex(),
            fixture_sha256_hex()
        );
    }

    #[test]
    fn synthetic_runtime_hash_is_marked_synthetic() {
        let budget = synthetic_reference_runtime_chrome_budget().expect("helper budget is valid");

        assert!(budget.runtime_nucleus_hash.is_synthetic_reference());
        assert_eq!(
            serde_json::to_value(budget.runtime_nucleus_hash).expect("hash serializes"),
            serde_json::json!(
                "SYNTHETIC_REFERENCE:sha256:8ed6d5bd8a9a2b6416f9de967a41f6115da41b5346f1167a52c23febf2ea1ddc"
            )
        );
    }

    #[test]
    fn synthetic_slots_pin_capacity_and_slack() {
        let budget = synthetic_reference_runtime_chrome_budget().expect("helper budget is valid");

        assert_eq!(budget.rom_slots.len(), 10);
        assert_eq!(budget.rom_slots[0].class, BudgetSlotClass::Bank0Free);
        assert_eq!(budget.rom_slots[0].usable_bytes, 8 * 1024);
        assert_eq!(
            budget.rom_slots[0].reserved_slack,
            synthetic_reference_future_module_slack_bytes()
        );
        assert_eq!(budget.rom_slots[1].class, BudgetSlotClass::CommonBank);
        assert_eq!(budget.rom_slots[1].usable_bytes, 16 * 1024);
        assert_eq!(budget.rom_slots[1].reserved_slack, 512);
        assert!(
            budget.rom_slots[2..]
                .iter()
                .all(|slot| slot.class == BudgetSlotClass::ExpertBank
                    && slot.usable_bytes == 16 * 1024
                    && slot.reserved_slack == 384)
        );
    }

    #[test]
    fn synthetic_wram_uses_structured_migrated_schema() {
        let budget = synthetic_reference_runtime_chrome_budget().expect("helper budget is valid");

        assert_eq!(
            budget.wram_reserved,
            WramReserved::new(512, 4096, 1536).expect("valid WRAM reservation")
        );

        let value = serde_json::to_value(&budget).expect("budget serializes");
        assert_eq!(
            value["wram_reserved"],
            serde_json::json!({
                "overlay": 512,
                "hot_arena_floor": 4096,
                "total": 1536
            })
        );
        assert_eq!(
            value["reference_shell_modules"],
            serde_json::json!([
                "boot",
                "interrupts",
                "scheduler",
                "banking",
                "joypad",
                "text",
                "keyboard",
                "video_commit"
            ])
        );
    }

    #[test]
    fn synthetic_reference_modules_and_binding_hash_are_deterministic() {
        let modules = synthetic_reference_shell_modules();
        assert_eq!(
            modules,
            BTreeSet::from([
                RuntimeShellModule::Boot,
                RuntimeShellModule::Interrupts,
                RuntimeShellModule::Scheduler,
                RuntimeShellModule::Banking,
                RuntimeShellModule::Joypad,
                RuntimeShellModule::Text,
                RuntimeShellModule::Keyboard,
                RuntimeShellModule::VideoCommit,
            ])
        );

        let base = synthetic_reference_module_binding_hash().expect("binding hash computes");
        let repeated = synthetic_reference_module_binding_hash().expect("binding hash recomputes");
        assert_eq!(base, repeated);

        let budget = synthetic_reference_runtime_chrome_budget().expect("helper budget is valid");
        assert_eq!(budget.reference_shell_modules, modules);

        let mut expanded = modules;
        expanded.insert(RuntimeShellModule::Panic);
        let changed = RuntimeChromeBudget::runtime_nucleus_module_binding_hash(
            budget.runtime_nucleus_hash.hash(),
            &expanded,
        )
        .expect("expanded binding hash computes");

        assert_ne!(base, changed);
    }
}
