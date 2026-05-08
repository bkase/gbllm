//! Runtime chrome budget fixture loaders.

use gbf_policy::RuntimeChromeBudget;
use sha2::{Digest, Sha256};

pub const BRINGUP_DMG_MBC5_CHROME_BUDGET_JSON: &str =
    include_str!("../../fixtures/runtime-chrome-budget/bringup-dmg-mbc5.chrome_budget.json");
pub const BRINGUP_DMG_MBC5_CHROME_BUDGET_SHA256_SIDECAR: &str =
    include_str!("../../fixtures/runtime-chrome-budget/bringup-dmg-mbc5.chrome_budget.sha256");
pub const BRINGUP_DMG_MBC5_CHROME_BUDGET_SHA256: &str =
    "6b48d1c8711c95456d1b5592ff8ad5a46b26aefa3580c0a30296c7a1b9209bf5";

#[must_use]
pub fn bringup_dmg_mbc5_chrome_budget_fixture() -> RuntimeChromeBudget {
    assert_fixture_hash(
        BRINGUP_DMG_MBC5_CHROME_BUDGET_JSON.as_bytes(),
        BRINGUP_DMG_MBC5_CHROME_BUDGET_SHA256,
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

fn assert_fixture_hash(bytes: &[u8], expected_hex: &str) {
    let actual = hex_sha256(bytes);
    assert_eq!(actual, expected_hex, "runtime chrome budget fixture hash");
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut out, "{byte:02x}").expect("hex write to string cannot fail");
    }
    out
}

#[cfg(test)]
mod tests {
    use gbf_policy::{BudgetSlotClass, PlacementProfile};

    use super::*;

    #[test]
    fn bringup_dmg_mbc5_chrome_budget_fixture_is_pinned() {
        let budget = bringup_dmg_mbc5_chrome_budget_fixture();

        assert_eq!(budget.target.as_str(), "dmg-mbc5-8mib-128kib");
        assert_eq!(budget.profile.as_str(), "Bringup");
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
    }
}
