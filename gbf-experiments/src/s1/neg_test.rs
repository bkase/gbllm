//! Negative-test and falsification harness plumbing for S1.

use std::error::Error;
use std::fmt;

use gbf_foundation::{Hash256, sha256};

use crate::s1::logging::{
    LoggingEventError, NegTestCompleteEvent, NegTestScoreCompleteEvent, NegTestScoreStartEvent,
    NegTestShuffleCompleteEvent, NegTestShufflePinMismatchEvent, NegTestShuffleStartEvent,
    S1LogEmitter,
};
use crate::s1::rng::{ShuffleRng, uniform_u64_inclusive};
use crate::s1::schema::{NegativeTestReport, S1SchemaError};
use crate::s1::score::{ResetContextScorer, ScoreError, reset_context_bpc};

/// Fisher-Yates seed pinned for `s1_negative_test.v1`.
pub const NEGATIVE_TEST_SHUFFLE_SEED: u64 = 0xDEAD_BEEF;

/// H3 sensitivity threshold in bits per character.
pub const NEGATIVE_TEST_SENSITIVE_DELTA_THRESHOLD: f64 = 2.0;

const NEGATIVE_TEST_DELTA_DRIFT_ULPS: f64 = 64.0;

/// Shuffle `bytes` using the RFC-pinned high-to-low Fisher-Yates loop.
///
/// The integer draw is [`uniform_u64_inclusive`] over [`ShuffleRng`], so the
/// production path uses rejection sampling rather than modulo reduction. This
/// routine is intentionally public because F-S1.24's F6 falsification variant
/// swaps only the draw algorithm while preserving the surrounding shuffle
/// shape.
#[must_use]
pub fn fisher_yates(bytes: &[u8], rng_seed: u64) -> Vec<u8> {
    let mut shuffled = bytes.to_vec();
    let mut rng = ShuffleRng::new(rng_seed);

    for i in (1..shuffled.len()).rev() {
        let j = uniform_u64_inclusive(&mut rng, 0, i as u64) as usize;
        shuffled.swap(i, j);
    }

    debug_assert!(same_multiset(bytes, &shuffled));
    shuffled
}

/// Run the seed-0 negative test against an existing reset-context scorer.
///
/// This bead does not deserialize a production checkpoint into a scorer. The
/// caller supplies the already-constructed scorer and checkpoint hash, keeping
/// fixture scorers and future production checkpoint scorers explicit at the
/// boundary.
pub fn run_negative_test<P>(
    scorer: &P,
    seed: u64,
    checkpoint_sha: Hash256,
    corpus_val_sha: Hash256,
    expected_shuffled_val_sha256: Hash256,
    val: &[u8],
) -> Result<NegativeTestReport, NegativeTestError>
where
    P: ResetContextScorer,
{
    let emitter = S1LogEmitter::new();
    let span = emitter.neg_test_span(seed);
    let _guard = span.enter();

    emitter.neg_test_shuffle_start(NegTestShuffleStartEvent {
        seed,
        shuffle_seed: NEGATIVE_TEST_SHUFFLE_SEED,
        token_count: val.len() as u64,
    })?;
    let shuffled = fisher_yates(val, NEGATIVE_TEST_SHUFFLE_SEED);
    let shuffled_val_sha256 = sha256(&shuffled);
    emitter.neg_test_shuffle_complete(&NegTestShuffleCompleteEvent {
        seed,
        shuffle_seed: NEGATIVE_TEST_SHUFFLE_SEED,
        token_count: val.len() as u64,
        shuffled_val_sha256: shuffled_val_sha256.to_string(),
    })?;
    if shuffled_val_sha256 != expected_shuffled_val_sha256 {
        emitter.neg_test_shuffle_pin_mismatch(&NegTestShufflePinMismatchEvent {
            seed,
            expected: expected_shuffled_val_sha256.to_string(),
            observed: shuffled_val_sha256.to_string(),
        })?;
        return Err(NegativeTestError::ShufflePinMismatch {
            expected: expected_shuffled_val_sha256,
            observed: shuffled_val_sha256,
        });
    }
    validate_shuffle_multiset(val, &shuffled)?;

    emitter.neg_test_score_start(NegTestScoreStartEvent {
        seed,
        token_count: val.len() as u64,
    })?;
    let original = reset_context_bpc(scorer, val)?;
    let shuffled_score = reset_context_bpc(scorer, &shuffled)?;
    emitter.neg_test_score_complete(NegTestScoreCompleteEvent {
        seed,
        bpc_original: original.bpc,
        bpc_shuffled: shuffled_score.bpc,
    })?;

    let report = negative_test_report_from_bpcs(
        seed,
        checkpoint_sha,
        corpus_val_sha,
        shuffled_val_sha256,
        original.bpc,
        shuffled_score.bpc,
    )?;
    emitter.neg_test_complete(&NegTestCompleteEvent {
        seed,
        bpc_original: report.bpc_original,
        bpc_shuffled: report.bpc_shuffled,
        delta: report.delta,
        sensitive: report.sensitive,
        negative_self_hash: report.negative_self_hash.to_string(),
    })?;

    Ok(report)
}

/// Build a self-hashed negative-test report from already computed BPC values.
///
/// This helper is intentionally small and checked so tests and future CLI/report
/// code can exercise the non-finite and negative-delta contracts without
/// manufacturing impossible scorer states.
pub fn negative_test_report_from_bpcs(
    seed: u64,
    checkpoint_sha: Hash256,
    corpus_val_sha: Hash256,
    shuffled_val_sha256: Hash256,
    bpc_original: f64,
    bpc_shuffled: f64,
) -> Result<NegativeTestReport, NegativeTestError> {
    validate_bpc("bpc_original", bpc_original)?;
    validate_bpc("bpc_shuffled", bpc_shuffled)?;

    let mut delta = bpc_shuffled - bpc_original;
    if !delta.is_finite() {
        return Err(NegativeTestError::NonFiniteDelta {
            bpc_original,
            bpc_shuffled,
        });
    }
    if delta < 0.0 {
        if delta >= -negative_delta_drift_tolerance(bpc_original, bpc_shuffled) {
            delta = 0.0;
        } else {
            return Err(NegativeTestError::NegativeDelta {
                bpc_original,
                bpc_shuffled,
                delta,
            });
        }
    }

    Ok(NegativeTestReport {
        schema: "s1_negative_test.v1".to_owned(),
        seed,
        checkpoint_sha,
        corpus_val_sha,
        shuffle_seed: NEGATIVE_TEST_SHUFFLE_SEED,
        bpc_original,
        bpc_shuffled,
        shuffled_val_sha256,
        delta,
        sensitive: delta > NEGATIVE_TEST_SENSITIVE_DELTA_THRESHOLD,
        negative_self_hash: Hash256::ZERO,
    }
    .with_computed_self_hash()?)
}

/// Return whether two byte slices contain the same byte multiset.
#[must_use]
pub fn same_multiset(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }

    let mut counts = [0_i64; 256];
    for byte in left {
        counts[usize::from(*byte)] += 1;
    }
    for byte in right {
        counts[usize::from(*byte)] -= 1;
    }
    counts.iter().all(|count| *count == 0)
}

/// Validate the defensive shuffle invariant separately from the production
/// Fisher-Yates implementation so tests can exercise the typed error branch.
pub fn validate_shuffle_multiset(
    original: &[u8],
    shuffled: &[u8],
) -> Result<(), NegativeTestError> {
    if same_multiset(original, shuffled) {
        Ok(())
    } else {
        Err(NegativeTestError::ShuffleMultisetMismatch)
    }
}

fn validate_bpc(name: &'static str, value: f64) -> Result<(), NegativeTestError> {
    if !value.is_finite() {
        return Err(NegativeTestError::NonFiniteBpc { name, value });
    }
    if value < 0.0 {
        return Err(NegativeTestError::NegativeBpc { name, value });
    }
    Ok(())
}

fn negative_delta_drift_tolerance(bpc_original: f64, bpc_shuffled: f64) -> f64 {
    f64::EPSILON
        * bpc_original.abs().max(bpc_shuffled.abs()).max(1.0)
        * NEGATIVE_TEST_DELTA_DRIFT_ULPS
}

/// Errors returned by the S1 negative-test producer.
#[derive(Debug)]
pub enum NegativeTestError {
    /// Reset-context scoring failed.
    Score(ScoreError),
    /// The computed shuffle hash did not match the manifest pin.
    ShufflePinMismatch {
        /// Manifest-pinned hash.
        expected: Hash256,
        /// Hash of the shuffled validation bytes.
        observed: Hash256,
    },
    /// The shuffle did not preserve the input byte multiset.
    ShuffleMultisetMismatch,
    /// A BPC input was not finite.
    NonFiniteBpc {
        /// Field name.
        name: &'static str,
        /// Rejected value.
        value: f64,
    },
    /// A BPC input was negative.
    NegativeBpc {
        /// Field name.
        name: &'static str,
        /// Rejected value.
        value: f64,
    },
    /// `bpc_shuffled - bpc_original` was not finite.
    NonFiniteDelta {
        /// Original validation BPC.
        bpc_original: f64,
        /// Shuffled validation BPC.
        bpc_shuffled: f64,
    },
    /// `bpc_shuffled - bpc_original` was negative.
    NegativeDelta {
        /// Original validation BPC.
        bpc_original: f64,
        /// Shuffled validation BPC.
        bpc_shuffled: f64,
        /// Negative delta.
        delta: f64,
    },
    /// Canonical schema/self-hash construction failed.
    Schema(S1SchemaError),
    /// Structured negative-test logging failed validation.
    Logging(LoggingEventError),
}

impl fmt::Display for NegativeTestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Score(error) => write!(f, "{error}"),
            Self::ShufflePinMismatch { expected, observed } => write!(
                f,
                "F-S1 negative-test shuffle pin mismatch: expected {expected}, observed {observed}"
            ),
            Self::ShuffleMultisetMismatch => {
                f.write_str("F-S1 negative-test shuffle did not preserve byte multiset")
            }
            Self::NonFiniteBpc { name, value } => {
                write!(f, "negative-test {name} must be finite, got {value}")
            }
            Self::NegativeBpc { name, value } => {
                write!(f, "negative-test {name} must be non-negative, got {value}")
            }
            Self::NonFiniteDelta {
                bpc_original,
                bpc_shuffled,
            } => write!(
                f,
                "negative-test delta must be finite: shuffled {bpc_shuffled} - original {bpc_original}"
            ),
            Self::NegativeDelta {
                bpc_original,
                bpc_shuffled,
                delta,
            } => write!(
                f,
                "negative-test delta must be non-negative: shuffled {bpc_shuffled} - original {bpc_original} = {delta}"
            ),
            Self::Schema(error) => write!(f, "{error}"),
            Self::Logging(error) => write!(f, "{error}"),
        }
    }
}

impl Error for NegativeTestError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Score(error) => Some(error),
            Self::Schema(error) => Some(error),
            Self::Logging(error) => Some(error),
            Self::ShufflePinMismatch { .. }
            | Self::ShuffleMultisetMismatch
            | Self::NonFiniteBpc { .. }
            | Self::NegativeBpc { .. }
            | Self::NonFiniteDelta { .. }
            | Self::NegativeDelta { .. } => None,
        }
    }
}

impl From<ScoreError> for NegativeTestError {
    fn from(error: ScoreError) -> Self {
        Self::Score(error)
    }
}

impl From<S1SchemaError> for NegativeTestError {
    fn from(error: S1SchemaError) -> Self {
        Self::Schema(error)
    }
}

impl From<LoggingEventError> for NegativeTestError {
    fn from(error: LoggingEventError) -> Self {
        Self::Logging(error)
    }
}
