//! Oracle integration for S1 verdicts.

use gbf_foundation::Hash256;

use crate::s1::baseline::{
    ADD_ALPHA, BYTE_VOCAB_SIZE, BaselineOrder, LAMBDA_1, LAMBDA_2, LAMBDA_3, NgramBaseline,
};
use crate::s1::logging::{
    LoggingEventError, OracleAggregateCompleteEvent, OracleCompleteEvent, OracleFailedEvent,
    OracleStartEvent, S1LogEmitter,
};
use crate::s1::neg_test::{NEGATIVE_TEST_SHUFFLE_SEED, fisher_yates};
use crate::s1::report::HypothesisStatus;
use crate::s1::rng::{S1Rng, uniform_u64_inclusive};
use crate::s1::schema::{OracleReport, S1SchemaError};
use crate::s1::score::{
    RESET_CONTEXT_CHUNK_SIZE, ResetContextScorer, ScoreError, ScoreObserver, reset_context_bpc,
    reset_context_bpc_with_observer,
};

/// Boolean results for the D7 measurement-oracle suite.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetricOracleResults {
    /// O-metric-0 rejection-sampler adversarial result.
    pub o_metric_0: bool,
    /// O-metric-1 uniform-logits scorer result.
    pub o_metric_1: bool,
    /// O-metric-2 hand-counted n-gram fixture result.
    pub o_metric_2: bool,
    /// O-metric-3 reset-boundary spy result.
    pub o_metric_3: bool,
    /// O-metric-4 shuffle permutation and pin result.
    pub o_metric_4: bool,
}

impl MetricOracleResults {
    /// H5 verdict: every D7 oracle must pass.
    #[must_use]
    pub const fn metric_oracle_passed(self) -> bool {
        self.o_metric_0 && self.o_metric_1 && self.o_metric_2 && self.o_metric_3 && self.o_metric_4
    }

    /// IDs of failed oracle checks in D7 order.
    #[must_use]
    pub fn failed_oracle_ids(self) -> Vec<&'static str> {
        [
            ("O-metric-0", self.o_metric_0),
            ("O-metric-1", self.o_metric_1),
            ("O-metric-2", self.o_metric_2),
            ("O-metric-3", self.o_metric_3),
            ("O-metric-4", self.o_metric_4),
        ]
        .into_iter()
        .filter_map(|(id, passed)| (!passed).then_some(id))
        .collect()
    }

    /// Convert the D7 aggregate into the H5 hypothesis status.
    #[must_use]
    pub fn h5_status(self) -> HypothesisStatus {
        if self.metric_oracle_passed() {
            HypothesisStatus::Confirmed
        } else {
            HypothesisStatus::Refuted
        }
    }

    /// Emit the canonical `s1_oracle.v1` artifact.
    pub fn report(self) -> Result<OracleReport, S1SchemaError> {
        OracleReport::from_oracle_bools(
            self.o_metric_0,
            self.o_metric_1,
            self.o_metric_2,
            self.o_metric_3,
            self.o_metric_4,
        )
    }
}

impl OracleReport {
    /// Convert a validated `s1_oracle.v1` artifact into the H5 hypothesis status.
    pub fn h5_status(&self) -> Result<HypothesisStatus, S1SchemaError> {
        self.validate_aggregate_consistency()?;
        if self.metric_oracle_passed {
            Ok(HypothesisStatus::Confirmed)
        } else {
            Ok(HypothesisStatus::Refuted)
        }
    }
}

/// Run the model-free D7 measurement-oracle suite and emit `s1_oracle.v1`.
pub fn run_metric_oracles(
    seed: u64,
    val_bytes: &[u8],
    expected_shuffle_pin: Hash256,
) -> Result<OracleReport, OracleEmitError> {
    let results = MetricOracleResults {
        o_metric_0: o_metric_0_rejection_sampler(),
        o_metric_1: o_metric_1_uniform_logits()?,
        o_metric_2: o_metric_2_hand_counted_ngram()?,
        o_metric_3: o_metric_3_reset_boundary_spy()?,
        o_metric_4: o_metric_4_shuffle_pin(val_bytes, expected_shuffle_pin),
    };
    emit_oracle_report(seed, results)
}

/// Emit S1 oracle structured log events and return the canonical report.
pub fn emit_oracle_report(
    seed: u64,
    results: MetricOracleResults,
) -> Result<OracleReport, OracleEmitError> {
    let emitter = S1LogEmitter::new();
    for (oracle_id, passed) in results.per_oracle() {
        let span = emitter.oracle_span(seed, oracle_id)?;
        let _guard = span.enter();
        emitter.oracle_start(OracleStartEvent { seed, oracle_id })?;
        if passed {
            emitter.oracle_complete(OracleCompleteEvent { seed, oracle_id })?;
        } else {
            emitter.oracle_failed(&OracleFailedEvent {
                seed,
                oracle_id,
                diagnostic: format!("O-metric-{oracle_id} returned false"),
            })?;
        }
    }

    let report = results.report()?;
    emitter.oracle_aggregate_complete(&OracleAggregateCompleteEvent {
        seed,
        metric_oracle_passed: report.metric_oracle_passed,
        failed_oracle_ids: serde_json::to_string(&report.failed_oracle_ids)?,
        oracle_self_hash: report.oracle_self_hash.to_string(),
    })?;
    Ok(report)
}

/// Errors from S1 oracle report emission.
#[derive(Debug)]
pub enum OracleEmitError {
    /// Canonical schema/self-hash construction failed.
    Schema(S1SchemaError),
    /// Structured logging failed validation.
    Logging(LoggingEventError),
    /// Scoring failed while executing an oracle.
    Score(ScoreError),
    /// JSON telemetry serialization failed.
    Json(serde_json::Error),
}

impl std::fmt::Display for OracleEmitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Schema(error) => write!(f, "{error}"),
            Self::Logging(error) => write!(f, "{error}"),
            Self::Score(error) => write!(f, "{error}"),
            Self::Json(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for OracleEmitError {}

impl From<S1SchemaError> for OracleEmitError {
    fn from(error: S1SchemaError) -> Self {
        Self::Schema(error)
    }
}

impl From<LoggingEventError> for OracleEmitError {
    fn from(error: LoggingEventError) -> Self {
        Self::Logging(error)
    }
}

impl From<ScoreError> for OracleEmitError {
    fn from(error: ScoreError) -> Self {
        Self::Score(error)
    }
}

impl From<serde_json::Error> for OracleEmitError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

impl MetricOracleResults {
    fn per_oracle(self) -> [(u8, bool); 5] {
        [
            (0, self.o_metric_0),
            (1, self.o_metric_1),
            (2, self.o_metric_2),
            (3, self.o_metric_3),
            (4, self.o_metric_4),
        ]
    }
}

fn o_metric_0_rejection_sampler() -> bool {
    let rejection_zone_u64 = (u64::MAX / 10) * 10;
    let accepted = 37_u64;
    let mut rng = ScriptedOracleRng::new([rejection_zone_u64, accepted]);
    let draw = uniform_u64_inclusive(&mut rng, 0, 9);
    draw == 7 && rng.is_empty()
}

fn o_metric_1_uniform_logits() -> Result<bool, ScoreError> {
    let product = reset_context_bpc(&UniformScorer, b"measurement oracle")?;
    Ok(approx_eq(product.bpc, 8.0) && product.token_count == 18)
}

fn o_metric_2_hand_counted_ngram() -> Result<bool, OracleEmitError> {
    let corpus = b"ababa";
    let baseline = NgramBaseline::fit(corpus.iter().copied())
        .map_err(|error| OracleEmitError::Schema(S1SchemaError::Custom(error.to_string())))?;
    let p1_a = (3.0 + ADD_ALPHA) / (5.0 + ADD_ALPHA * BYTE_VOCAB_SIZE as f64);
    let p1_b = (2.0 + ADD_ALPHA) / (5.0 + ADD_ALPHA * BYTE_VOCAB_SIZE as f64);
    let p2_b_given_a = (2.0 + ADD_ALPHA) / (2.0 + ADD_ALPHA * BYTE_VOCAB_SIZE as f64);
    let p2_a_given_b = (2.0 + ADD_ALPHA) / (2.0 + ADD_ALPHA * BYTE_VOCAB_SIZE as f64);
    let p3_a_given_ab = (2.0 + ADD_ALPHA) / (2.0 + ADD_ALPHA * BYTE_VOCAB_SIZE as f64);
    let p3_interp_a_after_ab = LAMBDA_3 * p3_a_given_ab + LAMBDA_2 * p2_a_given_b + LAMBDA_1 * p1_a;
    let bpc_trigram_on_aba = baseline
        .bpc(BaselineOrder::Trigram, b"aba")
        .map_err(|error| OracleEmitError::Schema(S1SchemaError::Custom(error.to_string())))?;
    Ok(baseline.train_bytes() == 5
        && baseline.unigram_count(b'a') == 3
        && baseline.unigram_count(b'b') == 2
        && baseline.bigram_count(b'a', b'b') == 2
        && baseline.bigram_count(b'b', b'a') == 2
        && baseline.trigram_count(b'a', b'b', b'a') == 2
        && baseline.trigram_count(b'b', b'a', b'b') == 1
        && approx_eq(baseline.unigram_probability(b'a'), p1_a)
        && approx_eq(baseline.unigram_probability(b'b'), p1_b)
        && approx_eq(baseline.bigram_probability(b'a', b'b'), p2_b_given_a)
        && approx_eq(baseline.bigram_probability(b'b', b'a'), p2_a_given_b)
        && approx_eq(
            baseline.trigram_probability(b'a', b'b', b'a'),
            p3_a_given_ab,
        )
        && approx_eq(
            baseline.probability_for_context(BaselineOrder::Trigram, b"ab", b'a'),
            p3_interp_a_after_ab,
        )
        && approx_eq(bpc_trigram_on_aba, 1.2549134813562748))
}

fn o_metric_3_reset_boundary_spy() -> Result<bool, ScoreError> {
    let mut observer = ContextSpy::default();
    let val = vec![0_u8; RESET_CONTEXT_CHUNK_SIZE + 1];
    reset_context_bpc_with_observer(&UniformScorer, &val, &mut observer)?;
    let expected = (0_usize..128).chain([0]).collect::<Vec<_>>();
    Ok(observer.context_lengths == expected
        && observer.chunk_indexes[..128] == [0_u64; 128]
        && observer.chunk_indexes[128] == 1)
}

fn o_metric_4_shuffle_pin(val_bytes: &[u8], expected_shuffle_pin: Hash256) -> bool {
    if val_bytes.len() < 2 {
        return false;
    }
    let shuffled = fisher_yates(val_bytes, NEGATIVE_TEST_SHUFFLE_SEED);
    same_byte_multiset(val_bytes, &shuffled)
        && shuffled != val_bytes
        && gbf_foundation::sha256(&shuffled) == expected_shuffle_pin
}

fn approx_eq(left: f64, right: f64) -> bool {
    (left - right).abs() <= 1.0e-12
}

fn same_byte_multiset(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut counts = [0_i64; 256];
    for &byte in left {
        counts[usize::from(byte)] += 1;
    }
    for &byte in right {
        counts[usize::from(byte)] -= 1;
    }
    counts.into_iter().all(|count| count == 0)
}

#[derive(Debug, Clone)]
struct ScriptedOracleRng {
    draws: std::collections::VecDeque<u64>,
}

impl ScriptedOracleRng {
    fn new<const N: usize>(draws: [u64; N]) -> Self {
        Self {
            draws: draws.into_iter().collect(),
        }
    }

    fn is_empty(&self) -> bool {
        self.draws.is_empty()
    }
}

impl S1Rng for ScriptedOracleRng {
    fn next_u64(&mut self) -> u64 {
        self.draws
            .pop_front()
            .expect("scripted oracle rng exhausted")
    }

    fn fill_bytes(&mut self, out: &mut [u8]) {
        for chunk in out.chunks_mut(8) {
            let draw = self.next_u64().to_le_bytes();
            chunk.copy_from_slice(&draw[..chunk.len()]);
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct UniformScorer;

impl ResetContextScorer for UniformScorer {
    type State = Vec<u8>;

    fn fresh_state(&self) -> Self::State {
        Vec::new()
    }

    fn logits(&self, _state: &Self::State) -> Vec<f64> {
        vec![0.0; 256]
    }

    fn consume(&self, state: &mut Self::State, byte: u8) {
        state.push(byte);
    }

    fn context_len(&self, state: &Self::State) -> Option<usize> {
        Some(state.len())
    }
}

#[derive(Debug, Default)]
struct ContextSpy {
    context_lengths: Vec<usize>,
    chunk_indexes: Vec<u64>,
}

impl ScoreObserver for ContextSpy {
    fn observe_context_len(&mut self, _byte_index: u64, chunk_index: u64, context_len: usize) {
        self.chunk_indexes.push(chunk_index);
        self.context_lengths.push(context_len);
    }
}
