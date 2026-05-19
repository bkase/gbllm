#![cfg(all(feature = "s3", feature = "s3-oracle-real"))]

mod denotational_s3_support;

use denotational_s3_support::{fixture_bundle, fixture_policy, fixture_workload};
use gbf_oracle::denotational::{
    DenotationalBackendKind, DenotationalOracle, DenotationalOracleInputs, RealDenotationalOracle,
    S3_DENOTATIONAL_FALLBACK_REAL_OWNER_BEAD, evaluate_with_backend_kind,
};

#[test]
fn oracle_real_vs_fallback_parity_s3() {
    let bundle = fixture_bundle();
    let workload = fixture_workload();
    let policy = fixture_policy();
    let inputs = DenotationalOracleInputs::new(&bundle, &workload, &policy);

    let real = RealDenotationalOracle
        .evaluate(inputs)
        .expect("real oracle evaluates");
    let fallback = evaluate_with_backend_kind(
        inputs,
        DenotationalBackendKind::Fallback,
        Some(S3_DENOTATIONAL_FALLBACK_REAL_OWNER_BEAD),
    )
    .expect("fallback-equivalent path evaluates");

    assert_eq!(
        real.observations.canonical_bytes().unwrap(),
        fallback.observations.canonical_bytes().unwrap()
    );
    assert_eq!(real.oracle_self_hash, fallback.oracle_self_hash);
}
