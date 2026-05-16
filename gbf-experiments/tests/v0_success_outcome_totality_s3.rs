#![cfg(all(feature = "s3", feature = "s3-phase-d"))]

mod v0_success_s3_support;

use gbf_experiments::s3::score::BpcCharValue;
use gbf_experiments::s3::workload::{V0SuccessPerSeed, V0SuccessProduct};
use proptest::prelude::*;
use v0_success_s3_support::passing_per_seed;

#[test]
fn v0_success_outcome_totality_s3() {
    for bits in 0_u8..64 {
        let q = [
            bits & 0b00_0001 != 0,
            bits & 0b00_0010 != 0,
            bits & 0b00_0100 != 0,
            bits & 0b00_1000 != 0,
            bits & 0b01_0000 != 0,
            bits & 0b10_0000 != 0,
        ];
        assert_totality(q);
    }

    let passing = product_from_seeds((0..5).map(passing_per_seed).collect());
    assert!(passing.overall_pass);
    assert!(!passing.suspicious_low_bpc);

    let mut one_failed_seed = (0..5).map(passing_per_seed).collect::<Vec<_>>();
    one_failed_seed[2] =
        V0SuccessPerSeed::from_quality_bits(2, true, true, true, false, true, true);
    let failed = product_from_seeds(one_failed_seed);
    assert!(!failed.overall_pass);
    assert!(!failed.suspicious_low_bpc);

    let mut suspicious = (0..5).map(passing_per_seed).collect::<Vec<_>>();
    for seed in &mut suspicious {
        seed.val_bpc_char_fp = BpcCharValue::try_new(0.1).expect("bpc");
    }
    let suspicious = product_from_seeds(suspicious);
    assert!(suspicious.suspicious_low_bpc);
    assert!(!suspicious.overall_pass);
}

#[test]
#[cfg(not(any(feature = "s3-oracle-real", feature = "s3-oracle-fallback")))]
fn v0_success_runner_requires_explicit_oracle_backend_feature_s3() {
    let mut workload = v0_success_s3_support::fixture_workload();
    workload.seeds.clear();
    workload.workload_self_hash = workload.compute_self_hash().expect("workload rehashes");

    let error = gbf_experiments::s3::workload::try_s3_run_v0_success(
        Vec::new(),
        Vec::new(),
        workload,
        v0_success_s3_support::tiny_val_post(),
        v0_success_s3_support::high_baseline(),
        v0_success_s3_support::budget(1_000_000),
    )
    .expect_err("v0_success runner must reject missing oracle backend feature");
    assert!(matches!(
        error,
        gbf_experiments::s3::workload::V0SuccessError::OracleBackendFeatureDisabled
    ));
}

proptest! {
    #[test]
    fn v0_success_outcome_totality_bits_s3(q in prop::array::uniform6(any::<bool>())) {
        assert_totality(q);
    }
}

fn assert_totality(q: [bool; 6]) {
    let per_seed = V0SuccessPerSeed::from_quality_bits(0, q[0], q[1], q[2], q[3], q[4], q[5]);
    assert_eq!(per_seed.Q1_holds, q[0]);
    assert_eq!(per_seed.Q2_holds, q[1]);
    assert_eq!(per_seed.Q3_holds, q[2]);
    assert_eq!(per_seed.Q4_holds, q[3]);
    assert_eq!(per_seed.Q5_holds, q[4]);
    assert_eq!(per_seed.Q6_holds, q[5]);
    assert_eq!(per_seed.pass, q.iter().all(|bit| *bit));
}

fn product_from_seeds(per_seed: Vec<V0SuccessPerSeed>) -> V0SuccessProduct {
    V0SuccessProduct::new(
        gbf_foundation::sha256(b"workload-totality"),
        gbf_foundation::sha256(b"baseline-totality"),
        gbf_foundation::sha256(b"chrome-totality"),
        per_seed,
    )
    .expect("v0_success product builds")
}
