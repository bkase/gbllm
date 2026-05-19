//! Denotational oracle for S3 `ReferenceModelBundle` evaluation.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use gbf_artifact::{
    CharId, LexicalError, ReductionOrderPolicy, ReferenceDeterminism, ReferenceModelBundle,
    ReferenceNumericProfile, ReferenceScalarFormat, TextCharSeq, VOCAB_SIZE,
    evaluate_reference_program, is_text_char_id,
};
use gbf_foundation::{CanonicalJsonError, DomainHash, Hash256};
use gbf_workload::{
    ObservationCheckpoint, ObservationPolicy_S3, PromptId, S3DeterminismRequirement,
    WorkloadManifest_v0,
};
use serde::{Deserialize, Serialize};

pub mod observations_canonical;

#[cfg(feature = "s3-real")]
pub mod real;
#[cfg(feature = "s3-real")]
pub use real::RealDenotationalOracle;

#[cfg(feature = "s3-fallback")]
pub mod fallback;
#[cfg(feature = "s3-fallback")]
pub use fallback::S3DenotationalFallback;

pub use observations_canonical::ReferenceObservationsCanonical;

/// Tracing target for denotational oracle evaluation.
pub const DENOTATIONAL_ORACLE_LOG_TARGET: &str = "gbf_oracle::denotational";

/// Evaluation started event name.
pub const EVENT_NAME_EVALUATION_STARTED: &str = "s3::denotational_oracle::evaluation_started";
/// Per-checkpoint observation event name.
pub const EVENT_NAME_OBSERVATION_CAPTURED: &str = "s3::denotational_oracle::observation_captured";
/// Evaluation complete event name.
pub const EVENT_NAME_EVALUATION_COMPLETE: &str = "s3::denotational_oracle::evaluation_complete";

const PRODUCT_SCHEMA_VERSION: &str = "1";
const PRODUCT_SCHEMA_ID: &str = "denotational_oracle_product.v1";

/// F-C1 owner bead named by the S3 fallback contract.
pub const S3_DENOTATIONAL_FALLBACK_REAL_OWNER_BEAD: &str = "bd-1rcc";

/// Oracle backend label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DenotationalBackendKind {
    /// Real S3 denotational oracle backend.
    Real,
    /// S3 fallback backend pending the richer F-C1 implementation.
    Fallback,
}

impl DenotationalBackendKind {
    /// Stable logging field.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Real => "real",
            Self::Fallback => "fallback",
        }
    }
}

/// Determinism class for denotational observations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum DenotationalDeterminismClass {
    /// Bit-exact canonical observations.
    BitExact,
}

impl DenotationalDeterminismClass {
    /// Stable logging field.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BitExact => "BitExact",
        }
    }
}

/// Semantic checkpoint key stored in canonical observations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[allow(clippy::enum_variant_names)]
#[serde(rename_all = "snake_case")]
pub enum SemanticCheckpoint {
    /// After embedding lookup.
    PostEmbedding,
    /// After final reference-program logits.
    PostLogits,
    /// After argmax decode.
    PostDecode,
}

impl SemanticCheckpoint {
    /// Stable string used in canonical rows and logs.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PostEmbedding => "post_embedding",
            Self::PostLogits => "post_logits",
            Self::PostDecode => "post_decode",
        }
    }
}

impl From<ObservationCheckpoint> for SemanticCheckpoint {
    fn from(value: ObservationCheckpoint) -> Self {
        match value {
            ObservationCheckpoint::PostEmbedding => Self::PostEmbedding,
            ObservationCheckpoint::PostLogits => Self::PostLogits,
            ObservationCheckpoint::PostDecode => Self::PostDecode,
        }
    }
}

/// Checkpoint-specific denotational observation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(clippy::enum_variant_names)]
#[serde(tag = "checkpoint", rename_all = "snake_case", deny_unknown_fields)]
pub enum Observation {
    /// Embedding checkpoint. B15 records shape-safe absence instead of logits.
    PostEmbedding {
        /// Optional embedding hidden state when a producer exposes it.
        hidden_state: Option<Vec<f32>>,
    },
    /// Logits checkpoint. Must contain one value per charset_v1 id.
    PostLogits {
        /// Final reference-program logits.
        logits: Vec<f32>,
    },
    /// Decode checkpoint. Must contain exactly the decoded token id.
    PostDecode {
        /// Argmax token id.
        token: CharId,
    },
}

impl Observation {
    /// Construct a post-embedding observation.
    pub fn post_embedding(hidden_state: Option<Vec<f32>>) -> Result<Self, OracleError> {
        if let Some(values) = &hidden_state {
            validate_finite("hidden_state", values)?;
        }
        Ok(Self::PostEmbedding { hidden_state })
    }

    /// Construct a post-logits observation.
    pub fn post_logits(logits: Vec<f32>) -> Result<Self, OracleError> {
        if logits.len() != VOCAB_SIZE {
            return Err(OracleError::LogitLength {
                expected: VOCAB_SIZE,
                actual: logits.len(),
            });
        }
        validate_finite("logits", &logits)?;
        Ok(Self::PostLogits { logits })
    }

    /// Construct a post-decode observation.
    pub fn post_decode(token: CharId) -> Result<Self, OracleError> {
        if usize::from(token) >= VOCAB_SIZE {
            return Err(OracleError::DecodeTokenOutOfRange { token });
        }
        Ok(Self::PostDecode { token })
    }

    /// Checkpoint variant represented by this observation.
    #[must_use]
    pub const fn checkpoint(&self) -> SemanticCheckpoint {
        match self {
            Self::PostEmbedding { .. } => SemanticCheckpoint::PostEmbedding,
            Self::PostLogits { .. } => SemanticCheckpoint::PostLogits,
            Self::PostDecode { .. } => SemanticCheckpoint::PostDecode,
        }
    }
}

/// Sorted denotational observations keyed by prompt, checkpoint, and step.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ReferenceObservations(pub BTreeMap<(PromptId, SemanticCheckpoint, u32), Observation>);

impl ReferenceObservations {
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
        ReferenceObservationsCanonical::to_vec(self)
    }
}

/// Inputs consumed by a denotational oracle backend.
#[derive(Debug, Clone, Copy)]
pub struct DenotationalOracleInputs<'a> {
    /// Reference bundle to evaluate.
    pub bundle: &'a ReferenceModelBundle,
    /// S3 workload manifest supplying prompt ids and prompt text.
    pub workload: &'a WorkloadManifest_v0,
    /// Observation policy selecting checkpoints and trace length.
    pub observation_policy: &'a ObservationPolicy_S3,
}

impl<'a> DenotationalOracleInputs<'a> {
    /// Construct denotational oracle inputs.
    #[must_use]
    pub const fn new(
        bundle: &'a ReferenceModelBundle,
        workload: &'a WorkloadManifest_v0,
        observation_policy: &'a ObservationPolicy_S3,
    ) -> Self {
        Self {
            bundle,
            workload,
            observation_policy,
        }
    }
}

/// Product returned by denotational oracle evaluation.
#[derive(Debug, Clone, PartialEq)]
pub struct DenotationalOracleProduct {
    /// Canonical sorted observations.
    pub observations: ReferenceObservations,
    /// Pinned determinism class.
    pub determinism_class: DenotationalDeterminismClass,
    /// Self-hash over observations and determinism class.
    pub oracle_self_hash: Hash256,
    /// Backend that produced this product.
    pub backend_kind: DenotationalBackendKind,
    /// Owner bead for fallback real-backend completion, when applicable.
    pub real_owner_bead: Option<&'static str>,
}

impl DenotationalOracleProduct {
    /// Construct a product and compute its self-hash.
    pub fn new(
        observations: ReferenceObservations,
        backend_kind: DenotationalBackendKind,
        real_owner_bead: Option<&'static str>,
    ) -> Result<Self, OracleError> {
        if observations.is_empty() {
            return Err(OracleError::EmptyObservations);
        }
        let determinism_class = DenotationalDeterminismClass::BitExact;
        let oracle_self_hash = compute_oracle_self_hash(&observations, determinism_class)?;
        Ok(Self {
            observations,
            determinism_class,
            oracle_self_hash,
            backend_kind,
            real_owner_bead,
        })
    }

    /// DomainHash context for denotational oracle products.
    #[must_use]
    pub const fn domain() -> DomainHash<'static> {
        DomainHash::new(
            "gbf-oracle",
            "DenotationalOracleProduct",
            PRODUCT_SCHEMA_ID,
            PRODUCT_SCHEMA_VERSION,
        )
    }
}

/// Denotational oracle backend trait.
pub trait DenotationalOracle: Send + Sync {
    /// Evaluate a reference bundle over the supplied workload prompts.
    fn evaluate(
        &self,
        inputs: DenotationalOracleInputs<'_>,
    ) -> Result<DenotationalOracleProduct, OracleError>;
}

/// Shared implementation used by both real and fallback backends.
pub fn evaluate_with_backend_kind(
    inputs: DenotationalOracleInputs<'_>,
    backend_kind: DenotationalBackendKind,
    real_owner_bead: Option<&'static str>,
) -> Result<DenotationalOracleProduct, OracleError> {
    validate_inputs(inputs)?;
    tracing::info!(
        target: DENOTATIONAL_ORACLE_LOG_TARGET,
        event_name = EVENT_NAME_EVALUATION_STARTED,
        backend_kind = backend_kind.as_str(),
        workload_self_hash = %inputs.workload.workload_self_hash,
        bundle_self_hash = %inputs.bundle.bundle_self_hash,
        prompt_count = inputs.workload.agreement_subset().len() as u64,
    );

    let mut observations = ReferenceObservations::new();
    for prompt in inputs.workload.agreement_subset() {
        let mut current_prompt = prompt.prompt_chars.clone();
        for step in 0..inputs.observation_policy.agreement_trace.generated_steps {
            let evaluated = evaluate_reference_program(
                inputs.bundle,
                &current_prompt,
                inputs.observation_policy,
            );
            for checkpoint in &inputs.observation_policy.checkpoints {
                let checkpoint = SemanticCheckpoint::from(*checkpoint);
                let observation = match checkpoint {
                    SemanticCheckpoint::PostEmbedding => Observation::post_embedding(None)?,
                    SemanticCheckpoint::PostLogits => {
                        Observation::post_logits(evaluated.logits.clone())?
                    }
                    SemanticCheckpoint::PostDecode => {
                        Observation::post_decode(evaluated.argmax_token)?
                    }
                };
                observations.insert(prompt.id.clone(), checkpoint, step, observation)?;
                tracing::trace!(
                    target: DENOTATIONAL_ORACLE_LOG_TARGET,
                    event_name = EVENT_NAME_OBSERVATION_CAPTURED,
                    prompt_id = prompt.id.as_str(),
                    checkpoint = checkpoint.as_str(),
                    step,
                );
            }
            current_prompt = append_generated_token(&current_prompt, evaluated.argmax_token)?;
        }
    }

    let product = DenotationalOracleProduct::new(observations, backend_kind, real_owner_bead)?;
    tracing::info!(
        target: DENOTATIONAL_ORACLE_LOG_TARGET,
        event_name = EVENT_NAME_EVALUATION_COMPLETE,
        backend_kind = backend_kind.as_str(),
        observation_count = product.observations.len() as u64,
        oracle_self_hash = %product.oracle_self_hash,
        determinism_class = product.determinism_class.as_str(),
    );
    Ok(product)
}

fn validate_inputs(inputs: DenotationalOracleInputs<'_>) -> Result<(), OracleError> {
    validate_numeric_profile(&inputs.bundle.numeric)?;
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

fn validate_numeric_profile(numeric: &ReferenceNumericProfile) -> Result<(), OracleError> {
    if numeric.scalar_format != ReferenceScalarFormat::F32
        || numeric.reduction_order_policy != ReductionOrderPolicy::Enforced
        || numeric.determinism != ReferenceDeterminism::BitExact
    {
        return Err(OracleError::NumericProfileMismatch);
    }
    Ok(())
}

fn append_generated_token(prompt: &TextCharSeq, token: CharId) -> Result<TextCharSeq, OracleError> {
    if !is_text_char_id(token) {
        return Err(OracleError::GeneratedTokenNotText { token });
    }
    let mut ids = prompt.as_slice().to_vec();
    ids.push(token);
    TextCharSeq::new(ids).map_err(OracleError::Lexical)
}

fn validate_finite(field: &'static str, values: &[f32]) -> Result<(), OracleError> {
    for (index, value) in values.iter().enumerate() {
        if !value.is_finite() {
            return Err(OracleError::NonFiniteObservation { field, index });
        }
    }
    Ok(())
}

fn compute_oracle_self_hash(
    observations: &ReferenceObservations,
    determinism_class: DenotationalDeterminismClass,
) -> Result<Hash256, OracleError> {
    let payload = observations_canonical::ProductHashPayload::from_observations(
        observations,
        determinism_class,
    );
    let canonical = gbf_foundation::CanonicalJson::to_vec(&payload)?;
    DenotationalOracleProduct::domain()
        .hash_canonical_bytes(&canonical)
        .map_err(OracleError::CanonicalJson)
}

/// Errors produced by denotational oracle evaluation.
#[derive(Debug)]
pub enum OracleError {
    /// The bundle numeric profile is not the S3 bit-exact profile.
    NumericProfileMismatch,
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
}

impl fmt::Display for OracleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NumericProfileMismatch => {
                f.write_str("reference bundle numeric profile must be F32, enforced, BitExact")
            }
            Self::DeterminismPolicyMismatch => {
                f.write_str("observation policy must require BitExact determinism")
            }
            Self::EmptyCheckpointPolicy => f.write_str("observation policy has no checkpoints"),
            Self::ZeroGeneratedSteps => {
                f.write_str("observation policy generated_steps must be nonzero")
            }
            Self::EmptyPromptSubset => f.write_str("workload agreement subset is empty"),
            Self::EmptyObservations => f.write_str("denotational observations are empty"),
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
                "duplicate denotational observation for {prompt_id}/{} step {step}",
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
        }
    }
}

impl Error for OracleError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::CanonicalJson(error) => Some(error),
            Self::Lexical(error) => Some(error),
            Self::NumericProfileMismatch
            | Self::DeterminismPolicyMismatch
            | Self::EmptyCheckpointPolicy
            | Self::ZeroGeneratedSteps
            | Self::EmptyPromptSubset
            | Self::EmptyObservations
            | Self::CheckpointMismatch { .. }
            | Self::DuplicateObservation { .. }
            | Self::LogitLength { .. }
            | Self::DecodeTokenOutOfRange { .. }
            | Self::GeneratedTokenNotText { .. }
            | Self::NonFiniteObservation { .. } => None,
        }
    }
}

impl From<CanonicalJsonError> for OracleError {
    fn from(error: CanonicalJsonError) -> Self {
        Self::CanonicalJson(error)
    }
}
