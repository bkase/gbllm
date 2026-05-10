mod common;

use common::assertions::{
    CanonicalTensor, assert_canonical_json_byte_eq, assert_canonical_tensor_payload_hash_invariant,
    assert_no_nondeterministic_field, assert_self_hash_excludes_field,
};
use common::fixtures::{
    ProbabilityProvider, fixture_uniform_logits_model, fixture_zero_state_model,
    hand_counted_ngram, tiny_corpus,
};
use common::injectable_rng::ScriptedRng;
use common::strategies::{
    arb_byte_seq, arb_canonical_json_value, arb_canonical_tensor_set, arb_seed_in_range,
};
use common::tempdir::{fresh_isolated_env, fresh_run_dir};
use common::tracing_capture::{TraceCapture, assert_event_at, captured_events};
use proptest::prelude::*;
use serde_json::json;
use tracing_subscriber::prelude::*;

#[test]
fn deterministic_fixtures_are_repeatable() {
    assert_eq!(tiny_corpus().name, "s1-tiny-corpus-v0");
    assert_eq!(tiny_corpus().bytes, b"abracadabra\n");
    assert_eq!(tiny_corpus().token_count, tiny_corpus().bytes.len());

    let ngram = hand_counted_ngram();
    assert_eq!(ngram.order, 2);
    assert_eq!(ngram.counts.len(), 8);
    assert_eq!(ngram.counts.get(&b"ab"[..]), Some(&2));
    assert_eq!(ngram.counts.get(&b"a\n"[..]), Some(&1));

    let uniform = fixture_uniform_logits_model();
    assert_eq!(uniform.logits(4), vec![0.0, 0.0, 0.0, 0.0]);
    assert_eq!(uniform.state_width(), 1);

    let zero_state = fixture_zero_state_model();
    assert_eq!(zero_state.logits(2), vec![0.0, 0.0]);
    assert_eq!(zero_state.state_width(), 0);
}

#[test]
fn scripted_rng_returns_scripted_draws_and_panics_on_exhaustion() {
    let mut rng = ScriptedRng::new([0x0102_0304_0506_0708, 0x1112_1314_1516_1718]);
    assert_eq!(rng.remaining(), 2);
    assert_eq!(rng.next_u64(), 0x0102_0304_0506_0708);

    let mut bytes = [0_u8; 3];
    rng.fill_bytes(&mut bytes);
    assert_eq!(bytes, [0x18, 0x17, 0x16]);
    assert!(rng.is_empty());

    let exhausted = std::panic::catch_unwind(move || {
        let mut rng = rng;
        rng.next_u64();
    });
    assert!(exhausted.is_err());
}

#[test]
fn assertion_helpers_cover_canonical_hash_and_field_checks() {
    assert_canonical_json_byte_eq(br#"{"a":1,"b":[true,null]}"#, br#"{"a":1,"b":[true,null]}"#);
    assert!(
        std::panic::catch_unwind(|| {
            assert_canonical_json_byte_eq(br#"{"a":1}"#, br#"{"a":2}"#);
        })
        .is_err()
    );

    let value = json!({"self_hash": "old", "payload": {"stable": true}});
    assert_self_hash_excludes_field(&value, "self_hash", json!("new"));
    assert_no_nondeterministic_field(&value);
    assert!(
        std::panic::catch_unwind(|| {
            assert_no_nondeterministic_field(&json!({"timestamp": "not allowed"}));
        })
        .is_err()
    );

    let tensors = vec![
        CanonicalTensor {
            name: "b".to_owned(),
            dtype: "u8".to_owned(),
            shape: vec![2],
            bytes: vec![1, 2],
        },
        CanonicalTensor {
            name: "a".to_owned(),
            dtype: "u8".to_owned(),
            shape: vec![1],
            bytes: vec![0],
        },
    ];
    // Fixture-local helper only; production tensor payload hashing lives in
    // gbf_artifact and has its own framed stream golden tests.
    assert_canonical_tensor_payload_hash_invariant(&tensors);
}

#[test]
fn tempdir_and_env_helpers_provide_isolation() {
    let run_dir = fresh_run_dir();
    assert!(run_dir.path().is_dir());

    let original_path = std::env::var_os("PATH");
    {
        let _guard = fresh_isolated_env(&[("GBF_S1_TEST_ONLY", "1")]);
        assert_eq!(std::env::var("GBF_S1_TEST_ONLY").as_deref(), Ok("1"));
        assert!(std::env::var_os("PATH").is_none());
    }
    assert_eq!(std::env::var_os("PATH"), original_path);
}

#[test]
fn tracing_capture_asserts_event_order_and_fields() {
    let capture = TraceCapture::default();
    let subscriber = tracing_subscriber::registry().with(capture.clone());

    tracing::subscriber::with_default(subscriber, || {
        tracing::info!(
            target: gbf_experiments::S1_LOG_TARGET,
            event_name = "s1.seed_selected",
            seed = 7_u64,
            accepted = true,
        );
    });

    let events = captured_events(&capture);
    assert_event_at(
        &events,
        0,
        "s1.seed_selected",
        &[("seed", json!(7)), ("accepted", json!(true))],
    );
}

proptest! {
    #[test]
    fn property_strategies_generate_usable_values(
        seed in arb_seed_in_range(10..=20),
        bytes in arb_byte_seq(1, 8),
        json in arb_canonical_json_value(),
        tensors in arb_canonical_tensor_set(),
    ) {
        prop_assert!((10..=20).contains(&seed));
        prop_assert!((1..=8).contains(&bytes.len()));
        assert_no_nondeterministic_field(&json);
        // This exercises only the fixture-local invariant helper, not the
        // production gbf_artifact hash contract.
        assert_canonical_tensor_payload_hash_invariant(&tensors);
    }
}
