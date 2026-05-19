#![cfg(all(
    feature = "s3",
    any(feature = "s3-oracle-real", feature = "s3-oracle-fallback")
))]
#![allow(dead_code)]

#[path = "artifact_oracle_s3_support.rs"]
mod artifact_fixture;
#[path = "denotational_s3_support.rs"]
mod denotational_fixture;

use gbf_artifact::{CanonicalTensorId, EOS_ID};
use gbf_experiments::s3::oracle::{
    S3OracleAgreementError, S3OracleAgreementInputs,
    run_surface_agreement_with_fixture_live_observations_default,
};
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
use gbf_workload::{ObservationPolicy_S3, PromptId, WorkloadManifest_v0};

pub const EXPECTED_FULL_RECORD_COUNT: usize = 5 * 2 * 3 * 16 * 2;

pub fn run_default_agreement() -> AgreementProduct {
    let bundle = denotational_fixture::fixture_bundle();
    let artifact = artifact_fixture::fixture_artifact(0);
    let workload = denotational_fixture::fixture_workload();
    let policy = denotational_fixture::fixture_policy();
    run_surface_agreement_with_fixture_live_observations_default(S3OracleAgreementInputs::new(
        &bundle, &artifact, &workload, &policy,
    ))
    .expect("S3 agreement runner succeeds")
}

pub fn run_default_agreement_with_workload(workload: &WorkloadManifest_v0) -> AgreementProduct {
    let bundle = denotational_fixture::fixture_bundle();
    let artifact = artifact_fixture::fixture_artifact(0);
    let policy = denotational_fixture::fixture_policy();
    run_surface_agreement_with_fixture_live_observations_default(S3OracleAgreementInputs::new(
        &bundle, &artifact, workload, &policy,
    ))
    .expect("S3 agreement runner succeeds")
}

pub fn run_default_agreement_with_policy(
    policy: &ObservationPolicy_S3,
) -> Result<AgreementProduct, S3OracleAgreementError> {
    let bundle = denotational_fixture::fixture_bundle();
    let artifact = artifact_fixture::fixture_artifact(0);
    let workload = denotational_fixture::fixture_workload();
    run_surface_agreement_with_fixture_live_observations_default(S3OracleAgreementInputs::new(
        &bundle, &artifact, &workload, policy,
    ))
}

pub fn workload_with_first_three_prompt_ids(ids: [&str; 3]) -> WorkloadManifest_v0 {
    let mut workload = denotational_fixture::fixture_workload();
    for (prompt, id) in workload.prompts.iter_mut().take(3).zip(ids) {
        prompt.id = PromptId::from(id);
    }
    workload.workload_self_hash = workload.compute_self_hash().expect("workload self-hash");
    workload.validate().expect("renamed workload validates");
    workload
}

pub fn force_length_eos_product() -> AgreementProduct {
    let prompt_id = PromptId::from("eos-at-step-five");
    let mut reference = ReferenceObservations::new();
    let mut artifact = ArtifactObservations::new();
    let mut train = TrainObservations::new();

    for step in 0..16 {
        let token = if step == 5 { EOS_ID } else { 7 };
        let observation = Observation::post_decode(token).expect("decode observation builds");
        reference
            .insert(
                prompt_id.clone(),
                SemanticCheckpoint::PostDecode,
                step,
                observation.clone(),
            )
            .expect("reference observation inserts");
        artifact
            .insert(
                prompt_id.clone(),
                SemanticCheckpoint::PostDecode,
                step,
                observation.clone(),
            )
            .expect("artifact observation inserts");
        train
            .insert(
                0,
                PhaseId::PhaseA,
                prompt_id.clone(),
                SemanticCheckpoint::PostDecode,
                step,
                observation.clone(),
            )
            .expect("phase A train observation inserts");
        train
            .insert(
                0,
                PhaseId::PhaseD,
                prompt_id.clone(),
                SemanticCheckpoint::PostDecode,
                step,
                observation,
            )
            .expect("phase D train observation inserts");
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
    .expect("force-length agreement compares")
}

pub fn write_product_if_requested(product: &AgreementProduct) {
    let Ok(path) = std::env::var("S3_ORACLE_AGREEMENT_PRODUCT_OUT") else {
        return;
    };
    std::fs::write(
        path,
        product
            .canonical_json_bytes()
            .expect("agreement product canonicalizes"),
    )
    .expect("writes agreement product");
}

pub fn fixture_policy() -> ObservationPolicy_S3 {
    denotational_fixture::fixture_policy()
}
