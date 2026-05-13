//! S2 reset-context score wrapper.

use std::error::Error;
use std::fmt;

use gbf_foundation::{Hash256, sha256};
use safetensors::{Dtype, SafeTensorError, SafeTensors};

use crate::S2_LOG_TARGET;
use crate::s1::schema::S1SchemaError;
use crate::s1::score::{ResetContextScorer, ScoreError, reset_context_bpc};
use crate::s2::run::CompletedRunProductS2;
use crate::s2::schema::{S2BuildKind, S2ScoreReport, ScaleStatsSummary, ThresholdStatsSummary};

/// Validation bytes and corpus identity consumed by the S2 score wrapper.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScoreInputs {
    /// Canonical held-out validation bytes.
    pub val_bytes: Vec<u8>,
    /// Expected SHA-256 of `val_bytes`.
    pub corpus_val_sha: Hash256,
}

impl ScoreInputs {
    /// Construct score inputs with the validation SHA computed from the bytes.
    #[must_use]
    pub fn new(val_bytes: impl Into<Vec<u8>>) -> Self {
        let val_bytes = val_bytes.into();
        let corpus_val_sha = sha256(&val_bytes);
        Self {
            val_bytes,
            corpus_val_sha,
        }
    }

    /// Construct score inputs with an externally pinned validation SHA.
    #[must_use]
    pub fn with_corpus_val_sha(val_bytes: impl Into<Vec<u8>>, corpus_val_sha: Hash256) -> Self {
        Self {
            val_bytes: val_bytes.into(),
            corpus_val_sha,
        }
    }
}

/// Emit an `s2_score.v1` report from a completed S2 run product.
///
/// The current S2 scorer is the Toy0 deterministic reset-context scorer used
/// by this experiment slice. It exercises the inherited S1 BPC primitive and
/// report contract; real model-forward scoring remains owned by the downstream
/// replay/integration path.
///
/// This convenience wrapper panics on invalid inputs. Use [`try_s2_score`] when
/// callers need to inspect the concrete error.
#[must_use]
pub fn s2_score(inputs: ScoreInputs, run_product: &CompletedRunProductS2) -> S2ScoreReport {
    try_s2_score(inputs, run_product).expect("s2_score inputs must satisfy the score contract")
}

/// Checked form of [`s2_score`].
pub fn try_s2_score(
    inputs: ScoreInputs,
    run_product: &CompletedRunProductS2,
) -> Result<S2ScoreReport, S2ScoreError> {
    let observed_val_sha = sha256(&inputs.val_bytes);
    if observed_val_sha != inputs.corpus_val_sha {
        return Err(S2ScoreError::CorpusValShaMismatch {
            expected: inputs.corpus_val_sha,
            observed: observed_val_sha,
        });
    }

    let build_kind = run_product.phase_log.build_kind;
    let scorer = S2CheckpointScorer::from_checkpoint(&run_product.final_checkpoint);
    let product = reset_context_bpc(&scorer, &inputs.val_bytes)?;
    let (threshold_stats, scale_stats) =
        qat_stats_from_checkpoint(build_kind, &run_product.final_checkpoint)?;
    let report = S2ScoreReport::new(
        run_product.phase_log.seed,
        build_kind,
        sha256(&run_product.final_checkpoint),
        inputs.corpus_val_sha,
        product.token_count,
        product.log2_sum,
        threshold_stats,
        scale_stats,
    )?;

    let score_self_hash = report.score_self_hash.to_hex();
    tracing::info!(
        target: S2_LOG_TARGET,
        event_name = "s2_score_computed",
        build_kind = ?report.build_kind,
        seed = report.seed,
        bpc = report.bpc,
        token_count = report.token_count,
        log2_sum = report.log2_sum,
        threshold_stats_present = report.threshold_stats.is_some(),
        scale_stats_present = report.scale_stats.is_some(),
        score_self_hash = score_self_hash.as_str(),
        "s2 score computed"
    );

    Ok(report)
}

/// Deterministic Toy0 scorer backed by checkpoint identity.
#[derive(Debug, Clone)]
struct S2CheckpointScorer {
    logits_bias: [f64; 256],
}

impl S2CheckpointScorer {
    fn from_checkpoint(checkpoint: &[u8]) -> Self {
        let digest = sha256(checkpoint);
        let mut logits_bias = [0.0_f64; 256];
        for (index, bias) in logits_bias.iter_mut().enumerate() {
            let digest_byte = digest.as_bytes()[index % digest.as_bytes().len()];
            let mixed = digest_byte ^ index as u8;
            *bias = (f64::from(mixed) / 255.0 - 0.5) * 0.25;
        }
        Self { logits_bias }
    }
}

impl ResetContextScorer for S2CheckpointScorer {
    type State = Vec<u8>;

    fn fresh_state(&self) -> Self::State {
        Vec::new()
    }

    fn logits(&self, state: &Self::State) -> Vec<f64> {
        let mut logits = self.logits_bias.to_vec();
        if let Some(&last_byte) = state.last() {
            logits[usize::from(last_byte)] += 0.05;
        }
        logits
    }

    fn consume(&self, state: &mut Self::State, byte: u8) {
        state.push(byte);
    }

    fn context_len(&self, state: &Self::State) -> Option<usize> {
        Some(state.len())
    }
}

fn qat_stats_from_checkpoint(
    build_kind: S2BuildKind,
    checkpoint: &[u8],
) -> Result<(Option<ThresholdStatsSummary>, Option<ScaleStatsSummary>), S2ScoreError> {
    if !needs_qat_stats(build_kind) {
        return Ok((None, None));
    }

    let safetensors = SafeTensors::deserialize(checkpoint)?;
    let mut threshold_tensors = Vec::new();
    let mut scale_tensors = Vec::new();

    for (name, view) in safetensors.iter() {
        if is_threshold_tensor_name(name) {
            if view.dtype() != Dtype::F32 {
                return Err(S2ScoreError::InvalidCheckpointTensor {
                    tensor: name.to_owned(),
                    reason: "threshold tensors must be F32".to_owned(),
                });
            }
            threshold_tensors.push((name.to_owned(), view));
        } else if is_scale_tensor_name(name) {
            if view.dtype() != Dtype::U16 {
                return Err(S2ScoreError::InvalidCheckpointTensor {
                    tensor: name.to_owned(),
                    reason: "scale tensors must be Q8.8/U16".to_owned(),
                });
            }
            scale_tensors.push((name.to_owned(), view));
        }
    }

    threshold_tensors.sort_by(|left, right| left.0.cmp(&right.0));
    scale_tensors.sort_by(|left, right| left.0.cmp(&right.0));

    let mut threshold_values = Vec::new();
    for (name, view) in &threshold_tensors {
        threshold_values.extend(read_f32_tensor(name, view.data())?);
    }
    let mut scale_values = Vec::new();
    for (name, view) in &scale_tensors {
        scale_values.extend(
            read_u16_tensor(name, view.data())?
                .into_iter()
                .map(q8_8_to_f32),
        );
    }

    if threshold_values.is_empty() || scale_values.is_empty() {
        return Err(S2ScoreError::MissingQatStats { build_kind });
    }
    if threshold_values.len() != scale_values.len() {
        return Err(S2ScoreError::QatStatsCountMismatch {
            threshold_count: threshold_values.len(),
            scale_count: scale_values.len(),
        });
    }

    Ok((
        Some(threshold_summary(
            checked_matrix_count(threshold_tensors.len(), "threshold matrices")?,
            &threshold_values,
        )?),
        Some(scale_summary(
            checked_matrix_count(scale_tensors.len(), "scale matrices")?,
            &scale_values,
        )?),
    ))
}

fn is_threshold_tensor_name(name: &str) -> bool {
    name.ends_with(".thresholds")
}

fn is_scale_tensor_name(name: &str) -> bool {
    name.ends_with(".scales")
}

fn checked_matrix_count(count: usize, field: &'static str) -> Result<u32, S2ScoreError> {
    u32::try_from(count).map_err(|_| S2ScoreError::QatStatsCountOverflow { field })
}

fn needs_qat_stats(build_kind: S2BuildKind) -> bool {
    matches!(
        build_kind,
        S2BuildKind::s2_ternary_full | S2BuildKind::s2_ternary_nodistill
    )
}

fn read_f32_tensor(name: &str, data: &[u8]) -> Result<Vec<f32>, S2ScoreError> {
    let chunks = data.chunks_exact(4);
    if !chunks.remainder().is_empty() {
        return Err(S2ScoreError::InvalidCheckpointTensor {
            tensor: name.to_owned(),
            reason: "F32 payload length is not divisible by 4".to_owned(),
        });
    }
    let mut values = Vec::with_capacity(data.len() / 4);
    for (index, chunk) in chunks.enumerate() {
        let value = f32::from_le_bytes(chunk.try_into().expect("chunk length is 4"));
        if !value.is_finite() {
            return Err(S2ScoreError::InvalidCheckpointTensor {
                tensor: name.to_owned(),
                reason: format!("non-finite F32 value at index {index}"),
            });
        }
        values.push(value);
    }
    Ok(values)
}

fn read_u16_tensor(name: &str, data: &[u8]) -> Result<Vec<u16>, S2ScoreError> {
    let chunks = data.chunks_exact(2);
    if !chunks.remainder().is_empty() {
        return Err(S2ScoreError::InvalidCheckpointTensor {
            tensor: name.to_owned(),
            reason: "U16 payload length is not divisible by 2".to_owned(),
        });
    }
    Ok(chunks
        .map(|chunk| u16::from_le_bytes(chunk.try_into().expect("chunk length is 2")))
        .collect())
}

fn q8_8_to_f32(value: u16) -> f32 {
    f32::from(value) / 256.0
}

fn threshold_summary(matrices: u32, values: &[f32]) -> Result<ThresholdStatsSummary, S2ScoreError> {
    let count = u32::try_from(values.len()).map_err(|_| S2ScoreError::QatStatsCountOverflow {
        field: "threshold_count",
    })?;
    let (min, max, mean) = summary_values(values);
    Ok(ThresholdStatsSummary {
        matrices,
        threshold_min: min,
        threshold_max: max,
        threshold_mean: mean,
        threshold_count: count,
    })
}

fn scale_summary(matrices: u32, values: &[f32]) -> Result<ScaleStatsSummary, S2ScoreError> {
    let count = u32::try_from(values.len()).map_err(|_| S2ScoreError::QatStatsCountOverflow {
        field: "scale_count",
    })?;
    let (min, max, mean) = summary_values(values);
    Ok(ScaleStatsSummary {
        matrices,
        scale_count: count,
        scale_min: min,
        scale_max: max,
        scale_mean_f32: mean,
    })
}

fn summary_values(values: &[f32]) -> (f32, f32, f32) {
    let min = values
        .iter()
        .copied()
        .fold(f32::INFINITY, |left, right| left.min(right));
    let max = values
        .iter()
        .copied()
        .fold(f32::NEG_INFINITY, |left, right| left.max(right));
    let mean =
        (values.iter().map(|&value| f64::from(value)).sum::<f64>() / values.len() as f64) as f32;
    (min, max, mean)
}

/// Errors returned by the checked S2 score wrapper.
#[derive(Debug)]
pub enum S2ScoreError {
    /// Validation bytes did not match the expected corpus hash.
    CorpusValShaMismatch {
        /// Expected validation corpus hash.
        expected: Hash256,
        /// Observed validation corpus hash.
        observed: Hash256,
    },
    /// The inherited S1 score primitive rejected the inputs.
    Score(ScoreError),
    /// The `s2_score.v1` schema rejected the report.
    Schema(S1SchemaError),
    /// Final checkpoint bytes were not valid SafeTensors.
    Safetensors(SafeTensorError),
    /// A final checkpoint tensor could not be interpreted as S2 QAT stats.
    InvalidCheckpointTensor {
        /// Tensor name.
        tensor: String,
        /// Rejection reason.
        reason: String,
    },
    /// A ternary build did not contain threshold/scale buffers.
    MissingQatStats {
        /// Build kind requiring QAT stats.
        build_kind: S2BuildKind,
    },
    /// Threshold and scale summaries were not row-aligned.
    QatStatsCountMismatch {
        /// Number of threshold rows.
        threshold_count: usize,
        /// Number of scale rows.
        scale_count: usize,
    },
    /// A QAT stat count could not fit the public schema.
    QatStatsCountOverflow {
        /// Count field that overflowed.
        field: &'static str,
    },
}

impl fmt::Display for S2ScoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CorpusValShaMismatch { expected, observed } => write!(
                f,
                "validation corpus hash mismatch: expected {expected}, observed {observed}"
            ),
            Self::Score(error) => write!(f, "{error}"),
            Self::Schema(error) => write!(f, "{error}"),
            Self::Safetensors(error) => write!(f, "{error}"),
            Self::InvalidCheckpointTensor { tensor, reason } => {
                write!(f, "invalid checkpoint tensor {tensor:?}: {reason}")
            }
            Self::MissingQatStats { build_kind } => {
                write!(f, "{build_kind:?} requires threshold and scale stats")
            }
            Self::QatStatsCountMismatch {
                threshold_count,
                scale_count,
            } => write!(
                f,
                "threshold_count {threshold_count} does not match scale_count {scale_count}"
            ),
            Self::QatStatsCountOverflow { field } => {
                write!(f, "QAT stats count field {field} overflowed")
            }
        }
    }
}

impl Error for S2ScoreError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Score(error) => Some(error),
            Self::Schema(error) => Some(error),
            Self::Safetensors(error) => Some(error),
            Self::CorpusValShaMismatch { .. }
            | Self::InvalidCheckpointTensor { .. }
            | Self::MissingQatStats { .. }
            | Self::QatStatsCountMismatch { .. }
            | Self::QatStatsCountOverflow { .. } => None,
        }
    }
}

impl From<ScoreError> for S2ScoreError {
    fn from(error: ScoreError) -> Self {
        Self::Score(error)
    }
}

impl From<S1SchemaError> for S2ScoreError {
    fn from(error: S1SchemaError) -> Self {
        Self::Schema(error)
    }
}

impl From<SafeTensorError> for S2ScoreError {
    fn from(error: SafeTensorError) -> Self {
        Self::Safetensors(error)
    }
}
