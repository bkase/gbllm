#![cfg(all(
    feature = "s3",
    feature = "s3-oracle-real",
    feature = "s3-oracle-adversarial"
))]

mod artifact_oracle_s3_support;

use artifact_oracle_s3_support::{max_abs_diff, prompt_for_logits};
use gbf_oracle::artifact::adversarial_fixture::{
    adversarial_artifact_fixture, canonical_naming_artifact_fixture,
    name_resolver_logits_for_fixture,
};
use gbf_oracle::artifact::{
    ArtifactOracle, ArtifactOracleInputs, Observation, RealArtifactOracle, SemanticCheckpoint,
    quant_spec_resolver_logits,
};

#[test]
fn oracle_quantspec_s3() {
    let workload = artifact_oracle_s3_support::fixture_workload();
    let policy = artifact_oracle_s3_support::fixture_policy();
    let artifact = adversarial_artifact_fixture();
    let first_prompt = &workload.agreement_subset()[0];

    let product = RealArtifactOracle
        .evaluate(ArtifactOracleInputs::new(&artifact, &workload, &policy))
        .expect("artifact oracle evaluates adversarial fixture");
    let oracle_logits = match product
        .observations
        .get(&first_prompt.id, SemanticCheckpoint::PostLogits, 0)
        .expect("post logits captured")
    {
        Observation::PostLogits { logits } => logits,
        other => panic!("expected post logits, got {other:?}"),
    };
    let quant_logits = quant_spec_resolver_logits(&artifact, &first_prompt.prompt_chars)
        .expect("QuantSpec resolver evaluates");
    let name_logits = name_resolver_logits_for_fixture(&artifact, &first_prompt.prompt_chars)
        .expect("name resolver evaluates");

    assert_eq!(oracle_logits, &quant_logits);
    assert!(
        max_abs_diff(oracle_logits, &name_logits) > 0.0,
        "adversarial fixture must separate QuantSpec and name resolver"
    );

    let canonical = canonical_naming_artifact_fixture();
    let prompt = prompt_for_logits();
    assert_eq!(
        quant_spec_resolver_logits(&canonical, &prompt).expect("canonical QuantSpec logits"),
        name_resolver_logits_for_fixture(&canonical, &prompt).expect("canonical name logits"),
        "canonical naming fixture sanity should match"
    );
}
