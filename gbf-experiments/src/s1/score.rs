//! Reset-context scoring primitives for S1.

use std::fmt;

use gbf_foundation::Hash256;

use crate::s1::logging::{
    LoggingEventError, S1LogEmitter, ScoreCompleteEvent, ScoreProgressEvent, ScoreStartEvent,
};
use crate::s1::schema::{S1SchemaError, ScoreReport};

/// RFC-pinned S1 score chunk size.
pub const RESET_CONTEXT_CHUNK_SIZE: usize = 128;

const BYTE_VOCAB_SIZE: usize = 256;
const LOG2_E: f64 = std::f64::consts::LOG2_E;

/// A byte-probability provider usable by the S1 reset-context scorer.
///
/// Logits are requested before the current byte is consumed. State is reset by
/// calling [`Self::fresh_state`] at every 128-byte chunk boundary, so the score
/// is a reset-context upper-bound metric rather than full-stream
/// autoregressive bpc.
pub trait ResetContextScorer {
    /// Provider-owned recurrent state.
    type State;

    /// Return a deterministic empty context state.
    fn fresh_state(&self) -> Self::State;

    /// Return one finite logit for each byte value, indexed by `u8`.
    fn logits(&self, state: &Self::State) -> Vec<f64>;

    /// Consume the just-scored byte into the state.
    fn consume(&self, state: &mut Self::State, byte: u8);

    /// Context length observed by the provider, for D7 O-metric-3 fixtures.
    fn context_len(&self, _state: &Self::State) -> Option<usize> {
        None
    }
}

/// Observer hook for score fixture instrumentation.
pub trait ScoreObserver {
    /// Called before each byte is scored.
    fn observe_context_len(&mut self, byte_index: u64, chunk_index: u64, context_len: usize);
}

impl ScoreObserver for () {
    fn observe_context_len(&mut self, _byte_index: u64, _chunk_index: u64, _context_len: usize) {}
}

/// Reset-context bpc primitive result.
#[derive(Debug, Clone, PartialEq)]
pub struct ScoreProduct {
    /// Sum of per-byte negative log probabilities in base 2.
    pub log2_sum: f64,
    /// Number of bytes scored.
    pub token_count: u64,
    /// Bits per character, computed as `log2_sum / token_count`.
    pub bpc: f64,
}

/// Errors from reset-context scoring.
#[derive(Debug)]
pub enum ScoreError {
    /// S1 scoring requires a non-empty validation byte sequence.
    EmptyValidation,
    /// Logits must contain exactly one class for each byte value.
    LogitsWrongLength {
        /// Number of logits returned by the provider.
        len: usize,
        /// Required byte vocabulary size.
        expected: usize,
    },
    /// Logits and computed losses must be finite.
    NonFiniteLogit {
        /// Logit index containing the non-finite value.
        index: usize,
        /// Non-finite value observed while scoring.
        value: f64,
    },
    /// Schema self-hash construction failed.
    Schema(S1SchemaError),
    /// Structured score logging failed validation.
    Logging(LoggingEventError),
}

impl fmt::Display for ScoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyValidation => write!(f, "S1 scoring requires non-empty validation bytes"),
            Self::LogitsWrongLength { len, expected } => {
                write!(
                    f,
                    "logits length {len} does not match byte vocabulary size {expected}"
                )
            }
            Self::NonFiniteLogit { index, value } => {
                write!(f, "non-finite logit at index {index}: {value}")
            }
            Self::Schema(error) => write!(f, "{error}"),
            Self::Logging(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for ScoreError {}

impl From<S1SchemaError> for ScoreError {
    fn from(error: S1SchemaError) -> Self {
        Self::Schema(error)
    }
}

impl From<LoggingEventError> for ScoreError {
    fn from(error: LoggingEventError) -> Self {
        Self::Logging(error)
    }
}

/// Score validation bytes with the RFC-pinned S1 reset-context semantics.
pub fn reset_context_bpc<P>(scorer: &P, val: &[u8]) -> Result<ScoreProduct, ScoreError>
where
    P: ResetContextScorer,
{
    reset_context_bpc_with_observer(scorer, val, &mut ())
}

/// Score validation bytes while recording provider-observed context lengths.
pub fn reset_context_bpc_with_observer<P, O>(
    scorer: &P,
    val: &[u8],
    observer: &mut O,
) -> Result<ScoreProduct, ScoreError>
where
    P: ResetContextScorer,
    O: ScoreObserver,
{
    reset_context_bpc_with_observer_and_progress(scorer, val, observer, |_, _| Ok(()))
}

/// Emit an `s1_score.v1` report from the shared reset-context primitive.
pub fn score<P>(
    scorer: &P,
    seed: u64,
    checkpoint_sha: Hash256,
    corpus_val_sha: Hash256,
    val: &[u8],
) -> Result<ScoreReport, ScoreError>
where
    P: ResetContextScorer,
{
    if val.is_empty() {
        return Err(ScoreError::EmptyValidation);
    }

    let emitter = S1LogEmitter::new();
    let span = emitter.score_span(seed);
    let _guard = span.enter();

    emitter.score_start(ScoreStartEvent {
        seed,
        token_count: val.len() as u64,
    })?;

    let product = reset_context_bpc_with_observer_and_progress(
        scorer,
        val,
        &mut (),
        |chunk_index, scored| {
            emitter.score_progress(ScoreProgressEvent {
                seed,
                chunk_index,
                token_count: scored,
            })?;
            Ok(())
        },
    )?;
    let report = ScoreReport {
        schema: "s1_score.v1".to_owned(),
        seed,
        checkpoint_sha,
        corpus_val_sha,
        chunk_size: RESET_CONTEXT_CHUNK_SIZE as u64,
        token_count: product.token_count,
        log2_sum: product.log2_sum,
        bpc: product.bpc,
        score_self_hash: Hash256::ZERO,
    }
    .with_computed_self_hash()?;

    emitter.score_complete(&ScoreCompleteEvent {
        seed,
        bpc_value: report.bpc,
        token_count: report.token_count,
        score_self_hash: report.score_self_hash.to_string(),
    })?;

    Ok(report)
}

fn reset_context_bpc_with_observer_and_progress<P, O>(
    scorer: &P,
    val: &[u8],
    observer: &mut O,
    mut progress: impl FnMut(u64, u64) -> Result<(), ScoreError>,
) -> Result<ScoreProduct, ScoreError>
where
    P: ResetContextScorer,
    O: ScoreObserver,
{
    if val.is_empty() {
        return Err(ScoreError::EmptyValidation);
    }

    let mut log2_sum = 0.0_f64;
    let mut byte_index = 0_u64;

    for (chunk_index, chunk) in val.chunks(RESET_CONTEXT_CHUNK_SIZE).enumerate() {
        let chunk_index = chunk_index as u64;
        let mut state = scorer.fresh_state();
        for &byte in chunk {
            if let Some(context_len) = scorer.context_len(&state) {
                observer.observe_context_len(byte_index, chunk_index, context_len);
            }

            let logits = scorer.logits(&state);
            log2_sum += negative_log2_probability(&logits, byte)?;
            scorer.consume(&mut state, byte);
            byte_index += 1;
        }
        progress(chunk_index, byte_index)?;
    }

    let token_count = val.len() as u64;
    let bpc = log2_sum / token_count as f64;
    if !log2_sum.is_finite() {
        return Err(ScoreError::NonFiniteLogit {
            index: usize::MAX,
            value: log2_sum,
        });
    }
    debug_assert!(bpc.is_finite());

    Ok(ScoreProduct {
        log2_sum,
        token_count,
        bpc,
    })
}

fn negative_log2_probability(logits: &[f64], target: u8) -> Result<f64, ScoreError> {
    let target_index = usize::from(target);
    if logits.len() != BYTE_VOCAB_SIZE {
        return Err(ScoreError::LogitsWrongLength {
            len: logits.len(),
            expected: BYTE_VOCAB_SIZE,
        });
    }
    for (index, value) in logits.iter().copied().enumerate() {
        if !value.is_finite() {
            return Err(ScoreError::NonFiniteLogit { index, value });
        }
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
    let log_sum_exp = max_logit + exp_sum.ln();
    let loss_nats = log_sum_exp - logits[target_index];
    let loss_log2 = loss_nats * LOG2_E;

    if loss_log2.is_finite() {
        Ok(loss_log2)
    } else {
        Err(ScoreError::NonFiniteLogit {
            index: target_index,
            value: loss_log2,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_log_softmax_handles_tiny_target_probability() {
        let mut logits = vec![0.0; BYTE_VOCAB_SIZE];
        logits[7] = -1.0e6;

        let loss = negative_log2_probability(&logits, 7).expect("finite loss");

        assert!(loss.is_finite());
        assert!(loss > 1.0e6);
    }
}
