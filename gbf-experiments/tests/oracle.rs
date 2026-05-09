mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use common::injectable_rng::ScriptedRng;
use common::tracing_capture::{TraceCapture, captured_events, with_trace_capture};
use gbf_data::SplitRole;
use gbf_experiments::s1::baseline::{
    ADD_ALPHA, BYTE_VOCAB_SIZE, BaselineOrder, LAMBDA_1, LAMBDA_2, LAMBDA_3, NgramBaseline,
};
use gbf_experiments::s1::logging::{event, field};
use gbf_experiments::s1::manifest::{load_val_bytes, read_tinystories_manifest};
use gbf_experiments::s1::neg_test::{NEGATIVE_TEST_SHUFFLE_SEED, fisher_yates};
use gbf_experiments::s1::oracle::{MetricOracleResults, emit_oracle_report, run_metric_oracles};
use gbf_experiments::s1::report::{
    HypothesisStatus, OutcomeDispatchInput, Verdict, dispatch_outcome,
};
use gbf_experiments::s1::rng::{S1Rng, uniform_u64_inclusive};
use gbf_experiments::s1::schema::{OracleReport, S1Outcome};
use gbf_experiments::s1::score::{
    RESET_CONTEXT_CHUNK_SIZE, ResetContextScorer, ScoreObserver, reset_context_bpc,
    reset_context_bpc_with_observer,
};
use gbf_foundation::{Hash256, sha256};
use proptest::prelude::*;
use serde::Deserialize;
use serde_json::json;
use tracing::{info, info_span};

const O_METRIC_2_CORPUS: &[u8] = b"ababa";
const O_METRIC_2_EXPECTED: &str = include_str!("fixtures/o_metric_2/expected_values.toml");
const TINY_FIXTURE_DIR: &str = "gbf-experiments/tests/fixtures/tiny_corpus";
const CANONICAL_SHUFFLE_PIN: &str =
    "sha256:33ab115b5d230b6286fd39347e7e542bb7663ed148d80e16fc3de1a866f60388";

impl S1Rng for ScriptedRng {
    fn next_u64(&mut self) -> u64 {
        ScriptedRng::next_u64(self)
    }

    fn fill_bytes(&mut self, out: &mut [u8]) {
        ScriptedRng::fill_bytes(self, out);
    }
}

#[test]
fn o_metric_0_rejection_sampler_rejects_adversarial_top_bucket_for_0_9() {
    let _span = enter_oracle(0);
    let rejection_zone_u64 = (u64::MAX / 10) * 10;
    let accepted = 37_u64;
    let mut rng = ScriptedRng::new([rejection_zone_u64, accepted]);

    let draw = uniform_u64_inclusive(&mut rng, 0, 9);

    assert_eq!(draw, 7);
    assert!(rng.is_empty(), "rejection fixture must consume both draws");
    assert_ne!(
        draw,
        rejection_zone_u64 % 10,
        "modulo reduction would incorrectly accept the rejected draw"
    );
    oracle_ok(0);
}

#[test]
fn o_metric_1_uniform_logits_score_exactly_8_bpc() {
    let _span = enter_oracle(1);
    let product = reset_context_bpc(&UniformScorer, b"measurement oracle").expect("score");

    assert_close(product.bpc, 8.0);
    assert_eq!(product.token_count, 18);
    oracle_ok(1);
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 100,
        failure_persistence: None,
        .. ProptestConfig::default()
    })]

    #[test]
    fn o_metric_1_uniform_logits_score_8_bpc_for_arbitrary_non_empty_bytes(
        val in proptest::collection::vec(any::<u8>(), 1..=256),
    ) {
        let product = reset_context_bpc(&UniformScorer, &val).expect("score");
        prop_assert!(
            (product.bpc - 8.0).abs() <= 1.0e-12,
            "bpc={:?} len={}",
            product.bpc,
            val.len()
        );
    }
}

#[test]
fn o_metric_2_hand_counted_ngram_fixture_matches_derivation() {
    let _span = enter_oracle(2);
    let expected = o_metric_2_expected();
    assert_eq!(expected.corpus.as_bytes(), O_METRIC_2_CORPUS);
    assert_close(expected.alpha, ADD_ALPHA);
    assert_eq!(expected.vocab_size, BYTE_VOCAB_SIZE);
    assert_eq!(expected.lambdas, [LAMBDA_3, LAMBDA_2, LAMBDA_1]);

    let baseline = NgramBaseline::fit(O_METRIC_2_CORPUS.iter().copied()).expect("fit baseline");

    assert_eq!(baseline.train_bytes(), expected.counts.train_bytes);
    assert_eq!(baseline.unigram_count(b'a'), expected.counts.unigram_a);
    assert_eq!(baseline.unigram_count(b'b'), expected.counts.unigram_b);
    assert_eq!(baseline.bigram_count(b'a', b'b'), expected.counts.bigram_ab);
    assert_eq!(baseline.bigram_count(b'b', b'a'), expected.counts.bigram_ba);
    assert_eq!(
        baseline.trigram_count(b'a', b'b', b'a'),
        expected.counts.trigram_aba
    );
    assert_eq!(
        baseline.trigram_count(b'b', b'a', b'b'),
        expected.counts.trigram_bab
    );

    let p1_a = (3.0 + ADD_ALPHA) / (5.0 + ADD_ALPHA * BYTE_VOCAB_SIZE as f64);
    let p1_b = (2.0 + ADD_ALPHA) / (5.0 + ADD_ALPHA * BYTE_VOCAB_SIZE as f64);
    let p2_b_given_a = (2.0 + ADD_ALPHA) / (2.0 + ADD_ALPHA * BYTE_VOCAB_SIZE as f64);
    let p2_a_given_b = (2.0 + ADD_ALPHA) / (2.0 + ADD_ALPHA * BYTE_VOCAB_SIZE as f64);
    let p3_a_given_ab = (2.0 + ADD_ALPHA) / (2.0 + ADD_ALPHA * BYTE_VOCAB_SIZE as f64);
    let p3_interp_a_after_ab = LAMBDA_3 * p3_a_given_ab + LAMBDA_2 * p2_a_given_b + LAMBDA_1 * p1_a;
    let bpc_trigram_on_aba = baseline
        .bpc(BaselineOrder::Trigram, b"aba")
        .expect("trigram bpc");

    assert_close(baseline.unigram_probability(b'a'), p1_a);
    assert_close(baseline.unigram_probability(b'b'), p1_b);
    assert_close(baseline.bigram_probability(b'a', b'b'), p2_b_given_a);
    assert_close(baseline.bigram_probability(b'b', b'a'), p2_a_given_b);
    assert_close(
        baseline.trigram_probability(b'a', b'b', b'a'),
        p3_a_given_ab,
    );
    assert_close(
        baseline.probability_for_context(BaselineOrder::Trigram, b"ab", b'a'),
        p3_interp_a_after_ab,
    );
    assert_close(
        bpc_trigram_on_aba,
        expected.probabilities.bpc_trigram_on_aba,
    );
    assert_close(expected.probabilities.p1_a, p1_a);
    assert_close(expected.probabilities.p1_b, p1_b);
    assert_close(expected.probabilities.p2_b_given_a, p2_b_given_a);
    assert_close(expected.probabilities.p2_a_given_b, p2_a_given_b);
    assert_close(expected.probabilities.p3_a_given_ab, p3_a_given_ab);
    assert_close(
        expected.probabilities.p3_interp_a_after_ab,
        p3_interp_a_after_ab,
    );
    oracle_ok(2);
}

#[test]
fn o_metric_3_reset_boundary_spy_records_0_to_127_then_0() {
    let _span = enter_oracle(3);
    let mut observer = ContextSpy::default();
    let val = vec![0_u8; RESET_CONTEXT_CHUNK_SIZE + 1];

    reset_context_bpc_with_observer(&UniformScorer, &val, &mut observer).expect("score");

    let expected = (0_usize..128).chain([0]).collect::<Vec<_>>();
    assert_eq!(observer.context_lengths, expected);
    assert_eq!(observer.chunk_indexes[..128], [0_u64; 128]);
    assert_eq!(observer.chunk_indexes[128], 1);
    oracle_ok(3);
}

#[test]
fn o_metric_4_tiny_fixture_shuffle_preserves_multiset_nonidentity_and_pin() {
    let _span = enter_oracle(4);
    let manifest = read_tinystories_manifest(tiny_manifest_path()).expect("tiny manifest");
    let val = load_val_bytes(&manifest).expect("tiny val loads");
    let shuffled = fisher_yates(&val, NEGATIVE_TEST_SHUFFLE_SEED);

    assert_shuffle_contract(
        &val,
        &shuffled,
        manifest
            .val_shuffle_deadeef_sha256
            .expect("tiny shuffle pin"),
    );
    oracle_ok(4);
}

#[test]
#[ignore = "requires canonical TinyStories validation bytes at corpus/tinystories/raw/TinyStoriesV2-GPT4-valid.txt"]
fn o_metric_4_canonical_tinystories_shuffle_matches_manifest_pin() {
    let _span = enter_oracle(4);
    let manifest = read_tinystories_manifest(canonical_manifest_path()).expect("manifest");
    let val_path = manifest.split_path(SplitRole::Validation);
    if !val_path.exists() {
        panic!(
            "canonical TinyStories validation bytes are absent at {}; fetch the validation split before running this oracle",
            val_path.display()
        );
    }
    let val = load_val_bytes(&manifest).expect("canonical val loads");
    let shuffled = fisher_yates(&val, NEGATIVE_TEST_SHUFFLE_SEED);

    assert_eq!(
        manifest.val_shuffle_deadeef_sha256,
        Some(expected_canonical_shuffle_pin())
    );
    assert_shuffle_contract(&val, &shuffled, expected_canonical_shuffle_pin());
    oracle_ok(4);
}

#[test]
fn aggregate_metric_oracle_passed_requires_all_five_oracles() {
    let all_pass = MetricOracleResults {
        o_metric_0: true,
        o_metric_1: true,
        o_metric_2: true,
        o_metric_3: true,
        o_metric_4: true,
    };
    assert!(all_pass.metric_oracle_passed());
    assert!(all_pass.failed_oracle_ids().is_empty());

    for failed in 0..5 {
        let mut results = [true; 5];
        results[failed] = false;
        let aggregate = MetricOracleResults {
            o_metric_0: results[0],
            o_metric_1: results[1],
            o_metric_2: results[2],
            o_metric_3: results[3],
            o_metric_4: results[4],
        };
        assert!(!aggregate.metric_oracle_passed(), "failed index {failed}");
        assert_eq!(aggregate.failed_oracle_ids().len(), 1);
    }
    let _span = info_span!("s1.oracle.aggregate.complete", metric_oracle_passed = true).entered();
    info!(failed_oracle_ids = "[]", "metric oracle aggregate passed");
}

#[test]
fn oracle_report_is_self_hashed_and_canonical() {
    let report = all_oracles_pass().report().expect("oracle report");

    assert_eq!(report.schema, "s1_oracle.v1");
    assert!(report.metric_oracle_passed);
    assert!(report.failed_oracle_ids.is_empty());
    assert_eq!(
        report.oracle_self_hash,
        report.computed_self_hash().expect("oracle self hash")
    );
    assert_eq!(
        report.canonical_json_bytes().expect("oracle canonical json"),
        br#"{"failed_oracle_ids":[],"metric_oracle_passed":true,"o_metric_0":true,"o_metric_1":true,"o_metric_2":true,"o_metric_3":true,"o_metric_4":true,"schema":"s1_oracle.v1"}"#
    );
}

#[test]
fn oracle_report_deserialization_rejects_inconsistent_aggregates() {
    let mut value = serde_json::to_value(all_oracles_pass().report().expect("oracle report"))
        .expect("oracle json");
    value["metric_oracle_passed"] = json!(false);

    let error = serde_json::from_value::<OracleReport>(value).expect_err("aggregate mismatch");
    assert!(
        error.to_string().contains("metric_oracle_passed disagrees"),
        "{error}"
    );
}

#[test]
fn oracle_report_deserialization_rejects_inconsistent_failed_oracle_ids() {
    let mut value = serde_json::to_value(all_oracles_pass().report().expect("oracle report"))
        .expect("oracle json");
    value["failed_oracle_ids"] = json!(["O-metric-4"]);

    let error = serde_json::from_value::<OracleReport>(value).expect_err("failed-id mismatch");
    assert!(
        error.to_string().contains("failed_oracle_ids disagree"),
        "{error}"
    );
}

#[test]
fn metric_oracle_results_drive_h5_and_outcome_dispatch() {
    let pass = all_oracles_pass();
    assert_eq!(pass.h5_status(), HypothesisStatus::Confirmed);
    assert_eq!(
        dispatch_outcome(&outcome_input_with_h5(pass.h5_status()))
            .expect("pass dispatch")
            .outcome,
        S1Outcome::PassClean
    );

    let fail = MetricOracleResults {
        o_metric_0: true,
        o_metric_1: true,
        o_metric_2: false,
        o_metric_3: true,
        o_metric_4: true,
    };
    assert_eq!(fail.h5_status(), HypothesisStatus::Refuted);
    assert_eq!(
        fail.failed_oracle_ids(),
        vec!["O-metric-2"],
        "failed oracle ids are emitted in D7 order"
    );
    assert_eq!(
        dispatch_outcome(&outcome_input_with_h5(fail.h5_status()))
            .expect("fail dispatch")
            .outcome,
        S1Outcome::FailMetric
    );
}

#[test]
fn oracle_report_emission_has_subscriber_captured_events() {
    let capture = TraceCapture::default();
    let results = MetricOracleResults {
        o_metric_0: true,
        o_metric_1: true,
        o_metric_2: false,
        o_metric_3: true,
        o_metric_4: true,
    };

    let report = with_trace_capture(&capture, || emit_oracle_report(7, results))
        .expect("oracle report emission");

    let events = captured_events(&capture);
    let oracle_events = events
        .iter()
        .filter(|event| {
            matches!(
                event.name.as_str(),
                event::ORACLE_START
                    | event::ORACLE_COMPLETE
                    | event::ORACLE_FAILED
                    | event::ORACLE_AGGREGATE_COMPLETE
            )
        })
        .collect::<Vec<_>>();

    assert_eq!(oracle_events.len(), 11, "oracle events: {oracle_events:?}");
    assert_eq!(
        oracle_events
            .iter()
            .filter(|event| event.name == event::ORACLE_START)
            .count(),
        5
    );
    assert_eq!(
        oracle_events
            .iter()
            .filter(|event| event.name == event::ORACLE_COMPLETE)
            .count(),
        4
    );

    let failure = oracle_events
        .iter()
        .find(|event| event.name == event::ORACLE_FAILED)
        .expect("failure event");
    assert_eq!(failure.name, event::ORACLE_FAILED);
    assert_eq!(failure.name, "s1.oracle.failed");
    assert_eq!(failure.fields.get(field::SEED), Some(&json!(7)));
    assert_eq!(failure.fields.get(field::ORACLE_ID), Some(&json!(2)));
    assert_eq!(
        failure.fields.get(field::DIAGNOSTIC),
        Some(&json!("O-metric-2 returned false"))
    );

    let aggregate = oracle_events
        .iter()
        .find(|event| event.name == event::ORACLE_AGGREGATE_COMPLETE)
        .expect("aggregate event");
    assert_eq!(
        aggregate.fields.get(field::METRIC_ORACLE_PASSED),
        Some(&json!(false))
    );
    assert_eq!(
        aggregate.fields.get(field::FAILED_ORACLE_IDS),
        Some(&json!("[\"O-metric-2\"]"))
    );
    let failed_ids: Vec<String> = serde_json::from_str(
        aggregate
            .fields
            .get(field::FAILED_ORACLE_IDS)
            .and_then(serde_json::Value::as_str)
            .expect("failed_oracle_ids string"),
    )
    .expect("parse failed ids");
    assert_eq!(failed_ids, vec!["O-metric-2"]);
    assert_eq!(
        aggregate.fields.get(field::ORACLE_SELF_HASH),
        Some(&json!(report.oracle_self_hash.to_string()))
    );
}

#[test]
fn oracle_report_emission_uses_seed_and_parse_safe_empty_failed_ids() {
    let capture = TraceCapture::default();

    let report = with_trace_capture(&capture, || emit_oracle_report(4, all_oracles_pass()))
        .expect("oracle report emission");

    assert!(report.metric_oracle_passed);
    let events = captured_events(&capture);
    let aggregate = events
        .iter()
        .find(|event| event.name == event::ORACLE_AGGREGATE_COMPLETE)
        .expect("aggregate event");
    assert_eq!(aggregate.fields.get(field::SEED), Some(&json!(4)));
    assert_eq!(
        aggregate.fields.get(field::FAILED_ORACLE_IDS),
        Some(&json!("[]"))
    );
    let failed_ids: Vec<String> = serde_json::from_str(
        aggregate
            .fields
            .get(field::FAILED_ORACLE_IDS)
            .and_then(serde_json::Value::as_str)
            .expect("failed_oracle_ids string"),
    )
    .expect("parse failed ids");
    assert!(failed_ids.is_empty());
}

#[test]
fn oracle_producer_emits_s1_oracle_report_for_tiny_fixture() {
    let manifest = read_tinystories_manifest(tiny_manifest_path()).expect("tiny manifest");
    let val = load_val_bytes(&manifest).expect("tiny val loads");
    let report = run_metric_oracles(
        3,
        &val,
        manifest
            .val_shuffle_deadeef_sha256
            .expect("tiny shuffle pin"),
    )
    .expect("oracle producer");

    assert_eq!(report.schema, "s1_oracle.v1");
    assert!(report.metric_oracle_passed);
    assert_eq!(
        report.h5_status().expect("oracle H5"),
        HypothesisStatus::Confirmed
    );
    assert_eq!(
        report.oracle_self_hash,
        report.computed_self_hash().expect("oracle self hash")
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

fn assert_shuffle_contract(original: &[u8], shuffled: &[u8], expected_hash: Hash256) {
    assert_eq!(byte_multiset(original), byte_multiset(shuffled));
    assert_ne!(shuffled, original);
    assert_eq!(sha256(shuffled), expected_hash);
}

fn byte_multiset(bytes: &[u8]) -> BTreeMap<u8, usize> {
    let mut counts = BTreeMap::new();
    for &byte in bytes {
        *counts.entry(byte).or_default() += 1;
    }
    counts
}

fn enter_oracle(id: u8) -> tracing::span::EnteredSpan {
    let oracle_id = format!("O-metric-{id}");
    let span = match id {
        0 => info_span!("s1.oracle.0", oracle_id = %oracle_id),
        1 => info_span!("s1.oracle.1", oracle_id = %oracle_id),
        2 => info_span!("s1.oracle.2", oracle_id = %oracle_id),
        3 => info_span!("s1.oracle.3", oracle_id = %oracle_id),
        4 => info_span!("s1.oracle.4", oracle_id = %oracle_id),
        _ => info_span!("s1.oracle.unknown", oracle_id = %oracle_id),
    };
    let entered = span.entered();
    info!(oracle_id = %oracle_id, status = "start");
    entered
}

fn oracle_ok(id: u8) {
    info!(oracle_id = format!("O-metric-{id}"), status = "ok");
}

fn assert_close(observed: f64, expected: f64) {
    assert!(
        (observed - expected).abs() <= 1.0e-12,
        "observed={observed:?} expected={expected:?}"
    );
}

fn o_metric_2_expected() -> OMetric2Expected {
    toml::from_str(O_METRIC_2_EXPECTED).expect("O-metric-2 expected TOML")
}

fn all_oracles_pass() -> MetricOracleResults {
    MetricOracleResults {
        o_metric_0: true,
        o_metric_1: true,
        o_metric_2: true,
        o_metric_3: true,
        o_metric_4: true,
    }
}

fn outcome_input_with_h5(h5: HypothesisStatus) -> OutcomeDispatchInput {
    OutcomeDispatchInput {
        h1: Verdict::Confirmed.into(),
        h2: Verdict::Confirmed.into(),
        h3: Verdict::Confirmed.into(),
        h4: Verdict::Confirmed.into(),
        h5,
        any_seed_diverged: false,
        suspicious_low_bpc: false,
    }
}

fn canonical_manifest_path() -> PathBuf {
    workspace_root().join("fixtures/corpora/tinystories.toml")
}

fn tiny_manifest_path() -> PathBuf {
    workspace_root()
        .join(TINY_FIXTURE_DIR)
        .join("manifest.toml")
}

fn workspace_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
}

fn expected_canonical_shuffle_pin() -> Hash256 {
    CANONICAL_SHUFFLE_PIN.parse().expect("canonical pin")
}

#[derive(Debug, Deserialize)]
struct OMetric2Expected {
    corpus: String,
    alpha: f64,
    vocab_size: usize,
    lambdas: [f64; 3],
    counts: OMetric2Counts,
    probabilities: OMetric2Probabilities,
}

#[derive(Debug, Deserialize)]
struct OMetric2Counts {
    train_bytes: u64,
    unigram_a: u64,
    unigram_b: u64,
    bigram_ab: u64,
    bigram_ba: u64,
    trigram_aba: u32,
    trigram_bab: u32,
}

#[derive(Debug, Deserialize)]
struct OMetric2Probabilities {
    p1_a: f64,
    p1_b: f64,
    p2_b_given_a: f64,
    p2_a_given_b: f64,
    p3_a_given_ab: f64,
    p3_interp_a_after_ab: f64,
    bpc_trigram_on_aba: f64,
}
