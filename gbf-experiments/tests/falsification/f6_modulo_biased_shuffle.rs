use gbf_experiments::s1::neg_test::{NEGATIVE_TEST_SHUFFLE_SEED, fisher_yates, same_multiset};
use gbf_experiments::s1::oracle::MetricOracleResults;
use gbf_experiments::s1::rng::{S1Rng, ShuffleRng};
use gbf_experiments::s1::schema::S1Outcome;
use gbf_foundation::sha256;

#[test]
fn f6_modulo_biased_shuffle_refutes_h5_and_fails_metric() {
    let val = crate::tiny_val_bytes();
    let canonical = fisher_yates(&val, NEGATIVE_TEST_SHUFFLE_SEED);
    let biased = modulo_biased_fisher_yates(&val, NEGATIVE_TEST_SHUFFLE_SEED);
    let expected_pin = sha256(&canonical);
    let observed_biased_pin = sha256(&biased);

    assert!(
        same_multiset(&val, &biased),
        "modulo-biased substitute must preserve byte multiset so F6 fails via shuffle pin/permutation evidence"
    );
    assert_ne!(biased, val, "broken F6 substitute should be non-identity");
    assert_ne!(
        observed_biased_pin, expected_pin,
        "modulo-biased substitute must disagree with canonical O-metric-4 shuffle pin"
    );
    let first_mismatch = canonical
        .iter()
        .zip(&biased)
        .position(|(expected, observed)| expected != observed)
        .expect("biased permutation should differ from canonical permutation");
    assert_ne!(
        canonical[first_mismatch], biased[first_mismatch],
        "first mismatch evidence should identify a byte-level permutation divergence"
    );

    let results = MetricOracleResults {
        o_metric_0: true,
        o_metric_1: true,
        o_metric_2: true,
        o_metric_3: true,
        o_metric_4: false,
    };

    let mut input = crate::confirmed_input();
    input.h5 = results.h5_status();
    crate::assert_falsification_outcome(
        "F6",
        input,
        S1Outcome::FailMetric,
        crate::fail_metric_decision(),
    );
}

fn modulo_biased_fisher_yates(bytes: &[u8], seed: u64) -> Vec<u8> {
    let mut shuffled = bytes.to_vec();
    let mut rng = ShuffleRng::new(seed);
    for i in (1..shuffled.len()).rev() {
        // Broken substitute: modulo reduction over the wrong exclusive bound.
        // It still swaps bytes in-place, so the multiset is preserved.
        let j = (rng.next_u64() as usize) % i;
        shuffled.swap(i, j);
    }
    shuffled
}
