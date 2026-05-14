mod common;

use common::assertions::{
    canonical_json_byte_eq, phase_entry_invariants_assert, self_hash_excludes_field,
};
use common::fixtures::synthetic_router::{four_experts, soft_top1_dispatch};
use common::fixtures::{tiny_corpus_s2, tiny_corpus_s2_fixture};
use common::helpers::gradient_capture::{
    capture_explicit_grad_norm, capture_trivial_burn_grad_norms,
};
use common::helpers::phase_log_capture::PhaseLogCapture;
use common::helpers::scripted_falsify_runner::{
    BrokenS2Kind, ScriptedFalsifyGuard, active_broken_kind, run_with_broken_kind,
};
use common::helpers::tiny_model_s2::five_phase_fixture;
use common::helpers::tracing_capture_s2::{capture_events, events_to_ndjson};
use common::proptest_strategies::{
    arb_canonical_tensor_set, arb_diagnostic_subcheck, arb_fixture_result, arb_hardness_triple,
    arb_loss_term_eval_point, arb_phase_effective_lambda, arb_phase_entry, arb_phase_event,
    arb_s2_outcome, arb_seed_in_range, arb_train_config_s2,
};
use gbf_train::logging::TrainingLogEmitter;
use gbf_train::scheduler::{PhaseStepOutcome, TrainingPhaseScheduler};
use proptest::prelude::*;
use serde_json::json;

#[test]
fn s2_fixtures_and_router_helpers_are_repeatable() {
    let corpus = tiny_corpus_s2();
    assert_eq!(corpus.name, "s2-tinystories-stub-with-eval-split");
    assert_eq!(corpus.bytes, b"Once upon a byte.\n");

    let fixture = tiny_corpus_s2_fixture();
    assert_eq!(fixture.inherited_s1.name, "s1-tiny-corpus-v0");
    assert_eq!(
        fixture.train_stub.token_count,
        fixture.train_stub.bytes.len()
    );
    assert_eq!(fixture.eval_stub.token_count, fixture.eval_stub.bytes.len());
    assert!(fixture.eval_stub.bytes.ends_with(b"Eval bytes follow.\n"));

    let hard = four_experts();
    let hard_output = hard
        .router
        .forward_stateless(
            &hard.input,
            hard.previous_distribution.as_deref(),
            &hard.options,
        )
        .expect("hard router fixture runs");
    assert_eq!(hard_output.routing_weights().iter().sum::<f32>(), 1.0);

    let soft = soft_top1_dispatch();
    let soft_output = soft
        .router
        .forward_stateless(
            &soft.input,
            soft.previous_distribution.as_deref(),
            &soft.options,
        )
        .expect("soft router fixture runs");
    assert_eq!(soft_output.routing_weights().len(), 4);
    assert!(
        soft_output
            .routing_weights()
            .iter()
            .all(|weight| *weight > 0.0)
    );
}

#[test]
fn s2_assertion_helpers_cover_json_self_hash_and_phase_entries() {
    canonical_json_byte_eq(br#"{"a":1}"#, br#"{"a":1}"#);
    self_hash_excludes_field(
        &json!({"payload": 1, "self_hash": "old"}),
        "self_hash",
        json!("new"),
    );
    phase_entry_invariants_assert(
        &json!({"event": "phase_transition", "from": "PhaseA", "to": "PhaseB", "step": 4}),
        Some("PhaseA"),
        "PhaseB",
    );
}

#[test]
fn phase_log_capture_serializes_expected_ndjson() {
    let mut capture = PhaseLogCapture::new();
    capture.push_transition(None, "PhaseA", 0);
    capture.push_transition(Some("PhaseA"), "PhaseB", 4);

    assert_eq!(capture.entries().len(), 2);
    assert_eq!(
        capture.to_ndjson(),
        br#"{"event":"phase_transition","from":null,"to":"PhaseA","step":0}
{"event":"phase_transition","from":"PhaseA","to":"PhaseB","step":4}
"#
    );
}

#[test]
fn tracing_capture_s2_records_events_and_macros_assert_sequence() {
    let (_, events) = capture_events(|| {
        tracing::debug!(
            event_name = "fixture_loaded",
            name = "manual-fixture",
            path = "fixture.toml",
            expected_bytes_sha = "abc123"
        );
        tracing::info!(
            event_name = "phase_transition",
            from = "PhaseA",
            to = "PhaseB"
        );
    });

    assert_event_emitted!(&events, name = "fixture_loaded");
    assert_event_emitted!(&events, name = "phase_transition");
    assert_no_event!(&events, name = "not_emitted");
    assert_log_sequence!(
        &events,
        [("event", "fixture_loaded"), ("event", "phase_transition")]
    );
    assert!(
        String::from_utf8(events_to_ndjson(&events))
            .expect("ndjson is utf8")
            .contains("phase_transition")
    );
}

#[test]
fn trace_capture_lock_serializes_different_return_types() {
    use common::tracing_capture::{TraceCapture, with_trace_capture};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::time::Duration;

    let active_captures = Arc::new(AtomicUsize::new(0));
    let (first_entered_tx, first_entered_rx) = mpsc::channel();
    let (release_first_tx, release_first_rx) = mpsc::channel();

    let first_active_captures = Arc::clone(&active_captures);
    let first = std::thread::spawn(move || {
        let capture = TraceCapture::default();
        with_trace_capture(&capture, || -> usize {
            assert_eq!(first_active_captures.fetch_add(1, Ordering::SeqCst), 0);
            first_entered_tx
                .send(())
                .expect("first trace capture entry is reported");
            release_first_rx
                .recv_timeout(Duration::from_secs(2))
                .expect("first trace capture is released");
            first_active_captures.fetch_sub(1, Ordering::SeqCst);
            7
        })
    });

    first_entered_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("first trace capture starts");

    let (second_started_tx, second_started_rx) = mpsc::channel();
    let (second_entered_tx, second_entered_rx) = mpsc::channel();
    let second_active_captures = Arc::clone(&active_captures);
    let second = std::thread::spawn(move || {
        let capture = TraceCapture::default();
        second_started_tx
            .send(())
            .expect("second trace capture start is reported");
        with_trace_capture(&capture, || -> String {
            let overlapping = second_active_captures.fetch_add(1, Ordering::SeqCst);
            second_entered_tx
                .send(overlapping)
                .expect("second trace capture entry is reported");
            second_active_captures.fetch_sub(1, Ordering::SeqCst);
            "captured".to_owned()
        })
    });

    second_started_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("second trace capture starts");
    assert!(
        second_entered_rx
            .recv_timeout(Duration::from_millis(100))
            .is_err(),
        "trace capture lock must be shared across return-type instantiations"
    );

    release_first_tx
        .send(())
        .expect("first trace capture release is sent");
    assert_eq!(first.join().expect("first trace capture thread joins"), 7);
    assert_eq!(
        second_entered_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("second trace capture enters after release"),
        0
    );
    assert_eq!(
        second.join().expect("second trace capture thread joins"),
        "captured"
    );
}

#[test]
fn tiny_model_s2_applies_five_phase_schedule() {
    let (mut model, schedule) = five_phase_fixture();
    let mut scheduler = TrainingPhaseScheduler::new(schedule);
    let emitter = TrainingLogEmitter::new();

    assert!(matches!(
        scheduler.apply_step(0, &mut model, &emitter).unwrap(),
        PhaseStepOutcome::EnteredInitial { .. }
    ));
    assert!(matches!(
        scheduler.apply_step(2, &mut model, &emitter).unwrap(),
        PhaseStepOutcome::Transitioned { .. }
    ));

    assert_eq!(model.applied_kinds().len(), 2);
    assert_eq!(model.applied_controls()[1].step(), 2);
}

#[test]
fn gradient_capture_records_nonzero_norms() {
    let explicit = capture_explicit_grad_norm("w", &[3.0, 4.0]);
    assert_eq!(explicit.l2_norm, 5.0);

    let burn_norms = capture_trivial_burn_grad_norms();
    assert_eq!(burn_norms[0].name, "input");
    assert!(burn_norms[0].l2_norm > 0.0);
}

#[test]
fn scripted_falsify_guard_releases_flag_after_drop_and_panic() {
    assert_eq!(active_broken_kind(), None);
    {
        let _guard = ScriptedFalsifyGuard::activate(BrokenS2Kind::F1PhaseBSkipsTernary);
        assert_eq!(
            active_broken_kind(),
            Some(BrokenS2Kind::F1PhaseBSkipsTernary)
        );
    }
    assert_eq!(active_broken_kind(), None);

    let panicked = std::panic::catch_unwind(|| {
        run_with_broken_kind(BrokenS2Kind::F5ZeroLossShortCircuit, || {
            assert_eq!(
                active_broken_kind(),
                Some(BrokenS2Kind::F5ZeroLossShortCircuit)
            );
            panic!("exercise guard drop during unwind");
        });
    });
    assert!(panicked.is_err());
    assert_eq!(active_broken_kind(), None);
}

proptest! {
    #[test]
    fn s2_proptest_strategies_generate_valid_fixture_values(
        phase_entry in arb_phase_entry(),
        phase_event in arb_phase_event(),
        loss in arb_loss_term_eval_point(),
        hardness in arb_hardness_triple(),
        lambda in arb_phase_effective_lambda(),
        outcome in arb_s2_outcome(),
        diagnostic in arb_diagnostic_subcheck(),
        fixture in arb_fixture_result(),
        seed in arb_seed_in_range(1..=9),
        tensors in arb_canonical_tensor_set(),
        config in arb_train_config_s2(),
    ) {
        prop_assert!(phase_entry.step <= 10_000);
        prop_assert!(phase_event.step <= 10_000);
        prop_assert!(loss.raw_loss.is_finite() && loss.raw_loss >= 0.0);
        prop_assert!((0.0..=1.0).contains(&hardness.expert_qat));
        prop_assert!((0.0..=1.0).contains(&hardness.activation_qat));
        prop_assert!((0.0..=1.0).contains(&hardness.norm_qat));
        prop_assert!(lambda.value.is_finite() && lambda.value >= 0.0);
        prop_assert!(matches!(outcome, common::proptest_strategies::S2Outcome::Pass | common::proptest_strategies::S2Outcome::FailGap | common::proptest_strategies::S2Outcome::PassWithDistillWarn | common::proptest_strategies::S2Outcome::Refuted));
        prop_assert!(!diagnostic.name.is_empty());
        prop_assert!(!fixture.fixture_name.is_empty());
        prop_assert!((1..=9).contains(&seed));
        prop_assert!(tensors.len() <= 8);
        prop_assert!(config.optimizer_steps >= 1);
    }
}
