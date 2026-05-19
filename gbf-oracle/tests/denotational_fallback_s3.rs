#![cfg(feature = "s3-fallback")]

mod denotational_support;

use denotational_support::{
    fixture_bundle, fixture_policy, fixture_workload, write_observations_if_requested,
};
use gbf_foundation::sha256;
use gbf_oracle::denotational::{
    DenotationalBackendKind, DenotationalDeterminismClass, DenotationalOracle,
    DenotationalOracleInputs, S3_DENOTATIONAL_FALLBACK_REAL_OWNER_BEAD, S3DenotationalFallback,
};

#[test]
fn denotational_fallback_evaluates_reference_bundle_and_records_owner() {
    let bundle = fixture_bundle();
    let workload = fixture_workload();
    let policy = fixture_policy();
    let product = S3DenotationalFallback
        .evaluate(DenotationalOracleInputs::new(&bundle, &workload, &policy))
        .expect("fallback denotational oracle evaluates");

    assert_eq!(product.backend_kind, DenotationalBackendKind::Fallback);
    assert_eq!(
        product.determinism_class,
        DenotationalDeterminismClass::BitExact
    );
    assert_eq!(
        product.real_owner_bead,
        Some(S3_DENOTATIONAL_FALLBACK_REAL_OWNER_BEAD)
    );
    assert_eq!(S3DenotationalFallback::REAL_OWNER_BEAD, "bd-1rcc");

    let bytes = product
        .observations
        .canonical_bytes()
        .expect("observations canonicalize");
    assert_eq!(
        sha256(&bytes).to_string(),
        "sha256:f34ab841c234394494008b89ee5327900b43b5643de341fe6029dd7f1204c2e8",
        "direct S3DenotationalFallback canonical observations must remain byte-stable"
    );
    write_observations_if_requested(&bytes);
}
