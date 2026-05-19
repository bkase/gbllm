#![cfg(all(feature = "s3", feature = "s3-oracle-real"))]

mod artifact_oracle_s3_support;

use gbf_oracle::artifact::{
    ArtifactBackendKind, ArtifactOracle, ArtifactOracleInputs, RealArtifactOracle,
    S3_ARTIFACT_FALLBACK_REAL_OWNER_BEAD, evaluate_with_backend_kind,
};

#[test]
fn oracle_artifact_real_vs_fallback_parity_s3() {
    let artifact = artifact_oracle_s3_support::fixture_artifact(0);
    let workload = artifact_oracle_s3_support::fixture_workload();
    let policy = artifact_oracle_s3_support::fixture_policy();

    // B16 intentionally keeps artifact real/fallback parity as a report-surface
    // contract over the shared S3 evaluator. Separate-binary backend identity
    // is owned by the smoke contract, not this in-process comparison.
    let real = RealArtifactOracle
        .evaluate(ArtifactOracleInputs::new(&artifact, &workload, &policy))
        .expect("real artifact oracle evaluates");
    let fallback = evaluate_with_backend_kind(
        ArtifactOracleInputs::new(&artifact, &workload, &policy),
        ArtifactBackendKind::Fallback,
        Some(S3_ARTIFACT_FALLBACK_REAL_OWNER_BEAD),
    )
    .expect("fallback artifact oracle report surface evaluates");

    assert_eq!(
        real.observations
            .canonical_bytes()
            .expect("real observations canonicalize"),
        fallback
            .observations
            .canonical_bytes()
            .expect("fallback observations canonicalize")
    );
    assert_eq!(real.weight_resolution_log, fallback.weight_resolution_log);
    assert_eq!(real.oracle_self_hash, fallback.oracle_self_hash);
    assert_eq!(fallback.backend_kind, ArtifactBackendKind::Fallback);
    assert_eq!(
        fallback.real_owner_bead,
        Some(S3_ARTIFACT_FALLBACK_REAL_OWNER_BEAD)
    );
}
