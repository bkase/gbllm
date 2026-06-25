use std::collections::BTreeSet;

use gbf_foundation::{BudgetSlotId, ByteCost, CompileProfileId, Hash256, TargetProfileId};
use gbf_policy::model_profile::ModelSizeProfile;
use gbf_policy::{
    BudgetSlotClass, PlacementProfile, ReValidationOutcome, RomBudgetSlot, RuntimeChromeBudget,
    RuntimeMemoryCapSection, RuntimeNucleusHash, WramReserved, bringup_profile,
    revalidate_runtime_chrome_budget,
};
use gbf_test::runtime_chrome_budget::{
    BRINGUP_DMG_MBC5_RUNTIME_NUCLEUS_HASH, bringup_dmg_mbc5_chrome_budget_fixture,
};
use gbf_train::preflight::{
    RuntimeChromePreflightCadence, RuntimeChromePreflightCheck, RuntimeChromePreflightCheckKind,
    RuntimeChromePreflightDemand, RuntimeChromePreflightReport, RuntimeChromePreflightStatus,
    preflight_runtime_chrome_budget, preflight_runtime_chrome_budget_with_profile,
};
use serde_json::json;

#[test]
fn budget_tests_runtime_chrome_budget_fixture_round_trips_and_pins_public_json_shape() {
    let budget = bringup_dmg_mbc5_chrome_budget_fixture();

    let value = serde_json::to_value(&budget).expect("budget serializes");
    assert_eq!(value["target"], json!("dmg-mbc5-8mib-128kib"));
    assert_eq!(value["profile"], json!("Bringup"));
    assert_eq!(
        value["runtime_nucleus_hash"],
        json!(BRINGUP_DMG_MBC5_RUNTIME_NUCLEUS_HASH)
    );
    assert_eq!(value["rom_slots"][0]["class"], json!({"kind": "Bank0Free"}));
    assert_eq!(value["rom_slots"][0]["usable_bytes"], json!(1024));
    assert_eq!(
        value["rom_slots"][1]["class"],
        json!({"kind": "ExpertBank"})
    );
    assert_eq!(value["memory_caps"]["wram_usable_bytes"], json!(8192));
    assert_eq!(value["wram_reserved"]["hot_arena_floor"], json!(4096));
    assert_eq!(value["sram_reserved"], json!(512));

    let decoded: RuntimeChromeBudget =
        serde_json::from_value(value).expect("budget deserializes through policy schema");
    assert_eq!(decoded, budget);
    assert_eq!(decoded.rom_slots[0].class, BudgetSlotClass::Bank0Free);
    assert_eq!(decoded.rom_slots[1].class, BudgetSlotClass::ExpertBank);
}

#[test]
fn budget_tests_runtime_chrome_preflight_pass_case_reports_structured_margins() {
    let budget = runtime_budget_fixture(runtime_hash(0x41), 16_384, 384, 4);
    let demand = RuntimeChromePreflightDemand::new(
        ModelSizeProfile::moe_tiny(4).expect("supported profile"),
        ByteCost::new(12_000),
        ByteCost::new(2_048),
        ByteCost::new(3_712),
        ByteCost::new(380),
        ByteCost::new(768),
    );

    let report = preflight_runtime_chrome_budget(&budget, demand);

    assert!(report.fits_envelope());
    assert!(report.hard_failures().is_empty());
    assert_eq!(report.required_expert_slots(), 4);
    assert_eq!(report.available_expert_bank_slots(), 4);
    assert_eq!(report.checks().len(), 6);

    let expert = check(&report, RuntimeChromePreflightCheckKind::ExpertBank);
    assert_eq!(expert.status(), RuntimeChromePreflightStatus::Pass);
    assert_eq!(expert.required_bytes(), ByteCost::new(4_530));
    assert_eq!(expert.available_bytes(), ByteCost::new(16_000));
    assert_eq!(expert.slack_bytes(), ByteCost::new(11_470));
    assert!(expert.diagnostic().contains("PASS with 11470 bytes margin"));

    let bank0 = check(&report, RuntimeChromePreflightCheckKind::Bank0Free);
    assert_eq!(bank0.status(), RuntimeChromePreflightStatus::Pass);
    assert_eq!(bank0.available_bytes(), ByteCost::new(7_680));
}

#[test]
fn budget_tests_runtime_chrome_preflight_fail_case_reports_hard_failures_and_diagnostics() {
    let budget = runtime_budget_fixture(runtime_hash(0x42), 512, 0, 4);
    let demand = RuntimeChromePreflightDemand::new(
        ModelSizeProfile::upper_bank_candidate(128, 4).expect("supported upper-bank profile"),
        ByteCost::new(16_500),
        ByteCost::new(9_000),
        ByteCost::new(4_097),
        ByteCost::new(513),
        ByteCost::new(1_025),
    );

    let report = preflight_runtime_chrome_budget(&budget, demand);

    assert!(!report.fits_envelope());
    assert!(!report.hard_failures().is_empty());

    let expert = check(&report, RuntimeChromePreflightCheckKind::ExpertBank);
    assert_eq!(expert.status(), RuntimeChromePreflightStatus::Fail);
    assert_eq!(expert.required_bytes(), ByteCost::new(12_978));
    assert_eq!(expert.available_bytes(), ByteCost::new(512));
    assert_eq!(expert.over_by_bytes(), ByteCost::new(12_466));
    assert!(expert.diagnostic().contains("d_model=128 d_ff=192"));
    assert!(expert.diagnostic().contains("FAIL by 12466 bytes"));

    let common = check(&report, RuntimeChromePreflightCheckKind::CommonBank);
    assert_eq!(common.status(), RuntimeChromePreflightStatus::Fail);
    assert_eq!(common.over_by_bytes(), ByteCost::new(116));

    let sram = check(&report, RuntimeChromePreflightCheckKind::Sram);
    assert_eq!(sram.status(), RuntimeChromePreflightStatus::Fail);
    assert_eq!(sram.over_by_bytes(), ByteCost::new(1));
}

#[test]
fn budget_tests_compile_profile_mismatch_is_a_deployability_failure() {
    let budget = runtime_budget_fixture(runtime_hash(0x43), 16_384, 384, 4);
    let mut detached_profile = bringup_profile().clone();
    detached_profile.id = CompileProfileId::from("Detached");
    let demand = RuntimeChromePreflightDemand::new(
        ModelSizeProfile::moe_tiny(4).expect("supported profile"),
        ByteCost::new(12_000),
        ByteCost::new(2_048),
        ByteCost::new(3_712),
        ByteCost::new(380),
        ByteCost::new(768),
    );

    let report = preflight_runtime_chrome_budget_with_profile(
        &budget,
        &detached_profile,
        demand,
        RuntimeChromePreflightCadence::PreTraining,
    );

    assert!(!report.fits_envelope());
    assert_eq!(report.hard_failures().len(), 1);
    assert!(report.hard_failures()[0].contains(
        "CompileProfile profile Detached does not match RuntimeChromeBudget profile Bringup"
    ));
}

#[test]
fn budget_tests_runtime_chrome_revalidation_warns_for_stale_hash_and_blocks_failed_fit() {
    let training = runtime_budget_fixture(runtime_hash(0x51), 16_384, 384, 4);
    let mut current = runtime_budget_fixture(runtime_hash(0x52), 16_384, 384, 4);
    current.rom_slots[2].usable_bytes += 128;

    let warning = revalidate_runtime_chrome_budget(&training, &current, true);

    assert!(!warning.runtime_nucleus_hashes_match);
    assert_eq!(warning.outcome, ReValidationOutcome::Warn);
    assert!(
        warning
            .diagnostic
            .contains("training_runtime_nucleus_hash=sha256:515151")
    );
    assert!(
        warning
            .diagnostic
            .contains("current_runtime_nucleus_hash=sha256:525252")
    );
    assert!(
        warning
            .per_slot_byte_deltas
            .iter()
            .any(|delta| delta.delta_bytes == 128)
    );

    let blocked = revalidate_runtime_chrome_budget(&training, &current, false);

    assert_eq!(blocked.outcome, ReValidationOutcome::BlockExport);
    assert!(blocked.diagnostic.contains("fits_envelope=false"));
}

fn check(
    report: &RuntimeChromePreflightReport,
    kind: RuntimeChromePreflightCheckKind,
) -> &RuntimeChromePreflightCheck {
    report
        .checks()
        .iter()
        .find(|check| check.kind() == kind)
        .expect("check kind is present")
}

fn hash(byte: u8) -> Hash256 {
    Hash256::from_bytes([byte; 32])
}

fn runtime_hash(byte: u8) -> RuntimeNucleusHash {
    RuntimeNucleusHash::real(hash(byte))
}

fn runtime_budget_fixture(
    runtime_nucleus_hash: RuntimeNucleusHash,
    expert_slot_usable_bytes: u32,
    expert_reserved_slack: u16,
    expert_slots: u16,
) -> RuntimeChromeBudget {
    let mut rom_slots = vec![
        RomBudgetSlot::new(
            BudgetSlotId::new(0),
            BudgetSlotClass::Bank0Free,
            8 * 1024,
            512,
            BTreeSet::from([PlacementProfile::StrictOnePerBank]),
        )
        .expect("valid Bank0Free slot"),
        RomBudgetSlot::new(
            BudgetSlotId::new(1),
            BudgetSlotClass::CommonBank,
            16 * 1024,
            512,
            BTreeSet::from([PlacementProfile::Budgeted]),
        )
        .expect("valid CommonBank slot"),
    ];

    for offset in 0..expert_slots {
        rom_slots.push(
            RomBudgetSlot::new(
                BudgetSlotId::new(2 + offset),
                BudgetSlotClass::ExpertBank,
                expert_slot_usable_bytes,
                expert_reserved_slack,
                BTreeSet::from([
                    PlacementProfile::StrictOnePerBank,
                    PlacementProfile::Budgeted,
                ]),
            )
            .expect("valid ExpertBank slot"),
        );
    }

    RuntimeChromeBudget::new(
        TargetProfileId::from("dmg-mbc5-8mib-128kib"),
        CompileProfileId::from("Bringup"),
        runtime_nucleus_hash,
        RuntimeChromeBudget::pinned_reference_shell_modules(),
        rom_slots,
        RuntimeMemoryCapSection {
            wram_usable_bytes: 8 * 1024,
            sram_usable_bytes: 32 * 1024,
            hram_usable_bytes: 127,
            source_target_profile_hash: hash(0x09),
        },
        WramReserved::new(512, 4_096, 1_536).expect("valid WRAM reservation"),
        1_024,
    )
    .expect("valid RuntimeChromeBudget")
}
