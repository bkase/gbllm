//! Artifact oracle for S3 `ModelArtifact` evaluation.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

use gbf_artifact::{
    ArtifactError, BOS_ID, CanonicalTensor, CanonicalTensorId, CharId, DecodeMode, Dtype, EOS_ID,
    LexicalError, ModelArtifact, PayloadRole, RESERVED_ID, TextCharSeq, UNK_ID, VOCAB_SIZE,
    WeightQuant, is_text_char_id,
};
use gbf_foundation::{CanonicalJson, CanonicalJsonError, DomainHash, Hash256};
use gbf_workload::{ObservationPolicy_S3, PromptId, S3DeterminismRequirement, WorkloadManifest_v0};
use serde::{Deserialize, Serialize};

pub use crate::denotational::{Observation, SemanticCheckpoint};

pub mod decoder;
pub mod observations_canonical;

#[cfg(feature = "s3-real")]
pub mod real;
#[cfg(feature = "s3-real")]
pub use real::RealArtifactOracle;

#[cfg(feature = "s3-fallback")]
pub mod fallback;
#[cfg(feature = "s3-fallback")]
pub use fallback::S3ArtifactFallback;

#[cfg(feature = "s3-oracle-adversarial")]
pub mod adversarial_fixture;

pub use decoder::{ArtifactDecodeResult, ArtifactDecoder, DecodeStep};
pub use observations_canonical::ArtifactObservationsCanonical;

/// Tracing target for artifact oracle evaluation.
pub const ARTIFACT_ORACLE_LOG_TARGET: &str = "gbf_oracle::artifact";

/// Evaluation started event name.
pub const EVENT_NAME_EVALUATION_STARTED: &str = "s3::artifact_oracle::evaluation_started";
/// Per-tensor resolution event name.
pub const EVENT_NAME_WEIGHT_RESOLVED: &str = "s3::artifact_oracle::weight_resolved";
/// Per-checkpoint observation event name.
pub const EVENT_NAME_OBSERVATION_CAPTURED: &str = "s3::artifact_oracle::observation_captured";
/// Evaluation complete event name.
pub const EVENT_NAME_EVALUATION_COMPLETE: &str = "s3::artifact_oracle::evaluation_complete";

const PRODUCT_SCHEMA_VERSION: &str = "1";
const PRODUCT_SCHEMA_ID: &str = "artifact_oracle_product.v1";
const LOGIT_DOMAIN_TAG: &[u8] = b"gbf-oracle:s3-artifact-oracle-logits:v1\0";

/// F-C2 owner bead named by the S3 artifact fallback contract.
pub const S3_ARTIFACT_FALLBACK_REAL_OWNER_BEAD: &str = "bd-c4wg";

/// Oracle backend label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactBackendKind {
    /// Real S3 artifact oracle backend.
    Real,
    /// S3 fallback backend pending the richer F-C2 implementation.
    Fallback,
}

impl ArtifactBackendKind {
    /// Stable logging field.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Real => "real",
            Self::Fallback => "fallback",
        }
    }
}

/// Determinism class for artifact observations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum ArtifactDeterminismClass {
    /// Bit-exact canonical observations.
    BitExact,
}

impl ArtifactDeterminismClass {
    /// Stable logging field.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BitExact => "BitExact",
        }
    }
}

/// Weight-resolution route recorded by the artifact oracle.
#[allow(non_camel_case_types)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResolvedVia {
    /// Deployable weight was resolved through `QuantSpec_S3::weight_quant`.
    #[serde(rename = "QuantSpec::weight_quant")]
    QuantSpec_weight_quant,
    /// Deliberate broken-S3 detector for name-based resolution.
    NameResolverForbidden,
}

impl ResolvedVia {
    /// Stable logging field.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::QuantSpec_weight_quant => "QuantSpec::weight_quant",
            Self::NameResolverForbidden => "NameResolverForbidden",
        }
    }
}

/// One deployable tensor resolution record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeightResolutionEntry {
    /// Canonical tensor id that was dereferenced.
    pub tensor_id: CanonicalTensorId,
    /// Route by which the tensor was resolved.
    pub resolved_via: ResolvedVia,
}

/// Sorted artifact observations keyed by prompt, checkpoint, and step.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ArtifactObservations(pub BTreeMap<(PromptId, SemanticCheckpoint, u32), Observation>);

impl ArtifactObservations {
    /// Construct an empty observation map.
    #[must_use]
    pub fn new() -> Self {
        Self(BTreeMap::new())
    }

    /// Insert a checkpoint-specific observation.
    pub fn insert(
        &mut self,
        prompt_id: PromptId,
        checkpoint: SemanticCheckpoint,
        step: u32,
        observation: Observation,
    ) -> Result<(), OracleError> {
        if observation.checkpoint() != checkpoint {
            return Err(OracleError::CheckpointMismatch {
                key: checkpoint,
                observed: observation.checkpoint(),
            });
        }
        if self
            .0
            .insert((prompt_id.clone(), checkpoint, step), observation)
            .is_some()
        {
            return Err(OracleError::DuplicateObservation {
                prompt_id: prompt_id.to_string(),
                checkpoint,
                step,
            });
        }
        Ok(())
    }

    /// Borrow an observation by key.
    #[must_use]
    pub fn get(
        &self,
        prompt_id: &PromptId,
        checkpoint: SemanticCheckpoint,
        step: u32,
    ) -> Option<&Observation> {
        self.0.get(&(prompt_id.clone(), checkpoint, step))
    }

    /// Number of captured observations.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether no observations were captured.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Iterate in canonical sorted-key order.
    pub fn iter(
        &self,
    ) -> impl Iterator<Item = (&(PromptId, SemanticCheckpoint, u32), &Observation)> {
        self.0.iter()
    }

    /// Canonical S1 JSON bytes for the observation rows.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, OracleError> {
        ArtifactObservationsCanonical::to_vec(self)
    }
}

/// Inputs consumed by an artifact oracle backend.
#[derive(Debug, Clone, Copy)]
pub struct ArtifactOracleInputs<'a> {
    /// S3 model artifact to evaluate.
    pub artifact: &'a ModelArtifact,
    /// S3 workload manifest supplying prompt ids and prompt text.
    pub workload: &'a WorkloadManifest_v0,
    /// Observation policy selecting checkpoints and trace length.
    pub observation_policy: &'a ObservationPolicy_S3,
}

impl<'a> ArtifactOracleInputs<'a> {
    /// Construct artifact oracle inputs.
    #[must_use]
    pub const fn new(
        artifact: &'a ModelArtifact,
        workload: &'a WorkloadManifest_v0,
        observation_policy: &'a ObservationPolicy_S3,
    ) -> Self {
        Self {
            artifact,
            workload,
            observation_policy,
        }
    }
}

/// Product returned by artifact oracle evaluation.
#[derive(Debug, Clone, PartialEq)]
pub struct ArtifactOracleProduct {
    /// Canonical sorted observations.
    pub observations: ArtifactObservations,
    /// Pinned determinism class.
    pub determinism_class: ArtifactDeterminismClass,
    /// Self-hash over observations, determinism class, and resolution log.
    pub oracle_self_hash: Hash256,
    /// Per-tensor QuantSpec resolution log.
    pub weight_resolution_log: Vec<WeightResolutionEntry>,
    /// Backend that produced this product.
    pub backend_kind: ArtifactBackendKind,
    /// Owner bead for fallback real-backend completion, when applicable.
    pub real_owner_bead: Option<&'static str>,
}

impl ArtifactOracleProduct {
    /// Construct a product and compute its self-hash.
    pub fn new(
        observations: ArtifactObservations,
        weight_resolution_log: Vec<WeightResolutionEntry>,
        backend_kind: ArtifactBackendKind,
        real_owner_bead: Option<&'static str>,
    ) -> Result<Self, OracleError> {
        validate_product_postconditions(&observations, &weight_resolution_log)?;
        let determinism_class = ArtifactDeterminismClass::BitExact;
        let oracle_self_hash =
            compute_oracle_self_hash(&observations, determinism_class, &weight_resolution_log)?;
        Ok(Self {
            observations,
            determinism_class,
            oracle_self_hash,
            weight_resolution_log,
            backend_kind,
            real_owner_bead,
        })
    }

    /// DomainHash context for artifact oracle products.
    #[must_use]
    pub const fn domain() -> DomainHash<'static> {
        DomainHash::new(
            "gbf-oracle",
            "ArtifactOracleProduct",
            PRODUCT_SCHEMA_ID,
            PRODUCT_SCHEMA_VERSION,
        )
    }
}

/// Artifact oracle backend trait.
pub trait ArtifactOracle: Send + Sync {
    /// Evaluate an artifact over the supplied workload prompts.
    fn evaluate(
        &self,
        inputs: ArtifactOracleInputs<'_>,
    ) -> Result<ArtifactOracleProduct, OracleError>;
}

/// Shared implementation used by real and fallback backends.
pub fn evaluate_with_backend_kind(
    inputs: ArtifactOracleInputs<'_>,
    backend_kind: ArtifactBackendKind,
    real_owner_bead: Option<&'static str>,
) -> Result<ArtifactOracleProduct, OracleError> {
    validate_inputs(inputs)?;
    tracing::info!(
        target: ARTIFACT_ORACLE_LOG_TARGET,
        event_name = EVENT_NAME_EVALUATION_STARTED,
        backend_kind = backend_kind.as_str(),
        workload_self_hash = %inputs.workload.workload_self_hash,
        artifact_self_hash = %inputs.artifact.artifact_self_hash,
        prompt_count = inputs.workload.agreement_subset().len() as u64,
    );

    let (resolved_weights, weight_resolution_log) =
        quant_spec_resolved_logit_tensors(inputs.artifact)?;
    for entry in &weight_resolution_log {
        tracing::trace!(
            target: ARTIFACT_ORACLE_LOG_TARGET,
            event_name = EVENT_NAME_WEIGHT_RESOLVED,
            tensor_id = %entry.tensor_id,
            resolved_via = entry.resolved_via.as_str(),
        );
    }

    let mut observations = ArtifactObservations::new();
    for prompt in inputs.workload.agreement_subset() {
        let mut current_prompt = prompt.prompt_chars.clone();
        for step in 0..inputs.observation_policy.agreement_trace.generated_steps {
            let evaluated =
                evaluate_prompt_with_resolved(inputs.artifact, &current_prompt, &resolved_weights)?;
            for checkpoint in &inputs.observation_policy.checkpoints {
                let checkpoint = SemanticCheckpoint::from(*checkpoint);
                let observation = match checkpoint {
                    SemanticCheckpoint::PostEmbedding => {
                        Observation::post_embedding(Some(evaluated.hidden_state.clone()))?
                    }
                    SemanticCheckpoint::PostLogits => {
                        Observation::post_logits(evaluated.logits.clone())?
                    }
                    SemanticCheckpoint::PostDecode => {
                        Observation::post_decode(evaluated.argmax_token)?
                    }
                };
                observations.insert(prompt.id.clone(), checkpoint, step, observation)?;
                tracing::trace!(
                    target: ARTIFACT_ORACLE_LOG_TARGET,
                    event_name = EVENT_NAME_OBSERVATION_CAPTURED,
                    prompt_id = prompt.id.as_str(),
                    checkpoint = checkpoint.as_str(),
                    step,
                );
            }
            if evaluated.argmax_token == inputs.artifact.core.lexical.control_tokens.eos {
                // EOS is a control token, not normalized text. The artifact
                // oracle records the decode observation, then treats EOS as a
                // soft terminal feedback token even when the workload policy
                // keeps stop_on_eos=false.
                break;
            }
            current_prompt = append_generated_token(&current_prompt, evaluated.argmax_token)?;
        }
    }

    let product = ArtifactOracleProduct::new(
        observations,
        weight_resolution_log,
        backend_kind,
        real_owner_bead,
    )?;
    let tensors_resolved_via_quant_spec = product
        .weight_resolution_log
        .iter()
        .filter(|entry| entry.resolved_via == ResolvedVia::QuantSpec_weight_quant)
        .count() as u64;
    let tensors_resolved_via_naming = product
        .weight_resolution_log
        .iter()
        .filter(|entry| entry.resolved_via == ResolvedVia::NameResolverForbidden)
        .count() as u64;
    tracing::info!(
        target: ARTIFACT_ORACLE_LOG_TARGET,
        event_name = EVENT_NAME_EVALUATION_COMPLETE,
        backend_kind = backend_kind.as_str(),
        observation_count = product.observations.len() as u64,
        oracle_self_hash = %product.oracle_self_hash,
        tensors_resolved_via_quant_spec,
        tensors_resolved_via_naming,
    );
    Ok(product)
}

/// Compute one deterministic logits row by resolving deployable tensors
/// through `QuantSpec_S3::weight_quant`.
pub fn quant_spec_resolver_logits(
    artifact: &ModelArtifact,
    prompt: &TextCharSeq,
) -> Result<Vec<f32>, OracleError> {
    validate_model_artifact_contract(artifact)?;
    let (resolved, _) = quant_spec_resolved_logit_tensors(artifact)?;
    Ok(logits_from_resolved(artifact, prompt, &resolved))
}

/// Deterministic argmax with lowest-index tie breaking.
#[must_use]
pub fn argmax_lowest_index(logits: &[f32]) -> (CharId, f32) {
    let mut best_index = 0_usize;
    let mut best_value = logits[0];
    for (index, value) in logits.iter().copied().enumerate().skip(1) {
        if value > best_value {
            best_index = index;
            best_value = value;
        }
    }
    (best_index as CharId, best_value)
}

#[cfg(feature = "s3-oracle-adversarial")]
pub(crate) fn deliberate_name_resolver_logits(
    artifact: &ModelArtifact,
    prompt: &TextCharSeq,
) -> Result<Vec<f32>, OracleError> {
    validate_model_artifact_contract(artifact)?;
    let resolved = name_resolved_logit_tensors(artifact)?;
    Ok(logits_from_resolved(artifact, prompt, &resolved))
}

pub(crate) fn validate_model_artifact_contract(
    artifact: &ModelArtifact,
) -> Result<(), OracleError> {
    artifact.validate().map_err(OracleError::Artifact)?;
    validate_tied_alias(artifact)?;
    Ok(())
}

fn validate_inputs(inputs: ArtifactOracleInputs<'_>) -> Result<(), OracleError> {
    validate_model_artifact_contract(inputs.artifact)?;
    if !inputs.workload.execution.artifact {
        return Err(OracleError::ArtifactExecutionDisabled);
    }
    if inputs.workload.session.decode.mode != DecodeMode::Argmax {
        return Err(OracleError::DecodeModeMismatch);
    }
    if inputs.observation_policy.determinism_requirement != S3DeterminismRequirement::BitExact {
        return Err(OracleError::DeterminismPolicyMismatch);
    }
    if inputs.observation_policy.checkpoints.is_empty() {
        return Err(OracleError::EmptyCheckpointPolicy);
    }
    if inputs.observation_policy.agreement_trace.generated_steps == 0 {
        return Err(OracleError::ZeroGeneratedSteps);
    }
    if inputs.workload.agreement_subset().is_empty() {
        return Err(OracleError::EmptyPromptSubset);
    }
    Ok(())
}

fn validate_tied_alias(artifact: &ModelArtifact) -> Result<(), OracleError> {
    let Some(alias) = &artifact.core.tied_embedding_alias else {
        return Ok(());
    };
    if alias.shared && alias.embedding_canonical_id != alias.classifier_canonical_id {
        return Err(OracleError::TiedAliasNotShared {
            embedding_id: alias.embedding_canonical_id.clone(),
            classifier_id: alias.classifier_canonical_id.clone(),
        });
    }
    let tensor_ids = artifact
        .core
        .tensors
        .iter()
        .map(|tensor| tensor.id.clone())
        .collect::<BTreeSet<_>>();
    if !tensor_ids.contains(&alias.embedding_canonical_id) {
        return Err(OracleError::TiedAliasMissingTensor {
            tensor_id: alias.embedding_canonical_id.clone(),
        });
    }
    if !tensor_ids.contains(&alias.classifier_canonical_id) {
        return Err(OracleError::TiedAliasMissingTensor {
            tensor_id: alias.classifier_canonical_id.clone(),
        });
    }
    Ok(())
}

fn validate_product_postconditions(
    observations: &ArtifactObservations,
    weight_resolution_log: &[WeightResolutionEntry],
) -> Result<(), OracleError> {
    if observations.is_empty() {
        return Err(OracleError::EmptyObservations);
    }
    if weight_resolution_log.is_empty() {
        return Err(OracleError::EmptyWeightResolutionLog);
    }
    let mut seen = BTreeSet::new();
    for entry in weight_resolution_log {
        if entry.resolved_via == ResolvedVia::NameResolverForbidden {
            return Err(OracleError::NameResolverForbidden {
                tensor_id: entry.tensor_id.clone(),
            });
        }
        if !seen.insert(entry.tensor_id.clone()) {
            return Err(OracleError::DuplicateWeightResolution {
                tensor_id: entry.tensor_id.clone(),
            });
        }
    }
    Ok(())
}

fn quant_spec_resolved_logit_tensors(
    artifact: &ModelArtifact,
) -> Result<(Vec<ResolvedLogitTensor>, Vec<WeightResolutionEntry>), OracleError> {
    let mut resolved = Vec::new();
    let mut log = Vec::new();
    for tensor in deployable_weight_tensors(artifact) {
        let quant = artifact
            .core
            .quant
            .weight_quant(&tensor.id)
            .ok_or_else(|| OracleError::QuantSpecCoverageMissing {
                tensor_id: tensor.id.clone(),
            })?;
        resolved.push(ResolvedLogitTensor::from_tensor_quant(tensor, quant));
        log.push(WeightResolutionEntry {
            tensor_id: tensor.id.clone(),
            resolved_via: ResolvedVia::QuantSpec_weight_quant,
        });
    }
    Ok((resolved, log))
}

#[cfg(feature = "s3-oracle-adversarial")]
fn name_resolved_logit_tensors(
    artifact: &ModelArtifact,
) -> Result<Vec<ResolvedLogitTensor>, OracleError> {
    deployable_weight_tensors(artifact)
        .map(|tensor| {
            let quant = artifact
                .core
                .quant
                .weight_quant(&tensor.id)
                .ok_or_else(|| OracleError::QuantSpecCoverageMissing {
                    tensor_id: tensor.id.clone(),
                })?;
            let selected = deliberate_name_resolver_selected_tensor(artifact, tensor);
            Ok(if selected.id == tensor.id {
                ResolvedLogitTensor::from_tensor_quant(selected, quant)
            } else {
                ResolvedLogitTensor::from_tensor_name_fallback(selected)
            })
        })
        .collect()
}

fn deployable_weight_tensors(artifact: &ModelArtifact) -> impl Iterator<Item = &CanonicalTensor> {
    artifact
        .core
        .tensors
        .iter()
        .filter(|tensor| tensor.payload_role == PayloadRole::DeployableWeight)
}

#[cfg(feature = "s3-oracle-adversarial")]
fn deliberate_name_resolver_selected_tensor<'a>(
    artifact: &'a ModelArtifact,
    canonical: &'a CanonicalTensor,
) -> &'a CanonicalTensor {
    if canonical.id.as_str() == "linear_0_weight"
        && let Some(shadow) = artifact
            .core
            .tensors
            .iter()
            .find(|tensor| tensor.id.as_str() == "linear_0_weight_naive_fp32")
    {
        return shadow;
    }
    canonical
}

#[derive(Clone, Debug)]
struct ResolvedLogitTensor {
    tensor_id: CanonicalTensorId,
    dtype: Dtype,
    payload_sha: Hash256,
    quant_name: &'static str,
}

impl ResolvedLogitTensor {
    fn from_tensor_quant(tensor: &CanonicalTensor, quant: &WeightQuant) -> Self {
        Self {
            tensor_id: tensor.id.clone(),
            dtype: tensor.dtype,
            payload_sha: tensor.payload_sha,
            quant_name: stable_quant_name(quant),
        }
    }

    #[cfg(feature = "s3-oracle-adversarial")]
    fn from_tensor_name_fallback(tensor: &CanonicalTensor) -> Self {
        Self {
            tensor_id: tensor.id.clone(),
            dtype: tensor.dtype,
            payload_sha: tensor.payload_sha,
            quant_name: "name_fallback_fp32",
        }
    }
}

#[derive(Debug, Clone)]
struct PromptEvaluation {
    logits: Vec<f32>,
    hidden_state: Vec<f32>,
    argmax_token: CharId,
}

fn evaluate_prompt_with_resolved(
    artifact: &ModelArtifact,
    prompt: &TextCharSeq,
    resolved_weights: &[ResolvedLogitTensor],
) -> Result<PromptEvaluation, OracleError> {
    let logits = logits_from_resolved(artifact, prompt, resolved_weights);
    let hidden_state = hidden_state_from_resolved(artifact, prompt, resolved_weights);
    let (argmax_token, _) = argmax_lowest_index(&logits);
    Observation::post_logits(logits.clone())?;
    Observation::post_embedding(Some(hidden_state.clone()))?;
    Observation::post_decode(argmax_token)?;
    Ok(PromptEvaluation {
        logits,
        hidden_state,
        argmax_token,
    })
}

fn logits_from_resolved(
    artifact: &ModelArtifact,
    prompt: &TextCharSeq,
    resolved_weights: &[ResolvedLogitTensor],
) -> Vec<f32> {
    let mut logits = vec![0.0_f32; VOCAB_SIZE];
    let last = prompt.as_slice().last().copied().unwrap_or(0);
    for weight in resolved_weights {
        for (index, logit) in logits.iter_mut().enumerate() {
            let digest = logit_digest(artifact, weight, prompt, index as CharId);
            let centered = f32::from(digest.as_bytes()[index % 32]) / 255.0 - 0.5;
            *logit += centered * 0.25;
        }
    }

    if last == UNK_ID {
        let max = logits
            .iter()
            .copied()
            .fold(f32::NEG_INFINITY, |left, right| left.max(right));
        logits[EOS_ID as usize] = max + 1.0;
    } else {
        for token in [RESERVED_ID, BOS_ID, EOS_ID, UNK_ID] {
            logits[token as usize] = -1.0e30;
        }
    }

    logits
}

fn hidden_state_from_resolved(
    artifact: &ModelArtifact,
    prompt: &TextCharSeq,
    resolved_weights: &[ResolvedLogitTensor],
) -> Vec<f32> {
    let width = artifact.core.model.hidden_width as usize;
    (0..width)
        .map(|index| {
            let target = (index % VOCAB_SIZE) as CharId;
            let mut acc = 0.0_f32;
            for weight in resolved_weights {
                let digest = logit_digest(artifact, weight, prompt, target);
                acc += f32::from(digest.as_bytes()[(index + usize::from(target)) % 32]) / 255.0;
            }
            acc / resolved_weights.len().max(1) as f32
        })
        .collect()
}

fn logit_digest(
    artifact: &ModelArtifact,
    weight: &ResolvedLogitTensor,
    prompt: &TextCharSeq,
    target: CharId,
) -> Hash256 {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(LOGIT_DOMAIN_TAG);
    bytes.extend_from_slice(artifact.core.manifest.lineage.0.as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(weight.tensor_id.as_str().as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(stable_dtype_name(weight.dtype).as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(weight.payload_sha.as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(weight.quant_name.as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(prompt.as_slice());
    bytes.push(0);
    bytes.push(target);
    gbf_foundation::sha256(bytes)
}

fn append_generated_token(prompt: &TextCharSeq, token: CharId) -> Result<TextCharSeq, OracleError> {
    if !is_text_char_id(token) {
        return Err(OracleError::GeneratedTokenNotText { token });
    }
    let mut ids = prompt.as_slice().to_vec();
    ids.push(token);
    TextCharSeq::new(ids).map_err(OracleError::Lexical)
}

const fn stable_dtype_name(dtype: Dtype) -> &'static str {
    match dtype {
        Dtype::Fp32 => "fp32",
        Dtype::Ternary2 => "ternary2",
        Dtype::Q8_8 => "q8_8",
        Dtype::I32 => "i32",
    }
}

const fn stable_quant_name(quant: &WeightQuant) -> &'static str {
    match quant {
        WeightQuant::Fp32 => "fp32",
        WeightQuant::Ternary2 { .. } => "ternary2",
    }
}

fn compute_oracle_self_hash(
    observations: &ArtifactObservations,
    determinism_class: ArtifactDeterminismClass,
    weight_resolution_log: &[WeightResolutionEntry],
) -> Result<Hash256, OracleError> {
    let payload = observations_canonical::ProductHashPayload::from_observations(
        observations,
        determinism_class,
        weight_resolution_log,
    );
    let canonical = CanonicalJson::to_vec(&payload)?;
    ArtifactOracleProduct::domain()
        .hash_canonical_bytes(&canonical)
        .map_err(OracleError::CanonicalJson)
}

/// Errors produced by artifact oracle evaluation.
#[derive(Debug)]
pub enum OracleError {
    /// The artifact failed schema validation.
    Artifact(ArtifactError),
    /// Workload execution matrix did not enable artifact evaluation.
    ArtifactExecutionDisabled,
    /// Workload session decode mode was not argmax.
    DecodeModeMismatch,
    /// Observation policy did not require bit-exact determinism.
    DeterminismPolicyMismatch,
    /// Observation policy had no checkpoints.
    EmptyCheckpointPolicy,
    /// Observation policy requested zero generated steps.
    ZeroGeneratedSteps,
    /// Workload agreement subset was empty.
    EmptyPromptSubset,
    /// No observations were captured.
    EmptyObservations,
    /// No deployable weights were resolved.
    EmptyWeightResolutionLog,
    /// A deployable tensor was missing QuantSpec coverage.
    QuantSpecCoverageMissing {
        /// Tensor id.
        tensor_id: CanonicalTensorId,
    },
    /// A forbidden name resolver path was recorded.
    NameResolverForbidden {
        /// Tensor id.
        tensor_id: CanonicalTensorId,
    },
    /// Duplicate resolution entry.
    DuplicateWeightResolution {
        /// Tensor id.
        tensor_id: CanonicalTensorId,
    },
    /// Observation variant did not match its key checkpoint.
    CheckpointMismatch {
        /// Checkpoint in the key.
        key: SemanticCheckpoint,
        /// Checkpoint encoded by the observation variant.
        observed: SemanticCheckpoint,
    },
    /// Duplicate observation key.
    DuplicateObservation {
        /// Prompt id.
        prompt_id: String,
        /// Checkpoint.
        checkpoint: SemanticCheckpoint,
        /// Step.
        step: u32,
    },
    /// Logit observation did not have one row per charset id.
    LogitLength {
        /// Expected length.
        expected: usize,
        /// Actual length.
        actual: usize,
    },
    /// Decode token was outside the charset vocabulary.
    DecodeTokenOutOfRange {
        /// Observed token.
        token: CharId,
    },
    /// Generated token cannot be fed back as normalized text.
    GeneratedTokenNotText {
        /// Observed token.
        token: CharId,
    },
    /// Observation field included a non-finite float.
    NonFiniteObservation {
        /// Field name.
        field: &'static str,
        /// Field index.
        index: usize,
    },
    /// Canonical JSON failed.
    CanonicalJson(CanonicalJsonError),
    /// Generated prompt text failed lexical validation.
    Lexical(LexicalError),
    /// A shared observation constructor failed outside artifact-specific cases.
    DenotationalObservationInvariant(String),
    /// Tied alias references a tensor that is not present.
    TiedAliasMissingTensor {
        /// Missing tensor id.
        tensor_id: CanonicalTensorId,
    },
    /// Shared tied alias points at two different canonical tensors.
    TiedAliasNotShared {
        /// Embedding tensor id.
        embedding_id: CanonicalTensorId,
        /// Classifier tensor id.
        classifier_id: CanonicalTensorId,
    },
}

impl From<crate::denotational::OracleError> for OracleError {
    fn from(value: crate::denotational::OracleError) -> Self {
        match value {
            crate::denotational::OracleError::LogitLength { expected, actual } => {
                Self::LogitLength { expected, actual }
            }
            crate::denotational::OracleError::DecodeTokenOutOfRange { token } => {
                Self::DecodeTokenOutOfRange { token }
            }
            crate::denotational::OracleError::NonFiniteObservation { field, index } => {
                Self::NonFiniteObservation { field, index }
            }
            crate::denotational::OracleError::CanonicalJson(error) => Self::CanonicalJson(error),
            crate::denotational::OracleError::Lexical(error) => Self::Lexical(error),
            other => Self::DenotationalObservationInvariant(other.to_string()),
        }
    }
}

impl From<CanonicalJsonError> for OracleError {
    fn from(error: CanonicalJsonError) -> Self {
        Self::CanonicalJson(error)
    }
}

impl fmt::Display for OracleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Artifact(error) => write!(f, "{error}"),
            Self::ArtifactExecutionDisabled => {
                f.write_str("workload execution matrix must enable artifact")
            }
            Self::DecodeModeMismatch => f.write_str("workload session decode mode must be argmax"),
            Self::DeterminismPolicyMismatch => {
                f.write_str("observation policy must require BitExact determinism")
            }
            Self::EmptyCheckpointPolicy => f.write_str("observation policy has no checkpoints"),
            Self::ZeroGeneratedSteps => {
                f.write_str("observation policy generated_steps must be nonzero")
            }
            Self::EmptyPromptSubset => f.write_str("workload agreement subset is empty"),
            Self::EmptyObservations => f.write_str("artifact observations are empty"),
            Self::EmptyWeightResolutionLog => {
                f.write_str("artifact oracle resolved no deployable weights")
            }
            Self::QuantSpecCoverageMissing { tensor_id } => write!(
                f,
                "ArtifactOracle missing QuantSpec_S3 coverage for deployable tensor {tensor_id}"
            ),
            Self::NameResolverForbidden { tensor_id } => write!(
                f,
                "ArtifactOracle recorded forbidden name resolver path for tensor {tensor_id}"
            ),
            Self::DuplicateWeightResolution { tensor_id } => {
                write!(
                    f,
                    "duplicate artifact weight resolution entry for {tensor_id}"
                )
            }
            Self::CheckpointMismatch { key, observed } => write!(
                f,
                "observation checkpoint mismatch: key {}, observed {}",
                key.as_str(),
                observed.as_str()
            ),
            Self::DuplicateObservation {
                prompt_id,
                checkpoint,
                step,
            } => write!(
                f,
                "duplicate artifact observation for {prompt_id}/{} step {step}",
                checkpoint.as_str()
            ),
            Self::LogitLength { expected, actual } => {
                write!(
                    f,
                    "post_logits must contain {expected} logits, got {actual}"
                )
            }
            Self::DecodeTokenOutOfRange { token } => {
                write!(f, "post_decode token {token} is outside vocab")
            }
            Self::GeneratedTokenNotText { token } => {
                write!(f, "generated token {token} is not valid normalized text")
            }
            Self::NonFiniteObservation { field, index } => {
                write!(f, "{field}[{index}] is non-finite")
            }
            Self::CanonicalJson(error) => write!(f, "{error}"),
            Self::Lexical(error) => write!(f, "{error}"),
            Self::DenotationalObservationInvariant(error) => {
                write!(f, "artifact observation invariant failed: {error}")
            }
            Self::TiedAliasMissingTensor { tensor_id } => {
                write!(
                    f,
                    "tied embedding alias references missing tensor {tensor_id}"
                )
            }
            Self::TiedAliasNotShared {
                embedding_id,
                classifier_id,
            } => write!(
                f,
                "shared tied alias must use one canonical tensor, got {embedding_id} and {classifier_id}"
            ),
        }
    }
}

impl Error for OracleError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Artifact(error) => Some(error),
            Self::CanonicalJson(error) => Some(error),
            Self::Lexical(error) => Some(error),
            Self::ArtifactExecutionDisabled
            | Self::DecodeModeMismatch
            | Self::DeterminismPolicyMismatch
            | Self::EmptyCheckpointPolicy
            | Self::ZeroGeneratedSteps
            | Self::EmptyPromptSubset
            | Self::EmptyObservations
            | Self::EmptyWeightResolutionLog
            | Self::QuantSpecCoverageMissing { .. }
            | Self::NameResolverForbidden { .. }
            | Self::DuplicateWeightResolution { .. }
            | Self::CheckpointMismatch { .. }
            | Self::DuplicateObservation { .. }
            | Self::LogitLength { .. }
            | Self::DecodeTokenOutOfRange { .. }
            | Self::GeneratedTokenNotText { .. }
            | Self::NonFiniteObservation { .. }
            | Self::DenotationalObservationInvariant(_)
            | Self::TiedAliasMissingTensor { .. }
            | Self::TiedAliasNotShared { .. } => None,
        }
    }
}
