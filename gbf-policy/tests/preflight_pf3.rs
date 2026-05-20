use std::collections::{BTreeSet, HashSet};

use gbf_foundation::{BudgetSlotId, CompileProfileId, Hash256, TargetProfileId};
use gbf_policy::budget::{RomBudgetSlot, RuntimeMemoryCapSection};
use gbf_policy::compile::{
    BRINGUP_COMPILE_PROFILE_ID, BRINGUP_COMPILE_PROFILE_TOML, DEFAULT_COMPILE_PROFILE_ID,
    DEFAULT_COMPILE_PROFILE_TOML, PF3_BRINGUP_WRAM_FIT_REPORT_FIELDS, load_compile_profile_spec,
    s5_pf3_preflight_profile_surface,
};
use gbf_policy::{BudgetSlotClass, PlacementProfile, RuntimeChromeBudget, RuntimeNucleusHash};

#[test]
fn pf3_default_and_bringup_preflight_reports_differ_only_by_profile_wram_fields() {
    let in_budget = in_budget_fixture();
    let default_profile =
        load_compile_profile_spec(DEFAULT_COMPILE_PROFILE_TOML).expect("Default profile parses");
    let bringup_profile =
        load_compile_profile_spec(BRINGUP_COMPILE_PROFILE_TOML).expect("Bringup profile parses");
    assert_eq!(default_profile.id.as_str(), DEFAULT_COMPILE_PROFILE_ID);
    assert_eq!(bringup_profile.id.as_str(), BRINGUP_COMPILE_PROFILE_ID);

    let default_report = s5_pf3_preflight_profile_surface(&in_budget, &default_profile);
    let bringup_report = s5_pf3_preflight_profile_surface(&in_budget, &bringup_profile);

    assert!(default_report.fits_envelope);
    assert!(default_report.hard_failures.is_empty());
    assert!(!default_report.has_bringup_wram_fit_report_fields());
    assert!(bringup_report.fits_envelope);
    assert!(bringup_report.hard_failures.is_empty());
    assert!(bringup_report.has_bringup_wram_fit_report_fields());

    let default_json = serde_json::to_value(&default_report).expect("Default report serializes");
    assert!(
        default_json.get("wram_fit_report").is_none(),
        "Default-profile PF-3 output must not include Bringup-specific WramFitReport fields"
    );

    let bringup_json = serde_json::to_value(&bringup_report).expect("Bringup report serializes");
    let bringup_wram = bringup_json
        .get("wram_fit_report")
        .and_then(serde_json::Value::as_object)
        .expect("Bringup report includes WramFitReport object");
    let expected_fields = PF3_BRINGUP_WRAM_FIT_REPORT_FIELDS
        .into_iter()
        .collect::<HashSet<_>>();
    let observed_fields = bringup_wram
        .keys()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    assert_eq!(observed_fields, expected_fields);

    assert_eq!(bringup_wram["overlay_bytes"], 4096);
    assert_eq!(bringup_wram["continuation_bytes"], 256);
    assert_eq!(bringup_wram["stack_bytes"], 256);
    assert_eq!(bringup_wram["hot_arena_bytes_min"], 2048);
    assert_eq!(bringup_wram["reserve_bytes"], 1536);
}

fn in_budget_fixture() -> RuntimeChromeBudget {
    RuntimeChromeBudget {
        target: TargetProfileId::from("dmg-mbc5-8mib-128kib"),
        profile: CompileProfileId::from(BRINGUP_COMPILE_PROFILE_ID),
        runtime_nucleus_hash: RuntimeNucleusHash::real(Hash256::from_bytes([0x51; 32])),
        rom_slots: vec![
            RomBudgetSlot {
                id: BudgetSlotId::new(0),
                class: BudgetSlotClass::Bank0Free,
                usable_bytes: 8 * 1024,
                reserved_slack: 256,
                placement_caps: BTreeSet::from([PlacementProfile::Budgeted]),
            },
            RomBudgetSlot {
                id: BudgetSlotId::new(7),
                class: BudgetSlotClass::ExpertBank,
                usable_bytes: 16 * 1024,
                reserved_slack: 384,
                placement_caps: BTreeSet::from([
                    PlacementProfile::StrictOnePerBank,
                    PlacementProfile::Budgeted,
                ]),
            },
        ],
        memory_caps: RuntimeMemoryCapSection {
            wram_usable_bytes: 8 * 1024,
            sram_usable_bytes: 32 * 1024,
            hram_usable_bytes: 127,
            source_target_profile_hash: Hash256::from_bytes([0x09; 32]),
        },
        wram_reserved: 128,
        sram_reserved: 512,
    }
}
