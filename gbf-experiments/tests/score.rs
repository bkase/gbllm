mod common;

use common::tracing_capture::{TraceCapture, captured_events, with_trace_capture};
use gbf_experiments::s1::logging::{event, field};
use gbf_experiments::s1::score::{
    RESET_CONTEXT_CHUNK_SIZE, ResetContextScorer, ScoreError, ScoreObserver, reset_context_bpc,
    reset_context_bpc_with_observer, score,
};
use gbf_foundation::Hash256;
use proptest::prelude::*;
use serde_json::json;

#[test]
fn reset_context_bpc_rejects_empty_validation() {
    assert!(matches!(
        reset_context_bpc(&UniformScorer, b""),
        Err(ScoreError::EmptyValidation)
    ));
}

#[test]
fn single_byte_scores_from_empty_context() {
    let product = reset_context_bpc(&UniformScorer, &[b'a']).expect("score");

    assert_eq!(product.token_count, 1);
    assert_eq!(product.log2_sum, 8.0);
    assert_eq!(product.bpc, 8.0);
}

#[test]
fn chunk_128_uses_one_reset_context_window() {
    let mut observer = ContextSpy::default();
    let val = vec![0_u8; RESET_CONTEXT_CHUNK_SIZE];

    let product =
        reset_context_bpc_with_observer(&UniformScorer, &val, &mut observer).expect("score");

    assert_eq!(product.token_count, 128);
    assert_eq!(observer.context_lengths, (0_usize..128).collect::<Vec<_>>());
    assert_eq!(observer.chunk_indexes, vec![0_u64; 128]);
}

#[test]
fn chunk_129_resets_second_chunk_first_byte() {
    let mut observer = ContextSpy::default();
    let val = vec![0_u8; RESET_CONTEXT_CHUNK_SIZE + 1];

    reset_context_bpc_with_observer(&UniformScorer, &val, &mut observer).expect("score");

    let expected = (0_usize..128).chain([0]).collect::<Vec<_>>();
    assert_eq!(observer.context_lengths, expected);
    assert_eq!(observer.context_lengths[128], 0);
    assert_eq!(observer.chunk_indexes[128], 1);
}

#[test]
fn chunk_256_consumes_two_full_chunks() {
    let mut observer = ContextSpy::default();
    let val = vec![0_u8; RESET_CONTEXT_CHUNK_SIZE * 2];

    let product =
        reset_context_bpc_with_observer(&UniformScorer, &val, &mut observer).expect("score");

    let one_chunk = (0_usize..128).collect::<Vec<_>>();
    let expected = one_chunk
        .iter()
        .copied()
        .chain(one_chunk.iter().copied())
        .collect::<Vec<_>>();
    assert_eq!(product.token_count, 256);
    assert_eq!(observer.context_lengths, expected);
    assert_eq!(observer.chunk_indexes[..128], [0_u64; 128]);
    assert_eq!(observer.chunk_indexes[128..], [1_u64; 128]);
}

#[test]
fn stable_log_softmax_keeps_extreme_losses_finite() {
    let product = reset_context_bpc(&ExtremeTargetScorer { target: 42 }, &[42]).expect("score");

    assert!(product.log2_sum.is_finite());
    assert!(product.bpc.is_finite());
    assert!(product.bpc > 1.0e6);
}

#[test]
fn rejects_short_logits_before_scoring_byte_vocab() {
    assert!(matches!(
        reset_context_bpc(&FixedLenScorer { len: 255 }, &[255]),
        Err(ScoreError::LogitsWrongLength {
            len: 255,
            expected: 256
        })
    ));
}

#[test]
fn rejects_overlong_logits_before_phantom_classes_affect_bpc() {
    assert!(matches!(
        reset_context_bpc(&FixedLenScorer { len: 257 }, &[7]),
        Err(ScoreError::LogitsWrongLength {
            len: 257,
            expected: 256
        })
    ));
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 128,
        failure_persistence: None,
        .. ProptestConfig::default()
    })]

    #[test]
    fn byte_vocab_contract_rejects_every_non_256_logit_length(
        len in prop_oneof![0_usize..256, 257_usize..512],
        target in any::<u8>(),
    ) {
        match reset_context_bpc(&FixedLenScorer { len }, &[target]) {
            Err(ScoreError::LogitsWrongLength { len: actual, expected }) => {
                prop_assert_eq!(actual, len);
                prop_assert_eq!(expected, 256);
            }
            other => prop_assert!(false, "unexpected result: {other:?}"),
        }
    }
}

#[test]
fn score_report_emission_is_deterministic_and_self_hashed() {
    let val = vec![7_u8; 129];
    let first = score(&UniformScorer, 3, hash(1), hash(2), &val).expect("score report");
    let second = score(&UniformScorer, 3, hash(1), hash(2), &val).expect("score report");

    assert_eq!(first.chunk_size, 128);
    assert_eq!(first.token_count, 129);
    assert_eq!(
        first.score_self_hash,
        first.computed_self_hash().expect("self hash")
    );
    assert_eq!(
        first.canonical_json_bytes().expect("first canonical JSON"),
        second
            .canonical_json_bytes()
            .expect("second canonical JSON")
    );
    assert_eq!(first, second);
}

#[test]
fn score_emits_subscriber_captured_start_progress_and_completion_events() {
    let capture = TraceCapture::default();
    let val = vec![7_u8; 129];

    let report = with_trace_capture(&capture, || {
        score(&UniformScorer, 3, hash(1), hash(2), &val).expect("score report")
    });

    let events = captured_events(&capture);
    let score_events = events
        .iter()
        .filter(|event| {
            matches!(
                event.name.as_str(),
                event::SCORE_START | event::SCORE_PROGRESS | event::SCORE_COMPLETE
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(score_events.len(), 4, "score events: {score_events:?}");

    assert_eq!(score_events[0].name, event::SCORE_START);
    assert_eq!(
        score_events[0].fields.get(field::GBF_LOG_SCHEMA_VERSION),
        Some(&json!("1.0.0"))
    );
    assert_eq!(score_events[0].fields.get(field::SEED), Some(&json!(3)));
    assert_eq!(
        score_events[0].fields.get(field::TOKEN_COUNT),
        Some(&json!(129))
    );

    assert_eq!(score_events[1].name, event::SCORE_PROGRESS);
    assert_eq!(score_events[1].fields.get(field::SEED), Some(&json!(3)));
    assert_eq!(
        score_events[1].fields.get(field::CHUNK_INDEX),
        Some(&json!(0))
    );
    assert_eq!(
        score_events[1].fields.get(field::TOKEN_COUNT),
        Some(&json!(128))
    );

    assert_eq!(score_events[2].name, event::SCORE_PROGRESS);
    assert_eq!(score_events[2].fields.get(field::SEED), Some(&json!(3)));
    assert_eq!(
        score_events[2].fields.get(field::CHUNK_INDEX),
        Some(&json!(1))
    );
    assert_eq!(
        score_events[2].fields.get(field::TOKEN_COUNT),
        Some(&json!(129))
    );

    let event = score_events
        .iter()
        .find(|event| event.name == event::SCORE_COMPLETE)
        .expect("score completion event");

    assert_eq!(
        event.fields.get(field::GBF_LOG_SCHEMA_VERSION),
        Some(&json!("1.0.0"))
    );
    assert_eq!(event.fields.get(field::SEED), Some(&json!(3)));
    assert_eq!(event.fields.get(field::BPC_VALUE), Some(&json!(8.0)));
    assert_eq!(event.fields.get(field::TOKEN_COUNT), Some(&json!(129)));
    assert_eq!(
        event.fields.get(field::SCORE_SELF_HASH),
        Some(&json!(report.score_self_hash.to_string()))
    );
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

#[derive(Debug, Clone, Copy)]
struct ExtremeTargetScorer {
    target: u8,
}

impl ResetContextScorer for ExtremeTargetScorer {
    type State = ();

    fn fresh_state(&self) -> Self::State {}

    fn logits(&self, _state: &Self::State) -> Vec<f64> {
        let mut logits = vec![0.0; 256];
        logits[usize::from(self.target)] = -1.0e6;
        logits
    }

    fn consume(&self, _state: &mut Self::State, _byte: u8) {}
}

#[derive(Debug, Clone, Copy)]
struct FixedLenScorer {
    len: usize,
}

impl ResetContextScorer for FixedLenScorer {
    type State = ();

    fn fresh_state(&self) -> Self::State {}

    fn logits(&self, _state: &Self::State) -> Vec<f64> {
        vec![0.0; self.len]
    }

    fn consume(&self, _state: &mut Self::State, _byte: u8) {}
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

fn hash(byte: u8) -> Hash256 {
    Hash256::from_bytes([byte; 32])
}
