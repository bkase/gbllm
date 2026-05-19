#![cfg(feature = "s3-schemas")]

#[path = "v0_success_s3_support/mod.rs"]
mod v0_success_s3_support;

use gbf_workload::{
    CheckpointRole, CheckpointRoles_S3, ObservationCheckpoint, ObservationPolicy_S3,
    S3CompareDomain, S3DeterminismRequirement, S3TraceLevel,
};

#[test]
fn observation_policy_s3() {
    let manifest = v0_success_s3_support::load_v0_success();
    let observation = &manifest.observation;

    assert_eq!(
        observation.checkpoints,
        [
            ObservationCheckpoint::PostEmbedding,
            ObservationCheckpoint::PostLogits,
            ObservationCheckpoint::PostDecode,
        ]
    );
    assert_eq!(observation.trace_level, S3TraceLevel::Standard);
    assert_eq!(
        observation.compare_domain,
        S3CompareDomain::LogitsF32CanonicalReduction
    );
    assert_eq!(
        observation.determinism_requirement,
        S3DeterminismRequirement::BitExact
    );
    assert_eq!(observation.agreement_trace.generated_steps, 16);
    assert!(!observation.agreement_trace.stop_on_eos);
    assert_eq!(
        observation.checkpoint_roles,
        CheckpointRoles_S3 {
            post_embedding: CheckpointRole::ObservationOnly,
            post_logits: CheckpointRole::AgreementGated,
            post_decode: CheckpointRole::AgreementGated,
        }
    );
    assert_eq!(*observation, ObservationPolicy_S3::pinned());
}
