#![cfg(all(feature = "s3", feature = "s3-oracle-real"))]

mod conformance_s3_support;

use conformance_s3_support::fixture_envelope_with_product;
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
    AgreementPolicy, AgreementProduct, PhaseId, TrainObservations, try_compare_phases,
};
use gbf_workload::PromptId;

#[test]
fn conformance_per_token_kl_s3() {
    let reference_logits = logits_with_prefix(&[2.0, 0.5, -0.25]);
    let artifact_logits = logits_with_prefix(&[1.0, 1.5, -0.5]);
    let agreement = agreement_product_for_logits(reference_logits, artifact_logits);

    assert!(agreement.records.iter().any(|record| {
        record.checkpoint == SemanticCheckpoint::PostLogits
            && record.bundle_vs_artifact_per_token_kl.is_some()
    }));

    let envelope = fixture_envelope_with_product(agreement).expect("conformance envelope builds");
    let kl_values = per_token_kl_values(&envelope);

    assert_eq!(kl_values.len(), 10, "5 seeds x 2 phases emit KL rows");
    assert!(kl_values.iter().all(|value| *value > 0.0));
    assert_close(
        envelope.quantization_gap_summary.mean_per_token_kl,
        mean(&kl_values),
    );
}

#[test]
fn conformance_skips_zero_norm_per_token_kl_s3() {
    let agreement = agreement_product_for_logits(vec![0.0; VOCAB_SIZE], vec![0.0; VOCAB_SIZE]);

    assert!(agreement.records.iter().all(|record| {
        record.checkpoint != SemanticCheckpoint::PostLogits
            || record.bundle_vs_artifact_per_token_kl.is_none()
    }));

    let envelope = fixture_envelope_with_product(agreement).expect("conformance envelope builds");
    assert_eq!(envelope.quantization_gap_summary.mean_per_token_kl, 0.0);
    assert!(per_token_kl_values(&envelope).is_empty());
    assert!(envelope.per_seed.iter().all(|seed| {
        seed.per_metric
            .keys()
            .all(|metric_id| !metric_id.as_str().contains("per_token_kl"))
    }));
}

fn agreement_product_for_logits(
    reference_logits: Vec<f32>,
    artifact_logits: Vec<f32>,
) -> AgreementProduct {
    let prompt_id = PromptId::from("prompt-00");
    let mut reference = ReferenceObservations::new();
    reference
        .insert(
            prompt_id.clone(),
            SemanticCheckpoint::PostLogits,
            0,
            Observation::post_logits(reference_logits.clone()).expect("reference logits"),
        )
        .expect("reference logits insert");
    reference
        .insert(
            prompt_id.clone(),
            SemanticCheckpoint::PostDecode,
            0,
            Observation::post_decode(7).expect("reference decode"),
        )
        .expect("reference decode insert");

    let mut artifact = ArtifactObservations::new();
    artifact
        .insert(
            prompt_id.clone(),
            SemanticCheckpoint::PostLogits,
            0,
            Observation::post_logits(artifact_logits.clone()).expect("artifact logits"),
        )
        .expect("artifact logits insert");
    artifact
        .insert(
            prompt_id.clone(),
            SemanticCheckpoint::PostDecode,
            0,
            Observation::post_decode(7).expect("artifact decode"),
        )
        .expect("artifact decode insert");

    let mut train = TrainObservations::new();
    for seed in 0..5 {
        train
            .insert(
                seed,
                PhaseId::PhaseA,
                prompt_id.clone(),
                SemanticCheckpoint::PostLogits,
                0,
                Observation::post_logits(reference_logits.clone()).expect("phase A train logits"),
            )
            .expect("phase A train logits insert");
        train
            .insert(
                seed,
                PhaseId::PhaseD,
                prompt_id.clone(),
                SemanticCheckpoint::PostLogits,
                0,
                Observation::post_logits(artifact_logits.clone()).expect("phase D train logits"),
            )
            .expect("phase D train logits insert");
        for phase in [PhaseId::PhaseA, PhaseId::PhaseD] {
            train
                .insert(
                    seed,
                    phase,
                    prompt_id.clone(),
                    SemanticCheckpoint::PostDecode,
                    0,
                    Observation::post_decode(7).expect("train decode"),
                )
                .expect("train decode insert");
        }
    }

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

    try_compare_phases(
        train,
        &denotational,
        &artifact,
        AgreementPolicy::phase_a(0.0, true),
        AgreementPolicy::phase_d(0.0, true),
        Vec::new(),
    )
    .expect("agreement compares")
}

fn logits_with_prefix(prefix: &[f32]) -> Vec<f32> {
    let mut logits = vec![0.0; VOCAB_SIZE];
    for (index, value) in prefix.iter().copied().enumerate() {
        logits[index] = value;
    }
    logits
}

fn per_token_kl_values(envelope: &gbf_artifact::ConformanceEnvelope) -> Vec<f32> {
    envelope
        .per_seed
        .iter()
        .flat_map(|seed| seed.per_metric.iter())
        .filter(|(metric_id, _)| metric_id.as_str().contains("per_token_kl"))
        .map(|(_, metric)| metric.value)
        .collect()
}

fn mean(values: &[f32]) -> f32 {
    values.iter().sum::<f32>() / values.len() as f32
}

fn assert_close(actual: f32, expected: f32) {
    let delta = (actual - expected).abs();
    assert!(
        delta <= 1.0e-6,
        "expected {actual} to be within 1e-6 of {expected}; delta={delta}"
    );
}
