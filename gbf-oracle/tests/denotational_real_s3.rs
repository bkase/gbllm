#![cfg(feature = "s3-real")]

mod denotational_support;

use denotational_support::{
    fixture_bundle, fixture_policy, fixture_workload, write_observations_if_requested,
};
use gbf_oracle::denotational::{
    DenotationalBackendKind, DenotationalDeterminismClass, DenotationalOracle,
    DenotationalOracleInputs, RealDenotationalOracle,
};

#[test]
fn denotational_real_evaluates_reference_bundle() {
    let bundle = fixture_bundle();
    let workload = fixture_workload();
    let policy = fixture_policy();
    let product = RealDenotationalOracle
        .evaluate(DenotationalOracleInputs::new(&bundle, &workload, &policy))
        .expect("real denotational oracle evaluates");

    assert_eq!(product.backend_kind, DenotationalBackendKind::Real);
    assert_eq!(
        product.determinism_class,
        DenotationalDeterminismClass::BitExact
    );
    assert_eq!(product.real_owner_bead, None);
    assert_eq!(
        product.observations.len(),
        workload.agreement_subset().len()
            * policy.agreement_trace.generated_steps as usize
            * policy.checkpoints.len()
    );
    assert_ne!(product.oracle_self_hash, gbf_foundation::Hash256::ZERO);

    let bytes = product
        .observations
        .canonical_bytes()
        .expect("observations canonicalize");
    write_observations_if_requested(&bytes);
}
