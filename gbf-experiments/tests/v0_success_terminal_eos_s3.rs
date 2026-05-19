#![cfg(all(feature = "s3", feature = "s3-phase-d"))]

mod v0_success_s3_support;

use gbf_experiments::s3::score::BpcCharValue;
use gbf_experiments::s3::workload::V0SuccessPerSeed;
use v0_success_s3_support::generation_record;

#[test]
fn v0_success_terminal_eos_s3() {
    let terminal_after_200 = generation_record("terminal-200", 200, 2, 1.0, true);
    assert_eq!(terminal_after_200.generated_char_count, 200);
    assert!(terminal_after_200.terminal_eos_seen);
    assert_eq!(
        terminal_after_200.charset_validity_rate, 1.0,
        "terminal EOS is excluded from generated text validity"
    );

    let passing_length = per_seed_with_records(vec![terminal_after_200]);
    assert!(passing_length.Q3_holds);
    assert!(passing_length.Q5_holds);
    assert!(passing_length.pass);

    let premature_eos =
        per_seed_with_records(vec![generation_record("terminal-100", 100, 2, 1.0, true)]);
    assert!(premature_eos.Q3_holds);
    assert!(!premature_eos.Q5_holds);
    assert!(!premature_eos.pass);

    let max_chars_without_eos =
        per_seed_with_records(vec![generation_record("max-chars", 256, 2, 1.0, false)]);
    assert!(!max_chars_without_eos.per_prompt_generation[0].terminal_eos_seen);
    assert!(max_chars_without_eos.Q5_holds);
}

fn per_seed_with_records(
    records: Vec<gbf_experiments::s3::workload::GenerationRecord>,
) -> V0SuccessPerSeed {
    V0SuccessPerSeed::new(0, bpc(1.0), bpc(1.1), 2.0, records, 90, 100)
        .expect("per-seed record builds")
}

fn bpc(value: f64) -> BpcCharValue {
    BpcCharValue::try_new(value).expect("test bpc value is finite and non-negative")
}
