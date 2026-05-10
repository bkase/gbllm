//! Baseline model and score comparison helpers for S1.

use std::fmt;

use gbf_foundation::{Hash256, sha256};

use crate::s1::logging::{
    BaselineCompleteEvent, BaselineFitCompleteEvent, BaselineFitProgressEvent,
    BaselineFitStartEvent, BaselineScoreCompleteEvent, BaselineScoreStartEvent, LoggingEventError,
    S1LogEmitter,
};
use crate::s1::schema::{BaselineReport, CountsSummary, S1SchemaError, SmoothingScheme};
use crate::s1::score::{RESET_CONTEXT_CHUNK_SIZE, ScoreError};

/// Byte vocabulary size pinned by F-S1.
pub const BYTE_VOCAB_SIZE: usize = 256;
/// Add-alpha smoothing value pinned by F-S1 D4.
pub const ADD_ALPHA: f64 = 0.01;
/// Trigram interpolation weight pinned by F-S1 D4.
pub const LAMBDA_3: f64 = 0.6;
/// Bigram interpolation weight pinned by F-S1 D4.
pub const LAMBDA_2: f64 = 0.3;
/// Unigram interpolation weight pinned by F-S1 D4.
pub const LAMBDA_1: f64 = 0.1;
/// Interpolation lambdas in D4 order `[λ3, λ2, λ1]`.
pub const INTERPOLATION_LAMBDAS: [f64; 3] = [LAMBDA_3, LAMBDA_2, LAMBDA_1];

const COUNTS_BLOB_DOMAIN: &[u8] = b"gbf-experiments.s1.baseline.counts.v1\0";
const PROGRESS_BYTES: u64 = 100 * 1024 * 1024;

/// Fitted raw-byte n-gram counts for the S1 3-gram baseline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NgramBaseline {
    train_bytes: u64,
    unigrams: [u64; BYTE_VOCAB_SIZE],
    bigrams: Vec<u64>,
    bigram_context_totals: [u64; BYTE_VOCAB_SIZE],
    trigrams: Vec<u32>,
    trigram_context_totals: Vec<u64>,
}

/// Reset-context state for S1 n-gram baseline scoring.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BaselineState {
    context: Vec<u8>,
}

/// Which order to score from a fitted baseline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BaselineOrder {
    /// Smoothed unigram only.
    Unigram,
    /// Smoothed bigram with unigram fall-through at chunk starts.
    Bigram,
    /// F-S1 interpolated trigram with position-aware reset semantics.
    Trigram,
}

/// Result of fitting and scoring an S1 baseline.
#[derive(Debug, Clone, PartialEq)]
pub struct BaselineFitProduct {
    /// Fitted counts.
    pub baseline: NgramBaseline,
    /// Emitted `s1_baseline.v1` report.
    pub report: BaselineReport,
}

/// Errors from S1 baseline fitting and report emission.
#[derive(Debug)]
pub enum BaselineError {
    /// F-S1 baseline fitting requires a non-empty training corpus.
    EmptyTrainingCorpus,
    /// Validation scoring failed.
    Score(ScoreError),
    /// Schema self-hash construction failed.
    Schema(S1SchemaError),
    /// Structured baseline logging failed validation.
    Logging(LoggingEventError),
}

impl fmt::Display for BaselineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyTrainingCorpus => write!(f, "S1 baseline requires non-empty training bytes"),
            Self::Score(error) => write!(f, "{error}"),
            Self::Schema(error) => write!(f, "{error}"),
            Self::Logging(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for BaselineError {}

impl From<ScoreError> for BaselineError {
    fn from(error: ScoreError) -> Self {
        Self::Score(error)
    }
}

impl From<S1SchemaError> for BaselineError {
    fn from(error: S1SchemaError) -> Self {
        Self::Schema(error)
    }
}

impl From<LoggingEventError> for BaselineError {
    fn from(error: LoggingEventError) -> Self {
        Self::Logging(error)
    }
}

impl NgramBaseline {
    /// Fit P1/P2/P3 raw-byte counts from a streaming byte iterator.
    pub fn fit<I>(bytes: I) -> Result<Self, BaselineError>
    where
        I: IntoIterator<Item = u8>,
    {
        fit_with_progress(bytes, |_, _| Ok(()))
    }

    /// Number of bytes consumed while fitting.
    #[must_use]
    pub const fn train_bytes(&self) -> u64 {
        self.train_bytes
    }

    /// Raw P1 count for `c`.
    #[must_use]
    pub fn unigram_count(&self, c: u8) -> u64 {
        self.unigrams[usize::from(c)]
    }

    /// Raw P2 count for `(a, c)`.
    #[must_use]
    pub fn bigram_count(&self, a: u8, c: u8) -> u64 {
        self.bigrams[bigram_index(a, c)]
    }

    /// Raw P3 count for `((a, b), c)`.
    #[must_use]
    pub fn trigram_count(&self, a: u8, b: u8, c: u8) -> u32 {
        self.trigrams[trigram_index(a, b, c)]
    }

    /// S1 `counts_summary`.
    #[must_use]
    pub fn counts_summary(&self) -> CountsSummary {
        CountsSummary {
            train_bytes: self.train_bytes,
            distinct_unigrams: self.unigrams.iter().filter(|&&count| count != 0).count() as u64,
            distinct_bigrams: self.bigrams.iter().filter(|&&count| count != 0).count() as u64,
            distinct_trigrams: self.trigrams.iter().filter(|&&count| count != 0).count() as u64,
        }
    }

    /// Deterministic hash of the fitted counts blob.
    #[must_use]
    pub fn counts_blob_sha256(&self) -> Hash256 {
        sha256(self.counts_blob_bytes())
    }

    /// Smoothed unigram probability.
    #[must_use]
    pub fn unigram_probability(&self, c: u8) -> f64 {
        smoothed(self.unigram_count(c), self.train_bytes)
    }

    /// Smoothed bigram probability.
    #[must_use]
    pub fn bigram_probability(&self, a: u8, c: u8) -> f64 {
        smoothed(
            self.bigram_count(a, c),
            self.bigram_context_totals[usize::from(a)],
        )
    }

    /// Smoothed trigram probability.
    #[must_use]
    pub fn trigram_probability(&self, a: u8, b: u8, c: u8) -> f64 {
        smoothed(
            u64::from(self.trigram_count(a, b, c)),
            self.trigram_context_totals[trigram_context_index(a, b)],
        )
    }

    /// Position-aware interpolated trigram probability.
    #[must_use]
    pub fn probability_for_context(&self, order: BaselineOrder, context: &[u8], c: u8) -> f64 {
        let p1 = self.unigram_probability(c);
        match order {
            BaselineOrder::Unigram => p1,
            BaselineOrder::Bigram => context
                .last()
                .map_or(p1, |&a| self.bigram_probability(a, c)),
            BaselineOrder::Trigram => match context {
                [] => p1,
                [.., a, b] => {
                    LAMBDA_3 * self.trigram_probability(*a, *b, c)
                        + LAMBDA_2 * self.bigram_probability(*b, c)
                        + LAMBDA_1 * p1
                }
                [.., a] => (LAMBDA_3 + LAMBDA_2) * self.bigram_probability(*a, c) + LAMBDA_1 * p1,
            },
        }
    }

    /// Sum `P(c | context)` over the byte vocabulary.
    #[must_use]
    pub fn probability_mass_for_context(&self, order: BaselineOrder, context: &[u8]) -> f64 {
        (0_u8..=u8::MAX)
            .map(|c| self.probability_for_context(order, context, c))
            .sum()
    }

    /// Score validation bytes with reset-context semantics for one baseline order.
    pub fn bpc(&self, order: BaselineOrder, val: &[u8]) -> Result<f64, ScoreError> {
        if val.is_empty() {
            return Err(ScoreError::EmptyValidation);
        }

        let mut log2_sum = 0.0_f64;
        for chunk in val.chunks(RESET_CONTEXT_CHUNK_SIZE) {
            let mut context = [0_u8; 2];
            let mut context_len = 0_usize;
            for &byte in chunk {
                let context_slice = &context[..context_len];
                let probability = self.probability_for_context(order, context_slice, byte);
                if !probability.is_finite() || probability <= 0.0 {
                    return Err(ScoreError::NonFiniteLogit {
                        index: usize::from(byte),
                        value: probability,
                    });
                }
                log2_sum -= probability.log2();

                if context_len < 2 {
                    context[context_len] = byte;
                    context_len += 1;
                } else {
                    context = [context[1], byte];
                }
            }
        }

        if !log2_sum.is_finite() {
            return Err(ScoreError::NonFiniteLogit {
                index: usize::MAX,
                value: log2_sum,
            });
        }

        Ok(log2_sum / val.len() as f64)
    }

    fn counts_blob_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(
            COUNTS_BLOB_DOMAIN.len()
                + 8
                + BYTE_VOCAB_SIZE * 8
                + BYTE_VOCAB_SIZE * BYTE_VOCAB_SIZE * 8
                + BYTE_VOCAB_SIZE * 8
                + BYTE_VOCAB_SIZE * BYTE_VOCAB_SIZE * BYTE_VOCAB_SIZE * 4
                + BYTE_VOCAB_SIZE * BYTE_VOCAB_SIZE * 8,
        );
        bytes.extend_from_slice(COUNTS_BLOB_DOMAIN);
        bytes.extend_from_slice(&self.train_bytes.to_le_bytes());
        for count in self.unigrams {
            bytes.extend_from_slice(&count.to_le_bytes());
        }
        for count in &self.bigrams {
            bytes.extend_from_slice(&count.to_le_bytes());
        }
        for count in self.bigram_context_totals {
            bytes.extend_from_slice(&count.to_le_bytes());
        }
        for count in &self.trigrams {
            bytes.extend_from_slice(&count.to_le_bytes());
        }
        for count in &self.trigram_context_totals {
            bytes.extend_from_slice(&count.to_le_bytes());
        }
        bytes
    }
}

/// Fit, score, log, and emit an `s1_baseline.v1` report.
pub fn fit_baseline_report(
    seed: u64,
    corpus_train_sha: Hash256,
    corpus_val_sha: Hash256,
    train: &[u8],
    val: &[u8],
) -> Result<BaselineFitProduct, BaselineError> {
    let emitter = S1LogEmitter::new();
    let fit_span = emitter.baseline_fit_span(seed);
    let fit_guard = fit_span.enter();

    emitter.baseline_fit_start(&BaselineFitStartEvent {
        seed,
        corpus_train_sha: corpus_train_sha.to_string(),
        train_bytes: train.len() as u64,
    })?;
    let baseline = fit_with_progress(train.iter().copied(), |bytes_done, train_bytes| {
        emitter.baseline_fit_progress(BaselineFitProgressEvent {
            seed,
            bytes_done,
            train_bytes,
        })?;
        Ok(())
    })?;
    drop(fit_guard);

    let score_span = emitter.baseline_score_span(seed);
    let _score_guard = score_span.enter();
    emitter.baseline_score_start(BaselineScoreStartEvent {
        seed,
        token_count: val.len() as u64,
    })?;

    let bpc_unigram = baseline.bpc(BaselineOrder::Unigram, val)?;
    let bpc_2gram = baseline.bpc(BaselineOrder::Bigram, val)?;
    let bpc_3gram = baseline.bpc(BaselineOrder::Trigram, val)?;

    let report = BaselineReport {
        schema: "s1_baseline.v1".to_owned(),
        corpus_train_sha,
        corpus_val_sha,
        smoothing: SmoothingScheme {
            alpha: ADD_ALPHA,
            lambdas: INTERPOLATION_LAMBDAS,
        },
        bpc_3gram,
        bpc_2gram,
        bpc_unigram,
        counts_summary: baseline.counts_summary(),
        counts_blob_sha256: baseline.counts_blob_sha256(),
        baseline_self_hash: Hash256::ZERO,
    }
    .with_computed_self_hash()?;

    emitter.baseline_score_complete(&BaselineScoreCompleteEvent {
        seed,
        token_count: val.len() as u64,
        baseline_self_hash: report.baseline_self_hash.to_string(),
    })?;
    emitter.baseline_fit_complete(&BaselineFitCompleteEvent {
        seed,
        bpc_3gram,
        bpc_2gram,
        bpc_unigram,
        counts_blob_sha256: report.counts_blob_sha256.to_string(),
        counts_summary: format!(
            "train_bytes={},distinct_unigrams={},distinct_bigrams={},distinct_trigrams={}",
            report.counts_summary.train_bytes,
            report.counts_summary.distinct_unigrams,
            report.counts_summary.distinct_bigrams,
            report.counts_summary.distinct_trigrams
        ),
        baseline_self_hash: report.baseline_self_hash.to_string(),
    })?;
    emitter.baseline_complete(BaselineCompleteEvent {
        seed,
        bpc_3gram,
        bpc_2gram,
        bpc_unigram,
    })?;

    Ok(BaselineFitProduct { baseline, report })
}

fn fit_with_progress<I>(
    bytes: I,
    mut progress: impl FnMut(u64, u64) -> Result<(), BaselineError>,
) -> Result<NgramBaseline, BaselineError>
where
    I: IntoIterator<Item = u8>,
{
    let mut unigrams = [0_u64; BYTE_VOCAB_SIZE];
    let mut bigrams = vec![0_u64; BYTE_VOCAB_SIZE * BYTE_VOCAB_SIZE];
    let mut bigram_context_totals = [0_u64; BYTE_VOCAB_SIZE];
    let mut trigrams = vec![0_u32; BYTE_VOCAB_SIZE * BYTE_VOCAB_SIZE * BYTE_VOCAB_SIZE];
    let mut trigram_context_totals = vec![0_u64; BYTE_VOCAB_SIZE * BYTE_VOCAB_SIZE];
    let mut train_bytes = 0_u64;
    let mut previous = [0_u8; 2];
    let mut previous_len = 0_usize;
    let mut next_progress = PROGRESS_BYTES;

    for byte in bytes {
        unigrams[usize::from(byte)] += 1;
        if previous_len >= 1 {
            let context = previous[previous_len - 1];
            bigrams[bigram_index(context, byte)] += 1;
            bigram_context_totals[usize::from(context)] += 1;
        }
        if previous_len >= 2 {
            let index = trigram_index(previous[0], previous[1], byte);
            trigrams[index] = trigrams[index]
                .checked_add(1)
                .expect("S1 trigram count exceeds u32 capacity");
            trigram_context_totals[trigram_context_index(previous[0], previous[1])] += 1;
        }

        if previous_len < 2 {
            previous[previous_len] = byte;
            previous_len += 1;
        } else {
            previous = [previous[1], byte];
        }

        train_bytes += 1;
        if train_bytes >= next_progress {
            progress(train_bytes, train_bytes)?;
            next_progress += PROGRESS_BYTES;
        }
    }

    if train_bytes == 0 {
        return Err(BaselineError::EmptyTrainingCorpus);
    }

    Ok(NgramBaseline {
        train_bytes,
        unigrams,
        bigrams,
        bigram_context_totals,
        trigrams,
        trigram_context_totals,
    })
}

fn smoothed(count: u64, context_count: u64) -> f64 {
    (count as f64 + ADD_ALPHA) / (context_count as f64 + ADD_ALPHA * BYTE_VOCAB_SIZE as f64)
}

const fn bigram_index(a: u8, c: u8) -> usize {
    (a as usize) * BYTE_VOCAB_SIZE + c as usize
}

const fn trigram_index(a: u8, b: u8, c: u8) -> usize {
    ((a as usize) * BYTE_VOCAB_SIZE + b as usize) * BYTE_VOCAB_SIZE + c as usize
}

const fn trigram_context_index(a: u8, b: u8) -> usize {
    (a as usize) * BYTE_VOCAB_SIZE + b as usize
}
