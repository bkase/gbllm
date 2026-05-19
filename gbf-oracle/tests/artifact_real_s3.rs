#![cfg(feature = "s3-real")]

mod artifact_support;
mod denotational_support;

use std::collections::BTreeSet;

use artifact_support::fixture_artifact;
use denotational_support::{fixture_policy, fixture_workload};
use gbf_artifact::{EOS_ID, TextCharSeq, UNK_ID};
use gbf_oracle::artifact::{
    ArtifactBackendKind, ArtifactDeterminismClass, ArtifactOracle, ArtifactOracleInputs,
    Observation, RealArtifactOracle, SemanticCheckpoint,
};

#[test]
fn artifact_real_evaluates_model_artifact() {
    let artifact = fixture_artifact();
    let workload = fixture_workload();
    let policy = fixture_policy();
    let product = RealArtifactOracle
        .evaluate(ArtifactOracleInputs::new(&artifact, &workload, &policy))
        .expect("real artifact oracle evaluates");

    assert_eq!(product.backend_kind, ArtifactBackendKind::Real);
    assert_eq!(
        product.determinism_class,
        ArtifactDeterminismClass::BitExact
    );
    assert_eq!(product.real_owner_bead, None);
    assert_eq!(
        product.observations.len(),
        workload.agreement_subset().len()
            * policy.agreement_trace.generated_steps as usize
            * policy.checkpoints.len()
    );
    assert_ne!(product.oracle_self_hash, gbf_foundation::Hash256::ZERO);
    assert!(!product.weight_resolution_log.is_empty());
}

#[test]
fn artifact_oracle_soft_stops_eos_feedback_with_stop_on_eos_false() {
    let artifact = fixture_artifact();
    let mut workload = fixture_workload();
    workload.prompts[0].prompt_chars =
        TextCharSeq::new(vec![UNK_ID]).expect("UNK prompt validates");
    workload.workload_self_hash = workload.compute_self_hash().expect("workload rehashes");
    let policy = fixture_policy();
    assert!(!policy.agreement_trace.stop_on_eos);

    let product = RealArtifactOracle
        .evaluate(ArtifactOracleInputs::new(&artifact, &workload, &policy))
        .expect("EOS is recorded but not fed back as normalized text");

    let prompt_zero_steps = product
        .observations
        .0
        .keys()
        .filter(|(prompt_id, _, _)| prompt_id.as_str() == "prompt-00")
        .map(|(_, _, step)| *step)
        .collect::<BTreeSet<_>>();
    assert_eq!(prompt_zero_steps, BTreeSet::from([0]));

    let post_decode = product
        .observations
        .0
        .iter()
        .find(|((prompt_id, checkpoint, step), _)| {
            prompt_id.as_str() == "prompt-00"
                && *checkpoint == SemanticCheckpoint::PostDecode
                && *step == 0
        })
        .map(|(_, observation)| observation)
        .expect("post-decode EOS observation exists");
    assert!(matches!(
        post_decode,
        Observation::PostDecode { token } if *token == EOS_ID
    ));
}
