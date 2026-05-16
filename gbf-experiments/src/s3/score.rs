//! S3 reset-context scoring helpers.

use std::error::Error;
use std::fmt;

use gbf_artifact::{CharId, TextCharSeq, VOCAB_SIZE};
use gbf_foundation::{DomainHash, Hash256, self_hash_omitting_fields, sha256};
use gbf_oracle::scorers::{ArtifactScorer, ReferenceScorer};
use serde::{Deserialize, Serialize};

pub mod kn_scorer;

pub use kn_scorer::KnScorer;

/// RFC-pinned S3 score chunk size in normalized characters.
pub const S3_SCORE_CHUNK_SIZE: usize = 128;

/// S3 score tracing target.
pub const S3_SCORE_LOG_TARGET: &str = "gbf_experiments::s3::score";

const SCORE_SCHEMA: &str = "s3_score.v1";
const SCORE_SCHEMA_VERSION: &str = "1";
const LOG2_E: f64 = std::f64::consts::LOG2_E;
const TARGET_LOGPROB_TOLERANCE: f64 = 1.0e-5;

/// Scorer implementation used for an `s3_score.v1` product.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ScorerKind {
    /// Full-precision reference-bundle scorer.
    ReferenceScorer,
    /// Quantized artifact scorer.
    ArtifactScorer,
    /// Five-gram Kneser-Ney baseline scorer.
    KnScorer,
}

impl ScorerKind {
    /// Stable field/log string.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ReferenceScorer => "ReferenceScorer",
            Self::ArtifactScorer => "ArtifactScorer",
            Self::KnScorer => "KnScorer",
        }
    }
}

/// Finite, non-negative bits-per-character value.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "f64", into = "f64")]
pub struct BpcCharValue(f64);

impl BpcCharValue {
    /// Construct a checked bpc_char value.
    pub fn try_new(value: f64) -> Result<Self, ScoreError> {
        if value.is_finite() && value >= 0.0 {
            Ok(Self(value))
        } else {
            Err(ScoreError::InvalidBpcChar { value })
        }
    }

    /// Return the inner f64.
    #[must_use]
    pub const fn get(self) -> f64 {
        self.0
    }
}

impl TryFrom<f64> for BpcCharValue {
    type Error = ScoreError;

    fn try_from(value: f64) -> Result<Self, Self::Error> {
        Self::try_new(value)
    }
}

impl From<BpcCharValue> for f64 {
    fn from(value: BpcCharValue) -> Self {
        value.0
    }
}

/// One per-token evaluator output row.
#[derive(Debug, Clone, PartialEq)]
pub struct EvaluatorOutput {
    /// Per-vocab logits for exactly one target position.
    pub logits: Vec<f32>,
    /// Natural-log probability of `target_ix` under `logits`.
    pub target_logprob: f64,
}

impl EvaluatorOutput {
    /// Build an output row and compute the target log-probability from logits.
    pub fn from_logits(logits: Vec<f32>, target_ix: usize) -> Result<Self, ScoreError> {
        let target_logprob = target_logprob_from_logits(&logits, target_ix)?;
        Ok(Self {
            logits,
            target_logprob,
        })
    }
}

/// Object-safe per-token evaluator consumed by `s3_score_bpc_char`.
pub trait Evaluator: Send + Sync {
    /// Public scorer kind for schema/logging.
    fn scorer_kind(&self) -> ScorerKind;

    /// Return the output row for `target_ix` given the reset-chunk prefix.
    fn forward(&self, prefix: &[CharId], target_ix: usize) -> EvaluatorOutput;

    /// Reset provider-owned recurrent state at each chunk boundary.
    fn reset_state(&mut self);
}

/// Canonical `s3_score.v1` product.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScoreCharProduct {
    /// Schema id.
    pub schema: String,
    /// Scorer used to produce this score.
    pub scorer_kind: ScorerKind,
    /// Reset chunk size in normalized characters.
    pub chunk_size: u64,
    /// Bits per normalized character.
    pub bpc_char: BpcCharValue,
    /// Number of normalized characters scored.
    pub char_count: u64,
    /// Sum of negative log probabilities in base 2.
    pub log2_sum: f64,
    /// Self-hash over this product with this field omitted.
    pub score_self_hash: Hash256,
}

impl ScoreCharProduct {
    /// DomainHash context.
    #[must_use]
    pub const fn domain() -> DomainHash<'static> {
        DomainHash::new(
            "gbf-experiments",
            "ScoreCharProduct",
            SCORE_SCHEMA,
            SCORE_SCHEMA_VERSION,
        )
    }

    /// Compute self-hash with `score_self_hash` omitted.
    pub fn computed_self_hash(&self) -> Result<Hash256, ScoreError> {
        Ok(self_hash_omitting_fields(
            Self::domain(),
            self,
            "score_self_hash",
            &[],
        )?)
    }

    /// Return a product with `score_self_hash` recomputed.
    pub fn with_computed_self_hash(mut self) -> Result<Self, ScoreError> {
        self.score_self_hash = self.computed_self_hash()?;
        Ok(self)
    }
}

/// Checked form of the S3 bpc-per-character primitive.
pub fn try_s3_score_bpc_char<E>(
    mut evaluator: E,
    val_post: &TextCharSeq,
    chunk_size: usize,
) -> Result<ScoreCharProduct, ScoreError>
where
    E: Evaluator,
{
    if val_post.is_empty() {
        return Err(ScoreError::EmptyValidation);
    }
    if chunk_size == 0 {
        return Err(ScoreError::InvalidChunkSize { chunk_size });
    }

    let scorer_kind = evaluator.scorer_kind();
    tracing::info!(
        target: S3_SCORE_LOG_TARGET,
        event_name = "s3::score::started",
        scorer_kind = scorer_kind.as_str(),
        char_count = val_post.len() as u64,
        chunk_size = chunk_size as u64,
    );

    let mut log2_sum = 0.0_f64;
    for (chunk_index, chunk) in val_post.as_slice().chunks(chunk_size).enumerate() {
        evaluator.reset_state();
        let mut chunk_log2_sum = 0.0_f64;
        for (offset, &target) in chunk.iter().enumerate() {
            let prefix = &chunk[..offset];
            let target_ix = usize::from(target);
            let output = evaluator.forward(prefix, target_ix);
            let loss = checked_loss_log2(&output, target_ix)?;
            log2_sum += loss;
            chunk_log2_sum += loss;
        }
        tracing::trace!(
            target: S3_SCORE_LOG_TARGET,
            event_name = "s3::score::chunk_complete",
            chunk_index = chunk_index as u64,
            chunk_log2_sum,
            chunk_char_count = chunk.len() as u64,
        );
    }

    if !log2_sum.is_finite() || log2_sum < 0.0 {
        return Err(ScoreError::InvalidLog2Sum { value: log2_sum });
    }
    let char_count = val_post.len() as u64;
    let bpc_char = BpcCharValue::try_new(log2_sum / char_count as f64)?;
    let product = ScoreCharProduct {
        schema: SCORE_SCHEMA.to_owned(),
        scorer_kind,
        chunk_size: chunk_size as u64,
        bpc_char,
        char_count,
        log2_sum,
        score_self_hash: Hash256::ZERO,
    }
    .with_computed_self_hash()?;

    tracing::info!(
        target: S3_SCORE_LOG_TARGET,
        event_name = "s3::score::complete",
        scorer_kind = scorer_kind.as_str(),
        char_count,
        log2_sum = product.log2_sum,
        bpc_char = product.bpc_char.get(),
        score_self_hash = %product.score_self_hash,
    );

    Ok(product)
}

/// Score normalized validation characters with S3 reset-context semantics.
///
/// This is the RFC-shaped convenience wrapper. Use
/// [`try_s3_score_bpc_char`] when callers need to inspect validation errors.
#[must_use]
pub fn s3_score_bpc_char<E>(
    evaluator: E,
    val_post: &TextCharSeq,
    chunk_size: usize,
) -> ScoreCharProduct
where
    E: Evaluator,
{
    try_s3_score_bpc_char(evaluator, val_post, chunk_size)
        .expect("S3 score inputs must satisfy the score contract")
}

impl Evaluator for ReferenceScorer<'_> {
    fn scorer_kind(&self) -> ScorerKind {
        ScorerKind::ReferenceScorer
    }

    fn forward(&self, prefix: &[CharId], target_ix: usize) -> EvaluatorOutput {
        let logits = self.forward_logits(prefix);
        EvaluatorOutput::from_logits(logits, target_ix)
            .expect("reference scorer emits valid logits")
    }

    fn reset_state(&mut self) {
        ReferenceScorer::reset_state(self);
    }
}

impl Evaluator for ArtifactScorer<'_> {
    fn scorer_kind(&self) -> ScorerKind {
        ScorerKind::ArtifactScorer
    }

    fn forward(&self, prefix: &[CharId], target_ix: usize) -> EvaluatorOutput {
        let logits = self.forward_logits(prefix);
        EvaluatorOutput::from_logits(logits, target_ix).expect("artifact scorer emits valid logits")
    }

    fn reset_state(&mut self) {
        ArtifactScorer::reset_state(self);
    }
}

/// Errors from S3 character scoring.
#[derive(Debug)]
pub enum ScoreError {
    /// S3 scoring requires a non-empty validation sequence.
    EmptyValidation,
    /// Chunk size must be positive.
    InvalidChunkSize {
        /// Observed chunk size.
        chunk_size: usize,
    },
    /// Evaluators must emit one finite logit per charset_v1 id.
    LogitsWrongLength {
        /// Observed logits length.
        len: usize,
        /// Required vocab size.
        expected: usize,
    },
    /// A logit was not finite.
    NonFiniteLogit {
        /// Logit index.
        index: usize,
        /// Observed value.
        value: f32,
    },
    /// Target index was outside the S3 vocab.
    TargetOutOfRange {
        /// Observed target index.
        target_ix: usize,
    },
    /// Evaluator target log-probability was invalid.
    InvalidTargetLogprob {
        /// Observed log-probability.
        value: f64,
    },
    /// Evaluator target log-probability did not match its per-token logits.
    TargetLogprobMismatch {
        /// Log-probability supplied by the evaluator.
        supplied: f64,
        /// Log-probability recomputed from the logits row.
        recomputed: f64,
    },
    /// Computed log2 sum was invalid.
    InvalidLog2Sum {
        /// Observed log2 sum.
        value: f64,
    },
    /// bpc_char must be finite and non-negative.
    InvalidBpcChar {
        /// Observed bpc value.
        value: f64,
    },
    /// Score self-hash construction failed.
    CanonicalJson(gbf_foundation::CanonicalJsonError),
    /// Kneser-Ney scorer setup failed.
    Baseline(crate::s3::baseline::BaselineError),
    /// Kneser-Ney scorer train hash did not match the report.
    KnTrainHashMismatch {
        /// Reported train hash.
        expected: Hash256,
        /// Observed train hash.
        observed: Hash256,
    },
}

impl fmt::Display for ScoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyValidation => f.write_str("S3 score requires non-empty validation text"),
            Self::InvalidChunkSize { chunk_size } => {
                write!(f, "S3 score chunk size must be positive, got {chunk_size}")
            }
            Self::LogitsWrongLength { len, expected } => {
                write!(
                    f,
                    "S3 evaluator logits length {len} does not match vocab size {expected}"
                )
            }
            Self::NonFiniteLogit { index, value } => {
                write!(f, "S3 evaluator logit {index} is non-finite: {value}")
            }
            Self::TargetOutOfRange { target_ix } => {
                write!(
                    f,
                    "S3 target index {target_ix} is outside vocab size {VOCAB_SIZE}"
                )
            }
            Self::InvalidTargetLogprob { value } => {
                write!(f, "S3 target log-probability is invalid: {value}")
            }
            Self::TargetLogprobMismatch {
                supplied,
                recomputed,
            } => write!(
                f,
                "S3 target log-probability mismatch: supplied {supplied}, recomputed {recomputed}"
            ),
            Self::InvalidLog2Sum { value } => write!(f, "S3 log2 sum is invalid: {value}"),
            Self::InvalidBpcChar { value } => write!(f, "S3 bpc_char is invalid: {value}"),
            Self::CanonicalJson(error) => write!(f, "{error}"),
            Self::Baseline(error) => write!(f, "{error}"),
            Self::KnTrainHashMismatch { expected, observed } => {
                write!(
                    f,
                    "KN scorer train hash mismatch: expected {expected}, observed {observed}"
                )
            }
        }
    }
}

impl Error for ScoreError {}

impl From<gbf_foundation::CanonicalJsonError> for ScoreError {
    fn from(error: gbf_foundation::CanonicalJsonError) -> Self {
        Self::CanonicalJson(error)
    }
}

impl From<crate::s3::baseline::BaselineError> for ScoreError {
    fn from(error: crate::s3::baseline::BaselineError) -> Self {
        Self::Baseline(error)
    }
}

fn checked_loss_log2(output: &EvaluatorOutput, target_ix: usize) -> Result<f64, ScoreError> {
    let recomputed = target_logprob_from_logits(&output.logits, target_ix)?;
    if !output.target_logprob.is_finite() || output.target_logprob > 0.0 {
        return Err(ScoreError::InvalidTargetLogprob {
            value: output.target_logprob,
        });
    }
    if (output.target_logprob - recomputed).abs() > TARGET_LOGPROB_TOLERANCE {
        return Err(ScoreError::TargetLogprobMismatch {
            supplied: output.target_logprob,
            recomputed,
        });
    }
    let loss = -output.target_logprob * LOG2_E;
    if loss.is_finite() && loss >= 0.0 {
        Ok(loss)
    } else {
        Err(ScoreError::InvalidLog2Sum { value: loss })
    }
}

fn target_logprob_from_logits(logits: &[f32], target_ix: usize) -> Result<f64, ScoreError> {
    if target_ix >= VOCAB_SIZE {
        return Err(ScoreError::TargetOutOfRange { target_ix });
    }
    if logits.len() != VOCAB_SIZE {
        return Err(ScoreError::LogitsWrongLength {
            len: logits.len(),
            expected: VOCAB_SIZE,
        });
    }
    for (index, &value) in logits.iter().enumerate() {
        if !value.is_finite() {
            return Err(ScoreError::NonFiniteLogit { index, value });
        }
    }

    let max_logit = logits
        .iter()
        .copied()
        .map(f64::from)
        .fold(f64::NEG_INFINITY, f64::max);
    let exp_sum = logits
        .iter()
        .copied()
        .map(|logit| (f64::from(logit) - max_logit).exp())
        .sum::<f64>();
    let log_sum_exp = max_logit + exp_sum.ln();
    let target_logprob = f64::from(logits[target_ix]) - log_sum_exp;
    if target_logprob.is_finite() && target_logprob <= 0.0 {
        Ok(target_logprob)
    } else {
        Err(ScoreError::InvalidTargetLogprob {
            value: target_logprob,
        })
    }
}

pub(crate) fn target_logprob_from_probabilities(
    probabilities: &[f64],
    target_ix: usize,
) -> Result<f64, ScoreError> {
    if target_ix >= probabilities.len() {
        return Err(ScoreError::TargetOutOfRange { target_ix });
    }
    let probability = probabilities[target_ix];
    if probability.is_finite() && probability > 0.0 {
        Ok(probability.ln())
    } else {
        Err(ScoreError::InvalidTargetLogprob {
            value: probability.ln(),
        })
    }
}

pub(crate) fn logits_from_probabilities(probabilities: &[f64]) -> Vec<f32> {
    probabilities
        .iter()
        .map(|probability| probability.max(0.0).ln().max(-1.0e30) as f32)
        .collect()
}

pub(crate) fn train_hash(seq: &TextCharSeq) -> Hash256 {
    sha256(seq.as_slice())
}
