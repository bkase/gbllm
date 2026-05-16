#![cfg(all(feature = "s3", feature = "s3-phase-d"))]

mod v0_success_s3_support;

use gbf_experiments::s3::workload::{GenerationRecord, aggregate_generation_records};
use proptest::prelude::*;
use v0_success_s3_support::generation_record;

proptest! {
    #[test]
    fn v0_success_proptest_s3(records in prop::collection::vec(arbitrary_per_prompt_record(), 1..8)) {
        let aggregate = aggregate_generation_records(&records);

        let expected_max_run = records
            .iter()
            .map(|record| record.max_consecutive_same_token)
            .max()
            .expect("strategy creates at least one record");
        let expected_min_count = records
            .iter()
            .map(|record| record.generated_char_count)
            .min()
            .expect("strategy creates at least one record");
        let total_chars = records
            .iter()
            .map(|record| u64::from(record.generated_char_count))
            .sum::<u64>();
        let expected_validity = if total_chars == 0 {
            1.0
        } else {
            records
                .iter()
                .map(|record| record.charset_validity_rate * f64::from(record.generated_char_count))
                .sum::<f64>() / total_chars as f64
        };

        prop_assert_eq!(aggregate.max_consecutive_same_token, expected_max_run);
        prop_assert_eq!(aggregate.min_generated_char_count, expected_min_count);
        prop_assert!(
            (aggregate.generated_token_charset_validity_rate - expected_validity).abs() <= 1.0e-12,
            "weighted validity differed: actual={} expected={}",
            aggregate.generated_token_charset_validity_rate,
            expected_validity,
        );
    }
}

fn arbitrary_per_prompt_record() -> impl Strategy<Value = GenerationRecord> {
    (
        0_u32..1000,
        0_u32..260,
        0_u32..20,
        prop::sample::select(vec![0.0_f64, 0.25, 0.5, 0.75, 1.0]),
        any::<bool>(),
    )
        .prop_map(|(id, generated_char_count, max_run, validity, eos)| {
            generation_record(
                &format!("prompt-{id}"),
                generated_char_count,
                max_run,
                validity,
                eos,
            )
        })
}
