mod common;

use common::tracing_capture::{TraceCapture, captured_events, with_trace_capture};
use gbf_experiments::s2::report::{decision_for_outcome, dispatch_outcome};
use gbf_experiments::s2::schema::{
    HypothesisStatus, S2Completion, S2Decision, S2Hypothesis, S2Outcome, S2VerifierBundle,
};
use serde_json::json;

type MutateBundle = fn(&mut S2VerifierBundle);
type FailIncompleteAliasCase = (
    &'static str,
    MutateBundle,
    &'static str,
    Option<WarnExpectation>,
);
type WarnExpectation = (&'static str, &'static str);

#[test]
fn pass_clean_canonical_bundle_dispatches_to_proceed() {
    let capture = TraceCapture::default();
    let bundle = S2VerifierBundle::closure_candidate();

    let outcome = with_trace_capture(&capture, || dispatch_outcome(&bundle));

    assert_eq!(outcome, S2Outcome::PassClean);
    assert_eq!(decision_for_outcome(outcome), S2Decision::ProceedToS3);
    assert_single_decision_event(&capture, "Pass-clean", "ProceedToS3");
    insta::with_settings!({prepend_module_to_snapshot => false}, {
        insta::assert_snapshot!(
            "outcome_dispatch__pass_clean_canonical",
            serde_json::to_string_pretty(&json!({
                "outcome": outcome,
                "decision": decision_for_outcome(outcome),
            }))
            .unwrap()
        );
    });
}

#[test]
fn h3_refuted_is_distill_warning_after_all_hard_gates_pass() {
    let mut bundle = S2VerifierBundle::closure_candidate();
    set_status(&mut bundle, S2Hypothesis::H3, HypothesisStatus::Refuted);

    let outcome = dispatch_outcome(&bundle);

    assert_eq!(outcome, S2Outcome::PassWithDistillWarn);
    assert_eq!(
        decision_for_outcome(outcome),
        S2Decision::ProceedToS3WithDistillReview
    );
}

#[test]
fn early_h6_failure_is_not_misclassified_as_metric_or_artifact() {
    let mut bundle = S2VerifierBundle::closure_candidate();
    set_status(&mut bundle, S2Hypothesis::H6, HypothesisStatus::Refuted);
    bundle.oracle_re_run_passed = false;
    bundle.artifact_integrity_passed = true;

    let outcome = dispatch_outcome(&bundle);

    assert_eq!(outcome, S2Outcome::FailLinearstate);
}

#[test]
fn missing_nodistill_control_after_gates_is_fail_incomplete() {
    let mut bundle = S2VerifierBundle::closure_candidate();
    bundle.methodological_controls_present = false;

    let outcome = dispatch_outcome(&bundle);

    assert_eq!(outcome, S2Outcome::FailIncomplete);
    insta::with_settings!({prepend_module_to_snapshot => false}, {
        insta::assert_snapshot!(
            "outcome_dispatch__fail_incomplete_nodistill",
            serde_json::to_string_pretty(&json!({
                "outcome": outcome,
                "decision": decision_for_outcome(outcome),
            }))
            .unwrap()
        );
    });
}

#[test]
fn earlier_gate_explains_missing_control() {
    let mut bundle = S2VerifierBundle::closure_candidate();
    bundle.methodological_controls_present = false;
    set_status(&mut bundle, S2Hypothesis::H6, HypothesisStatus::Refuted);

    assert_eq!(dispatch_outcome(&bundle), S2Outcome::FailLinearstate);
}

#[test]
fn fail_incomplete_aliases_one_payload_but_keeps_distinct_branch_evidence() {
    let cases: [FailIncompleteAliasCase; 3] = [
        (
            "missing_h3_control",
            |b| b.methodological_controls_present = false,
            "h3_incomplete",
            None,
        ),
        (
            "not_reached_row",
            |b| b.completions[0] = S2Completion::NotReached,
            "not_reached",
            None,
        ),
        (
            "h2_not_evaluated_status",
            |b| {
                set_status(
                    b,
                    S2Hypothesis::H2,
                    HypothesisStatus::NotEvaluatedDueToPriorGate {
                        reason: "h2-input-validation-skipped".to_owned(),
                    },
                );
            },
            "unbinary_hypothesis",
            Some((
                "H2",
                "NotEvaluatedDueToPriorGate(h2-input-validation-skipped)",
            )),
        ),
    ];

    for (name, mutate, branch_id, expected_warn) in cases {
        let capture = TraceCapture::default();
        let mut bundle = S2VerifierBundle::closure_candidate();
        mutate(&mut bundle);

        let outcome = with_trace_capture(&capture, || dispatch_outcome(&bundle));

        assert_eq!(outcome, S2Outcome::FailIncomplete, "{name}");
        assert_eq!(
            decision_for_outcome(outcome),
            S2Decision::Halt {
                reason: "required-methodological-control-missing".to_owned()
            },
            "{name}"
        );
        assert_matched_branch(&capture, branch_id);
        if let Some((hypothesis_id, status)) = expected_warn {
            assert_unbinary_warn_event(&capture, hypothesis_id, status);
        } else {
            assert_no_unbinary_warn_event(&capture, name);
        }
    }
}

#[test]
fn round4_branch_order_is_pinned() {
    let cases: [(&str, MutateBundle, S2Outcome); 15] = [
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
            "h6",
            |b| set_status(b, S2Hypothesis::H6, HypothesisStatus::Refuted),
            S2Outcome::FailLinearstate,
        ),
        (
            "h5",
            |b| set_status(b, S2Hypothesis::H5, HypothesisStatus::Refuted),
            S2Outcome::FailLossGradFlow,
        ),
        (
            "d8",
            |b| b.phase_transition_integ_passed = false,
            S2Outcome::FailPhaseIntegration,
        ),
        (
            "falsification",
            |b| b.falsification_s2_passed = false,
            S2Outcome::FailFalsification,
        ),
        (
            "oracle",
            |b| b.oracle_re_run_passed = false,
            S2Outcome::FailMetric,
        ),
        (
            "api",
            |b| b.api_drift_check_passed = false,
            S2Outcome::FailApiDrift,
        ),
        (
            "substrate_diverged",
            |b| b.completions[0] = S2Completion::DivergedAt { step: 17 },
            S2Outcome::FailSubstrate,
        ),
        (
            "substrate_h1",
            |b| set_status(b, S2Hypothesis::H1, HypothesisStatus::Refuted),
            S2Outcome::FailSubstrate,
        ),
        (
            "h4",
            |b| set_status(b, S2Hypothesis::H4, HypothesisStatus::Refuted),
            S2Outcome::FailPhase,
        ),
        (
            "suspicious",
            |b| b.suspicious_low_bpc = true,
            S2Outcome::FailSuspicious,
        ),
        (
            "h2",
            |b| set_status(b, S2Hypothesis::H2, HypothesisStatus::Refuted),
            S2Outcome::FailGap,
        ),
        (
            "not_reached",
            |b| b.completions[1] = S2Completion::NotReached,
            S2Outcome::FailIncomplete,
        ),
        (
            "h3_refuted",
            |b| set_status(b, S2Hypothesis::H3, HypothesisStatus::Refuted),
            S2Outcome::PassWithDistillWarn,
        ),
    ];

    for (name, mutate, expected) in cases {
        let mut bundle = S2VerifierBundle::closure_candidate();
        mutate(&mut bundle);
        assert_eq!(dispatch_outcome(&bundle), expected, "{name}");
    }
}

fn set_status(bundle: &mut S2VerifierBundle, hypothesis: S2Hypothesis, status: HypothesisStatus) {
    bundle.hypothesis_statuses.insert(hypothesis, status);
}

fn assert_matched_branch(capture: &TraceCapture, branch_id: &str) {
    let events = captured_events(capture);
    assert!(
        events.iter().any(|event| {
            event.name == "outcome_branch_evaluated"
                && event.fields.get("branch_id") == Some(&json!(branch_id))
                && event.fields.get("matched") == Some(&json!(true))
        }),
        "missing matched branch {branch_id:?} in {events:#?}"
    );
}

fn assert_unbinary_warn_event(capture: &TraceCapture, hypothesis_id: &str, status: &str) {
    let events = captured_events(capture);
    let event = events
        .iter()
        .find(|event| event.name == "outcome_unbinary_hypothesis_at_closure")
        .unwrap_or_else(|| panic!("missing unbinary hypothesis warning in {events:#?}"));
    assert_eq!(event.level, "WARN");
    assert_eq!(
        event.fields.get("hypothesis_id"),
        Some(&json!(hypothesis_id))
    );
    assert_eq!(event.fields.get("status"), Some(&json!(status)));
}

fn assert_no_unbinary_warn_event(capture: &TraceCapture, name: &str) {
    let events = captured_events(capture);
    assert!(
        events
            .iter()
            .all(|event| event.name != "outcome_unbinary_hypothesis_at_closure"),
        "{name} should not emit unbinary warning events: {events:#?}"
    );
}

fn assert_single_decision_event(capture: &TraceCapture, outcome: &str, decision: &str) {
    let events = captured_events(capture);
    assert_eq!(
        events
            .iter()
            .filter(|event| event.name == "outcome_dispatch_decided")
            .count(),
        1
    );
    let event = events
        .iter()
        .find(|event| event.name == "outcome_dispatch_decided")
        .unwrap();
    assert_eq!(event.fields.get("outcome"), Some(&json!(outcome)));
    assert_eq!(event.fields.get("decision"), Some(&json!(decision)));
}
