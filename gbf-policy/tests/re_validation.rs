use std::collections::BTreeSet;

use gbf_foundation::{BudgetSlotId, CompileProfileId, Hash256, TargetProfileId};
use gbf_policy::{
    BudgetSlotClass, PlacementProfile, ReValidationOutcome, RomBudgetSlot, RuntimeChromeBudget,
    RuntimeMemoryCapSection, RuntimeNucleusHash, revalidate_runtime_chrome_budget,
};

fn hash(byte: u8) -> Hash256 {
    Hash256::from_bytes([byte; 32])
}

fn runtime_hash(byte: u8) -> RuntimeNucleusHash {
    RuntimeNucleusHash::real(hash(byte))
}

fn budget(runtime_nucleus_hash: RuntimeNucleusHash, usable_bytes: u32) -> RuntimeChromeBudget {
    RuntimeChromeBudget {
        target: TargetProfileId::from("dmg-mbc5-8mib-128kib"),
        profile: CompileProfileId::from("Bringup"),
        runtime_nucleus_hash,
        rom_slots: vec![RomBudgetSlot {
            id: BudgetSlotId::new(7),
            class: BudgetSlotClass::ExpertBank,
            usable_bytes,
            reserved_slack: 384,
            placement_caps: BTreeSet::from([PlacementProfile::Budgeted]),
        }],
        memory_caps: RuntimeMemoryCapSection {
            wram_usable_bytes: 8192,
            sram_usable_bytes: 32768,
            hram_usable_bytes: 127,
            source_target_profile_hash: hash(0x09),
        },
        wram_reserved: 128,
        sram_reserved: 512,
    }
}

#[test]
fn hash_drift_with_delta_at_d9_tolerance_warns() {
    let training = budget(runtime_hash(1), 1024);
    let current = budget(runtime_hash(2), 1024 + 256);

    let report = revalidate_runtime_chrome_budget(&training, &current, true);

    assert_eq!(report.outcome, ReValidationOutcome::Warn);
    assert_eq!(report.per_slot_byte_deltas[0].delta_bytes, 256);
}

#[test]
fn hash_drift_with_delta_above_d9_tolerance_blocks_export() {
    let training = budget(runtime_hash(1), 1024);
    let current = budget(runtime_hash(2), 1024 + 257);

    let report = revalidate_runtime_chrome_budget(&training, &current, true);

    assert_eq!(report.outcome, ReValidationOutcome::BlockExport);
    assert!(report.diagnostic.contains("offending_slot_id=7"));
}

#[test]
fn matching_hashes_with_delta_above_d9_tolerance_passes() {
    let training = budget(runtime_hash(1), 1024);
    let current = budget(runtime_hash(1), 1024 + 257);

    let report = revalidate_runtime_chrome_budget(&training, &current, true);

    assert!(report.runtime_nucleus_hashes_match);
    assert_eq!(report.outcome, ReValidationOutcome::Pass);
    assert_eq!(report.per_slot_byte_deltas[0].delta_bytes, 257);
}

#[test]
fn failed_fit_envelope_blocks_export() {
    let training = budget(runtime_hash(1), 1024);
    let current = budget(runtime_hash(2), 1024 + 128);

    let report = revalidate_runtime_chrome_budget(&training, &current, false);

    assert_eq!(report.outcome, ReValidationOutcome::BlockExport);
    assert!(report.diagnostic.contains("fits_envelope=false"));
}
