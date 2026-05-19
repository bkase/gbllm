#![cfg(all(feature = "s3", feature = "s3-phase-d"))]

mod v0_success_s3_support;

use gbf_experiments::s3::score::BpcCharValue;
use gbf_experiments::s3::workload::{
    V0_SUCCESS_MIN_BPC_GAIN_VS_KN5, V0SuccessPerSeed, aggregate_generation_records,
};
use v0_success_s3_support::generation_record;

#[test]
fn v0_success_per_prompt_aggregation_s3() {
    let records = vec![
        generation_record("prompt-a", 128, 3, 1.0, true),
        generation_record("prompt-b", 64, 9, 1.0, true),
        generation_record("prompt-c", 32, 2, 0.5, true),
    ];

    let aggregate = aggregate_generation_records(&records);
    let expected_validity = (128.0 + 64.0 + 16.0) / (128.0 + 64.0 + 32.0);
    assert_close(
        aggregate.generated_token_charset_validity_rate,
        expected_validity,
    );
    assert_eq!(aggregate.max_consecutive_same_token, 9);
    assert_eq!(aggregate.min_generated_char_count, 32);

    let per_seed = V0SuccessPerSeed::new(0, bpc(1.0), bpc(1.1), 2.0, records, 90, 100)
        .expect("per-seed quality record builds");

    assert!(per_seed.Q1_holds);
    assert!(per_seed.Q2_holds);
    assert!(!per_seed.Q3_holds, "Q3 is weighted across prompts");
    assert!(!per_seed.Q4_holds, "Q4 uses max over prompts");
    assert!(!per_seed.Q5_holds, "Q5 uses min over prompts");
    assert!(per_seed.Q6_holds);
    assert!(!per_seed.pass);
}

#[test]
fn v0_success_q1_uses_measured_baseline_margin_s3() {
    let records = vec![generation_record("prompt-a", 128, 1, 1.0, true)];

    let below_threshold = V0SuccessPerSeed::new(
        0,
        bpc(1.0),
        bpc(1.1),
        1.0 + V0_SUCCESS_MIN_BPC_GAIN_VS_KN5 - 1.0e-9,
        records.clone(),
        90,
        100,
    )
    .expect("per-seed record builds");
    assert!(!below_threshold.Q1_holds);

    let independently_above_threshold = V0SuccessPerSeed::new(
        0,
        bpc(1.0),
        bpc(1.1),
        1.0 + V0_SUCCESS_MIN_BPC_GAIN_VS_KN5 + 1.0e-9,
        records,
        90,
        100,
    )
    .expect("per-seed record builds");
    assert!(independently_above_threshold.Q1_holds);
}

#[test]
fn v0_success_q3_requires_exact_full_charset_validity_s3() {
    let almost_valid = vec![generation_record(
        "prompt-a",
        128,
        1,
        0.999_999_999_999,
        true,
    )];
    let per_seed = V0SuccessPerSeed::new(0, bpc(1.0), bpc(1.1), 2.0, almost_valid, 90, 100)
        .expect("per-seed record builds");
    assert!(
        !per_seed.Q3_holds,
        "Q3 requires exact full validity, not an epsilon-rounded aggregate"
    );

    let all_valid = vec![generation_record("prompt-a", 128, 1, 1.0, true)];
    let per_seed = V0SuccessPerSeed::new(0, bpc(1.0), bpc(1.1), 2.0, all_valid, 90, 100)
        .expect("per-seed record builds");
    assert!(per_seed.Q3_holds);
}

fn bpc(value: f64) -> BpcCharValue {
    BpcCharValue::try_new(value).expect("test bpc value is finite and non-negative")
}

fn assert_close(actual: f64, expected: f64) {
    let delta = (actual - expected).abs();
    assert!(
        delta <= 1.0e-12,
        "expected {actual} to be within 1e-12 of {expected}; delta={delta}"
    );
}
