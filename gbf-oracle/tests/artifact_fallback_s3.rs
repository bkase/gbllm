#![cfg(feature = "s3-fallback")]

mod artifact_support;
mod denotational_support;

use artifact_support::fixture_artifact;
use denotational_support::{fixture_policy, fixture_workload};
use gbf_oracle::artifact::{
    ArtifactBackendKind, ArtifactDeterminismClass, ArtifactOracle, ArtifactOracleInputs,
    S3_ARTIFACT_FALLBACK_REAL_OWNER_BEAD, S3ArtifactFallback,
};

#[test]
fn artifact_fallback_evaluates_model_artifact_and_records_owner() {
    let artifact = fixture_artifact();
    let workload = fixture_workload();
    let policy = fixture_policy();
    let product = S3ArtifactFallback
        .evaluate(ArtifactOracleInputs::new(&artifact, &workload, &policy))
        .expect("fallback artifact oracle evaluates");

    assert_eq!(product.backend_kind, ArtifactBackendKind::Fallback);
    assert_eq!(
        product.determinism_class,
        ArtifactDeterminismClass::BitExact
    );
    assert_eq!(
        product.real_owner_bead,
        Some(S3_ARTIFACT_FALLBACK_REAL_OWNER_BEAD)
    );
    assert_eq!(S3ArtifactFallback::REAL_OWNER_BEAD, "bd-c4wg");
    assert!(!product.weight_resolution_log.is_empty());
}
