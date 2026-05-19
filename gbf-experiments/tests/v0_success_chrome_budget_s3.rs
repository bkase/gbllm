#![cfg(all(feature = "s3", feature = "s3-phase-d"))]

use gbf_experiments::s3::workload::{
    ConservativeChromeBudget, RomBudgetSlot, conservative_chrome_budget_bytes,
};

#[test]
fn v0_success_chrome_budget_s3() {
    let slots = vec![
        RomBudgetSlot::new("cart-rom", 1_000),
        RomBudgetSlot::new("sram-window", 101),
        RomBudgetSlot::new("register-spare", 1),
    ];

    let expected_single_discount = 900 + 90 + 0;
    assert_eq!(
        conservative_chrome_budget_bytes(&slots),
        expected_single_discount
    );

    let budget = ConservativeChromeBudget::new(slots.clone()).expect("budget builds");
    assert_eq!(
        budget.conservative_chrome_budget_bytes,
        expected_single_discount
    );

    let double_discount = slots
        .iter()
        .map(|slot| (0.90_f64 * (0.90_f64 * slot.default_bytes as f64).floor()).floor() as u64)
        .sum::<u64>();
    assert_ne!(
        budget.conservative_chrome_budget_bytes, double_discount,
        "D15 budget must apply exactly one 0.90 discount"
    );

    assert_eq!(
        budget.chrome_budget_self_hash,
        budget
            .compute_self_hash()
            .expect("budget self-hash computes")
    );
}
