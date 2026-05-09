mod common;

use common::tracing_capture::{TraceCapture, captured_events, with_trace_capture};
use gbf_experiments::s1::logging::{event, field};
use gbf_experiments::s1::report::{
    Hypothesis, HypothesisStatus, OutcomeDispatchError, OutcomeDispatchInput, Verdict,
    decision_for_outcome, dispatch_outcome,
};
use gbf_experiments::s1::schema::{S1Decision, S1Outcome};
use proptest::prelude::*;
use serde_json::json;

#[test]
fn o7_dispatch_is_total_for_all_128_binary_inputs() {
    let mut rows = Vec::new();

    for bits in 0_u8..32 {
        for any_seed_diverged in [false, true] {
            for suspicious_low_bpc in [false, true] {
                let input = binary_input(bits, any_seed_diverged, suspicious_low_bpc);
                let dispatch = dispatch_outcome(&input).expect("binary dispatch is total");
                rows.push(format!(
                    "bits={bits:05b} diverged={any_seed_diverged} suspicious={suspicious_low_bpc} => {} / {}",
                    dispatch.outcome, dispatch.decision
                ));
            }
        }
    }

    assert_eq!(rows.len(), 128);
    insta::assert_debug_snapshot!(rows, @r###"
    [
        "bits=00000 diverged=false suspicious=false => Pass-clean / ProceedToS2",
        "bits=00000 diverged=false suspicious=true => Fail-suspicious / Halt(audit-split-and-bpc)",
        "bits=00000 diverged=true suspicious=false => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=00000 diverged=true suspicious=true => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=00001 diverged=false suspicious=false => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=00001 diverged=false suspicious=true => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=00001 diverged=true suspicious=false => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=00001 diverged=true suspicious=true => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=00010 diverged=false suspicious=false => Fail-capacity / Investigate(propose-Toy1)",
        "bits=00010 diverged=false suspicious=true => Fail-suspicious / Halt(audit-split-and-bpc)",
        "bits=00010 diverged=true suspicious=false => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=00010 diverged=true suspicious=true => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=00011 diverged=false suspicious=false => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=00011 diverged=false suspicious=true => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=00011 diverged=true suspicious=false => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=00011 diverged=true suspicious=true => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=00100 diverged=false suspicious=false => Pass-with-warning / ProceedToS2-with-T12.5-prereq",
        "bits=00100 diverged=false suspicious=true => Fail-suspicious / Halt(audit-split-and-bpc)",
        "bits=00100 diverged=true suspicious=false => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=00100 diverged=true suspicious=true => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=00101 diverged=false suspicious=false => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=00101 diverged=false suspicious=true => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=00101 diverged=true suspicious=false => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=00101 diverged=true suspicious=true => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=00110 diverged=false suspicious=false => Fail-capacity / Investigate(propose-Toy1)",
        "bits=00110 diverged=false suspicious=true => Fail-suspicious / Halt(audit-split-and-bpc)",
        "bits=00110 diverged=true suspicious=false => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=00110 diverged=true suspicious=true => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=00111 diverged=false suspicious=false => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=00111 diverged=false suspicious=true => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=00111 diverged=true suspicious=false => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=00111 diverged=true suspicious=true => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=01000 diverged=false suspicious=false => Fail-phase / Investigate(F4-phase-contract)",
        "bits=01000 diverged=false suspicious=true => Fail-phase / Investigate(F4-phase-contract)",
        "bits=01000 diverged=true suspicious=false => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=01000 diverged=true suspicious=true => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=01001 diverged=false suspicious=false => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=01001 diverged=false suspicious=true => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=01001 diverged=true suspicious=false => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=01001 diverged=true suspicious=true => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=01010 diverged=false suspicious=false => Fail-phase / Investigate(F4-phase-contract)",
        "bits=01010 diverged=false suspicious=true => Fail-phase / Investigate(F4-phase-contract)",
        "bits=01010 diverged=true suspicious=false => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=01010 diverged=true suspicious=true => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=01011 diverged=false suspicious=false => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=01011 diverged=false suspicious=true => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=01011 diverged=true suspicious=false => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=01011 diverged=true suspicious=true => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=01100 diverged=false suspicious=false => Fail-phase / Investigate(F4-phase-contract)",
        "bits=01100 diverged=false suspicious=true => Fail-phase / Investigate(F4-phase-contract)",
        "bits=01100 diverged=true suspicious=false => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=01100 diverged=true suspicious=true => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=01101 diverged=false suspicious=false => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=01101 diverged=false suspicious=true => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=01101 diverged=true suspicious=false => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=01101 diverged=true suspicious=true => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=01110 diverged=false suspicious=false => Fail-phase / Investigate(F4-phase-contract)",
        "bits=01110 diverged=false suspicious=true => Fail-phase / Investigate(F4-phase-contract)",
        "bits=01110 diverged=true suspicious=false => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=01110 diverged=true suspicious=true => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=01111 diverged=false suspicious=false => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=01111 diverged=false suspicious=true => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=01111 diverged=true suspicious=false => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=01111 diverged=true suspicious=true => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=10000 diverged=false suspicious=false => Fail-metric / Halt(measurement-broken)",
        "bits=10000 diverged=false suspicious=true => Fail-metric / Halt(measurement-broken)",
        "bits=10000 diverged=true suspicious=false => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=10000 diverged=true suspicious=true => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=10001 diverged=false suspicious=false => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=10001 diverged=false suspicious=true => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=10001 diverged=true suspicious=false => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=10001 diverged=true suspicious=true => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=10010 diverged=false suspicious=false => Fail-metric / Halt(measurement-broken)",
        "bits=10010 diverged=false suspicious=true => Fail-metric / Halt(measurement-broken)",
        "bits=10010 diverged=true suspicious=false => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=10010 diverged=true suspicious=true => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=10011 diverged=false suspicious=false => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=10011 diverged=false suspicious=true => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=10011 diverged=true suspicious=false => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=10011 diverged=true suspicious=true => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=10100 diverged=false suspicious=false => Fail-metric / Halt(measurement-broken)",
        "bits=10100 diverged=false suspicious=true => Fail-metric / Halt(measurement-broken)",
        "bits=10100 diverged=true suspicious=false => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=10100 diverged=true suspicious=true => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=10101 diverged=false suspicious=false => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=10101 diverged=false suspicious=true => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=10101 diverged=true suspicious=false => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=10101 diverged=true suspicious=true => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=10110 diverged=false suspicious=false => Fail-metric / Halt(measurement-broken)",
        "bits=10110 diverged=false suspicious=true => Fail-metric / Halt(measurement-broken)",
        "bits=10110 diverged=true suspicious=false => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=10110 diverged=true suspicious=true => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=10111 diverged=false suspicious=false => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=10111 diverged=false suspicious=true => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=10111 diverged=true suspicious=false => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=10111 diverged=true suspicious=true => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=11000 diverged=false suspicious=false => Fail-metric / Halt(measurement-broken)",
        "bits=11000 diverged=false suspicious=true => Fail-metric / Halt(measurement-broken)",
        "bits=11000 diverged=true suspicious=false => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=11000 diverged=true suspicious=true => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=11001 diverged=false suspicious=false => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=11001 diverged=false suspicious=true => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=11001 diverged=true suspicious=false => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=11001 diverged=true suspicious=true => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=11010 diverged=false suspicious=false => Fail-metric / Halt(measurement-broken)",
        "bits=11010 diverged=false suspicious=true => Fail-metric / Halt(measurement-broken)",
        "bits=11010 diverged=true suspicious=false => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=11010 diverged=true suspicious=true => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=11011 diverged=false suspicious=false => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=11011 diverged=false suspicious=true => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=11011 diverged=true suspicious=false => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=11011 diverged=true suspicious=true => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=11100 diverged=false suspicious=false => Fail-metric / Halt(measurement-broken)",
        "bits=11100 diverged=false suspicious=true => Fail-metric / Halt(measurement-broken)",
        "bits=11100 diverged=true suspicious=false => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=11100 diverged=true suspicious=true => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=11101 diverged=false suspicious=false => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=11101 diverged=false suspicious=true => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=11101 diverged=true suspicious=false => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=11101 diverged=true suspicious=true => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=11110 diverged=false suspicious=false => Fail-metric / Halt(measurement-broken)",
        "bits=11110 diverged=false suspicious=true => Fail-metric / Halt(measurement-broken)",
        "bits=11110 diverged=true suspicious=false => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=11110 diverged=true suspicious=true => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=11111 diverged=false suspicious=false => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=11111 diverged=false suspicious=true => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=11111 diverged=true suspicious=false => Fail-substrate / Investigate(burn-or-autodiff)",
        "bits=11111 diverged=true suspicious=true => Fail-substrate / Investigate(burn-or-autodiff)",
    ]
    "###);
}

#[test]
fn decision_dispatch_matches_section_8() {
    for (outcome, decision) in [
        (S1Outcome::PassClean, S1Decision::ProceedToS2),
        (
            S1Outcome::PassWithWarning,
            S1Decision::ProceedToS2WithT125Prereq,
        ),
        (
            S1Outcome::FailCapacity,
            S1Decision::Investigate {
                reason: "propose-Toy1".to_owned(),
            },
        ),
        (
            S1Outcome::FailSubstrate,
            S1Decision::Investigate {
                reason: "burn-or-autodiff".to_owned(),
            },
        ),
        (
            S1Outcome::FailPhase,
            S1Decision::Investigate {
                reason: "F4-phase-contract".to_owned(),
            },
        ),
        (
            S1Outcome::FailMetric,
            S1Decision::Halt {
                reason: "measurement-broken".to_owned(),
            },
        ),
        (
            S1Outcome::FailSuspicious,
            S1Decision::Halt {
                reason: "audit-split-and-bpc".to_owned(),
            },
        ),
    ] {
        assert_eq!(decision_for_outcome(outcome), decision);
    }
}

#[test]
fn not_evaluated_hypothesis_refuses_closure_or_proceed_dispatch() {
    for (hypothesis, mut input) in [
        (
            Hypothesis::H1,
            input_with_not_evaluated(|input, status| input.h1 = status),
        ),
        (
            Hypothesis::H2,
            input_with_not_evaluated(|input, status| input.h2 = status),
        ),
        (
            Hypothesis::H3,
            input_with_not_evaluated(|input, status| input.h3 = status),
        ),
        (
            Hypothesis::H4,
            input_with_not_evaluated(|input, status| input.h4 = status),
        ),
        (
            Hypothesis::H5,
            input_with_not_evaluated(|input, status| input.h5 = status),
        ),
    ] {
        input.any_seed_diverged = false;
        input.suspicious_low_bpc = false;
        assert!(matches!(
            dispatch_outcome(&input),
            Err(OutcomeDispatchError::NotEvaluatedHypothesis {
                hypothesis: observed,
                reason,
            }) if observed == hypothesis && reason == "prior gate"
        ));
    }
}

#[test]
fn early_failures_short_circuit_downstream_not_evaluated_statuses() {
    let downstream_not_evaluated = OutcomeDispatchInput {
        h1: HypothesisStatus::Confirmed,
        h2: not_evaluated("capacity skipped"),
        h3: not_evaluated("sequence skipped"),
        h4: not_evaluated("phase skipped"),
        h5: not_evaluated("metric skipped"),
        any_seed_diverged: true,
        suspicious_low_bpc: false,
    };
    let diverged = dispatch_outcome(&downstream_not_evaluated).expect("divergence short-circuits");
    assert_eq!(diverged.outcome, S1Outcome::FailSubstrate);
    assert_eq!(
        diverged.decision,
        S1Decision::Investigate {
            reason: "burn-or-autodiff".to_owned()
        }
    );

    let h1_refuted = OutcomeDispatchInput {
        any_seed_diverged: false,
        h1: HypothesisStatus::Refuted,
        ..downstream_not_evaluated
    };
    let dispatched = dispatch_outcome(&h1_refuted).expect("H1 refutation short-circuits");
    assert_eq!(dispatched.outcome, S1Outcome::FailSubstrate);
}

#[test]
fn proceed_outcomes_remain_protected_against_not_evaluated_inputs() {
    for (hypothesis, input) in [
        (
            Hypothesis::H2,
            input_with_not_evaluated(|input, status| input.h2 = status),
        ),
        (
            Hypothesis::H3,
            input_with_not_evaluated(|input, status| input.h3 = status),
        ),
    ] {
        assert!(matches!(
            dispatch_outcome(&input),
            Err(OutcomeDispatchError::NotEvaluatedHypothesis {
                hypothesis: observed,
                ..
            }) if observed == hypothesis
        ));
    }
}

#[test]
fn outcome_dispatch_events_are_subscriber_captured_without_refuted_placeholder() {
    let capture = TraceCapture::default();
    let input = OutcomeDispatchInput {
        h1: HypothesisStatus::Confirmed,
        h2: HypothesisStatus::Refuted,
        h3: not_evaluated("capacity failed"),
        h4: HypothesisStatus::Confirmed,
        h5: HypothesisStatus::Confirmed,
        any_seed_diverged: false,
        suspicious_low_bpc: false,
    };

    let dispatch = with_trace_capture(&capture, || {
        dispatch_outcome(&input).expect("capacity dispatch")
    });
    assert_eq!(dispatch.outcome, S1Outcome::FailCapacity);

    let events = captured_events(&capture)
        .into_iter()
        .filter(|event| event.name.starts_with("s1.outcome."))
        .collect::<Vec<_>>();
    assert_eq!(
        events
            .iter()
            .map(|event| event.name.as_str())
            .collect::<Vec<_>>(),
        vec![
            event::OUTCOME_DISPATCH_START,
            event::OUTCOME_DISPATCH_COMPLETE,
        ]
    );
    assert_eq!(events[0].level, "DEBUG");
    assert_eq!(events[0].fields.get("h2"), Some(&json!("Refuted")));
    assert_eq!(
        events[0].fields.get(field::ANY_SEED_DIVERGED),
        Some(&json!(false))
    );
    assert_eq!(
        events[0].fields.get(field::SUSPICIOUS_LOW_BPC),
        Some(&json!(false))
    );
    assert_eq!(events[1].level, "INFO");
    assert_eq!(
        events[1].fields.get(field::OUTCOME),
        Some(&json!("Fail-capacity"))
    );
    assert_eq!(
        events[1].fields.get(field::DECISION),
        Some(&json!("Investigate(propose-Toy1)"))
    );
    assert!(
        !events
            .iter()
            .any(|event| event.name == "s1.outcome.refuted_input"),
        "refuted_input needs real producer observations and is owned by bd-3v7y"
    );
}

#[test]
fn display_strings_are_pinned_for_report_sections() {
    let outcomes = [
        S1Outcome::PassClean,
        S1Outcome::PassWithWarning,
        S1Outcome::FailSubstrate,
        S1Outcome::FailCapacity,
        S1Outcome::FailSuspicious,
        S1Outcome::FailPhase,
        S1Outcome::FailMetric,
    ]
    .map(|outcome| outcome.to_string());
    let decisions = [
        S1Decision::ProceedToS2,
        S1Decision::ProceedToS2WithT125Prereq,
        S1Decision::Investigate {
            reason: "propose-Toy1".to_owned(),
        },
        S1Decision::Investigate {
            reason: "burn-or-autodiff".to_owned(),
        },
        S1Decision::Investigate {
            reason: "F4-phase-contract".to_owned(),
        },
        S1Decision::Halt {
            reason: "measurement-broken".to_owned(),
        },
        S1Decision::Halt {
            reason: "audit-split-and-bpc".to_owned(),
        },
    ]
    .map(|decision| decision.to_string());
    let statuses = [
        HypothesisStatus::Confirmed,
        HypothesisStatus::Refuted,
        not_evaluated("prior gate"),
    ]
    .map(|status| status.to_string());
    let hypotheses = [
        Hypothesis::H1,
        Hypothesis::H2,
        Hypothesis::H3,
        Hypothesis::H4,
        Hypothesis::H5,
    ]
    .map(|hypothesis| hypothesis.to_string());
    let verdicts = [Verdict::Confirmed, Verdict::Refuted].map(|verdict| verdict.to_string());

    insta::assert_debug_snapshot!((outcomes, decisions, statuses, hypotheses, verdicts), @r###"
    (
        [
            "Pass-clean",
            "Pass-with-warning",
            "Fail-substrate",
            "Fail-capacity",
            "Fail-suspicious",
            "Fail-phase",
            "Fail-metric",
        ],
        [
            "ProceedToS2",
            "ProceedToS2-with-T12.5-prereq",
            "Investigate(propose-Toy1)",
            "Investigate(burn-or-autodiff)",
            "Investigate(F4-phase-contract)",
            "Halt(measurement-broken)",
            "Halt(audit-split-and-bpc)",
        ],
        [
            "Confirmed",
            "Refuted",
            "NotEvaluatedDueToPriorGate(prior gate)",
        ],
        [
            "H1",
            "H2",
            "H3",
            "H4",
            "H5",
        ],
        [
            "Confirmed",
            "Refuted",
        ],
    )
    "###);
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 256,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    #[test]
    fn arbitrary_status_dispatch_is_total_no_panic_and_idempotent(
        h1 in arb_hypothesis_status(),
        h2 in arb_hypothesis_status(),
        h3 in arb_hypothesis_status(),
        h4 in arb_hypothesis_status(),
        h5 in arb_hypothesis_status(),
        any_seed_diverged in any::<bool>(),
        suspicious_low_bpc in any::<bool>(),
    ) {
        let input = OutcomeDispatchInput {
            h1,
            h2,
            h3,
            h4,
            h5,
            any_seed_diverged,
            suspicious_low_bpc,
        };

        let first = dispatch_outcome(&input);
        let second = dispatch_outcome(&input);
        prop_assert_eq!(first, second);
    }

    #[test]
    fn proceed_decisions_never_hide_not_evaluated_statuses(
        h1 in arb_hypothesis_status(),
        h2 in arb_hypothesis_status(),
        h3 in arb_hypothesis_status(),
        h4 in arb_hypothesis_status(),
        h5 in arb_hypothesis_status(),
    ) {
        let input = OutcomeDispatchInput {
            h1,
            h2,
            h3,
            h4,
            h5,
            any_seed_diverged: false,
            suspicious_low_bpc: false,
        };

        if let Ok(dispatch) = dispatch_outcome(&input)
            && matches!(
                dispatch.decision,
                S1Decision::ProceedToS2 | S1Decision::ProceedToS2WithT125Prereq
            )
        {
            prop_assert!(matches!(
                input.h1,
                HypothesisStatus::Confirmed | HypothesisStatus::Refuted
            ));
            prop_assert!(matches!(
                input.h2,
                HypothesisStatus::Confirmed | HypothesisStatus::Refuted
            ));
            prop_assert!(matches!(
                input.h3,
                HypothesisStatus::Confirmed | HypothesisStatus::Refuted
            ));
            prop_assert!(matches!(
                input.h4,
                HypothesisStatus::Confirmed | HypothesisStatus::Refuted
            ));
            prop_assert!(matches!(
                input.h5,
                HypothesisStatus::Confirmed | HypothesisStatus::Refuted
            ));
        }
    }
}

fn binary_input(
    bits: u8,
    any_seed_diverged: bool,
    suspicious_low_bpc: bool,
) -> OutcomeDispatchInput {
    OutcomeDispatchInput {
        h1: bit_status(bits, 0),
        h2: bit_status(bits, 1),
        h3: bit_status(bits, 2),
        h4: bit_status(bits, 3),
        h5: bit_status(bits, 4),
        any_seed_diverged,
        suspicious_low_bpc,
    }
}

fn bit_status(bits: u8, index: u8) -> HypothesisStatus {
    if bits & (1 << index) == 0 {
        Verdict::Confirmed.into()
    } else {
        Verdict::Refuted.into()
    }
}

fn input_with_not_evaluated(
    apply: impl FnOnce(&mut OutcomeDispatchInput, HypothesisStatus),
) -> OutcomeDispatchInput {
    let mut input = binary_input(0, false, false);
    apply(
        &mut input,
        HypothesisStatus::NotEvaluatedDueToPriorGate("prior gate".to_owned()),
    );
    input
}

fn not_evaluated(reason: &str) -> HypothesisStatus {
    HypothesisStatus::NotEvaluatedDueToPriorGate(reason.to_owned())
}

fn arb_hypothesis_status() -> impl Strategy<Value = HypothesisStatus> {
    prop_oneof![
        Just(HypothesisStatus::Confirmed),
        Just(HypothesisStatus::Refuted),
        "[a-z][a-z0-9_-]{0,16}".prop_map(HypothesisStatus::NotEvaluatedDueToPriorGate),
    ]
}
