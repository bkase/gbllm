//! S3 oracle agreement runner.

use std::error::Error;
use std::fmt;

use gbf_artifact::{ModelArtifact, ReferenceModelBundle};
use gbf_oracle::artifact::{ArtifactOracle, ArtifactOracleInputs, ArtifactOracleProduct};
use gbf_oracle::denotational::{
    DenotationalOracle, DenotationalOracleInputs, DenotationalOracleProduct, Observation,
    SemanticCheckpoint,
};
use gbf_oracle::phase_surface_agreement::{
    AgreementError, AgreementPolicy, AgreementProduct, LiveTrainCapture, PhaseId,
    S3_LIVE_OBSERVATION_REAL_OWNER_BEAD, TrainObservations, fallback_tags,
    try_compare_phases_with_source,
};
use gbf_workload::{ObservationPolicy_S3, PromptId, WorkloadManifest_v0};

#[cfg(feature = "s3-oracle-real")]
use gbf_oracle::artifact::RealArtifactOracle as DefaultArtifactOracle;
#[cfg(feature = "s3-oracle-fallback")]
use gbf_oracle::artifact::S3ArtifactFallback as DefaultArtifactOracle;
#[cfg(feature = "s3-oracle-real")]
use gbf_oracle::denotational::RealDenotationalOracle as DefaultDenotationalOracle;
#[cfg(feature = "s3-oracle-fallback")]
use gbf_oracle::denotational::S3DenotationalFallback as DefaultDenotationalOracle;

/// Tracing target for S3 oracle agreement.
pub const AGREEMENT_LOG_TARGET: &str = "gbf_experiments::s3::oracle";

/// Agreement run-started event name.
pub const EVENT_NAME_RUN_STARTED: &str = "s3::agreement::run_started";
/// Live observation capture event name.
pub const EVENT_NAME_LIVE_OBSERVATION_CAPTURED: &str = "s3::agreement::live_observation_captured";
/// Agreement record emission event name.
pub const EVENT_NAME_RECORD_EMITTED: &str = "s3::agreement::record_emitted";
/// Agreement run-complete event name.
pub const EVENT_NAME_RUN_COMPLETE: &str = "s3::agreement::run_complete";

/// Inputs consumed by the S3 oracle agreement runner.
#[derive(Debug, Clone, Copy)]
pub struct S3OracleAgreementInputs<'a> {
    /// Reference bundle for Phase A oracle observations.
    pub bundle: &'a ReferenceModelBundle,
    /// Model artifact for Phase D oracle observations.
    pub artifact: &'a ModelArtifact,
    /// Workload manifest whose first three prompts form the agreement subset.
    pub workload: &'a WorkloadManifest_v0,
    /// Observation policy selecting checkpoints and forced trace length.
    pub observation_policy: &'a ObservationPolicy_S3,
}

impl<'a> S3OracleAgreementInputs<'a> {
    /// Construct runner inputs.
    #[must_use]
    pub const fn new(
        bundle: &'a ReferenceModelBundle,
        artifact: &'a ModelArtifact,
        workload: &'a WorkloadManifest_v0,
        observation_policy: &'a ObservationPolicy_S3,
    ) -> Self {
        Self {
            bundle,
            artifact,
            workload,
            observation_policy,
        }
    }
}

/// Run agreement using the backend selected by cargo features.
#[cfg(any(feature = "s3-oracle-real", feature = "s3-oracle-fallback"))]
pub fn run_surface_agreement_default(
    inputs: S3OracleAgreementInputs<'_>,
    live_capture: LiveTrainCapture,
) -> Result<AgreementProduct, S3OracleAgreementError> {
    run_surface_agreement(
        inputs,
        &DefaultDenotationalOracle,
        &DefaultArtifactOracle,
        live_capture,
    )
}

/// Run agreement with explicit fixture-derived live observations.
///
/// This helper is intentionally named as a fixture path: it derives the live
/// side from oracle outputs and tags the product with the real capture owner
/// bead so H4 consumers do not treat it as a true live-training proof.
#[cfg(any(feature = "s3-oracle-real", feature = "s3-oracle-fallback"))]
pub fn run_surface_agreement_with_fixture_live_observations_default(
    inputs: S3OracleAgreementInputs<'_>,
) -> Result<AgreementProduct, S3OracleAgreementError> {
    run_surface_agreement_with_fixture_live_observations(
        inputs,
        &DefaultDenotationalOracle,
        &DefaultArtifactOracle,
    )
}

/// Run phase-specific surface agreement through public oracle backend traits.
pub fn run_surface_agreement<D, A>(
    inputs: S3OracleAgreementInputs<'_>,
    denotational_oracle: &D,
    artifact_oracle: &A,
    live_capture: LiveTrainCapture,
) -> Result<AgreementProduct, S3OracleAgreementError>
where
    D: DenotationalOracle,
    A: ArtifactOracle,
{
    start_agreement_run(inputs, &live_capture)?;
    let (denotational, artifact) = evaluate_oracles(inputs, denotational_oracle, artifact_oracle)?;
    finish_surface_agreement(inputs, &denotational, &artifact, live_capture)
}

/// Run agreement with fixture-derived live observations through backend traits.
pub fn run_surface_agreement_with_fixture_live_observations<D, A>(
    inputs: S3OracleAgreementInputs<'_>,
    denotational_oracle: &D,
    artifact_oracle: &A,
) -> Result<AgreementProduct, S3OracleAgreementError>
where
    D: DenotationalOracle,
    A: ArtifactOracle,
{
    ensure_forced_trace(inputs)?;
    let (denotational, artifact) = evaluate_oracles(inputs, denotational_oracle, artifact_oracle)?;
    let live_capture = fixture_oracle_derived_live_capture(
        inputs,
        &denotational,
        &artifact,
        S3_LIVE_OBSERVATION_REAL_OWNER_BEAD,
    )?;
    start_agreement_run(inputs, &live_capture)?;
    finish_surface_agreement(inputs, &denotational, &artifact, live_capture)
}

fn start_agreement_run(
    inputs: S3OracleAgreementInputs<'_>,
    live_capture: &LiveTrainCapture,
) -> Result<(), S3OracleAgreementError> {
    tracing::info!(
        target: AGREEMENT_LOG_TARGET,
        event_name = EVENT_NAME_RUN_STARTED,
        workload_self_hash = %inputs.workload.workload_self_hash,
        seed_count = inputs.workload.seeds.len() as u64,
        prompt_subset_size = inputs.workload.agreement_subset().len() as u64,
        agreement_trace_steps = inputs.observation_policy.agreement_trace.generated_steps as u64,
        stop_on_eos = inputs.observation_policy.agreement_trace.stop_on_eos,
        live_observation_source = live_capture.source.kind.as_str(),
        live_observation_real_owner_bead = live_capture.source.real_owner_bead.as_deref().unwrap_or(""),
        live_observation_count = live_capture.observations.len() as u64,
    );

    ensure_forced_trace(inputs)
}

fn ensure_forced_trace(inputs: S3OracleAgreementInputs<'_>) -> Result<(), S3OracleAgreementError> {
    if inputs.observation_policy.agreement_trace.stop_on_eos {
        return Err(S3OracleAgreementError::StopOnEosEnabled);
    }
    Ok(())
}

fn evaluate_oracles<D, A>(
    inputs: S3OracleAgreementInputs<'_>,
    denotational_oracle: &D,
    artifact_oracle: &A,
) -> Result<(DenotationalOracleProduct, ArtifactOracleProduct), S3OracleAgreementError>
where
    D: DenotationalOracle,
    A: ArtifactOracle,
{
    let denotational = denotational_oracle
        .evaluate(DenotationalOracleInputs::new(
            inputs.bundle,
            inputs.workload,
            inputs.observation_policy,
        ))
        .map_err(S3OracleAgreementError::Denotational)?;
    let artifact = artifact_oracle
        .evaluate(ArtifactOracleInputs::new(
            inputs.artifact,
            inputs.workload,
            inputs.observation_policy,
        ))
        .map_err(S3OracleAgreementError::Artifact)?;
    Ok((denotational, artifact))
}

fn finish_surface_agreement(
    inputs: S3OracleAgreementInputs<'_>,
    denotational: &DenotationalOracleProduct,
    artifact: &ArtifactOracleProduct,
    live_capture: LiveTrainCapture,
) -> Result<AgreementProduct, S3OracleAgreementError> {
    emit_live_observations(&live_capture);

    let phase_a_gate = inputs
        .workload
        .acceptance
        .live_phase_a_vs_bundle
        .ok_or(S3OracleAgreementError::MissingPhaseAGate)?;
    let phase_d_gate = inputs
        .workload
        .acceptance
        .live_phase_d_vs_artifact
        .ok_or(S3OracleAgreementError::MissingPhaseDGate)?;
    let product = try_compare_phases_with_source(
        live_capture.observations,
        denotational,
        artifact,
        AgreementPolicy::phase_a(
            phase_a_gate.max_per_token_logit_abs_diff as f32,
            phase_a_gate.argmax_token_must_match,
        ),
        AgreementPolicy::phase_d(
            phase_d_gate.max_per_token_logit_abs_diff as f32,
            phase_d_gate.argmax_token_must_match,
        ),
        fallback_tags(denotational, artifact),
        live_capture.source,
    )
    .map_err(S3OracleAgreementError::Agreement)?;

    for record in &product.records {
        tracing::trace!(
            target: AGREEMENT_LOG_TARGET,
            event_name = EVENT_NAME_RECORD_EMITTED,
            seed = record.seed,
            prompt_id = record.prompt_id.as_str(),
            phase = record.phase.as_str(),
            checkpoint = record.checkpoint.as_str(),
            step = record.step,
            train_vs_bundle_max_abs_diff = ?record.train_vs_bundle_max_abs_diff,
            train_vs_artifact_max_abs_diff = ?record.train_vs_artifact_max_abs_diff,
            bundle_vs_artifact_max_abs_diff = ?record.bundle_vs_artifact_max_abs_diff,
        );
    }

    tracing::info!(
        target: AGREEMENT_LOG_TARGET,
        event_name = EVENT_NAME_RUN_COMPLETE,
        seed_count = inputs.workload.seeds.len() as u64,
        total_records = product.records.len() as u64,
        phase_a_pass = product.phase_a_pass,
        phase_d_pass = product.phase_d_pass,
        overall_pass = product.overall_pass,
        live_observation_source = product.live_observation_source.kind.as_str(),
        live_observation_real_owner_bead = product.live_observation_source.real_owner_bead.as_deref().unwrap_or(""),
        fallback_used = ?product.fallback_used,
        agreement_self_hash = %product.agreement_self_hash,
    );

    Ok(product)
}

fn fixture_oracle_derived_live_capture(
    inputs: S3OracleAgreementInputs<'_>,
    denotational: &DenotationalOracleProduct,
    artifact: &ArtifactOracleProduct,
    real_owner_bead: &str,
) -> Result<LiveTrainCapture, S3OracleAgreementError> {
    let mut train = TrainObservations::new();
    for seed in &inputs.workload.seeds {
        capture_fixture_oracle_derived_observations(
            *seed,
            PhaseId::PhaseA,
            inputs.workload,
            inputs.observation_policy,
            &mut train,
            |prompt_id, checkpoint, step| {
                denotational
                    .observations
                    .0
                    .get(&(prompt_id.clone(), checkpoint, step))
                    .cloned()
            },
        )?;
        capture_fixture_oracle_derived_observations(
            *seed,
            PhaseId::PhaseD,
            inputs.workload,
            inputs.observation_policy,
            &mut train,
            |prompt_id, checkpoint, step| {
                artifact
                    .observations
                    .0
                    .get(&(prompt_id.clone(), checkpoint, step))
                    .cloned()
            },
        )?;
    }
    LiveTrainCapture::oracle_derived_fixture(train, real_owner_bead)
        .map_err(S3OracleAgreementError::Agreement)
}

fn capture_fixture_oracle_derived_observations(
    seed: u64,
    phase: PhaseId,
    workload: &WorkloadManifest_v0,
    policy: &ObservationPolicy_S3,
    train: &mut TrainObservations,
    mut lookup: impl FnMut(&PromptId, SemanticCheckpoint, u32) -> Option<Observation>,
) -> Result<(), S3OracleAgreementError> {
    for prompt in workload.agreement_subset() {
        for step in 0..policy.agreement_trace.generated_steps {
            for checkpoint in &policy.checkpoints {
                let checkpoint = SemanticCheckpoint::from(*checkpoint);
                if !is_agreement_gated_checkpoint(checkpoint) {
                    continue;
                }
                let observation = lookup(&prompt.id, checkpoint, step).ok_or_else(|| {
                    S3OracleAgreementError::MissingLiveSource {
                        phase,
                        prompt_id: prompt.id.clone(),
                        checkpoint,
                        step,
                    }
                })?;
                train
                    .insert(
                        seed,
                        phase,
                        prompt.id.clone(),
                        checkpoint,
                        step,
                        observation,
                    )
                    .map_err(S3OracleAgreementError::Agreement)?;
            }
        }
    }
    Ok(())
}

fn emit_live_observations(live_capture: &LiveTrainCapture) {
    for observation in live_capture.observations.iter() {
        tracing::trace!(
            target: AGREEMENT_LOG_TARGET,
            event_name = EVENT_NAME_LIVE_OBSERVATION_CAPTURED,
            seed = observation.seed,
            prompt_id = observation.prompt_id.as_str(),
            phase = observation.phase.as_str(),
            checkpoint = observation.checkpoint.as_str(),
            step = observation.step,
            live_observation_source = live_capture.source.kind.as_str(),
            live_observation_real_owner_bead = live_capture.source.real_owner_bead.as_deref().unwrap_or(""),
        );
    }
}

const fn is_agreement_gated_checkpoint(checkpoint: SemanticCheckpoint) -> bool {
    matches!(
        checkpoint,
        SemanticCheckpoint::PostLogits | SemanticCheckpoint::PostDecode
    )
}

/// Errors produced by the S3 oracle agreement runner.
#[derive(Debug)]
pub enum S3OracleAgreementError {
    /// `agreement_trace.stop_on_eos` must remain false for forced traces.
    StopOnEosEnabled,
    /// Workload was missing the Phase A gate.
    MissingPhaseAGate,
    /// Workload was missing the Phase D gate.
    MissingPhaseDGate,
    /// Denotational oracle evaluation failed.
    Denotational(gbf_oracle::denotational::OracleError),
    /// Artifact oracle evaluation failed.
    Artifact(gbf_oracle::artifact::OracleError),
    /// Agreement comparison failed.
    Agreement(AgreementError),
    /// Live-observation fixture source was missing an oracle observation.
    MissingLiveSource {
        /// Phase being captured.
        phase: PhaseId,
        /// Prompt id.
        prompt_id: PromptId,
        /// Checkpoint.
        checkpoint: SemanticCheckpoint,
        /// Step.
        step: u32,
    },
}

impl fmt::Display for S3OracleAgreementError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StopOnEosEnabled => f.write_str("S3 agreement traces must set stop_on_eos=false"),
            Self::MissingPhaseAGate => f.write_str("workload missing Phase A agreement gate"),
            Self::MissingPhaseDGate => f.write_str("workload missing Phase D agreement gate"),
            Self::Denotational(error) => write!(f, "denotational oracle failed: {error}"),
            Self::Artifact(error) => write!(f, "artifact oracle failed: {error}"),
            Self::Agreement(error) => write!(f, "agreement comparison failed: {error}"),
            Self::MissingLiveSource {
                phase,
                prompt_id,
                checkpoint,
                step,
            } => write!(
                f,
                "missing live source observation for {phase:?}, {prompt_id}, {checkpoint:?}, step {step}"
            ),
        }
    }
}

impl Error for S3OracleAgreementError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Denotational(error) => Some(error),
            Self::Artifact(error) => Some(error),
            Self::Agreement(error) => Some(error),
            Self::StopOnEosEnabled
            | Self::MissingPhaseAGate
            | Self::MissingPhaseDGate
            | Self::MissingLiveSource { .. } => None,
        }
    }
}

#[allow(dead_code)]
fn _assert_public_products(
    _: &DenotationalOracleProduct,
    _: &ArtifactOracleProduct,
    _: &AgreementProduct,
) {
}
