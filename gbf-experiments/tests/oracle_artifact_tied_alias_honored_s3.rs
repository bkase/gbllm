#![cfg(all(feature = "s3", feature = "s3-oracle-real"))]

mod artifact_oracle_s3_support;

use gbf_oracle::artifact::{
    ArtifactDecoder, ArtifactOracle, ArtifactOracleInputs, RealArtifactOracle,
};

#[test]
fn oracle_artifact_tied_alias_honored_s3() {
    let artifact = artifact_oracle_s3_support::fixture_artifact(0);
    let alias = artifact
        .core
        .tied_embedding_alias
        .as_ref()
        .expect("fixture preserves tied alias");
    assert!(alias.shared);
    assert_eq!(alias.embedding_canonical_id, alias.classifier_canonical_id);

    let workload = artifact_oracle_s3_support::fixture_workload();
    let policy = artifact_oracle_s3_support::fixture_policy();
    let product = RealArtifactOracle
        .evaluate(ArtifactOracleInputs::new(&artifact, &workload, &policy))
        .expect("artifact oracle evaluates");
    let alias_resolution_count = product
        .weight_resolution_log
        .iter()
        .filter(|entry| entry.tensor_id == alias.embedding_canonical_id)
        .count();
    assert_eq!(alias_resolution_count, 1);

    let decode = ArtifactDecoder::new(&artifact).decode_argmax(
        &artifact_oracle_s3_support::prompt_for_decode(),
        8,
        false,
    );
    assert_eq!(decode.decode_log.len(), 8);
    assert_eq!(decode.generated.len(), 8);
}
