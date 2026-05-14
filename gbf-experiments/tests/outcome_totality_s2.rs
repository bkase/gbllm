use std::collections::BTreeMap;

use gbf_experiments::s2::report::{decision_for_outcome, dispatch_outcome};
use gbf_experiments::s2::schema::{
    HypothesisStatus, S2Completion, S2Decision, S2Hypothesis, S2Outcome, S2VerifierBundle,
};
use proptest::prelude::*;
use serde_json::json;

type MutateBundle = fn(&mut S2VerifierBundle);

#[test]
fn every_s2_outcome_maps_to_a_closure_decision() {
    for outcome in S2Outcome::ALL {
        let decision = decision_for_outcome(outcome);
        match outcome {
            S2Outcome::PassClean => assert_eq!(decision, S2Decision::ProceedToS3),
            S2Outcome::PassWithDistillWarn => {
                assert_eq!(decision, S2Decision::ProceedToS3WithDistillReview);
            }
            S2Outcome::FailMetric
            | S2Outcome::FailSuspicious
            | S2Outcome::FailPreregistration
            | S2Outcome::FailArtifact
            | S2Outcome::FailIncomplete => {
                assert!(
                    matches!(decision, S2Decision::Halt { .. }),
                    "{outcome:?} should halt, got {decision:?}"
                );
            }
            S2Outcome::FailSubstrate
            | S2Outcome::FailGap
            | S2Outcome::FailPhase
            | S2Outcome::FailLossGradFlow
            | S2Outcome::FailLinearstate
            | S2Outcome::FailPhaseIntegration
            | S2Outcome::FailFalsification
            | S2Outcome::FailApiDrift => {
                assert!(
                    matches!(decision, S2Decision::Investigate { .. }),
                    "{outcome:?} should investigate, got {decision:?}"
                );
            }
        }
    }
}

#[test]
fn every_s2_outcome_is_reachable_via_dispatcher() {
    let cases: [(&str, MutateBundle, S2Outcome); 15] = [
        ("pass_clean", |_| {}, S2Outcome::PassClean),
        (
            "distill_warn",
            |b| set_status(b, S2Hypothesis::H3, HypothesisStatus::Refuted),
            S2Outcome::PassWithDistillWarn,
        ),
        (
            "substrate",
            |b| b.completions[0] = S2Completion::DivergedAt { step: 17 },
            S2Outcome::FailSubstrate,
        ),
        (
            "gap",
            |b| set_status(b, S2Hypothesis::H2, HypothesisStatus::Refuted),
            S2Outcome::FailGap,
        ),
        (
            "suspicious",
            |b| b.suspicious_low_bpc = true,
            S2Outcome::FailSuspicious,
        ),
        (
            "phase",
            |b| set_status(b, S2Hypothesis::H4, HypothesisStatus::Refuted),
            S2Outcome::FailPhase,
        ),
        (
            "loss_grad_flow",
            |b| set_status(b, S2Hypothesis::H5, HypothesisStatus::Refuted),
            S2Outcome::FailLossGradFlow,
        ),
        (
            "linearstate",
            |b| set_status(b, S2Hypothesis::H6, HypothesisStatus::Refuted),
            S2Outcome::FailLinearstate,
        ),
        (
            "phase_integration",
            |b| b.phase_transition_integ_passed = false,
            S2Outcome::FailPhaseIntegration,
        ),
        (
            "falsification",
            |b| b.falsification_s2_passed = false,
            S2Outcome::FailFalsification,
        ),
        (
            "api_drift",
            |b| b.api_drift_check_passed = false,
            S2Outcome::FailApiDrift,
        ),
        (
            "metric",
            |b| b.oracle_re_run_passed = false,
            S2Outcome::FailMetric,
        ),
        (
            "preregistration",
            |b| b.preregistration_passed = false,
            S2Outcome::FailPreregistration,
        ),
        (
            "artifact",
            |b| b.artifact_integrity_passed = false,
            S2Outcome::FailArtifact,
        ),
        (
            "incomplete",
            |b| b.completions[0] = S2Completion::NotReached,
            S2Outcome::FailIncomplete,
        ),
    ];

    let reached = cases
        .into_iter()
        .map(|(name, mutate, expected)| {
            let mut bundle = S2VerifierBundle::closure_candidate();
            mutate(&mut bundle);
            let actual = dispatch_outcome(&bundle);
            assert_eq!(actual, expected, "{name}");
            actual
        })
        .collect::<Vec<_>>();

    for outcome in S2Outcome::ALL {
        assert!(
            reached.contains(&outcome),
            "{outcome:?} lacks a direct dispatcher fixture"
        );
    }
}

#[test]
fn future_schema_variants_are_rejected_at_deserialization_boundary() {
    assert!(serde_json::from_value::<S2Outcome>(json!("Pass-future")).is_err());
    assert!(serde_json::from_value::<S2Decision>(json!({"kind": "ProceedToS4"})).is_err());
    assert!(serde_json::from_value::<HypothesisStatus>(json!({"kind": "future-status"})).is_err());
    assert!(
        serde_json::from_value::<HypothesisStatus>(json!({
            "kind": "not-evaluated-due-to-prior-gate",
            "reason": "blocked",
            "unexpected": true,
        }))
        .is_err()
    );
}

proptest! {
    // The generated bundle space is only 10 booleans/status streams deep; 512
    // cases gives stable branch mixing without turning this review guard into a
    // slow fuzz target. Deterministic branch reachability is owned by
    // every_s2_outcome_is_reachable_via_dispatcher; this property guards totality
    // over generated combinations. Persistence is disabled so local concurrent
    // worktrees do not write proptest regression files outside this test's
    // ownership.
    #![proptest_config(ProptestConfig {
        cases: 512,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    #[test]
    fn dispatcher_is_total_and_decision_table_covers_every_generated_bundle(
        bundle in arb_bundle(),
    ) {
        let first = dispatch_outcome(&bundle);
        let second = dispatch_outcome(&bundle);
        prop_assert_eq!(first, second);
        prop_assert_eq!(first, expected_outcome_for_bundle(&bundle));
        prop_assert!(S2Outcome::ALL.contains(&first));
        let decision = decision_for_outcome(first);
        let decision_is_covered = matches!(
            decision,
            S2Decision::ProceedToS3
                | S2Decision::ProceedToS3WithDistillReview
                | S2Decision::Investigate { .. }
                | S2Decision::Halt { .. }
        );
        prop_assert!(decision_is_covered, "uncovered decision: {decision:?}");
    }

    #[test]
    fn proceed_decisions_require_binary_closure_hypotheses_and_completed_rows(
        bundle in arb_bundle(),
    ) {
        let outcome = dispatch_outcome(&bundle);
        let decision = decision_for_outcome(outcome);
        if matches!(
            decision,
            S2Decision::ProceedToS3 | S2Decision::ProceedToS3WithDistillReview
        ) {
            prop_assert!(bundle.completions.iter().all(|completion| matches!(completion, S2Completion::Completed)));
            prop_assert!(bundle.methodological_controls_present);
            for hypothesis in S2Hypothesis::ALL {
                prop_assert!(bundle.status(hypothesis).is_binary_closure_verdict());
            }
            prop_assert!(matches!(bundle.status(S2Hypothesis::H1), HypothesisStatus::Confirmed));
            prop_assert!(matches!(bundle.status(S2Hypothesis::H2), HypothesisStatus::Confirmed));
            prop_assert!(matches!(bundle.status(S2Hypothesis::H4), HypothesisStatus::Confirmed));
            prop_assert!(matches!(bundle.status(S2Hypothesis::H5), HypothesisStatus::Confirmed));
            prop_assert!(matches!(bundle.status(S2Hypothesis::H6), HypothesisStatus::Confirmed));
        }
    }
}

// Keep this oracle intentionally separate from dispatch_outcome. If the
// dispatcher order drifts, the proptest above should compare it against this
// independently spelled-out RFC round-4 intent rather than reusing production
// helper logic.
fn expected_outcome_for_bundle(bundle: &S2VerifierBundle) -> S2Outcome {
    if !bundle.preregistration_passed {
        S2Outcome::FailPreregistration
    } else if !bundle.artifact_integrity_passed {
        S2Outcome::FailArtifact
    } else if bundle.status(S2Hypothesis::H6) == HypothesisStatus::Refuted {
        S2Outcome::FailLinearstate
    } else if bundle.status(S2Hypothesis::H5) == HypothesisStatus::Refuted {
        S2Outcome::FailLossGradFlow
    } else if !bundle.phase_transition_integ_passed {
        S2Outcome::FailPhaseIntegration
    } else if !bundle.falsification_s2_passed {
        S2Outcome::FailFalsification
    } else if !bundle.oracle_re_run_passed {
        S2Outcome::FailMetric
    } else if !bundle.api_drift_check_passed {
        S2Outcome::FailApiDrift
    } else if bundle.any_seed_diverged()
        || bundle.status(S2Hypothesis::H1) == HypothesisStatus::Refuted
    {
        S2Outcome::FailSubstrate
    } else if bundle.status(S2Hypothesis::H4) == HypothesisStatus::Refuted {
        S2Outcome::FailPhase
    } else if bundle.suspicious_low_bpc {
        S2Outcome::FailSuspicious
    } else if bundle.status(S2Hypothesis::H2) == HypothesisStatus::Refuted {
        S2Outcome::FailGap
    } else if !bundle.methodological_controls_present
        || !bundle.status(S2Hypothesis::H3).is_binary_closure_verdict()
        || bundle.any_not_reached()
        || bundle.first_not_evaluated().is_some()
    {
        S2Outcome::FailIncomplete
    } else if bundle.status(S2Hypothesis::H3) == HypothesisStatus::Refuted {
        S2Outcome::PassWithDistillWarn
    } else {
        S2Outcome::PassClean
    }
}

fn set_status(bundle: &mut S2VerifierBundle, hypothesis: S2Hypothesis, status: HypothesisStatus) {
    bundle.hypothesis_statuses.insert(hypothesis, status);
}

fn arb_bundle() -> impl Strategy<Value = S2VerifierBundle> {
    (
        any::<bool>(),
        any::<bool>(),
        any::<bool>(),
        any::<bool>(),
        any::<bool>(),
        any::<bool>(),
        any::<bool>(),
        any::<bool>(),
        prop::collection::vec(arb_completion(), 15),
        arb_statuses(),
    )
        .prop_map(
            |(
                preregistration_passed,
                artifact_integrity_passed,
                oracle_re_run_passed,
                api_drift_check_passed,
                falsification_s2_passed,
                phase_transition_integ_passed,
                methodological_controls_present,
                suspicious_low_bpc,
                completions,
                hypothesis_statuses,
            )| S2VerifierBundle {
                preregistration_passed,
                artifact_integrity_passed,
                oracle_re_run_passed,
                api_drift_check_passed,
                falsification_s2_passed,
                phase_transition_integ_passed,
                methodological_controls_present,
                suspicious_low_bpc,
                completions,
                hypothesis_statuses,
            },
        )
}

fn arb_statuses() -> impl Strategy<Value = BTreeMap<S2Hypothesis, HypothesisStatus>> {
    (
        arb_hypothesis_status(),
        arb_hypothesis_status(),
        arb_hypothesis_status(),
        arb_hypothesis_status(),
        arb_hypothesis_status(),
        arb_hypothesis_status(),
    )
        .prop_map(|(h1, h2, h3, h4, h5, h6)| {
            BTreeMap::from([
                (S2Hypothesis::H1, h1),
                (S2Hypothesis::H2, h2),
                (S2Hypothesis::H3, h3),
                (S2Hypothesis::H4, h4),
                (S2Hypothesis::H5, h5),
                (S2Hypothesis::H6, h6),
            ])
        })
}

fn arb_hypothesis_status() -> impl Strategy<Value = HypothesisStatus> {
    prop_oneof![
        Just(HypothesisStatus::Confirmed),
        Just(HypothesisStatus::Refuted),
        "[a-z][a-z0-9_-]{0,16}"
            .prop_map(|reason| { HypothesisStatus::NotEvaluatedDueToPriorGate { reason } }),
    ]
}

fn arb_completion() -> impl Strategy<Value = S2Completion> {
    prop_oneof![
        Just(S2Completion::Completed),
        (1_u64..=10_000_u64).prop_map(|step| S2Completion::DivergedAt { step }),
        Just(S2Completion::NotReached),
    ]
}
