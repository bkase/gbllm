mod common;

use common::tracing_capture::{TraceCapture, captured_events, with_trace_capture};
use gbf_experiments::s1::baseline::{
    ADD_ALPHA, BYTE_VOCAB_SIZE, BaselineError, BaselineOrder, INTERPOLATION_LAMBDAS, LAMBDA_1,
    LAMBDA_2, LAMBDA_3, NgramBaseline, fit_baseline_report,
};
use gbf_experiments::s1::logging::{event, field};
use gbf_foundation::{Hash256, sha256};
use proptest::prelude::*;
use serde_json::json;

const O_METRIC_2_CORPUS: &[u8] = b"ababa";
const O_METRIC_2_EXPECTED: &str = include_str!("fixtures/o_metric_2/expected_values.toml");

#[test]
fn streaming_counts_do_not_inject_boundary_tokens() {
    let baseline = NgramBaseline::fit(O_METRIC_2_CORPUS.iter().copied()).expect("fit baseline");

    assert_eq!(baseline.train_bytes(), 5);
    assert_eq!(baseline.unigram_count(b'a'), 3);
    assert_eq!(baseline.unigram_count(b'b'), 2);
    assert_eq!(baseline.bigram_count(b'a', b'b'), 2);
    assert_eq!(baseline.bigram_count(b'b', b'a'), 2);
    assert_eq!(baseline.trigram_count(b'a', b'b', b'a'), 2);
    assert_eq!(baseline.trigram_count(b'b', b'a', b'b'), 1);
    assert_eq!(baseline.bigram_count(0, b'a'), 0);
    assert_eq!(baseline.trigram_count(0, b'a', b'b'), 0);
    assert_eq!(baseline.bigram_count(b'a', 0), 0);
    assert_eq!(baseline.trigram_count(b'b', b'a', 0), 0);
}

#[test]
fn empty_training_corpus_is_typed_error() {
    assert!(matches!(
        NgramBaseline::fit([].into_iter()),
        Err(BaselineError::EmptyTrainingCorpus)
    ));
}

#[test]
fn single_byte_validation_degenerates_to_unigram_for_all_orders() {
    let baseline = NgramBaseline::fit([b'x']).expect("fit baseline");
    let val = [b'z'];

    let unigram = baseline.bpc(BaselineOrder::Unigram, &val).expect("unigram");
    let bigram = baseline.bpc(BaselineOrder::Bigram, &val).expect("bigram");
    let trigram = baseline.bpc(BaselineOrder::Trigram, &val).expect("trigram");

    assert!(unigram.is_finite());
    assert_eq!(bigram, unigram);
    assert_eq!(trigram, unigram);
}

#[test]
fn o_metric_2_hand_counted_probs_and_bpc_match_derivation() {
    let baseline = NgramBaseline::fit(O_METRIC_2_CORPUS.iter().copied()).expect("fit baseline");

    let p1_a = (3.0 + ADD_ALPHA) / (5.0 + ADD_ALPHA * BYTE_VOCAB_SIZE as f64);
    let p1_b = (2.0 + ADD_ALPHA) / (5.0 + ADD_ALPHA * BYTE_VOCAB_SIZE as f64);
    let p2_b_given_a = (2.0 + ADD_ALPHA) / (2.0 + ADD_ALPHA * BYTE_VOCAB_SIZE as f64);
    let p2_a_given_b = (2.0 + ADD_ALPHA) / (2.0 + ADD_ALPHA * BYTE_VOCAB_SIZE as f64);
    let p3_a_given_ab = (2.0 + ADD_ALPHA) / (2.0 + ADD_ALPHA * BYTE_VOCAB_SIZE as f64);

    assert_close(baseline.unigram_probability(b'a'), p1_a);
    assert_close(baseline.bigram_probability(b'a', b'b'), p2_b_given_a);
    assert_close(
        baseline.trigram_probability(b'a', b'b', b'a'),
        p3_a_given_ab,
    );

    let expected_p0 = p1_a;
    let expected_p1 = (LAMBDA_3 + LAMBDA_2) * p2_b_given_a + LAMBDA_1 * p1_b;
    let expected_p2 = LAMBDA_3 * p3_a_given_ab + LAMBDA_2 * p2_a_given_b + LAMBDA_1 * p1_a;
    assert_close(
        baseline.probability_for_context(BaselineOrder::Trigram, b"ab", b'a'),
        expected_p2,
    );

    let expected_bpc = -[expected_p0, expected_p1, expected_p2]
        .into_iter()
        .map(f64::log2)
        .sum::<f64>()
        / 3.0;
    let observed = baseline.bpc(BaselineOrder::Trigram, b"aba").expect("bpc");

    assert_close(observed, expected_bpc);
    assert_close(toml_f64("p1_a"), p1_a);
    assert_close(toml_f64("p2_b_given_a"), p2_b_given_a);
    assert_close(toml_f64("p3_a_given_ab"), p3_a_given_ab);
    assert_close(toml_f64("p3_interp_a_after_ab"), expected_p2);
    assert_close(toml_f64("bpc_trigram_on_aba"), expected_bpc);
}

#[test]
fn probability_mass_is_normalized_for_present_contexts() {
    let baseline = NgramBaseline::fit(O_METRIC_2_CORPUS.iter().copied()).expect("fit baseline");

    for (order, contexts) in [
        (BaselineOrder::Unigram, vec![b"".as_slice()]),
        (
            BaselineOrder::Bigram,
            vec![b"".as_slice(), b"a".as_slice(), b"b".as_slice()],
        ),
        (
            BaselineOrder::Trigram,
            vec![
                b"".as_slice(),
                b"a".as_slice(),
                b"ab".as_slice(),
                b"ba".as_slice(),
            ],
        ),
    ] {
        for context in contexts {
            assert_close(baseline.probability_mass_for_context(order, context), 1.0);
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        failure_persistence: None,
        .. ProptestConfig::default()
    })]

    #[test]
    fn tiny_corpus_probability_mass_is_normalized(
        train in proptest::collection::vec(any::<u8>(), 1..=100),
    ) {
        let baseline = NgramBaseline::fit(train.iter().copied()).expect("fit baseline");
        let mut contexts = vec![Vec::new()];
        contexts.extend(train.iter().copied().map(|byte| vec![byte]));
        contexts.extend(train.windows(2).map(|window| window.to_vec()));

        for order in [BaselineOrder::Unigram, BaselineOrder::Bigram, BaselineOrder::Trigram] {
            for context in &contexts {
                prop_assert!(
                    (baseline.probability_mass_for_context(order, context) - 1.0).abs() <= 1.0e-12,
                    "order={order:?} context={context:?}"
                );
            }
        }
    }
}

#[test]
fn baseline_report_is_self_hashed_and_logs_are_subscriber_captured() {
    let capture = TraceCapture::default();

    let product = with_trace_capture(&capture, || {
        fit_baseline_report(
            0,
            sha256(O_METRIC_2_CORPUS),
            hash(2),
            O_METRIC_2_CORPUS,
            b"aba",
        )
        .expect("baseline report")
    });

    assert_eq!(product.report.schema, "s1_baseline.v1");
    assert_eq!(product.report.smoothing.alpha, ADD_ALPHA);
    assert_eq!(product.report.smoothing.lambdas, INTERPOLATION_LAMBDAS);
    assert_eq!(product.report.counts_summary.train_bytes, 5);
    assert_eq!(
        product.report.baseline_self_hash,
        product.report.computed_self_hash().expect("self hash")
    );

    let events = captured_events(&capture);
    let baseline_events = events
        .iter()
        .filter(|event| {
            matches!(
                event.name.as_str(),
                event::BASELINE_FIT_START
                    | event::BASELINE_SCORE_START
                    | event::BASELINE_SCORE_COMPLETE
                    | event::BASELINE_FIT_COMPLETE
                    | event::BASELINE_COMPLETE
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        baseline_events.len(),
        5,
        "baseline events: {baseline_events:?}"
    );
    assert_eq!(baseline_events[0].name, event::BASELINE_FIT_START);
    assert_eq!(
        baseline_events[0].fields.get(field::GBF_LOG_SCHEMA_VERSION),
        Some(&json!("1.0.0"))
    );
    assert_eq!(
        baseline_events[0].fields.get(field::CORPUS_TRAIN_SHA),
        Some(&json!(sha256(O_METRIC_2_CORPUS).to_string()))
    );

    let fit_complete = baseline_events
        .iter()
        .find(|event| event.name == event::BASELINE_FIT_COMPLETE)
        .expect("fit complete");
    assert_eq!(
        fit_complete.fields.get(field::BPC_3GRAM),
        Some(&json!(product.report.bpc_3gram))
    );
    assert_eq!(
        fit_complete.fields.get(field::BPC_2GRAM),
        Some(&json!(product.report.bpc_2gram))
    );
    assert_eq!(
        fit_complete.fields.get(field::BPC_UNIGRAM),
        Some(&json!(product.report.bpc_unigram))
    );
    assert_eq!(
        fit_complete.fields.get(field::COUNTS_BLOB_SHA256),
        Some(&json!(product.report.counts_blob_sha256.to_string()))
    );
    assert_eq!(
        fit_complete.fields.get(field::BASELINE_SELF_HASH),
        Some(&json!(product.report.baseline_self_hash.to_string()))
    );

    let completion = baseline_events
        .iter()
        .find(|event| event.name == event::BASELINE_COMPLETE)
        .expect("baseline complete");
    assert_eq!(
        completion.fields.get(field::BPC_3GRAM),
        Some(&json!(product.report.bpc_3gram))
    );
    assert_eq!(
        completion.fields.get(field::BPC_2GRAM),
        Some(&json!(product.report.bpc_2gram))
    );
    assert_eq!(
        completion.fields.get(field::BPC_UNIGRAM),
        Some(&json!(product.report.bpc_unigram))
    );
}

#[test]
fn baseline_math_does_not_reference_f32() {
    let source = include_str!("../src/s1/baseline.rs");
    let scorer = include_str!("../src/s1/score.rs");

    assert!(!source.contains("f32"));
    assert!(!scorer.contains("as f32"));
    assert!(!scorer.contains("from_f32"));
}

fn assert_close(observed: f64, expected: f64) {
    assert!(
        (observed - expected).abs() <= 1.0e-12,
        "observed={observed:?} expected={expected:?}"
    );
}

fn toml_f64(key: &str) -> f64 {
    let prefix = format!("{key} = ");
    let value = O_METRIC_2_EXPECTED
        .lines()
        .find_map(|line| line.strip_prefix(&prefix))
        .unwrap_or_else(|| panic!("missing {key} in O-metric-2 expected values"));
    value.parse::<f64>().expect("expected TOML f64 value")
}

fn hash(byte: u8) -> Hash256 {
    Hash256::from_bytes([byte; 32])
}
