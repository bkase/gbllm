#![cfg(all(feature = "s3", feature = "s3-oracle-real"))]

mod artifact_oracle_s3_support;

use gbf_artifact::PayloadRole;
use gbf_oracle::artifact::{ArtifactOracle, ArtifactOracleInputs, RealArtifactOracle, ResolvedVia};

#[test]
fn oracle_artifact_weight_resolution_log_s3() {
    let artifact = artifact_oracle_s3_support::fixture_artifact(0);
    let workload = artifact_oracle_s3_support::fixture_workload();
    let policy = artifact_oracle_s3_support::fixture_policy();
    let product = RealArtifactOracle
        .evaluate(ArtifactOracleInputs::new(&artifact, &workload, &policy))
        .expect("artifact oracle evaluates");

    let deployable_count = artifact
        .core
        .tensors
        .iter()
        .filter(|tensor| tensor.payload_role == PayloadRole::DeployableWeight)
        .count();
    assert_eq!(product.weight_resolution_log.len(), deployable_count);
    assert!(
        product
            .weight_resolution_log
            .iter()
            .all(|entry| { entry.resolved_via == ResolvedVia::QuantSpec_weight_quant })
    );
    assert!(
        !product
            .weight_resolution_log
            .iter()
            .any(|entry| { entry.resolved_via == ResolvedVia::NameResolverForbidden })
    );
    for entry in &product.weight_resolution_log {
        assert!(artifact.core.quant.weight_quant(&entry.tensor_id).is_some());
    }
}
