//! Phase A run orchestration for S1.

use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::Path;

use gbf_artifact::ids::ArtifactPath;
use gbf_artifact::tensor::{
    CanonicalTensor, CanonicalTensorKind, CanonicalTensorLayout, CanonicalTensorPayload,
    CanonicalTensorShape, TensorElementType, canonical_tensor_payload_hash,
};
use gbf_foundation::{Hash256, SemVer, sha256};
use gbf_policy::model_profile::ModelSizeProfile;
use gbf_train::adapter::burn::{
    BurnAdapterError, BurnAutodiffBackend, BurnBackend, BurnDevice, BurnFloatTensor,
    BurnGradientsParams, BurnModule, BurnNdArrayAutodiffBackend, BurnOptimizer, BurnParam,
    adamw_config, burn_linear, burn_log_softmax, burn_relu, float_tensor_from_vec,
    float_tensor_into_vec,
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::s1::build_metadata::{BUILD_KIND, BuildMetadata, build_metadata};
use crate::s1::device_profile::{
    DeviceProfileEnforceError, DeviceProfileEnforcement, S1CpuDeterministic, enforce,
    enforce_with_environment,
};
use crate::s1::logging::{
    DivergenceObserved as LogDivergenceObserved, RunDivergenceEvent, S1LogEmitter,
};
use crate::s1::manifest::ByteSeq;
use crate::s1::rng::{BatchRng, InitRng, S1Rng, rng_stream_def_hash, uniform_u64_inclusive};
use crate::s1::schema::{
    CheckpointMetadata as S1CheckpointMetadata, DomainHash, GradNormSummary, RunLog, S1BuildKind,
    S1Completion, S1SchemaError,
};

/// RFC-pinned production sequence length for S1 training samples.
pub const S1_SEQUENCE_LENGTH: usize = 128;
/// RFC-pinned production batch size for S1.
pub const S1_BATCH_SIZE: usize = 32;
/// RFC-pinned production optimizer-step count for S1.
pub const S1_OPTIMIZER_STEPS: u64 = 10_000;
/// RFC-pinned progress-eval cadence for S1.
pub const S1_EVAL_EVERY_STEPS: u64 = 1_000;
/// RFC-pinned progress-eval subset size in 128-byte sequences.
pub const S1_EVAL_SUBSET_SIZE: u64 = 4_096;
/// Integration-only optimizer-step count.
pub const S1_INTEGRATION_OPTIMIZER_STEPS: u64 = 100;
/// Integration-only batch size.
pub const S1_INTEGRATION_BATCH_SIZE: usize = 4;
/// Integration-only sequence length.
pub const S1_INTEGRATION_SEQUENCE_LENGTH: usize = 32;
/// Integration-only eval cadence.
pub const S1_INTEGRATION_EVAL_EVERY_STEPS: u64 = 25;
/// Integration-only eval subset size in 32-byte sequences.
pub const S1_INTEGRATION_EVAL_SUBSET_SIZE: u64 = 8;
const S1_BYTE_VOCAB_SIZE: usize = 256;
const S1_DENSE_STATE_DECAY: f32 = 0.5;

fn s1_dense_state_slots(profile: ModelSizeProfile) -> Result<usize, S1RunError> {
    match profile {
        ModelSizeProfile::Toy0 => Ok(4),
        ModelSizeProfile::Toy1 => Ok(8),
        _ => Err(S1RunError::InvalidModelConfig {
            field: "model_config",
        }),
    }
}

fn s1_dense_block_count(profile: ModelSizeProfile) -> Result<usize, S1RunError> {
    match profile {
        ModelSizeProfile::Toy0 => Ok(1),
        ModelSizeProfile::Toy1 => Ok(usize::from(ModelSizeProfile::TOY1_N_BLOCKS)),
        _ => Err(S1RunError::InvalidModelConfig {
            field: "model_config",
        }),
    }
}

fn s1_profile_prefix(profile: ModelSizeProfile) -> Result<&'static str, S1RunError> {
    match profile {
        ModelSizeProfile::Toy0 => Ok("toy0"),
        ModelSizeProfile::Toy1 => Ok("toy1"),
        _ => Err(S1RunError::InvalidModelConfig {
            field: "model_config",
        }),
    }
}

fn s1_production_tensor_id(profile: ModelSizeProfile, suffix: &str) -> Result<String, S1RunError> {
    Ok(format!(
        "{}.production.{suffix}",
        s1_profile_prefix(profile)?
    ))
}

fn s1_production_block_tensor_id(
    profile: ModelSizeProfile,
    block_index: usize,
    suffix: &str,
) -> Result<String, S1RunError> {
    if s1_dense_block_count(profile)? == 1 {
        s1_production_tensor_id(profile, suffix)
    } else {
        s1_production_tensor_id(profile, &format!("blocks.{block_index}.{suffix}"))
    }
}

/// Deterministic inputs to one S1 run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunInputs {
    /// Raw train split bytes verified against the TinyStories manifest.
    pub corpus_train: ByteSeq,
    /// Raw validation split bytes verified against the TinyStories manifest.
    pub corpus_val: ByteSeq,
    /// Registered model profile. S1 production runners accept Toy0 and the
    /// Toy1 follow-up profile only.
    pub model_config: ModelSizeProfile,
    /// RFC-pinned train configuration.
    pub train_config: TrainConfig,
    /// S1 seed.
    pub seed: u64,
    /// Training budget profile. Only Production artifacts are closure candidates.
    pub budget_profile: TrainBudgetProfile,
}

/// S1 training budget profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrainBudgetProfile {
    /// RFC D3 production budget. Artifacts are closure candidates.
    Production,
    /// Test-only dry run budget. Artifacts are explicitly non-production.
    IntegrationFixture,
}

impl TrainBudgetProfile {
    /// Return the pinned config for this budget profile.
    #[must_use]
    pub fn train_config(self) -> TrainConfig {
        match self {
            Self::Production => TrainConfig::pinned(),
            Self::IntegrationFixture => TrainConfig::integration_fixture(),
        }
    }

    /// Stable metadata value embedded in checkpoint artifacts.
    #[must_use]
    pub const fn as_metadata_str(self) -> &'static str {
        match self {
            Self::Production => "production",
            Self::IntegrationFixture => "integration_fixture",
        }
    }
}

/// RFC-pinned TrainConfig for F-S1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrainConfig {
    /// Number of optimizer steps.
    pub optimizer_steps: u64,
    /// Number of sampled sequences per optimizer step.
    pub batch_size: usize,
    /// Number of bytes per sampled sequence.
    pub sequence_length: usize,
    /// Progress evaluation cadence.
    pub eval_every_steps: u64,
    /// Progress evaluation subset size in 128-byte sequences.
    pub eval_subset_size: u64,
    /// AdamW optimizer configuration.
    pub optimizer: AdamWConfig,
    /// Fixed Phase A scheduler state.
    pub phase: TrainPhase,
    /// Deterministic RNG kind.
    pub rng_kind: RngKind,
    /// Required CPU deterministic device profile.
    pub device_profile: S1CpuDeterministic,
}

impl TrainConfig {
    /// Return the production S1 TrainConfig pinned by D3 and D10.
    #[must_use]
    pub fn pinned() -> Self {
        Self {
            optimizer_steps: S1_OPTIMIZER_STEPS,
            batch_size: S1_BATCH_SIZE,
            sequence_length: S1_SEQUENCE_LENGTH,
            eval_every_steps: S1_EVAL_EVERY_STEPS,
            eval_subset_size: S1_EVAL_SUBSET_SIZE,
            optimizer: AdamWConfig::pinned(),
            phase: TrainPhase::A,
            rng_kind: RngKind::Pcg64Mcg,
            device_profile: S1CpuDeterministic::canonical(),
        }
    }

    /// Return the test-only S1 TrainConfig used by integration fixtures.
    #[must_use]
    pub fn integration_fixture() -> Self {
        Self {
            optimizer_steps: S1_INTEGRATION_OPTIMIZER_STEPS,
            batch_size: S1_INTEGRATION_BATCH_SIZE,
            sequence_length: S1_INTEGRATION_SEQUENCE_LENGTH,
            eval_every_steps: S1_INTEGRATION_EVAL_EVERY_STEPS,
            eval_subset_size: S1_INTEGRATION_EVAL_SUBSET_SIZE,
            optimizer: AdamWConfig::pinned(),
            phase: TrainPhase::A,
            rng_kind: RngKind::Pcg64Mcg,
            device_profile: S1CpuDeterministic::canonical(),
        }
    }

    /// Whether the config is the RFC-pinned production profile.
    #[must_use]
    pub fn is_pinned(&self) -> bool {
        self == &Self::pinned()
    }

    /// Whether this is exactly the integration-fixture budget.
    #[must_use]
    pub fn is_integration_fixture(&self) -> bool {
        self == &Self::integration_fixture()
    }
}

impl Default for TrainConfig {
    fn default() -> Self {
        Self::pinned()
    }
}

/// AdamW parameters pinned by D10.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdamWConfig {
    /// Learning rate.
    pub lr: f32,
    /// First-moment decay.
    pub beta1: f32,
    /// Second-moment decay.
    pub beta2: f32,
    /// Numerical epsilon.
    pub eps: f32,
    /// Weight decay.
    pub weight_decay: f32,
}

impl AdamWConfig {
    /// Return the RFC-pinned AdamW parameters.
    #[must_use]
    pub const fn pinned() -> Self {
        Self {
            lr: 1.0e-3,
            beta1: 0.9,
            beta2: 0.999,
            eps: 1.0e-8,
            weight_decay: 0.0,
        }
    }
}

impl Eq for AdamWConfig {}

/// S1 run phase selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrainPhase {
    /// Phase A with all quantization hardness off.
    A,
}

/// Deterministic RNG implementation selected for S1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RngKind {
    /// PCG XSL RR 128/64 MCG.
    Pcg64Mcg,
}

/// A sampled S1 training sequence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sequence {
    /// Lexicographic batch element index within the step.
    pub batch_index: usize,
    /// Byte offset in `corpus_train` where this sequence begins.
    pub start_offset: u64,
    /// Raw sequence bytes, always `sequence_length` long.
    pub bytes: Vec<u8>,
}

/// Deterministic training batch sampler backed by the BatchRng stream.
#[derive(Debug, Clone)]
pub struct BatchSampler<'a> {
    corpus_train: &'a [u8],
    batch_size: usize,
    sequence_length: usize,
    max_start_offset: u64,
    rng: BatchRng,
}

impl<'a> BatchSampler<'a> {
    /// Construct the sampler, initializing BatchRng exactly once.
    pub fn new(
        corpus_train: &'a ByteSeq,
        train_config: &TrainConfig,
        seed: u64,
    ) -> Result<Self, BatchSamplerError> {
        if train_config.sequence_length != S1_SEQUENCE_LENGTH
            && train_config.sequence_length != S1_INTEGRATION_SEQUENCE_LENGTH
        {
            return Err(BatchSamplerError::NonProductionSequenceLength {
                observed: train_config.sequence_length,
                required: S1_SEQUENCE_LENGTH,
            });
        }
        if corpus_train.len() < train_config.sequence_length {
            return Err(BatchSamplerError::TrainingCorpusTooShort {
                corpus_len: corpus_train.len(),
                sequence_length: train_config.sequence_length,
            });
        }

        let max_start_offset = corpus_train.len() - train_config.sequence_length;
        let max_start_offset = u64::try_from(max_start_offset).map_err(|_| {
            BatchSamplerError::CorpusLengthOverflow {
                corpus_len: corpus_train.len(),
            }
        })?;

        tracing::debug!(
            target: "gbf_experiments::s1",
            event = "s1.batch_sampler.init",
            seed,
            corpus_byte_count = corpus_train.len()
        );

        Ok(Self {
            corpus_train,
            batch_size: train_config.batch_size,
            sequence_length: train_config.sequence_length,
            max_start_offset,
            rng: BatchRng::new(seed),
        })
    }

    /// Draw one optimizer-step batch in lexicographic batch-index order.
    pub fn draw_step(&mut self, step: u64) -> Result<Vec<Sequence>, BatchSamplerError> {
        tracing::trace!(
            target: "gbf_experiments::s1",
            event = "s1.batch_sampler.draw_step",
            step,
            batch_size = self.batch_size
        );

        let mut batch = Vec::with_capacity(self.batch_size);
        for batch_index in 0..self.batch_size {
            let start_offset = uniform_u64_inclusive(&mut self.rng, 0, self.max_start_offset);
            let start = usize::try_from(start_offset)
                .map_err(|_| BatchSamplerError::OffsetOverflow { start_offset })?;
            let end = start.checked_add(self.sequence_length).ok_or(
                BatchSamplerError::SliceEndOverflow {
                    start,
                    sequence_length: self.sequence_length,
                },
            )?;
            batch.push(Sequence {
                batch_index,
                start_offset,
                bytes: self.corpus_train[start..end].to_vec(),
            });
        }
        Ok(batch)
    }
}

/// Batch sampler construction and draw errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BatchSamplerError {
    /// S1 production sampling is pinned to 128-byte sequences.
    NonProductionSequenceLength {
        /// Observed sequence length.
        observed: usize,
        /// Required production sequence length.
        required: usize,
    },
    /// S1-Pre-5: training corpus must contain at least one full sequence.
    TrainingCorpusTooShort {
        /// Training corpus byte length.
        corpus_len: usize,
        /// Required sequence length.
        sequence_length: usize,
    },
    /// Corpus length could not be represented as `u64`.
    CorpusLengthOverflow {
        /// Training corpus byte length.
        corpus_len: usize,
    },
    /// A sampled offset could not be represented as `usize`.
    OffsetOverflow {
        /// Sampled offset.
        start_offset: u64,
    },
    /// Slice end overflowed while materializing a sample.
    SliceEndOverflow {
        /// Sample start.
        start: usize,
        /// Sequence length.
        sequence_length: usize,
    },
}

impl fmt::Display for BatchSamplerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonProductionSequenceLength { observed, required } => write!(
                f,
                "S1 production sequence_length must be {required}, observed {observed}"
            ),
            Self::TrainingCorpusTooShort {
                corpus_len,
                sequence_length,
            } => write!(
                f,
                "S1-Pre-5 requires corpus_train length {corpus_len} >= sequence_length {sequence_length}"
            ),
            Self::CorpusLengthOverflow { corpus_len } => {
                write!(f, "corpus_train length {corpus_len} exceeds u64")
            }
            Self::OffsetOverflow { start_offset } => {
                write!(f, "sampled offset {start_offset} exceeds usize")
            }
            Self::SliceEndOverflow {
                start,
                sequence_length,
            } => write!(
                f,
                "sample slice end overflowed for start {start} and sequence_length {sequence_length}"
            ),
        }
    }
}

impl Error for BatchSamplerError {}

/// S1 run product emitted by the train runner.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RunProduct {
    /// Completed run.
    Completed(Box<CompletedRunProduct>),
    /// Diverged run.
    Diverged(Box<DivergedRunProduct>),
}

/// Completed S1 run product.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CompletedRunProduct {
    /// S1 seed.
    pub seed: u64,
    /// Final SafeTensors checkpoint bytes.
    pub final_checkpoint: Vec<u8>,
    /// SHA-256 of the checkpoint bytes.
    pub final_checkpoint_sha: Hash256,
    /// Runtime checkpoint metadata.
    pub metadata: S1CheckpointMetadata,
    /// Run log artifact.
    pub run_log: RunLog,
    /// Weight stats recorded at eval cadence.
    pub weight_stats: Vec<WeightStatsPoint>,
    /// Gradient log recorded per optimizer step.
    pub grad_log: Vec<GradLogPoint>,
    /// Completion state.
    pub completion: S1Completion,
}

/// Diverged S1 run product.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DivergedRunProduct {
    /// S1 seed.
    pub seed: u64,
    /// Run log recorded until divergence.
    pub run_log: RunLog,
    /// Weight stats recorded until divergence.
    pub weight_stats: Vec<WeightStatsPoint>,
    /// Gradient log recorded until divergence.
    pub grad_log: Vec<GradLogPoint>,
    /// Completion state.
    pub completion: S1Completion,
    /// First non-finite event. This record contains no NaN/Inf payload.
    pub divergence_event: DivergenceEvent,
}

/// First non-finite loss or gradient observation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DivergenceEvent {
    /// First diverged training step.
    pub step: u64,
    /// Which signal was non-finite.
    pub observed: DivergenceObserved,
    /// Last finite loss before divergence, if one had been observed.
    pub last_finite_loss: Option<LossNatsPerByte>,
}

/// Non-finite signal kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DivergenceObserved {
    /// Training loss was NaN or infinite.
    NonFiniteLoss,
    /// Global gradient norm was NaN or infinite.
    NonFiniteGradNorm,
}

/// Finite, non-negative natural-log cross entropy per byte.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct LossNatsPerByte(f32);

impl LossNatsPerByte {
    /// Construct a checked finite, non-negative loss value.
    pub fn new(value: f32) -> Result<Self, NonFiniteRunScalarError> {
        if !value.is_finite() {
            return Err(NonFiniteRunScalarError::NonFiniteLoss);
        }
        if value < 0.0 {
            return Err(NonFiniteRunScalarError::NegativeLoss);
        }
        Ok(Self(if value == 0.0 { 0.0 } else { value }))
    }

    /// Return the wrapped scalar.
    #[must_use]
    pub const fn get(self) -> f32 {
        self.0
    }
}

impl Serialize for LossNatsPerByte {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_f32(self.0)
    }
}

impl<'de> Deserialize<'de> for LossNatsPerByte {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = f32::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

/// Checked run-scalar construction errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NonFiniteRunScalarError {
    /// Loss was NaN or infinite.
    NonFiniteLoss,
    /// Loss was negative.
    NegativeLoss,
}

impl fmt::Display for NonFiniteRunScalarError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFiniteLoss => f.write_str("loss must be finite"),
            Self::NegativeLoss => f.write_str("loss must be non-negative"),
        }
    }
}

impl Error for NonFiniteRunScalarError {}

/// Weight statistics sidecar point.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeightStatsPoint {
    /// Training or eval step.
    pub step: u64,
    /// Stable hash of the canonical tensor payload at this point.
    pub tensor_payload_hash: Hash256,
    /// Minimum finite f32 tensor value at this point.
    pub min_weight: f32,
    /// Maximum finite f32 tensor value at this point.
    pub max_weight: f32,
}

/// Gradient log sidecar point.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GradLogPoint {
    /// Optimizer step.
    pub step: u64,
    /// Finite, global L2 gradient norm for this step.
    pub grad_norm_l2: f32,
    /// Finite training loss for this step.
    pub loss_nats_per_byte: f32,
}

/// Options used by focused tests to exercise non-production failure paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RunTestOptions {
    /// Inject a non-finite loss at this optimizer step.
    #[cfg(feature = "falsify")]
    pub inject_non_finite_loss_at_step: Option<u64>,
    /// Inject a non-finite grad norm at this optimizer step.
    #[cfg(feature = "falsify")]
    pub inject_non_finite_grad_norm_at_step: Option<u64>,
    /// Zero every gradient before optimizer application.
    #[cfg(feature = "falsify")]
    pub zero_gradients: bool,
    /// Test-only cap on runtime optimizer steps. Does not alter TrainConfig hashes.
    pub effective_optimizer_steps: Option<u64>,
    /// Test-only eval cadence override. Does not alter TrainConfig hashes.
    pub effective_eval_every_steps: Option<u64>,
    /// Test-only eval subset override. Does not alter TrainConfig hashes.
    pub effective_eval_subset_size: Option<u64>,
}

impl RunTestOptions {
    #[cfg(feature = "falsify")]
    fn inject_non_finite_loss_at_step(self) -> Option<u64> {
        self.inject_non_finite_loss_at_step
    }

    #[cfg(not(feature = "falsify"))]
    fn inject_non_finite_loss_at_step(self) -> Option<u64> {
        None
    }

    #[cfg(feature = "falsify")]
    fn inject_non_finite_grad_norm_at_step(self) -> Option<u64> {
        self.inject_non_finite_grad_norm_at_step
    }

    #[cfg(not(feature = "falsify"))]
    fn inject_non_finite_grad_norm_at_step(self) -> Option<u64> {
        None
    }

    #[cfg(feature = "falsify")]
    fn zero_gradients(self) -> bool {
        self.zero_gradients
    }

    #[cfg(not(feature = "falsify"))]
    fn zero_gradients(self) -> bool {
        false
    }
}

/// Errors returned before an S1 run product can be produced.
#[derive(Debug)]
pub enum S1RunError {
    /// S1-Pre-2: model profile must be Toy0 or Toy1.
    InvalidModelConfig {
        /// Field name.
        field: &'static str,
    },
    /// S1-Pre-3: train config must match the selected budget profile exactly.
    InvalidTrainConfig {
        /// Field name.
        field: &'static str,
        /// Selected budget profile.
        budget_profile: TrainBudgetProfile,
    },
    /// S1-Pre-4: seed must be in 0..=4.
    InvalidSeed {
        /// Field name.
        field: &'static str,
        /// Observed seed.
        observed: u64,
    },
    /// S1-Pre-5: train split is too short for one sequence.
    TrainCorpusTooShort {
        /// Field name.
        field: &'static str,
        /// Observed byte length.
        len: usize,
        /// Required byte length.
        required_at_least: usize,
    },
    /// S1-Pre-6: validation split must not be empty.
    ValCorpusEmpty {
        /// Field name.
        field: &'static str,
    },
    /// F-S1.04 deterministic device-profile enforcement failed.
    DeviceProfile(DeviceProfileEnforceError),
    /// Test-only run options were invalid.
    InvalidTestRunOptions {
        /// Field name.
        field: &'static str,
    },
    /// Batch sampler construction or draw failed.
    BatchSampler(BatchSamplerError),
    /// Schema hash construction failed.
    Schema(S1SchemaError),
    /// Canonical checkpoint construction failed.
    Checkpoint(CheckpointWriteError),
    /// Canonical tensor construction failed.
    Tensor(gbf_artifact::tensor::CanonicalTensorError),
    /// Artifact tensor path construction failed.
    ArtifactPath(gbf_artifact::ids::ArtifactPathError),
    /// Checked scalar construction failed.
    Scalar(NonFiniteRunScalarError),
    /// Burn adapter operation failed.
    BurnAdapter(BurnAdapterError),
    /// A Burn parameter that participates in the loss did not receive a gradient.
    MissingGradient {
        /// Parameter name.
        parameter: &'static str,
    },
}

impl fmt::Display for S1RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidModelConfig { field } => {
                write!(f, "S1-Pre-2 failed: {field} must equal Toy0 or Toy1")
            }
            Self::InvalidTrainConfig {
                field,
                budget_profile,
            } => write!(
                f,
                "S1-Pre-3 failed: {field} does not match {budget_profile:?} pinned TrainConfig"
            ),
            Self::InvalidSeed { field, observed } => write!(
                f,
                "S1-Pre-4 failed: {field} must be in 0..=4, observed {observed}"
            ),
            Self::TrainCorpusTooShort {
                field,
                len,
                required_at_least,
            } => write!(
                f,
                "S1-Pre-5 failed: {field} length {len} < {required_at_least}"
            ),
            Self::ValCorpusEmpty { field } => {
                write!(f, "S1-Pre-6 failed: {field} must not be empty")
            }
            Self::DeviceProfile(error) => write!(f, "{error}"),
            Self::InvalidTestRunOptions { field } => {
                write!(f, "test-only run option {field} is invalid")
            }
            Self::BatchSampler(error) => write!(f, "{error}"),
            Self::Schema(error) => write!(f, "{error}"),
            Self::Checkpoint(error) => write!(f, "{error}"),
            Self::Tensor(error) => write!(f, "{error}"),
            Self::ArtifactPath(error) => write!(f, "{error}"),
            Self::Scalar(error) => write!(f, "{error}"),
            Self::BurnAdapter(error) => write!(f, "{error}"),
            Self::MissingGradient { parameter } => {
                write!(f, "missing gradient for production parameter {parameter}")
            }
        }
    }
}

impl Error for S1RunError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::DeviceProfile(error) => Some(error),
            Self::BatchSampler(error) => Some(error),
            Self::Schema(error) => Some(error),
            Self::Checkpoint(error) => Some(error),
            Self::Tensor(error) => Some(error),
            Self::ArtifactPath(error) => Some(error),
            Self::Scalar(error) => Some(error),
            Self::BurnAdapter(error) => Some(error),
            Self::InvalidModelConfig { .. }
            | Self::InvalidTrainConfig { .. }
            | Self::InvalidSeed { .. }
            | Self::TrainCorpusTooShort { .. }
            | Self::ValCorpusEmpty { .. }
            | Self::InvalidTestRunOptions { .. }
            | Self::MissingGradient { .. } => None,
        }
    }
}

impl From<DeviceProfileEnforceError> for S1RunError {
    fn from(error: DeviceProfileEnforceError) -> Self {
        Self::DeviceProfile(error)
    }
}

impl From<BatchSamplerError> for S1RunError {
    fn from(error: BatchSamplerError) -> Self {
        Self::BatchSampler(error)
    }
}

impl From<S1SchemaError> for S1RunError {
    fn from(error: S1SchemaError) -> Self {
        Self::Schema(error)
    }
}

impl From<CheckpointWriteError> for S1RunError {
    fn from(error: CheckpointWriteError) -> Self {
        Self::Checkpoint(error)
    }
}

impl From<gbf_artifact::tensor::CanonicalTensorError> for S1RunError {
    fn from(error: gbf_artifact::tensor::CanonicalTensorError) -> Self {
        Self::Tensor(error)
    }
}

impl From<gbf_artifact::ids::ArtifactPathError> for S1RunError {
    fn from(error: gbf_artifact::ids::ArtifactPathError) -> Self {
        Self::ArtifactPath(error)
    }
}

impl From<NonFiniteRunScalarError> for S1RunError {
    fn from(error: NonFiniteRunScalarError) -> Self {
        Self::Scalar(error)
    }
}

impl From<BurnAdapterError> for S1RunError {
    fn from(error: BurnAdapterError) -> Self {
        Self::BurnAdapter(error)
    }
}

/// Run one S1 training attempt.
///
/// Production preconditions are validated and the deterministic device profile
/// is enforced before tensor/checkpoint allocation. Production uses the Burn
/// CPU autodiff stack and the RFC-pinned Toy0 train budget.
pub fn s1_train_run(inputs: RunInputs) -> Result<RunProduct, S1RunError> {
    validate_preconditions(&inputs)?;
    let enforcement = enforce(&inputs.train_config.device_profile)?;
    s1_train_run_after_enforcement(inputs, enforcement, RunTestOptions::default())
}

/// Test helper that runs with explicit falsification/runtime options and the current environment.
pub fn s1_train_run_with_options(
    inputs: RunInputs,
    options: RunTestOptions,
) -> Result<RunProduct, S1RunError> {
    validate_preconditions(&inputs)?;
    let enforcement = enforce(&inputs.train_config.device_profile)?;
    s1_train_run_after_enforcement(inputs, enforcement, options)
}

/// Test helper that runs with an explicit environment snapshot.
pub fn s1_train_run_with_environment<I, K, V>(
    inputs: RunInputs,
    environment: I,
) -> Result<RunProduct, S1RunError>
where
    I: IntoIterator<Item = (K, V)>,
    K: Into<std::ffi::OsString>,
    V: Into<std::ffi::OsString>,
{
    validate_preconditions(&inputs)?;
    let enforcement = enforce_with_environment(&inputs.train_config.device_profile, environment)?;
    s1_train_run_after_enforcement(inputs, enforcement, RunTestOptions::default())
}

/// Test helper that runs with explicit environment and falsification options.
pub fn s1_train_run_with_environment_and_options<I, K, V>(
    inputs: RunInputs,
    environment: I,
    options: RunTestOptions,
) -> Result<RunProduct, S1RunError>
where
    I: IntoIterator<Item = (K, V)>,
    K: Into<std::ffi::OsString>,
    V: Into<std::ffi::OsString>,
{
    validate_preconditions(&inputs)?;
    let enforcement = enforce_with_environment(&inputs.train_config.device_profile, environment)?;
    s1_train_run_after_enforcement(inputs, enforcement, options)
}

fn s1_train_run_after_enforcement(
    inputs: RunInputs,
    enforcement: DeviceProfileEnforcement,
    options: RunTestOptions,
) -> Result<RunProduct, S1RunError> {
    let effective_budget =
        EffectiveTrainBudget::new(inputs.budget_profile, &inputs.train_config, options)?;

    if inputs.budget_profile == TrainBudgetProfile::Production {
        return production_train_run::<BurnNdArrayAutodiffBackend>(
            inputs,
            enforcement,
            options,
            effective_budget,
        );
    }

    integration_fixture_train_run(inputs, enforcement, options, effective_budget)
}

fn integration_fixture_train_run(
    inputs: RunInputs,
    enforcement: DeviceProfileEnforcement,
    options: RunTestOptions,
    effective_budget: EffectiveTrainBudget,
) -> Result<RunProduct, S1RunError> {
    let mut model = IntegrationFixtureModel::initialize(inputs.seed)?;
    let mut sampler = BatchSampler::new(&inputs.corpus_train, &inputs.train_config, inputs.seed)?;
    let train_config_hash = train_config_hash(&inputs.train_config)?;
    let mut losses = Vec::with_capacity(effective_budget.optimizer_steps as usize);
    let mut eval_points = Vec::with_capacity(effective_budget.eval_point_count());
    let mut grad_log = Vec::with_capacity(effective_budget.optimizer_steps as usize);
    let mut weight_stats = Vec::with_capacity(effective_budget.eval_point_count());
    let mut last_finite_loss = None;

    eval_points.push((0, evaluate_fixture_bpc(&model, &inputs, effective_budget)));
    weight_stats.push(model.weight_stats(0)?);

    for step in 1..=effective_budget.optimizer_steps {
        let batch = sampler.draw_step(step)?;
        let mut loss = fixture_step_loss(&model, &batch, step);
        if options.inject_non_finite_loss_at_step() == Some(step) {
            loss = f32::NAN;
        }

        if !loss.is_finite() {
            return diverged_product(
                inputs,
                train_config_hash,
                losses,
                eval_points,
                weight_stats,
                grad_log,
                DivergenceEvent {
                    step,
                    observed: DivergenceObserved::NonFiniteLoss,
                    last_finite_loss,
                },
            );
        }

        let loss = LossNatsPerByte::new(loss)?;
        let mut grad_norm = if options.zero_gradients() {
            0.0
        } else {
            model.apply_fixture_adamw_step(step, loss.get(), &inputs.train_config)
        };
        if options.inject_non_finite_grad_norm_at_step() == Some(step) {
            grad_norm = f32::INFINITY;
        }

        if !grad_norm.is_finite() {
            return diverged_product(
                inputs,
                train_config_hash,
                losses,
                eval_points,
                weight_stats,
                grad_log,
                DivergenceEvent {
                    step,
                    observed: DivergenceObserved::NonFiniteGradNorm,
                    last_finite_loss,
                },
            );
        }

        losses.push((step, loss.get()));
        grad_log.push(GradLogPoint {
            step,
            grad_norm_l2: grad_norm,
            loss_nats_per_byte: loss.get(),
        });
        last_finite_loss = Some(loss);

        if step % effective_budget.eval_every_steps == 0 {
            eval_points.push((
                step,
                evaluate_fixture_bpc(&model, &inputs, effective_budget),
            ));
            weight_stats.push(model.weight_stats(step)?);
        }
    }

    let tensors = model.tensors()?;
    let writer_metadata = CheckpointMetadata::from_build_metadata(build_metadata());
    let final_checkpoint = canonical_checkpoint_bytes(&tensors, &writer_metadata)?;
    let final_checkpoint_sha = sha256(&final_checkpoint);
    let metadata = checkpoint_metadata(
        &inputs,
        enforcement.device_profile_hash(),
        final_checkpoint_sha,
        effective_budget.optimizer_steps,
        last_finite_loss
            .expect("completed integration run records at least one finite loss")
            .get(),
    )?;
    let run_log = run_log(
        inputs.seed,
        train_config_hash,
        losses,
        eval_points,
        grad_norm_summary(&grad_log),
    )?;

    Ok(RunProduct::Completed(Box::new(CompletedRunProduct {
        seed: inputs.seed,
        final_checkpoint,
        final_checkpoint_sha,
        metadata,
        run_log,
        weight_stats,
        grad_log,
        completion: S1Completion::Completed,
    })))
}

fn validate_preconditions(inputs: &RunInputs) -> Result<(), S1RunError> {
    s1_profile_prefix(inputs.model_config)?;
    if inputs.train_config != inputs.budget_profile.train_config() {
        return Err(S1RunError::InvalidTrainConfig {
            field: "train_config",
            budget_profile: inputs.budget_profile,
        });
    }
    if inputs.seed > 4 {
        return Err(S1RunError::InvalidSeed {
            field: "seed",
            observed: inputs.seed,
        });
    }
    if inputs.corpus_train.len() < inputs.train_config.sequence_length {
        return Err(S1RunError::TrainCorpusTooShort {
            field: "corpus_train",
            len: inputs.corpus_train.len(),
            required_at_least: inputs.train_config.sequence_length,
        });
    }
    if inputs.corpus_val.is_empty() {
        return Err(S1RunError::ValCorpusEmpty {
            field: "corpus_val",
        });
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EffectiveTrainBudget {
    optimizer_steps: u64,
    eval_every_steps: u64,
    eval_subset_size: u64,
}

impl EffectiveTrainBudget {
    fn new(
        budget_profile: TrainBudgetProfile,
        train_config: &TrainConfig,
        options: RunTestOptions,
    ) -> Result<Self, S1RunError> {
        let optimizer_steps = options
            .effective_optimizer_steps
            .unwrap_or(train_config.optimizer_steps);
        let eval_every_steps = options
            .effective_eval_every_steps
            .unwrap_or(train_config.eval_every_steps);
        let eval_subset_size = options
            .effective_eval_subset_size
            .unwrap_or(train_config.eval_subset_size);

        if optimizer_steps == 0 || optimizer_steps > train_config.optimizer_steps {
            return Err(S1RunError::InvalidTestRunOptions {
                field: "effective_optimizer_steps",
            });
        }
        if eval_every_steps == 0 || eval_every_steps > train_config.eval_every_steps {
            return Err(S1RunError::InvalidTestRunOptions {
                field: "effective_eval_every_steps",
            });
        }
        if eval_subset_size == 0 || eval_subset_size > train_config.eval_subset_size {
            return Err(S1RunError::InvalidTestRunOptions {
                field: "effective_eval_subset_size",
            });
        }
        if budget_profile == TrainBudgetProfile::Production
            && (optimizer_steps != train_config.optimizer_steps
                || eval_every_steps != train_config.eval_every_steps
                || eval_subset_size != train_config.eval_subset_size)
        {
            #[cfg(not(feature = "falsify"))]
            {
                return Err(S1RunError::InvalidTestRunOptions {
                    field: "production_effective_budget",
                });
            }
        }

        Ok(Self {
            optimizer_steps,
            eval_every_steps,
            eval_subset_size,
        })
    }

    fn eval_point_count(self) -> usize {
        usize::try_from(self.optimizer_steps / self.eval_every_steps + 1)
            .expect("S1 eval point count fits usize")
    }
}

fn production_train_run<B>(
    inputs: RunInputs,
    enforcement: DeviceProfileEnforcement,
    options: RunTestOptions,
    effective_budget: EffectiveTrainBudget,
) -> Result<RunProduct, S1RunError>
where
    B: BurnAutodiffBackend,
{
    let device = BurnDevice::<B>::default();
    let mut model =
        ProductionDenseModel::<B>::initialize(inputs.model_config, inputs.seed, &device)?;
    let mut optimizer = adamw_config()
        .with_beta_1(inputs.train_config.optimizer.beta1)
        .with_beta_2(inputs.train_config.optimizer.beta2)
        .with_epsilon(inputs.train_config.optimizer.eps)
        .with_weight_decay(inputs.train_config.optimizer.weight_decay)
        .init::<B, ProductionDenseModel<B>>();
    let mut sampler = BatchSampler::new(&inputs.corpus_train, &inputs.train_config, inputs.seed)?;
    let train_config_hash = train_config_hash(&inputs.train_config)?;
    let mut losses = Vec::with_capacity(effective_budget.optimizer_steps as usize);
    let mut eval_points = Vec::with_capacity(effective_budget.eval_point_count());
    let mut grad_log = Vec::with_capacity(effective_budget.optimizer_steps as usize);
    let mut weight_stats = Vec::with_capacity(effective_budget.eval_point_count());
    let mut last_finite_loss = None;

    production_progress(
        inputs.seed,
        "start",
        0,
        effective_budget.optimizer_steps,
        None,
        None,
    );
    production_progress(
        inputs.seed,
        "eval_start",
        0,
        effective_budget.optimizer_steps,
        None,
        None,
    );
    eval_points.push((
        0,
        evaluate_production_bpc(&model, &inputs, effective_budget, &device)?,
    ));
    production_progress(
        inputs.seed,
        "eval_complete",
        0,
        effective_budget.optimizer_steps,
        eval_points.last().map(|(_, bpc)| *bpc),
        None,
    );
    weight_stats.push(model.weight_stats(0)?);

    for step in 1..=effective_budget.optimizer_steps {
        let batch = sampler.draw_step(step)?;
        let sequences = batch
            .iter()
            .map(|sequence| sequence.bytes.as_slice())
            .collect::<Vec<_>>();
        let mut loss_tensor = production_loss_nats_per_byte(&model, &sequences, &device)?;
        let mut loss = scalar_tensor_value(loss_tensor.clone())?;
        if options.inject_non_finite_loss_at_step() == Some(step) {
            loss = f32::NAN;
        }

        if !loss.is_finite() {
            return diverged_product(
                inputs,
                train_config_hash,
                losses,
                eval_points,
                weight_stats,
                grad_log,
                DivergenceEvent {
                    step,
                    observed: DivergenceObserved::NonFiniteLoss,
                    last_finite_loss,
                },
            );
        }

        let loss = LossNatsPerByte::new(loss)?;
        if options.zero_gradients() {
            loss_tensor = loss_tensor * 0.0;
        }
        if options.inject_non_finite_grad_norm_at_step() == Some(step) {
            loss_tensor = loss_tensor * f32::INFINITY;
        }
        let gradients = loss_tensor.backward();
        let grad_norm = model.grad_norm_l2(&gradients)?;

        if !grad_norm.is_finite() {
            return diverged_product(
                inputs,
                train_config_hash,
                losses,
                eval_points,
                weight_stats,
                grad_log,
                DivergenceEvent {
                    step,
                    observed: DivergenceObserved::NonFiniteGradNorm,
                    last_finite_loss,
                },
            );
        }

        let gradients = BurnGradientsParams::from_grads(gradients, &model);
        model = optimizer.step(
            f64::from(inputs.train_config.optimizer.lr),
            model,
            gradients,
        );

        losses.push((step, loss.get()));
        grad_log.push(GradLogPoint {
            step,
            grad_norm_l2: grad_norm,
            loss_nats_per_byte: loss.get(),
        });
        last_finite_loss = Some(loss);

        if step % effective_budget.eval_every_steps == 0 {
            production_progress(
                inputs.seed,
                "eval_start",
                step,
                effective_budget.optimizer_steps,
                None,
                Some(loss.get()),
            );
            eval_points.push((
                step,
                evaluate_production_bpc(&model, &inputs, effective_budget, &device)?,
            ));
            production_progress(
                inputs.seed,
                "eval_complete",
                step,
                effective_budget.optimizer_steps,
                eval_points.last().map(|(_, bpc)| *bpc),
                Some(loss.get()),
            );
            weight_stats.push(model.weight_stats(step)?);
        } else if should_log_production_step(step, effective_budget.optimizer_steps) {
            production_progress(
                inputs.seed,
                "step",
                step,
                effective_budget.optimizer_steps,
                None,
                Some(loss.get()),
            );
        }
    }

    production_progress(
        inputs.seed,
        "checkpoint_start",
        effective_budget.optimizer_steps,
        effective_budget.optimizer_steps,
        None,
        last_finite_loss.map(|loss| loss.get()),
    );
    let tensors = model.tensors()?;
    let writer_metadata = CheckpointMetadata::from_build_metadata(build_metadata());
    let final_checkpoint = canonical_checkpoint_bytes(&tensors, &writer_metadata)?;
    let final_checkpoint_sha = sha256(&final_checkpoint);
    let metadata = checkpoint_metadata(
        &inputs,
        enforcement.device_profile_hash(),
        final_checkpoint_sha,
        effective_budget.optimizer_steps,
        last_finite_loss
            .expect("completed production run records at least one finite loss")
            .get(),
    )?;
    let run_log = run_log(
        inputs.seed,
        train_config_hash,
        losses,
        eval_points,
        grad_norm_summary(&grad_log),
    )?;
    production_progress(
        inputs.seed,
        "complete",
        effective_budget.optimizer_steps,
        effective_budget.optimizer_steps,
        None,
        last_finite_loss.map(|loss| loss.get()),
    );

    Ok(RunProduct::Completed(Box::new(CompletedRunProduct {
        seed: inputs.seed,
        final_checkpoint,
        final_checkpoint_sha,
        metadata,
        run_log,
        weight_stats,
        grad_log,
        completion: S1Completion::Completed,
    })))
}

fn should_log_production_step(step: u64, total_steps: u64) -> bool {
    step == 1 || step == total_steps || step.is_multiple_of(100)
}

fn production_progress(
    seed: u64,
    phase: &'static str,
    step: u64,
    total_steps: u64,
    bpc: Option<f64>,
    loss_nats_per_byte: Option<f32>,
) {
    match (bpc, loss_nats_per_byte) {
        (Some(bpc), Some(loss)) => eprintln!(
            "[S1-REPLAY] seed={seed} phase={phase} step={step}/{total_steps} bpc={bpc:.6} loss_nats_per_byte={loss:.6}"
        ),
        (Some(bpc), None) => eprintln!(
            "[S1-REPLAY] seed={seed} phase={phase} step={step}/{total_steps} bpc={bpc:.6}"
        ),
        (None, Some(loss)) => eprintln!(
            "[S1-REPLAY] seed={seed} phase={phase} step={step}/{total_steps} loss_nats_per_byte={loss:.6}"
        ),
        (None, None) => {
            eprintln!("[S1-REPLAY] seed={seed} phase={phase} step={step}/{total_steps}");
        }
    }
}

fn diverged_product(
    inputs: RunInputs,
    train_config_hash: Hash256,
    losses: Vec<(u64, f32)>,
    eval_points: Vec<(u64, f64)>,
    weight_stats: Vec<WeightStatsPoint>,
    grad_log: Vec<GradLogPoint>,
    divergence_event: DivergenceEvent,
) -> Result<RunProduct, S1RunError> {
    let _ = S1LogEmitter::new().run_divergence(RunDivergenceEvent {
        seed: inputs.seed,
        step: divergence_event.step,
        observed: log_divergence_observed(divergence_event.observed),
        last_finite_loss: divergence_event
            .last_finite_loss
            .map(|loss| f64::from(loss.get()))
            .unwrap_or(0.0),
    });
    let run_log = run_log(
        inputs.seed,
        train_config_hash,
        losses,
        eval_points,
        grad_norm_summary(&grad_log),
    )?;
    Ok(RunProduct::Diverged(Box::new(DivergedRunProduct {
        seed: inputs.seed,
        run_log,
        weight_stats,
        grad_log,
        completion: S1Completion::DivergedAt {
            step: divergence_event.step,
        },
        divergence_event,
    })))
}

fn log_divergence_observed(observed: DivergenceObserved) -> LogDivergenceObserved {
    match observed {
        DivergenceObserved::NonFiniteLoss => LogDivergenceObserved::NonFiniteLoss,
        DivergenceObserved::NonFiniteGradNorm => LogDivergenceObserved::NonFiniteGrad,
    }
}

fn run_log(
    seed: u64,
    train_config_hash: Hash256,
    losses: Vec<(u64, f32)>,
    eval_points: Vec<(u64, f64)>,
    final_grad_norms: GradNormSummary,
) -> Result<RunLog, S1RunError> {
    let log = RunLog {
        schema: "s1_run_log.v1".to_owned(),
        seed,
        train_config_hash,
        losses,
        eval_points,
        final_grad_norms,
        run_log_self_hash: Hash256::ZERO,
    };
    Ok(log.with_computed_self_hash()?)
}

fn grad_norm_summary(grad_log: &[GradLogPoint]) -> GradNormSummary {
    let Some(last) = grad_log.last() else {
        return GradNormSummary {
            global_l2: 0.0,
            max_l2: 0.0,
            mean_l2: 0.0,
        };
    };

    let max_l2 = grad_log
        .iter()
        .map(|point| point.grad_norm_l2)
        .fold(0.0_f32, f32::max);
    let mean_l2 =
        grad_log.iter().map(|point| point.grad_norm_l2).sum::<f32>() / grad_log.len() as f32;

    GradNormSummary {
        global_l2: last.grad_norm_l2,
        max_l2,
        mean_l2,
    }
}

fn checkpoint_metadata(
    inputs: &RunInputs,
    device_profile_hash: Hash256,
    checkpoint_sha: Hash256,
    final_step: u64,
    final_train_loss: f32,
) -> Result<S1CheckpointMetadata, S1RunError> {
    let metadata = S1CheckpointMetadata {
        schema: "s1_checkpoint.v1".to_owned(),
        seed: inputs.seed,
        corpus_train_sha: sha256(&inputs.corpus_train),
        corpus_val_sha: sha256(&inputs.corpus_val),
        model_config_hash: model_config_hash(&inputs.model_config)?,
        train_config_hash: train_config_hash(&inputs.train_config)?,
        build_kind: active_build_kind(),
        build_config_hash: build_config_hash()?,
        dependency_lockfile_sha: sha256(include_bytes!("../../../Cargo.lock")),
        rust_toolchain_hash: rust_toolchain_hash(),
        device_profile_hash,
        rng_stream_def_hash: rng_stream_def_hash(),
        pass_version: SemVer::new(0, 1, 0),
        budget_profile: inputs.budget_profile.as_metadata_str().to_owned(),
        final_step,
        final_train_loss,
        completion: S1Completion::Completed,
        checkpoint_safetensors_sha256: checkpoint_sha,
        checkpoint_self_hash: Hash256::ZERO,
    };
    Ok(metadata.with_computed_self_hash()?)
}

fn active_build_kind() -> S1BuildKind {
    match BUILD_KIND {
        "phase_a" => S1BuildKind::PhaseA,
        "ablation" => S1BuildKind::Ablation,
        _ => S1BuildKind::PhaseA,
    }
}

fn model_config_hash(model_config: &ModelSizeProfile) -> Result<Hash256, S1SchemaError> {
    DomainHash::new(
        "gbf-policy",
        "ModelSizeProfile",
        "model_size_profile.v1",
        "1",
    )
    .hash(model_config)
}

fn train_config_hash(train_config: &TrainConfig) -> Result<Hash256, S1SchemaError> {
    DomainHash::new("gbf-experiments", "TrainConfig", "s1_train_config.v1", "1").hash(train_config)
}

fn build_config_hash() -> Result<Hash256, S1SchemaError> {
    DomainHash::new(
        "gbf-experiments",
        "BuildMetadata",
        "s1_build_metadata.v1",
        "1",
    )
    .hash(&build_metadata())
}

fn rust_toolchain_hash() -> Hash256 {
    sha256(format!(
        "rustc:{version};gbf_experiments:{exp};gbf_train:{train}",
        version = env!("CARGO_PKG_RUST_VERSION"),
        exp = build_metadata().gbf_experiments_sha,
        train = build_metadata().gbf_train_sha
    ))
}

fn evaluate_fixture_bpc(
    model: &IntegrationFixtureModel,
    inputs: &RunInputs,
    effective_budget: EffectiveTrainBudget,
) -> f64 {
    let max_len = inputs
        .train_config
        .sequence_length
        .saturating_mul(effective_budget.eval_subset_size as usize);
    let token_count = inputs.corpus_val.len().min(max_len).max(1);
    let byte_mean = inputs
        .corpus_val
        .iter()
        .take(token_count)
        .map(|byte| f64::from(*byte))
        .sum::<f64>()
        / token_count as f64;
    let weight_mean = model
        .weights
        .iter()
        .map(|value| f64::from(*value))
        .sum::<f64>()
        / model.weights.len() as f64;
    8.0 - (byte_mean / 255.0) * 0.25 + weight_mean.abs().min(1.0) * 0.01
}

fn fixture_step_loss(model: &IntegrationFixtureModel, batch: &[Sequence], step: u64) -> f32 {
    let byte_mean = batch
        .iter()
        .flat_map(|sequence| sequence.bytes.iter())
        .map(|byte| f32::from(*byte))
        .sum::<f32>()
        / (batch.len() * batch[0].bytes.len()) as f32;
    let weight_mean = model.weights.iter().copied().sum::<f32>() / model.weights.len() as f32;
    let decay = 1.0 / (1.0 + step as f32 * 0.01);
    (2.0 + byte_mean / 255.0 + weight_mean.abs() * 0.1) * decay
}

#[derive(BurnModule, Debug)]
struct ProductionDenseBlock<B: BurnBackend> {
    input_to_state: BurnParam<BurnFloatTensor<B, 2>>,
    state_to_output: BurnParam<BurnFloatTensor<B, 2>>,
    dense_up: BurnParam<BurnFloatTensor<B, 2>>,
    dense_down: BurnParam<BurnFloatTensor<B, 2>>,
}

impl<B: BurnAutodiffBackend> ProductionDenseBlock<B> {
    fn initialize(
        rng: &mut InitRng,
        d_model: usize,
        state_slots: usize,
        d_ff: usize,
        device: &BurnDevice<B>,
    ) -> Result<Self, S1RunError> {
        Ok(Self {
            input_to_state: BurnParam::from_tensor(init_weight_matrix(
                rng,
                d_model,
                state_slots,
                device,
            )?),
            state_to_output: BurnParam::from_tensor(init_weight_matrix(
                rng,
                state_slots,
                d_model,
                device,
            )?),
            dense_up: BurnParam::from_tensor(init_weight_matrix(rng, d_model, d_ff, device)?),
            dense_down: BurnParam::from_tensor(init_weight_matrix(rng, d_ff, d_model, device)?),
        })
    }

    fn input_to_state(&self) -> BurnFloatTensor<B, 2> {
        self.input_to_state.val()
    }

    fn state_to_output(&self) -> BurnFloatTensor<B, 2> {
        self.state_to_output.val()
    }

    fn dense_up(&self) -> BurnFloatTensor<B, 2> {
        self.dense_up.val()
    }

    fn dense_down(&self) -> BurnFloatTensor<B, 2> {
        self.dense_down.val()
    }

    fn push_tensors(
        &self,
        profile: ModelSizeProfile,
        block_index: usize,
        d_model: usize,
        state_slots: usize,
        d_ff: usize,
        tensors: &mut Vec<CanonicalTensor>,
    ) -> Result<(), S1RunError> {
        tensors.push(canonical_f32_tensor(
            &s1_production_block_tensor_id(
                profile,
                block_index,
                "linear_state.input_to_state.weight",
            )?,
            &[d_model, state_slots],
            float_tensor_into_vec(self.input_to_state().detach())?,
        )?);
        tensors.push(canonical_f32_tensor(
            &s1_production_block_tensor_id(
                profile,
                block_index,
                "linear_state.state_to_output.weight",
            )?,
            &[state_slots, d_model],
            float_tensor_into_vec(self.state_to_output().detach())?,
        )?);
        tensors.push(canonical_f32_tensor(
            &s1_production_block_tensor_id(profile, block_index, "dense_ffn.up.weight")?,
            &[d_model, d_ff],
            float_tensor_into_vec(self.dense_up().detach())?,
        )?);
        tensors.push(canonical_f32_tensor(
            &s1_production_block_tensor_id(profile, block_index, "dense_ffn.down.weight")?,
            &[d_ff, d_model],
            float_tensor_into_vec(self.dense_down().detach())?,
        )?);
        Ok(())
    }

    fn accumulate_grad_norm(
        &self,
        block_name: &'static str,
        gradients: &B::Gradients,
        sum_sq: &mut f64,
    ) -> Result<(), S1RunError> {
        accumulate_grad_norm(
            block_parameter_name(block_name, "input_to_state"),
            self.input_to_state().grad(gradients),
            sum_sq,
        )?;
        accumulate_grad_norm(
            block_parameter_name(block_name, "state_to_output"),
            self.state_to_output().grad(gradients),
            sum_sq,
        )?;
        accumulate_grad_norm(
            block_parameter_name(block_name, "dense_up"),
            self.dense_up().grad(gradients),
            sum_sq,
        )?;
        accumulate_grad_norm(
            block_parameter_name(block_name, "dense_down"),
            self.dense_down().grad(gradients),
            sum_sq,
        )
    }
}

fn block_parameter_name(block_name: &'static str, parameter: &'static str) -> &'static str {
    match (block_name, parameter) {
        ("block0", "input_to_state") => "block0.input_to_state",
        ("block0", "state_to_output") => "block0.state_to_output",
        ("block0", "dense_up") => "block0.dense_up",
        ("block0", "dense_down") => "block0.dense_down",
        ("block1", "input_to_state") => "block1.input_to_state",
        ("block1", "state_to_output") => "block1.state_to_output",
        ("block1", "dense_up") => "block1.dense_up",
        ("block1", "dense_down") => "block1.dense_down",
        _ => parameter,
    }
}

#[derive(BurnModule, Debug)]
struct ProductionDenseModel<B: BurnBackend> {
    #[module(skip)]
    profile: ModelSizeProfile,
    #[module(skip)]
    vocab_size: usize,
    #[module(skip)]
    d_model: usize,
    #[module(skip)]
    state_slots: usize,
    #[module(skip)]
    state_decay: f32,
    #[module(skip)]
    d_ff: usize,
    token_embedding: BurnParam<BurnFloatTensor<B, 2>>,
    block0: ProductionDenseBlock<B>,
    block1: Option<ProductionDenseBlock<B>>,
}

impl<B: BurnAutodiffBackend> ProductionDenseModel<B> {
    fn initialize(
        profile: ModelSizeProfile,
        seed: u64,
        device: &BurnDevice<B>,
    ) -> Result<Self, S1RunError> {
        let d_model = usize::from(profile.d_model());
        let state_slots = s1_dense_state_slots(profile)?;
        let block_count = s1_dense_block_count(profile)?;
        let d_ff = usize::from(profile.d_ff());
        let mut rng = InitRng::new(seed);
        let token_embedding = BurnParam::from_tensor(init_weight_matrix(
            &mut rng,
            S1_BYTE_VOCAB_SIZE,
            d_model,
            device,
        )?);
        let block0 =
            ProductionDenseBlock::initialize(&mut rng, d_model, state_slots, d_ff, device)?;
        let block1 = if block_count == 2 {
            Some(ProductionDenseBlock::initialize(
                &mut rng,
                d_model,
                state_slots,
                d_ff,
                device,
            )?)
        } else {
            None
        };

        Ok(Self {
            profile,
            vocab_size: S1_BYTE_VOCAB_SIZE,
            d_model,
            state_slots,
            state_decay: S1_DENSE_STATE_DECAY,
            d_ff,
            token_embedding,
            block0,
            block1,
        })
    }

    fn token_embedding(&self) -> BurnFloatTensor<B, 2> {
        self.token_embedding.val()
    }

    fn tensors(&self) -> Result<Vec<CanonicalTensor>, S1RunError> {
        let mut tensors = vec![canonical_f32_tensor(
            &s1_production_tensor_id(self.profile, "embedding_tied.weight")?,
            &[self.vocab_size, self.d_model],
            float_tensor_into_vec(self.token_embedding().detach())?,
        )?];
        self.block0.push_tensors(
            self.profile,
            0,
            self.d_model,
            self.state_slots,
            self.d_ff,
            &mut tensors,
        )?;
        if let Some(block1) = &self.block1 {
            block1.push_tensors(
                self.profile,
                1,
                self.d_model,
                self.state_slots,
                self.d_ff,
                &mut tensors,
            )?;
        }
        Ok(tensors)
    }

    fn weight_stats(&self, step: u64) -> Result<WeightStatsPoint, S1RunError> {
        let tensors = self.tensors()?;
        let mut min_weight = f32::INFINITY;
        let mut max_weight = f32::NEG_INFINITY;
        for tensor in &tensors {
            let CanonicalTensorPayload::F32(values) = &tensor.payload else {
                continue;
            };
            for value in values {
                min_weight = min_weight.min(*value);
                max_weight = max_weight.max(*value);
            }
        }
        Ok(WeightStatsPoint {
            step,
            tensor_payload_hash: canonical_tensor_payload_hash(&tensors),
            min_weight,
            max_weight,
        })
    }

    fn grad_norm_l2(&self, gradients: &B::Gradients) -> Result<f32, S1RunError> {
        let mut sum_sq = 0.0_f64;
        accumulate_grad_norm(
            "token_embedding",
            self.token_embedding().grad(gradients),
            &mut sum_sq,
        )?;
        self.block0
            .accumulate_grad_norm("block0", gradients, &mut sum_sq)?;
        if let Some(block1) = &self.block1 {
            block1.accumulate_grad_norm("block1", gradients, &mut sum_sq)?;
        }
        let norm = sum_sq.sqrt();
        if norm.is_finite() && norm <= f64::from(f32::MAX) {
            Ok(norm as f32)
        } else {
            Ok(f32::INFINITY)
        }
    }
}

fn init_weight_matrix<B: BurnBackend>(
    rng: &mut InitRng,
    rows: usize,
    cols: usize,
    device: &BurnDevice<B>,
) -> Result<BurnFloatTensor<B, 2>, S1RunError> {
    let scale = (2.0 / (rows + cols) as f64).sqrt();
    let mut weights = Vec::with_capacity(rows * cols);
    for _ in 0..rows * cols {
        let draw = rng.next_u64();
        let centered = (draw as f64 / u64::MAX as f64) * 2.0 - 1.0;
        weights.push((centered * scale) as f32);
    }
    Ok(float_tensor_from_vec(weights, [rows, cols], device)?)
}

fn canonical_f32_tensor(
    name: &str,
    shape: &[usize],
    values: Vec<f32>,
) -> Result<CanonicalTensor, S1RunError> {
    Ok(CanonicalTensor::new(
        ArtifactPath::new(name)?,
        CanonicalTensorKind::DenseWeight,
        CanonicalTensorLayout::new(
            CanonicalTensorShape::from_usize_dims(shape)?,
            TensorElementType::Float32,
        ),
        CanonicalTensorPayload::F32(values),
    )?)
}

fn production_loss_nats_per_byte<B: BurnAutodiffBackend>(
    model: &ProductionDenseModel<B>,
    sequences: &[&[u8]],
    device: &BurnDevice<B>,
) -> Result<BurnFloatTensor<B, 1>, S1RunError> {
    let batch_size = sequences.len();
    let sequence_length = sequences.first().map_or(0, |sequence| sequence.len());
    let mut state0 = float_tensor_from_vec(
        vec![0.0; batch_size * model.state_slots],
        [batch_size, model.state_slots],
        device,
    )?;
    let mut state1 = if model.block1.is_some() {
        Some(float_tensor_from_vec(
            vec![0.0; batch_size * model.state_slots],
            [batch_size, model.state_slots],
            device,
        )?)
    } else {
        None
    };
    let mut total_loss = None;

    for position in 0..sequence_length {
        let target = one_hot_targets(sequences, position, device)?;
        let hidden0 = production_block_output(&model.block0, state0.clone());
        let hidden = if let (Some(block1), Some(block1_state)) = (&model.block1, &state1) {
            hidden0 + production_block_output(block1, block1_state.clone())
        } else {
            hidden0
        };
        let logits = burn_linear(
            hidden,
            model.token_embedding().transpose(),
            None::<BurnFloatTensor<B, 1>>,
        );
        let log_probs = burn_log_softmax(logits, 1);
        let position_loss = (log_probs * target.clone()).sum() * -1.0;
        total_loss = Some(match total_loss {
            Some(loss) => loss + position_loss,
            None => position_loss,
        });

        let token_embedding = burn_linear(
            target,
            model.token_embedding(),
            None::<BurnFloatTensor<B, 1>>,
        );
        let delta = burn_linear(
            token_embedding.clone(),
            model.block0.input_to_state(),
            None::<BurnFloatTensor<B, 1>>,
        );
        state0 = state0 * model.state_decay + delta;
        if let (Some(block1), Some(block1_state)) = (&model.block1, &mut state1) {
            let delta = burn_linear(
                token_embedding,
                block1.input_to_state(),
                None::<BurnFloatTensor<B, 1>>,
            );
            *block1_state = block1_state.clone() * model.state_decay + delta;
        }
    }

    let denominator = (batch_size * sequence_length) as f32;
    Ok(total_loss.expect("S1 preconditions ensure non-empty training sequences") / denominator)
}

fn production_block_output<B: BurnAutodiffBackend>(
    block: &ProductionDenseBlock<B>,
    state: BurnFloatTensor<B, 2>,
) -> BurnFloatTensor<B, 2> {
    let hidden = burn_linear(
        state,
        block.state_to_output(),
        None::<BurnFloatTensor<B, 1>>,
    );
    let ffn = burn_linear(
        burn_relu(burn_linear(
            hidden.clone(),
            block.dense_up(),
            None::<BurnFloatTensor<B, 1>>,
        )),
        block.dense_down(),
        None::<BurnFloatTensor<B, 1>>,
    );
    hidden + ffn
}

fn one_hot_targets<B: BurnAutodiffBackend>(
    sequences: &[&[u8]],
    position: usize,
    device: &BurnDevice<B>,
) -> Result<BurnFloatTensor<B, 2>, S1RunError> {
    let mut values = vec![0.0; sequences.len() * S1_BYTE_VOCAB_SIZE];
    for (batch_index, sequence) in sequences.iter().enumerate() {
        let token = usize::from(sequence[position]);
        values[batch_index * S1_BYTE_VOCAB_SIZE + token] = 1.0;
    }
    Ok(float_tensor_from_vec(
        values,
        [sequences.len(), S1_BYTE_VOCAB_SIZE],
        device,
    )?)
}

fn scalar_tensor_value<B: BurnBackend>(tensor: BurnFloatTensor<B, 1>) -> Result<f32, S1RunError> {
    let values = float_tensor_into_vec(tensor.detach())?;
    Ok(values[0])
}

#[cfg(test)]
pub(crate) fn production_loss_nats_per_byte_for_tensors(
    tensors: &[CanonicalTensor],
    sequence: &[u8],
) -> Result<f32, S1RunError> {
    production_loss_nats_per_byte_for_profile_tensors(ModelSizeProfile::toy0(), tensors, sequence)
}

#[cfg(test)]
pub(crate) fn production_loss_nats_per_byte_for_profile_tensors(
    profile: ModelSizeProfile,
    tensors: &[CanonicalTensor],
    sequence: &[u8],
) -> Result<f32, S1RunError> {
    let device = BurnDevice::<BurnNdArrayAutodiffBackend>::default();
    let d_model = usize::from(profile.d_model());
    let state_slots = s1_dense_state_slots(profile)?;
    let d_ff = usize::from(profile.d_ff());
    let block0 = production_test_block(profile, 0, tensors, d_model, state_slots, d_ff, &device)?;
    let block1 = if s1_dense_block_count(profile)? == 2 {
        Some(production_test_block(
            profile,
            1,
            tensors,
            d_model,
            state_slots,
            d_ff,
            &device,
        )?)
    } else {
        None
    };
    let model = ProductionDenseModel::<BurnNdArrayAutodiffBackend> {
        profile,
        vocab_size: S1_BYTE_VOCAB_SIZE,
        d_model,
        state_slots,
        state_decay: S1_DENSE_STATE_DECAY,
        d_ff,
        token_embedding: BurnParam::from_tensor(float_tensor_from_vec(
            production_test_tensor(
                tensors,
                &s1_production_tensor_id(profile, "embedding_tied.weight")?,
                &[S1_BYTE_VOCAB_SIZE, d_model],
            )?,
            [S1_BYTE_VOCAB_SIZE, d_model],
            &device,
        )?),
        block0,
        block1,
    };
    let loss = production_loss_nats_per_byte(&model, &[sequence], &device)?;
    scalar_tensor_value(loss)
}

#[cfg(test)]
fn production_test_block(
    profile: ModelSizeProfile,
    block_index: usize,
    tensors: &[CanonicalTensor],
    d_model: usize,
    state_slots: usize,
    d_ff: usize,
    device: &BurnDevice<BurnNdArrayAutodiffBackend>,
) -> Result<ProductionDenseBlock<BurnNdArrayAutodiffBackend>, S1RunError> {
    Ok(ProductionDenseBlock {
        input_to_state: BurnParam::from_tensor(float_tensor_from_vec(
            production_test_tensor(
                tensors,
                &s1_production_block_tensor_id(
                    profile,
                    block_index,
                    "linear_state.input_to_state.weight",
                )?,
                &[d_model, state_slots],
            )?,
            [d_model, state_slots],
            device,
        )?),
        state_to_output: BurnParam::from_tensor(float_tensor_from_vec(
            production_test_tensor(
                tensors,
                &s1_production_block_tensor_id(
                    profile,
                    block_index,
                    "linear_state.state_to_output.weight",
                )?,
                &[state_slots, d_model],
            )?,
            [state_slots, d_model],
            device,
        )?),
        dense_up: BurnParam::from_tensor(float_tensor_from_vec(
            production_test_tensor(
                tensors,
                &s1_production_block_tensor_id(profile, block_index, "dense_ffn.up.weight")?,
                &[d_model, d_ff],
            )?,
            [d_model, d_ff],
            device,
        )?),
        dense_down: BurnParam::from_tensor(float_tensor_from_vec(
            production_test_tensor(
                tensors,
                &s1_production_block_tensor_id(profile, block_index, "dense_ffn.down.weight")?,
                &[d_ff, d_model],
            )?,
            [d_ff, d_model],
            device,
        )?),
    })
}

#[cfg(test)]
fn production_test_tensor(
    tensors: &[CanonicalTensor],
    name: &str,
    expected_shape: &[usize],
) -> Result<Vec<f32>, S1RunError> {
    let tensor = tensors
        .iter()
        .find(|tensor| tensor.id.as_str() == name)
        .ok_or_else(|| {
            S1RunError::Schema(S1SchemaError::Custom(format!(
                "production parity tensor {name} missing"
            )))
        })?;
    let expected_shape = expected_shape
        .iter()
        .copied()
        .map(|dim| {
            u32::try_from(dim).map_err(|_| {
                S1RunError::Schema(S1SchemaError::Custom(format!(
                    "production parity tensor {name} expected shape is too large"
                )))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if tensor.layout.shape.dims() != expected_shape.as_slice() {
        return Err(S1RunError::Schema(S1SchemaError::Custom(format!(
            "production parity tensor {name} shape mismatch: expected {:?}, observed {:?}",
            expected_shape,
            tensor.layout.shape.dims()
        ))));
    }
    let CanonicalTensorPayload::F32(values) = &tensor.payload else {
        return Err(S1RunError::Schema(S1SchemaError::Custom(format!(
            "production parity tensor {name} must be Float32"
        ))));
    };
    Ok(values.clone())
}

fn accumulate_grad_norm<B: BurnBackend>(
    parameter: &'static str,
    gradient: Option<BurnFloatTensor<B, 2>>,
    sum_sq: &mut f64,
) -> Result<(), S1RunError> {
    let gradient = gradient.ok_or(S1RunError::MissingGradient { parameter })?;
    for value in float_tensor_into_vec(gradient)? {
        if !value.is_finite() {
            *sum_sq = f64::INFINITY;
            return Ok(());
        }
        *sum_sq += f64::from(value) * f64::from(value);
    }
    Ok(())
}

fn evaluate_production_bpc<B: BurnAutodiffBackend>(
    model: &ProductionDenseModel<B>,
    inputs: &RunInputs,
    effective_budget: EffectiveTrainBudget,
    _device: &BurnDevice<B>,
) -> Result<f64, S1RunError> {
    let max_len = inputs
        .train_config
        .sequence_length
        .saturating_mul(effective_budget.eval_subset_size as usize);
    let eval_bytes = &inputs.corpus_val[..inputs.corpus_val.len().min(max_len)];
    let tensors = model.tensors()?;
    let scorer = ProductionEvalScorer::from_tensors(inputs.model_config, &tensors)?;
    scorer.bpc(eval_bytes, inputs.train_config.sequence_length)
}

#[derive(Debug, Clone)]
struct ProductionEvalBlock {
    input_to_state: Vec<f64>,
    state_to_output: Vec<f64>,
    dense_up: Vec<f64>,
    dense_down: Vec<f64>,
}

impl ProductionEvalBlock {
    fn from_tensors(
        profile: ModelSizeProfile,
        block_index: usize,
        tensors: &[CanonicalTensor],
        d_model: usize,
        state_slots: usize,
        d_ff: usize,
    ) -> Result<Self, S1RunError> {
        Ok(Self {
            input_to_state: production_eval_tensor(
                tensors,
                &s1_production_block_tensor_id(
                    profile,
                    block_index,
                    "linear_state.input_to_state.weight",
                )?,
                &[d_model, state_slots],
            )?,
            state_to_output: production_eval_tensor(
                tensors,
                &s1_production_block_tensor_id(
                    profile,
                    block_index,
                    "linear_state.state_to_output.weight",
                )?,
                &[state_slots, d_model],
            )?,
            dense_up: production_eval_tensor(
                tensors,
                &s1_production_block_tensor_id(profile, block_index, "dense_ffn.up.weight")?,
                &[d_model, d_ff],
            )?,
            dense_down: production_eval_tensor(
                tensors,
                &s1_production_block_tensor_id(profile, block_index, "dense_ffn.down.weight")?,
                &[d_ff, d_model],
            )?,
        })
    }
}

#[derive(Debug, Clone)]
struct ProductionEvalScorer {
    d_model: usize,
    state_slots: usize,
    state_decay: f64,
    d_ff: usize,
    embedding: Vec<f64>,
    blocks: Vec<ProductionEvalBlock>,
}

impl ProductionEvalScorer {
    fn from_tensors(
        profile: ModelSizeProfile,
        tensors: &[CanonicalTensor],
    ) -> Result<Self, S1RunError> {
        let d_model = usize::from(profile.d_model());
        let state_slots = s1_dense_state_slots(profile)?;
        let block_count = s1_dense_block_count(profile)?;
        let d_ff = usize::from(profile.d_ff());
        let blocks = (0..block_count)
            .map(|block_index| {
                ProductionEvalBlock::from_tensors(
                    profile,
                    block_index,
                    tensors,
                    d_model,
                    state_slots,
                    d_ff,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            d_model,
            state_slots,
            state_decay: f64::from(S1_DENSE_STATE_DECAY),
            d_ff,
            embedding: production_eval_tensor(
                tensors,
                &s1_production_tensor_id(profile, "embedding_tied.weight")?,
                &[S1_BYTE_VOCAB_SIZE, d_model],
            )?,
            blocks,
        })
    }

    fn bpc(&self, bytes: &[u8], reset_context_len: usize) -> Result<f64, S1RunError> {
        let mut log2_sum = 0.0_f64;
        let mut token_count = 0usize;

        for chunk in bytes.chunks(reset_context_len) {
            let mut state = vec![0.0; self.blocks.len() * self.state_slots];
            for &byte in chunk {
                let logits = self.logits(&state);
                log2_sum += negative_log2_probability(&logits, byte)?;
                self.consume(&mut state, byte);
                token_count += 1;
            }
        }

        if token_count == 0 || !log2_sum.is_finite() {
            return Err(S1RunError::Scalar(NonFiniteRunScalarError::NonFiniteLoss));
        }

        Ok(log2_sum / token_count as f64)
    }

    fn logits(&self, state: &[f64]) -> Vec<f64> {
        let mut hidden = vec![0.0; self.d_model];
        for (block_index, block) in self.blocks.iter().enumerate() {
            let state_offset = block_index * self.state_slots;
            let block_hidden =
                self.block_hidden(block, &state[state_offset..state_offset + self.state_slots]);
            for (value, block_value) in hidden.iter_mut().zip(block_hidden) {
                *value += block_value;
            }
        }

        let mut logits = vec![0.0; S1_BYTE_VOCAB_SIZE];
        for (token, logit) in logits.iter_mut().enumerate() {
            for (dim, hidden_value) in hidden.iter().enumerate() {
                *logit += *hidden_value * row_major(&self.embedding, token, dim, self.d_model);
            }
        }
        logits
    }

    fn consume(&self, state: &mut [f64], byte: u8) {
        let token = usize::from(byte);
        for (block_index, block) in self.blocks.iter().enumerate() {
            let mut delta = vec![0.0_f64; self.state_slots];
            for dim in 0..self.d_model {
                let embedding_value = row_major(&self.embedding, token, dim, self.d_model);
                for (slot, value) in delta.iter_mut().enumerate() {
                    *value += embedding_value
                        * row_major(&block.input_to_state, dim, slot, self.state_slots);
                }
            }
            let state_offset = block_index * self.state_slots;
            for (slot, delta_value) in delta.iter().enumerate().take(self.state_slots) {
                let state_index = state_offset + slot;
                state[state_index] = state[state_index] * self.state_decay + *delta_value;
            }
        }
    }

    fn block_hidden(&self, block: &ProductionEvalBlock, state: &[f64]) -> Vec<f64> {
        let mut hidden = vec![0.0; self.d_model];
        for (slot, state_value) in state.iter().enumerate().take(self.state_slots) {
            for (dim, value) in hidden.iter_mut().enumerate() {
                *value += *state_value * row_major(&block.state_to_output, slot, dim, self.d_model);
            }
        }

        let mut up = vec![0.0; self.d_ff];
        for (ff, value) in up.iter_mut().enumerate() {
            for (dim, hidden_value) in hidden.iter().enumerate() {
                *value += *hidden_value * row_major(&block.dense_up, dim, ff, self.d_ff);
            }
            *value = (*value).max(0.0);
        }

        let mut ffn = vec![0.0; self.d_model];
        for (dim, value) in ffn.iter_mut().enumerate() {
            for (ff, up_value) in up.iter().enumerate() {
                *value += *up_value * row_major(&block.dense_down, ff, dim, self.d_model);
            }
        }

        for (dim, value) in hidden.iter_mut().enumerate() {
            *value += ffn[dim];
        }
        hidden
    }
}

fn production_eval_tensor(
    tensors: &[CanonicalTensor],
    name: &str,
    expected_shape: &[usize],
) -> Result<Vec<f64>, S1RunError> {
    let tensor = tensors
        .iter()
        .find(|tensor| tensor.id.as_str() == name)
        .ok_or_else(|| {
            S1RunError::Schema(S1SchemaError::Custom(format!(
                "production eval tensor {name} missing"
            )))
        })?;
    let expected_shape = expected_shape
        .iter()
        .copied()
        .map(|dim| {
            u32::try_from(dim).map_err(|_| {
                S1RunError::Schema(S1SchemaError::Custom(format!(
                    "production eval tensor {name} expected shape is too large"
                )))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if tensor.layout.shape.dims() != expected_shape.as_slice() {
        return Err(S1RunError::Schema(S1SchemaError::Custom(format!(
            "production eval tensor {name} shape mismatch: expected {:?}, observed {:?}",
            expected_shape,
            tensor.layout.shape.dims()
        ))));
    }
    let CanonicalTensorPayload::F32(values) = &tensor.payload else {
        return Err(S1RunError::Schema(S1SchemaError::Custom(format!(
            "production eval tensor {name} must be Float32"
        ))));
    };
    Ok(values.iter().copied().map(f64::from).collect())
}

fn negative_log2_probability(logits: &[f64], target: u8) -> Result<f64, S1RunError> {
    if logits.len() != S1_BYTE_VOCAB_SIZE || logits.iter().any(|value| !value.is_finite()) {
        return Err(S1RunError::Scalar(NonFiniteRunScalarError::NonFiniteLoss));
    }
    let max_logit = logits
        .iter()
        .copied()
        .fold(f64::NEG_INFINITY, |left, right| left.max(right));
    let exp_sum = logits
        .iter()
        .copied()
        .map(|logit| (logit - max_logit).exp())
        .sum::<f64>();
    let loss = (max_logit + exp_sum.ln() - logits[usize::from(target)]) * std::f64::consts::LOG2_E;
    if loss.is_finite() && loss >= 0.0 {
        Ok(loss)
    } else {
        Err(S1RunError::Scalar(NonFiniteRunScalarError::NonFiniteLoss))
    }
}

const fn row_major(values: &[f64], row: usize, col: usize, cols: usize) -> f64 {
    values[row * cols + col]
}

#[derive(Debug, Clone, PartialEq)]
struct IntegrationFixtureModel {
    weights: Vec<f32>,
}

impl IntegrationFixtureModel {
    fn initialize(seed: u64) -> Result<Self, S1RunError> {
        let mut rng = InitRng::new(seed);
        let mut weights = Vec::with_capacity(16);
        for _ in 0..16 {
            let draw = rng.next_u64();
            let centered = (draw as f64 / u64::MAX as f64) * 2.0 - 1.0;
            weights.push((centered * 0.01) as f32);
        }
        Ok(Self { weights })
    }

    fn apply_fixture_adamw_step(
        &mut self,
        step: u64,
        loss: f32,
        train_config: &TrainConfig,
    ) -> f32 {
        let mut grad_l2 = 0.0_f32;
        for (index, weight) in self.weights.iter_mut().enumerate() {
            let grad = (loss * 0.001)
                + (*weight * train_config.optimizer.weight_decay)
                + ((step as usize + index) % 7) as f32 * 0.00001;
            *weight -= train_config.optimizer.lr * grad;
            grad_l2 += grad * grad;
        }
        grad_l2.sqrt()
    }

    fn tensors(&self) -> Result<Vec<CanonicalTensor>, S1RunError> {
        Ok(vec![CanonicalTensor::new(
            ArtifactPath::new("toy0.fixture.weight")?,
            CanonicalTensorKind::DenseWeight,
            CanonicalTensorLayout::new(
                CanonicalTensorShape::from_usize_dims(&[4, 4])?,
                TensorElementType::Float32,
            ),
            CanonicalTensorPayload::F32(self.weights.clone()),
        )?])
    }

    fn weight_stats(&self, step: u64) -> Result<WeightStatsPoint, S1RunError> {
        let tensors = self.tensors()?;
        let (min_weight, max_weight) = self
            .weights
            .iter()
            .copied()
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(min, max), value| {
                (min.min(value), max.max(value))
            });
        Ok(WeightStatsPoint {
            step,
            tensor_payload_hash: canonical_tensor_payload_hash(&tensors),
            min_weight,
            max_weight,
        })
    }
}

/// Metadata associated with a canonical checkpoint write.
///
/// The SafeTensors container written by [`canonical_checkpoint_write`] is
/// intentionally metadata-free. This type carries runtime metadata for the S1
/// run record and sidecars while keeping the byte payload contract limited to
/// canonical tensor names, layouts, and raw payload bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct CheckpointMetadata {
    /// Feature-selected S1 checkpoint build identity.
    pub build_kind: &'static str,
}

impl CheckpointMetadata {
    /// Return checkpoint metadata for the active S1 Cargo build.
    #[must_use]
    pub const fn current() -> Self {
        Self {
            build_kind: BUILD_KIND,
        }
    }

    /// Return checkpoint metadata from the full S1 build metadata record.
    #[must_use]
    pub const fn from_build_metadata(metadata: BuildMetadata) -> Self {
        Self {
            build_kind: metadata.build_kind,
        }
    }
}

impl Default for CheckpointMetadata {
    fn default() -> Self {
        Self::from_build_metadata(build_metadata())
    }
}

/// Write `tensors` as a canonical, metadata-free SafeTensors file.
///
/// Tensor entries and payload bytes are ordered by ascending tensor name using
/// UTF-8 byte order. The JSON header is emitted with fixed field order and
/// padded according to the SafeTensors container contract. No timestamps,
/// filesystem paths, build durations, or caller metadata are embedded in the
/// file bytes.
pub fn canonical_checkpoint_write(
    path: &Path,
    tensors: &[CanonicalTensor],
    metadata: &CheckpointMetadata,
) -> Result<(), CheckpointWriteError> {
    let bytes = canonical_checkpoint_bytes(tensors, metadata)?;
    fs::write(path, &bytes).map_err(CheckpointWriteError::Io)?;

    tracing::info!(
        target: "gbf_experiments::s1",
        event = "s1.checkpoint_writer.write.complete",
        path = %path.display(),
        file_byte_count = bytes.len(),
        tensor_payload_hash = %canonical_tensor_payload_hash(tensors)
    );

    Ok(())
}

/// Return the canonical SafeTensors byte stream for `tensors`.
///
/// `metadata` is accepted to keep the run-orchestration call shape aligned with
/// checkpoint sidecar emission, but none of its fields are embedded in the
/// SafeTensors bytes.
pub fn canonical_checkpoint_bytes(
    tensors: &[CanonicalTensor],
    _metadata: &CheckpointMetadata,
) -> Result<Vec<u8>, CheckpointWriteError> {
    let mut ordered_tensors = tensors.iter().collect::<Vec<_>>();
    ordered_tensors.sort_by(|left, right| {
        left.id
            .as_str()
            .as_bytes()
            .cmp(right.id.as_str().as_bytes())
    });

    if let Some(window) = ordered_tensors
        .windows(2)
        .find(|window| window[0].id.as_str() == window[1].id.as_str())
    {
        return Err(CheckpointWriteError::DuplicateTensorName {
            name: window[0].id.to_string(),
        });
    }

    let total_payload_bytes = ordered_tensors
        .iter()
        .map(|tensor| tensor_payload_byte_len(tensor))
        .sum::<usize>();
    tracing::debug!(
        target: "gbf_experiments::s1",
        event = "s1.checkpoint_writer.write.start",
        n_tensors = ordered_tensors.len(),
        total_payload_bytes
    );

    let mut header = String::new();
    header.push('{');

    let mut offset = 0usize;
    for (index, tensor) in ordered_tensors.iter().enumerate() {
        if index > 0 {
            header.push(',');
        }

        let payload_len = tensor_payload_byte_len(tensor);
        let next_offset =
            offset
                .checked_add(payload_len)
                .ok_or(CheckpointWriteError::PayloadOffsetOverflow {
                    tensor_name: tensor.id.to_string(),
                })?;

        header.push_str(&json_string(tensor.id.as_str())?);
        header.push_str(r#":{"dtype":"#);
        header.push('"');
        header.push_str(safetensors_dtype(tensor.layout.element_type));
        header.push_str(r#"","shape":["#);
        for (dim_index, dim) in tensor.layout.shape.dims().iter().enumerate() {
            if dim_index > 0 {
                header.push(',');
            }
            header.push_str(&dim.to_string());
        }
        header.push_str(r#"],"data_offsets":["#);
        header.push_str(&offset.to_string());
        header.push(',');
        header.push_str(&next_offset.to_string());
        header.push_str("]}");

        offset = next_offset;
    }

    header.push('}');

    let aligned_header_len = header.len().next_multiple_of(size_of::<u64>());
    let mut header_bytes = header.into_bytes();
    header_bytes.resize(aligned_header_len, b' ');

    let header_len = u64::try_from(header_bytes.len())
        .map_err(|_| CheckpointWriteError::HeaderLengthOverflow)?;
    let mut bytes = Vec::with_capacity(size_of::<u64>() + header_bytes.len() + offset);
    bytes.extend_from_slice(&header_len.to_le_bytes());
    bytes.extend_from_slice(&header_bytes);
    for tensor in ordered_tensors {
        write_tensor_payload_bytes(tensor, &mut bytes);
    }

    Ok(bytes)
}

/// Errors returned while writing canonical S1 checkpoint bytes.
#[derive(Debug)]
pub enum CheckpointWriteError {
    /// More than one tensor used the same canonical tensor name.
    DuplicateTensorName {
        /// Duplicate tensor name.
        name: String,
    },
    /// The SafeTensors header could not be encoded as JSON.
    HeaderJson(serde_json::Error),
    /// The SafeTensors header length exceeded `u64::MAX`.
    HeaderLengthOverflow,
    /// Payload byte offsets overflowed while writing a tensor.
    PayloadOffsetOverflow {
        /// Tensor whose payload offset overflowed.
        tensor_name: String,
    },
    /// Filesystem write failed.
    Io(io::Error),
}

impl fmt::Display for CheckpointWriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateTensorName { name } => {
                write!(f, "canonical checkpoint contains duplicate tensor {name:?}")
            }
            Self::HeaderJson(error) => {
                write!(
                    f,
                    "failed to encode canonical checkpoint header JSON: {error}"
                )
            }
            Self::HeaderLengthOverflow => {
                f.write_str("canonical checkpoint header length exceeds u64")
            }
            Self::PayloadOffsetOverflow { tensor_name } => {
                write!(
                    f,
                    "canonical checkpoint payload offsets overflow at tensor {tensor_name:?}"
                )
            }
            Self::Io(error) => write!(f, "failed to write canonical checkpoint: {error}"),
        }
    }
}

impl Error for CheckpointWriteError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::HeaderJson(error) => Some(error),
            Self::Io(error) => Some(error),
            Self::DuplicateTensorName { .. }
            | Self::HeaderLengthOverflow
            | Self::PayloadOffsetOverflow { .. } => None,
        }
    }
}

fn json_string(value: &str) -> Result<String, CheckpointWriteError> {
    serde_json::to_string(value).map_err(CheckpointWriteError::HeaderJson)
}

fn safetensors_dtype(element_type: TensorElementType) -> &'static str {
    // SafeTensors records storage dtypes. Canonical semantic tensor kinds remain
    // part of the artifact/sidecar contract, not this metadata-free container.
    match element_type {
        TensorElementType::Float32 => "F32",
        TensorElementType::TernaryI2 => "I8",
        TensorElementType::Q8_8 => "U16",
    }
}

fn tensor_payload_byte_len(tensor: &CanonicalTensor) -> usize {
    match &tensor.payload {
        CanonicalTensorPayload::F32(values) => values.len() * size_of::<f32>(),
        CanonicalTensorPayload::I8(values) => values.len() * size_of::<i8>(),
        CanonicalTensorPayload::U16(values) => values.len() * size_of::<u16>(),
    }
}

fn write_tensor_payload_bytes(tensor: &CanonicalTensor, bytes: &mut Vec<u8>) {
    match &tensor.payload {
        CanonicalTensorPayload::F32(values) => {
            for value in values {
                bytes.extend_from_slice(&value.to_bits().to_le_bytes());
            }
        }
        CanonicalTensorPayload::I8(values) => {
            for value in values {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        CanonicalTensorPayload::U16(values) => {
            for value in values {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_eval_scorer_matches_burn_forward_for_fixed_tensors() {
        let tensors = fixed_production_tensors();
        let sequence = [3_u8, 7, 3, 11, 5, 19];

        let scorer = ProductionEvalScorer::from_tensors(ModelSizeProfile::toy0(), &tensors)
            .expect("eval scorer");
        let eval_bpc = scorer.bpc(&sequence, S1_SEQUENCE_LENGTH).expect("eval bpc");
        let burn_nats =
            production_loss_nats_per_byte_for_tensors(&tensors, &sequence).expect("burn forward");
        let burn_bpc = f64::from(burn_nats) / std::f64::consts::LN_2;

        assert!(
            (eval_bpc - burn_bpc).abs() < 1.0e-5,
            "eval bpc={eval_bpc:.10} burn bpc={burn_bpc:.10}"
        );
    }

    fn fixed_production_tensors() -> Vec<CanonicalTensor> {
        let profile = ModelSizeProfile::toy0();
        let d_model = usize::from(profile.d_model());
        let state_slots = s1_dense_state_slots(profile).expect("state slots");
        let d_ff = usize::from(profile.d_ff());
        vec![
            f32_tensor(
                &s1_production_tensor_id(profile, "embedding_tied.weight").expect("tensor id"),
                &[S1_BYTE_VOCAB_SIZE, d_model],
                production_values(S1_BYTE_VOCAB_SIZE * d_model, 1),
            ),
            f32_tensor(
                &s1_production_tensor_id(profile, "linear_state.input_to_state.weight")
                    .expect("tensor id"),
                &[d_model, state_slots],
                production_values(d_model * state_slots, 2),
            ),
            f32_tensor(
                &s1_production_tensor_id(profile, "linear_state.state_to_output.weight")
                    .expect("tensor id"),
                &[state_slots, d_model],
                production_values(state_slots * d_model, 3),
            ),
            f32_tensor(
                &s1_production_tensor_id(profile, "dense_ffn.up.weight").expect("tensor id"),
                &[d_model, d_ff],
                production_values(d_model * d_ff, 4),
            ),
            f32_tensor(
                &s1_production_tensor_id(profile, "dense_ffn.down.weight").expect("tensor id"),
                &[d_ff, d_model],
                production_values(d_ff * d_model, 5),
            ),
        ]
    }

    fn production_values(len: usize, salt: usize) -> Vec<f32> {
        (0..len)
            .map(|index| {
                let value = ((index * 37 + salt * 11) % 29) as f32 - 14.0;
                value / 75.0
            })
            .collect()
    }

    fn f32_tensor(name: &str, shape: &[usize], values: Vec<f32>) -> CanonicalTensor {
        CanonicalTensor::new(
            ArtifactPath::new(name).expect("valid artifact path"),
            CanonicalTensorKind::DenseWeight,
            CanonicalTensorLayout::new(
                CanonicalTensorShape::from_usize_dims(shape).expect("shape"),
                TensorElementType::Float32,
            ),
            CanonicalTensorPayload::F32(values),
        )
        .expect("tensor")
    }
}
