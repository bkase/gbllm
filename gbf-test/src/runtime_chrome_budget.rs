//! Runtime chrome budget fixture loaders.

use gbf_policy::RuntimeChromeBudget;

use crate::helpers::assert_fixture_hash;

pub const BRINGUP_DMG_MBC5_CHROME_BUDGET_JSON: &str =
    include_str!("../../fixtures/runtime-chrome-budget/bringup-dmg-mbc5.chrome_budget.json");
pub const BRINGUP_DMG_MBC5_CHROME_BUDGET_SHA256_SIDECAR: &str =
    include_str!("../../fixtures/runtime-chrome-budget/bringup-dmg-mbc5.chrome_budget.sha256");
pub const BRINGUP_DMG_MBC5_CHROME_BUDGET_SHA256: &str =
    "cf5ef1f9777075fdc202f66be7c53cbd5193cc0225f51db92cb330bd9f0a4eb9";
pub const BRINGUP_DMG_MBC5_RUNTIME_NUCLEUS_HASH: &str =
    "sha256:2a1fc3405e389733a0006c5b1e6a314a7d81fbc671466a3bc02cdbb876cd1ec5";

#[must_use]
pub fn bringup_dmg_mbc5_chrome_budget_fixture() -> RuntimeChromeBudget {
    assert_fixture_hash(
        BRINGUP_DMG_MBC5_CHROME_BUDGET_JSON.as_bytes(),
        BRINGUP_DMG_MBC5_CHROME_BUDGET_SHA256,
        "runtime chrome budget",
    );
    assert_eq!(
        BRINGUP_DMG_MBC5_CHROME_BUDGET_SHA256_SIDECAR
            .split_whitespace()
            .next(),
        Some(BRINGUP_DMG_MBC5_CHROME_BUDGET_SHA256),
        "runtime chrome budget sidecar hash matches loader constant",
    );
    serde_json::from_str(BRINGUP_DMG_MBC5_CHROME_BUDGET_JSON)
        .expect("bringup DMG/MBC5 runtime chrome budget fixture deserializes")
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use crate::calibration::bootstrap_dmg_mbc5_target_profile_hash;
    use gbf_foundation::Hash256;
    use gbf_policy::{BudgetSlotClass, PlacementProfile};

    use super::*;

    #[test]
    fn bringup_dmg_mbc5_chrome_budget_fixture_is_pinned() {
        let budget = bringup_dmg_mbc5_chrome_budget_fixture();

        assert_eq!(budget.target.as_str(), "dmg-mbc5-8mib-128kib");
        assert_eq!(budget.profile.as_str(), "Bringup");
        assert_eq!(
            budget.runtime_nucleus_hash,
            Hash256::from_str(BRINGUP_DMG_MBC5_RUNTIME_NUCLEUS_HASH)
                .expect("pinned runtime nucleus hash is valid"),
        );
        assert_eq!(budget.rom_slots.len(), 2);
        assert_eq!(budget.rom_slots[0].class, BudgetSlotClass::Bank0Free);
        assert_eq!(budget.rom_slots[0].reserved_slack, 64);
        assert!(
            budget.rom_slots[0]
                .placement_caps
                .contains(&PlacementProfile::StrictOnePerBank)
        );
        assert_eq!(budget.rom_slots[1].class, BudgetSlotClass::ExpertBank);
        assert_eq!(budget.rom_slots[1].usable_bytes, 16_384);
        assert_eq!(budget.rom_slots[1].reserved_slack, 128);
        assert!(
            budget.rom_slots[1]
                .placement_caps
                .contains(&PlacementProfile::Budgeted)
        );
        assert_eq!(
            budget.memory_caps.source_target_profile_hash,
            bootstrap_dmg_mbc5_target_profile_hash(),
        );
        assert_eq!(budget.wram_reserved, 128);
        assert_eq!(budget.sram_reserved, 512);
    }
}
