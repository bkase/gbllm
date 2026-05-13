mod common;

use gbf_experiments::s2::run::state_machine::{
    PreTrainGateResults, State, run_pretrain_state_machine, validate_transition,
};
use gbf_experiments::s2::schema::{S2Decision, S2Outcome};

use crate::common::helpers::tracing_capture_s2::capture_events;

#[test]
fn happy_path_reaches_decided_after_ablation_attempted() {
    let (run, events) =
        capture_events(|| run_pretrain_state_machine(PreTrainGateResults::default()));

    assert_eq!(run.final_state, State::Decided);
    assert_eq!(run.outcome, S2Outcome::PassClean);
    assert_eq!(run.decision, S2Decision::ProceedToS3);
    assert!(run.train_attempted);
    assert!(run.transitions.iter().any(|transition| {
        transition.from == State::GapComputed && transition.to == State::AblationAttempted
    }));
    assert_eq!(
        events
            .iter()
            .filter(|event| event.name == "state_transition")
            .count(),
        run.transitions.len()
    );
    insta::assert_snapshot!(
        "state_machine__happy_path_tiny",
        run.transitions
            .iter()
            .map(|transition| format!("{} -> {}", transition.from, transition.to))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn early_h6_failure_reports_without_training() {
    let gates = PreTrainGateResults {
        linearstate_smoke_passed: false,
        ..PreTrainGateResults::default()
    };
    let (run, events) = capture_events(|| run_pretrain_state_machine(gates));

    assert_eq!(run.final_state, State::Decided);
    assert_eq!(run.outcome, S2Outcome::FailLinearstate);
    assert!(matches!(run.decision, S2Decision::Investigate { .. }));
    assert!(!run.train_attempted);
    assert!(run.transitions.iter().all(|transition| {
        transition.from != State::FalsificationChecked && transition.to != State::TrainAttempted
    }));
    assert_eq!(
        events
            .iter()
            .filter(|event| event.name == "state_transition_failed")
            .count(),
        1
    );
    let cleanup = run
        .transitions
        .iter()
        .filter(|transition| transition.to == State::Reported || transition.to == State::Decided)
        .collect::<Vec<_>>();
    assert_eq!(cleanup.len(), 2);
    assert!(
        cleanup.iter().all(|transition| !transition.success),
        "post-failure report/decision cleanup transitions should not be logged as successful progress"
    );
    let cleanup_events = events
        .iter()
        .filter(|event| {
            event.name == "state_transition"
                && event
                    .fields
                    .get("success")
                    .and_then(serde_json::Value::as_bool)
                    == Some(false)
        })
        .count();
    assert_eq!(cleanup_events, cleanup.len());
}

#[test]
fn failure_branches_map_to_expected_outcomes() {
    for (mut gates, expected) in [
        (
            PreTrainGateResults {
                loss_grad_flow_passed: false,
                ..PreTrainGateResults::default()
            },
            S2Outcome::FailLossGradFlow,
        ),
        (
            PreTrainGateResults {
                phase_transition_integ_passed: false,
                ..PreTrainGateResults::default()
            },
            S2Outcome::FailPhaseIntegration,
        ),
        (
            PreTrainGateResults {
                oracle_re_run_passed: false,
                ..PreTrainGateResults::default()
            },
            S2Outcome::FailMetric,
        ),
        (
            PreTrainGateResults {
                api_drift_check_passed: false,
                ..PreTrainGateResults::default()
            },
            S2Outcome::FailApiDrift,
        ),
        (
            PreTrainGateResults {
                falsification_s2_passed: false,
                ..PreTrainGateResults::default()
            },
            S2Outcome::FailFalsification,
        ),
    ] {
        gates.linearstate_smoke_passed = true;
        let run = run_pretrain_state_machine(gates);
        assert_eq!(run.outcome, expected);
        assert!(!run.train_attempted);
    }
}

#[test]
fn ablation_not_required_skips_ablation_and_still_proceeds() {
    let gates = PreTrainGateResults {
        ablation_required: false,
        ..PreTrainGateResults::default()
    };

    let run = run_pretrain_state_machine(gates);

    assert_eq!(run.final_state, State::Decided);
    assert_eq!(run.outcome, S2Outcome::PassClean);
    assert_eq!(run.decision, S2Decision::ProceedToS3);
    assert!(run.train_attempted);
    assert!(run.transitions.iter().any(|transition| {
        transition.from == State::GapComputed && transition.to == State::Reported
    }));
    assert!(run.transitions.iter().all(|transition| {
        transition.from != State::GapComputed || transition.to != State::AblationAttempted
    }));
    assert!(run.transitions.iter().all(|transition| {
        transition.from != State::AblationAttempted && transition.to != State::AblationCompared
    }));
    assert!(run.transitions.iter().all(|transition| transition.success));
}

#[test]
fn ablation_attempted_only_after_gap_computed() {
    assert!(validate_transition(State::GapComputed, State::AblationAttempted).is_ok());
    assert!(validate_transition(State::Scored, State::AblationAttempted).is_err());
    assert!(validate_transition(State::FalsificationChecked, State::TrainAttempted).is_ok());
}
