use gbf_experiments::s1::logging::{event, field, span};
use gbf_experiments::s1::neg_test::{
    NEGATIVE_TEST_SHUFFLE_SEED, NegativeTestError, fisher_yates, negative_test_report_from_bpcs,
    run_negative_test, same_multiset, validate_shuffle_multiset,
};
use gbf_experiments::s1::score::ResetContextScorer;
use gbf_foundation::{Hash256, sha256};
use proptest::prelude::*;
use std::cell::Cell;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use tracing_subscriber::prelude::*;
use tracing_subscriber::registry::LookupSpan;

#[test]
fn fisher_yates_empty_and_singleton_are_identity() {
    assert_eq!(fisher_yates(b"", NEGATIVE_TEST_SHUFFLE_SEED), b"");
    assert_eq!(
        fisher_yates(b"x", NEGATIVE_TEST_SHUFFLE_SEED),
        b"x".to_vec()
    );
}

#[test]
fn fisher_yates_seed_deadeef_vector_pins_loop_direction_and_draws() {
    let shuffled = fisher_yates(b"abcdefghi", NEGATIVE_TEST_SHUFFLE_SEED);

    assert_eq!(shuffled, b"hfbadcegi".to_vec());
}

#[test]
fn fisher_yates_is_deterministic_and_preserves_multiset() {
    let input = b"abracadabra abracadabra";

    let first = fisher_yates(input, NEGATIVE_TEST_SHUFFLE_SEED);
    let second = fisher_yates(input, NEGATIVE_TEST_SHUFFLE_SEED);

    assert_eq!(first, second);
    assert_ne!(first, input);
    assert!(same_multiset(input, &first));
}

#[test]
fn fisher_yates_two_byte_outputs_are_only_valid_permutations() {
    for seed in 0..256 {
        let shuffled = fisher_yates(b"ab", seed);
        assert!(
            shuffled == b"ab" || shuffled == b"ba",
            "invalid permutation for seed {seed}: {shuffled:?}"
        );
    }
}

#[test]
fn fisher_yates_three_byte_seed_sweep_is_bounded_uniformity_sanity() {
    let mut counts = BTreeMap::<Vec<u8>, usize>::new();
    let seed_count = 6_144_u64;

    for seed in 0..seed_count {
        *counts.entry(fisher_yates(b"abc", seed)).or_default() += 1;
    }

    assert_eq!(counts.len(), 6);
    for (permutation, count) in counts {
        assert!(
            (512..=1536).contains(&count),
            "permutation {permutation:?} appeared {count} times"
        );
    }
}

#[test]
fn fisher_yates_output_is_invertible_permutation_for_unique_bytes() {
    let input = (0_u8..32).collect::<Vec<_>>();
    let shuffled = fisher_yates(&input, NEGATIVE_TEST_SHUFFLE_SEED);
    let inverse = inverse_permutation(&input, &shuffled);
    let mut recovered = vec![0_u8; shuffled.len()];

    for (shuffled_index, original_index) in inverse.into_iter().enumerate() {
        recovered[original_index] = shuffled[shuffled_index];
    }

    assert_eq!(recovered, input);
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 128,
        failure_persistence: None,
        .. ProptestConfig::default()
    })]

    #[test]
    fn fisher_yates_preserves_arbitrary_byte_multisets(bytes in prop::collection::vec(any::<u8>(), 0..2048), seed in any::<u64>()) {
        let shuffled = fisher_yates(&bytes, seed);

        prop_assert_eq!(shuffled.len(), bytes.len());
        prop_assert!(same_multiset(&bytes, &shuffled));
    }
}

#[test]
fn same_multiset_and_validation_reject_length_and_count_mismatches() {
    assert!(!same_multiset(b"abc", b"ab"));
    assert!(!same_multiset(b"aabc", b"abbc"));
    assert!(matches!(
        validate_shuffle_multiset(b"aabc", b"abbc"),
        Err(NegativeTestError::ShuffleMultisetMismatch)
    ));
}

#[test]
fn uniform_scorer_negative_test_is_context_insensitive_and_self_hashed() {
    let val = (0_u8..64).collect::<Vec<_>>();
    let expected_shuffle_sha = sha256(fisher_yates(&val, NEGATIVE_TEST_SHUFFLE_SEED));

    let report = run_negative_test(
        &UniformScorer,
        0,
        hash(1),
        sha256(&val),
        expected_shuffle_sha,
        &val,
    )
    .expect("negative test report");

    assert_eq!(report.schema, "s1_negative_test.v1");
    assert_eq!(report.seed, 0);
    assert_eq!(report.shuffle_seed, NEGATIVE_TEST_SHUFFLE_SEED);
    assert_eq!(report.bpc_original, 8.0);
    assert_eq!(report.bpc_shuffled, 8.0);
    assert_eq!(report.delta, 0.0);
    assert!(!report.sensitive);
    assert_eq!(report.shuffled_val_sha256, expected_shuffle_sha);
    assert_eq!(
        report.negative_self_hash,
        report.computed_self_hash().expect("self hash")
    );
}

#[test]
fn negative_test_logs_ordered_events_and_complete_self_hash() {
    let capture = TraceCapture::default();
    let subscriber = tracing_subscriber::registry().with(capture.clone());
    let val = (0_u8..32).collect::<Vec<_>>();
    let expected_shuffle_sha = sha256(fisher_yates(&val, NEGATIVE_TEST_SHUFFLE_SEED));

    let report = tracing::subscriber::with_default(subscriber, || {
        run_negative_test(
            &UniformScorer,
            0,
            hash(1),
            sha256(&val),
            expected_shuffle_sha,
            &val,
        )
    })
    .expect("negative test report");

    let records = capture.records();
    let event_names = records
        .iter()
        .filter(|record| record.kind == TraceRecordKind::Event)
        .map(|record| {
            record
                .field(field::EVENT_NAME)
                .expect("structured event name")
                .to_owned()
        })
        .filter(|event_name| event_name.starts_with("s1.neg_test."))
        .collect::<Vec<_>>();
    assert_eq!(
        event_names,
        vec![
            event::NEG_TEST_SHUFFLE_START,
            event::NEG_TEST_SHUFFLE_COMPLETE,
            event::NEG_TEST_SCORE_START,
            event::NEG_TEST_SCORE_COMPLETE,
            event::NEG_TEST_COMPLETE,
        ]
    );

    for event_name in &event_names {
        assert_event_scope(&records, event_name, &[span::NEG_TEST]);
    }
    assert_event_field(
        &records,
        event::NEG_TEST_COMPLETE,
        field::NEGATIVE_SELF_HASH,
        &report.negative_self_hash.to_string(),
    );
}

#[test]
fn context_sensitive_scorer_crosses_negative_test_threshold() {
    let val = (0_u8..64).collect::<Vec<_>>();
    let expected_shuffle_sha = sha256(fisher_yates(&val, NEGATIVE_TEST_SHUFFLE_SEED));

    let report = run_negative_test(
        &NextByteScorer,
        0,
        hash(1),
        sha256(&val),
        expected_shuffle_sha,
        &val,
    )
    .expect("negative test report");

    assert!(report.bpc_shuffled > report.bpc_original);
    assert!(report.delta > 2.0, "delta was {}", report.delta);
    assert!(report.sensitive);
}

#[test]
fn shuffled_hash_pin_mismatch_is_typed_error_before_shuffled_scoring() {
    let val = (0_u8..32).collect::<Vec<_>>();
    let scorer = CountingScorer::default();

    let error = run_negative_test(&scorer, 0, hash(1), sha256(&val), hash(9), &val)
        .expect_err("wrong pin must abort");

    assert_eq!(scorer.logit_calls.get(), 0);
    assert!(matches!(
        error,
        NegativeTestError::ShufflePinMismatch {
            expected,
            observed
        } if expected == hash(9) && observed == sha256(fisher_yates(&val, NEGATIVE_TEST_SHUFFLE_SEED))
    ));
}

#[test]
fn pin_mismatch_logs_error_event_before_return() {
    let capture = TraceCapture::default();
    let subscriber = tracing_subscriber::registry().with(capture.clone());
    let val = (0_u8..32).collect::<Vec<_>>();
    let scorer = CountingScorer::default();

    let error = tracing::subscriber::with_default(subscriber, || {
        run_negative_test(&scorer, 0, hash(1), sha256(&val), hash(9), &val)
    })
    .expect_err("wrong pin must abort");

    assert!(matches!(
        error,
        NegativeTestError::ShufflePinMismatch { .. }
    ));
    assert_eq!(scorer.logit_calls.get(), 0);
    let records = capture.records();
    let pin_mismatch = event_record(&records, event::NEG_TEST_SHUFFLE_PIN_MISMATCH);
    assert_eq!(pin_mismatch.level, "ERROR");
    assert_eq!(
        pin_mismatch.field(field::EXPECTED),
        Some(hash(9).to_string().as_str())
    );
    assert_eq!(
        pin_mismatch.field(field::OBSERVED),
        Some(
            sha256(fisher_yates(&val, NEGATIVE_TEST_SHUFFLE_SEED))
                .to_string()
                .as_str()
        )
    );
    assert!(event_record_optional(&records, event::NEG_TEST_SCORE_START).is_none());
}

#[test]
fn report_builder_rejects_non_finite_negative_bpc_and_real_negative_delta() {
    assert!(matches!(
        negative_test_report_from_bpcs(0, hash(1), hash(2), hash(3), f64::NAN, 1.0),
        Err(NegativeTestError::NonFiniteBpc {
            name: "bpc_original",
            ..
        })
    ));
    assert!(matches!(
        negative_test_report_from_bpcs(0, hash(1), hash(2), hash(3), 1.0, f64::INFINITY),
        Err(NegativeTestError::NonFiniteBpc {
            name: "bpc_shuffled",
            ..
        })
    ));
    assert!(matches!(
        negative_test_report_from_bpcs(0, hash(1), hash(2), hash(3), -0.1, 1.0),
        Err(NegativeTestError::NegativeBpc {
            name: "bpc_original",
            ..
        })
    ));
    assert!(matches!(
        negative_test_report_from_bpcs(0, hash(1), hash(2), hash(3), 2.0, 1.0),
        Err(NegativeTestError::NegativeDelta { delta, .. }) if delta == -1.0
    ));
}

#[test]
fn report_builder_clamps_tiny_negative_delta_summation_drift_only() {
    let report =
        negative_test_report_from_bpcs(0, hash(1), hash(2), hash(3), 8.0, 8.0 - f64::EPSILON * 8.0)
            .expect("tiny negative drift is clamped");

    assert_eq!(report.delta, 0.0);
    assert_eq!(report.bpc_original, 8.0);
    assert!(report.bpc_shuffled < report.bpc_original);
}

#[derive(Debug, Clone, Copy)]
struct UniformScorer;

impl ResetContextScorer for UniformScorer {
    type State = ();

    fn fresh_state(&self) -> Self::State {}

    fn logits(&self, _state: &Self::State) -> Vec<f64> {
        vec![0.0; 256]
    }

    fn consume(&self, _state: &mut Self::State, _byte: u8) {}
}

#[derive(Debug, Default)]
struct CountingScorer {
    logit_calls: Cell<usize>,
}

impl ResetContextScorer for CountingScorer {
    type State = ();

    fn fresh_state(&self) -> Self::State {}

    fn logits(&self, _state: &Self::State) -> Vec<f64> {
        self.logit_calls.set(self.logit_calls.get() + 1);
        vec![0.0; 256]
    }

    fn consume(&self, _state: &mut Self::State, _byte: u8) {}
}

#[derive(Debug, Clone, Copy)]
struct NextByteScorer;

impl ResetContextScorer for NextByteScorer {
    type State = Option<u8>;

    fn fresh_state(&self) -> Self::State {
        None
    }

    fn logits(&self, state: &Self::State) -> Vec<f64> {
        let mut logits = vec![-10.0; 256];
        match state {
            Some(previous) => {
                logits[usize::from(previous.wrapping_add(1))] = 10.0;
            }
            None => {
                logits.fill(0.0);
            }
        }
        logits
    }

    fn consume(&self, state: &mut Self::State, byte: u8) {
        *state = Some(byte);
    }
}

fn hash(byte: u8) -> Hash256 {
    Hash256::from_bytes([byte; 32])
}

fn inverse_permutation(original: &[u8], shuffled: &[u8]) -> Vec<usize> {
    shuffled
        .iter()
        .map(|byte| {
            original
                .iter()
                .position(|candidate| candidate == byte)
                .expect("unique shuffled byte exists in original")
        })
        .collect()
}

fn event_record<'a>(records: &'a [TraceRecord], event_name: &str) -> &'a TraceRecord {
    event_record_optional(records, event_name)
        .unwrap_or_else(|| panic!("missing structured event {event_name}"))
}

fn event_record_optional<'a>(
    records: &'a [TraceRecord],
    event_name: &str,
) -> Option<&'a TraceRecord> {
    records.iter().find(|record| {
        record.kind == TraceRecordKind::Event && record.field(field::EVENT_NAME) == Some(event_name)
    })
}

fn assert_event_field(records: &[TraceRecord], event_name: &str, field_name: &str, expected: &str) {
    let record = event_record(records, event_name);
    assert_eq!(record.field(field_name), Some(expected));
}

fn assert_event_scope(records: &[TraceRecord], event_name: &str, expected: &[&str]) {
    let record = event_record(records, event_name);
    let actual = record
        .span_scope
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    assert_eq!(actual, expected);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TraceRecordKind {
    Event,
}

#[derive(Debug, Clone)]
struct TraceRecord {
    kind: TraceRecordKind,
    level: String,
    fields: BTreeMap<String, String>,
    span_scope: Vec<String>,
}

impl TraceRecord {
    fn field(&self, name: &str) -> Option<&str> {
        self.fields.get(name).map(String::as_str)
    }
}

#[derive(Debug, Clone, Default)]
struct TraceCapture {
    records: Arc<Mutex<Vec<TraceRecord>>>,
}

impl TraceCapture {
    fn records(&self) -> Vec<TraceRecord> {
        self.records
            .lock()
            .expect("trace capture mutex is not poisoned")
            .clone()
    }
}

impl<S> tracing_subscriber::layer::Layer<S> for TraceCapture
where
    S: tracing::Subscriber + for<'span> LookupSpan<'span>,
{
    fn on_event(&self, event: &tracing::Event<'_>, ctx: tracing_subscriber::layer::Context<'_, S>) {
        let mut visitor = TraceFieldVisitor::default();
        event.record(&mut visitor);
        let span_scope = ctx
            .event_scope(event)
            .map(|scope| {
                scope
                    .from_root()
                    .map(|span| span.metadata().name().to_owned())
                    .collect()
            })
            .unwrap_or_default();
        self.records
            .lock()
            .expect("trace capture mutex is not poisoned")
            .push(TraceRecord {
                kind: TraceRecordKind::Event,
                level: event.metadata().level().to_string(),
                fields: visitor.fields,
                span_scope,
            });
    }
}

#[derive(Debug, Default)]
struct TraceFieldVisitor {
    fields: BTreeMap<String, String>,
}

impl TraceFieldVisitor {
    fn insert(&mut self, field: &tracing::field::Field, value: String) {
        self.fields.insert(field.name().to_owned(), value);
    }
}

impl tracing::field::Visit for TraceFieldVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.insert(field, format!("{value:?}"));
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.insert(field, value.to_owned());
    }

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.insert(field, value.to_string());
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.insert(field, value.to_string());
    }

    fn record_f64(&mut self, field: &tracing::field::Field, value: f64) {
        self.insert(field, value.to_string());
    }
}
