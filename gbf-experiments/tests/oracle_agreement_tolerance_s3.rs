#![cfg(all(feature = "s3", feature = "s3-oracle-real"))]

use gbf_artifact::{CanonicalTensorId, VOCAB_SIZE};
use gbf_oracle::artifact::{
    ArtifactBackendKind, ArtifactObservations, ArtifactOracleProduct, ResolvedVia,
    WeightResolutionEntry,
};
use gbf_oracle::denotational::{
    DenotationalBackendKind, DenotationalOracleProduct, Observation, ReferenceObservations,
    SemanticCheckpoint,
};
use gbf_oracle::phase_surface_agreement::{
    AgreementPolicy, LiveObservationSourceKind, PhaseId, TrainObservations, try_compare_phases,
};
use gbf_workload::PromptId;

#[test]
fn oracle_agreement_nonzero_tolerance_s3() {
    let prompt_id = PromptId::from("nonzero-tolerance");
    let reference_logits = logits_with_delta(0, 0.0);
    let phase_a_train_logits = logits_with_delta(0, 0.25);
    let artifact_logits = reference_logits.clone();

    let mut reference = ReferenceObservations::new();
    reference
        .insert(
            prompt_id.clone(),
            SemanticCheckpoint::PostLogits,
            0,
            Observation::post_logits(reference_logits).expect("reference logits"),
        )
        .expect("reference observation inserts");
    let mut artifact = ArtifactObservations::new();
    artifact
        .insert(
            prompt_id.clone(),
            SemanticCheckpoint::PostLogits,
            0,
            Observation::post_logits(artifact_logits.clone()).expect("artifact logits"),
        )
        .expect("artifact observation inserts");

    let mut train = TrainObservations::new();
    train
        .insert(
            0,
            PhaseId::PhaseA,
            prompt_id.clone(),
            SemanticCheckpoint::PostLogits,
            0,
            Observation::post_logits(phase_a_train_logits).expect("phase A train logits"),
        )
        .expect("phase A train observation inserts");
    train
        .insert(
            0,
            PhaseId::PhaseD,
            prompt_id.clone(),
            SemanticCheckpoint::PostLogits,
            0,
            Observation::post_logits(artifact_logits).expect("phase D train logits"),
        )
        .expect("phase D train observation inserts");

    let denotational =
        DenotationalOracleProduct::new(reference, DenotationalBackendKind::Real, None)
            .expect("denotational product builds");
    let artifact = ArtifactOracleProduct::new(
        artifact,
        vec![WeightResolutionEntry {
            tensor_id: CanonicalTensorId::new("tensor.embedding").expect("canonical tensor id"),
            resolved_via: ResolvedVia::QuantSpec_weight_quant,
        }],
        ArtifactBackendKind::Real,
        None,
    )
    .expect("artifact product builds");

    let product = try_compare_phases(
        train,
        &denotational,
        &artifact,
        AgreementPolicy::phase_a(0.25, true),
        AgreementPolicy::phase_d(0.0, true),
        Vec::new(),
    )
    .expect("agreement compares");

    assert!(product.phase_a_pass);
    assert!(product.phase_d_pass);
    assert_eq!(
        product.live_observation_source.kind,
        LiveObservationSourceKind::RealTrainCapture
    );
    let phase_a = product
        .records
        .iter()
        .find(|record| record.phase == PhaseId::PhaseA)
        .expect("phase A record");
    assert_eq!(phase_a.train_vs_bundle_max_abs_diff, Some(0.25));
    assert_eq!(phase_a.train_vs_bundle_argmax_match, Some(true));
}

fn logits_with_delta(index: usize, delta: f32) -> Vec<f32> {
    let mut logits = vec![0.0; VOCAB_SIZE];
    logits[index] = delta;
    logits
}
