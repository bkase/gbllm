#![cfg(feature = "s7")]

use gbf_artifact::{S7Completion, S7Topology};
use gbf_experiments::s7::outcome::{
    MATCHED_BYTES_INVALID_REASON, S7Decision, S7Outcome, decision_for_s7_outcome,
};
use gbf_experiments::s7::report::{
    S7_REQUIRED_CLOSURE_ARTIFACTS, S7ArtifactHashStatus, S7ClosureArtifactKind,
    S7ClosureArtifactStatus, S7ClosureGateStatus, S7ClosureValidationError,
    S7ClosureValidationInput, S7PerSeedClosureArtifacts, validate_s7_closure,
};
use gbf_foundation::Hash256;

#[test]
fn validator_accepts_pass_clean_proceed_to_s8() {
    let input = clean_input(S7Outcome::PassClean);

    assert_eq!(
        validate_s7_closure(&input).expect("pass-clean closure"),
        S7Decision::ProceedToS8
    );
}

#[test]
fn validator_accepts_dense_only_only_for_valid_parity_failure() {
    let input = clean_input(S7Outcome::FailParity);

    assert_eq!(
        validate_s7_closure(&input).expect("dense-only parity closure"),
        S7Decision::ProceedToS8DenseOnly
    );

    let mut invalid_bytes = input.clone();
    invalid_bytes.bytes_within_tolerance = false;
    assert!(matches!(
        validate_s7_closure(&invalid_bytes),
        Err(S7ClosureValidationError::DenseOnlyNotPermitted {
            outcome: S7Outcome::FailParity,
            bytes_within_tolerance: false,
            per_seed_bpc_parity_failed: true,
        })
    ));

    let mut no_parity_loss = input;
    no_parity_loss.per_seed_bpc_parity_failed = false;
    assert!(matches!(
        validate_s7_closure(&no_parity_loss),
        Err(S7ClosureValidationError::DenseOnlyNotPermitted {
            outcome: S7Outcome::FailParity,
            bytes_within_tolerance: true,
            per_seed_bpc_parity_failed: false,
        })
    ));
}

#[test]
fn validator_requires_dense_emulator_only_for_dense_only_closure() {
    let pass_clean = clean_input(S7Outcome::PassClean);
    assert!(
        !pass_clean
            .required_artifacts
            .iter()
            .any(|artifact| artifact.kind == S7ClosureArtifactKind::EmulatorOneTokenDense)
    );
    validate_s7_closure(&pass_clean).expect("pass-clean does not require dense emulator");

    let mut dense_only = clean_input(S7Outcome::FailParity);
    dense_only
        .required_artifacts
        .retain(|artifact| artifact.kind != S7ClosureArtifactKind::EmulatorOneTokenDense);
    assert!(matches!(
        validate_s7_closure(&dense_only),
        Err(S7ClosureValidationError::MissingArtifact {
            kind: S7ClosureArtifactKind::EmulatorOneTokenDense,
        })
    ));
}

#[test]
fn validator_rejects_fail_bytes_dense_only_misuse_with_halt_expected() {
    let mut input = clean_input(S7Outcome::FailBytes);
    input.decision = S7Decision::ProceedToS8DenseOnly;

    assert_eq!(
        decision_for_s7_outcome(S7Outcome::FailBytes),
        S7Decision::Halt {
            reason: MATCHED_BYTES_INVALID_REASON,
        }
    );
    assert!(matches!(
        validate_s7_closure(&input),
        Err(S7ClosureValidationError::DecisionMismatch {
            outcome: S7Outcome::FailBytes,
            expected: S7Decision::Halt {
                reason: MATCHED_BYTES_INVALID_REASON,
            },
            observed: S7Decision::ProceedToS8DenseOnly,
        })
    ));
}

#[test]
fn validator_requires_all_ten_artifact_families_and_valid_self_hashes() {
    assert_eq!(S7_REQUIRED_CLOSURE_ARTIFACTS.len(), 10);

    let mut missing = clean_input(S7Outcome::PassClean);
    missing
        .required_artifacts
        .retain(|artifact| artifact.kind != S7ClosureArtifactKind::Report);
    assert!(matches!(
        validate_s7_closure(&missing),
        Err(S7ClosureValidationError::MissingArtifact {
            kind: S7ClosureArtifactKind::Report,
        })
    ));

    let mut invalid = clean_input(S7Outcome::PassClean);
    invalid
        .required_artifacts
        .iter_mut()
        .find(|artifact| artifact.kind == S7ClosureArtifactKind::OracleRouted)
        .expect("oracle artifact")
        .status = S7ArtifactHashStatus::present_invalid(hash(99));
    assert!(matches!(
        validate_s7_closure(&invalid),
        Err(S7ClosureValidationError::InvalidArtifactSelfHash {
            kind: S7ClosureArtifactKind::OracleRouted,
        })
    ));
}

#[test]
fn validator_rejects_incomplete_runs_and_invalid_per_seed_hashes() {
    let mut incomplete = clean_input(S7Outcome::PassClean);
    let row = incomplete
        .per_seed_artifacts
        .iter_mut()
        .find(|row| row.seed == 3 && row.topology == S7Topology::MoeTiny)
        .expect("moe seed 3");
    row.completion = S7Completion::CollapsedAt { step: 501 };
    assert!(matches!(
        validate_s7_closure(&incomplete),
        Err(S7ClosureValidationError::RunNotCompleted {
            seed: 3,
            topology: S7Topology::MoeTiny,
            completion: S7Completion::CollapsedAt { step: 501 },
        })
    ));

    let mut invalid_hash = clean_input(S7Outcome::PassClean);
    let row = invalid_hash
        .per_seed_artifacts
        .iter_mut()
        .find(|row| row.seed == 4 && row.topology == S7Topology::MoeTinyDenseMatched)
        .expect("dense seed 4");
    row.score_self_hash = S7ArtifactHashStatus::present_invalid(hash(77));
    assert!(matches!(
        validate_s7_closure(&invalid_hash),
        Err(S7ClosureValidationError::InvalidPerSeedArtifactSelfHash {
            seed: 4,
            topology: S7Topology::MoeTinyDenseMatched,
            field: "score_self_hash",
        })
    ));
}

#[test]
fn validator_rejects_refuted_unconditional_closure_gates() {
    let mut input = clean_input(S7Outcome::PassClean);
    input.gates.h7_loss_gradient_provenance_confirmed = false;

    assert!(matches!(
        validate_s7_closure(&input),
        Err(S7ClosureValidationError::GateRefuted { gate: "H7" })
    ));
}

fn clean_input(outcome: S7Outcome) -> S7ClosureValidationInput {
    let mut required_artifacts = required_artifacts();
    if outcome == S7Outcome::FailParity {
        required_artifacts.push(dense_emulator_artifact());
    }

    S7ClosureValidationInput {
        outcome,
        decision: decision_for_s7_outcome(outcome),
        bytes_within_tolerance: true,
        per_seed_bpc_parity_failed: outcome == S7Outcome::FailParity,
        predictions_verified: true,
        gates: S7ClosureGateStatus::all_confirmed(),
        per_seed_artifacts: completed_run_rows(),
        required_artifacts,
    }
}

fn completed_run_rows() -> Vec<S7PerSeedClosureArtifacts> {
    let mut rows = Vec::new();
    for seed in 0..5 {
        for topology in [S7Topology::MoeTiny, S7Topology::MoeTinyDenseMatched] {
            let base = 10
                + seed as u8 * 2
                + match topology {
                    S7Topology::MoeTiny => 0,
                    S7Topology::MoeTinyDenseMatched => 1,
                };
            rows.push(S7PerSeedClosureArtifacts {
                seed,
                topology,
                completion: S7Completion::Completed,
                checkpoint_self_hash: S7ArtifactHashStatus::present_valid(hash(base)),
                run_log_self_hash: S7ArtifactHashStatus::present_valid(hash(base + 40)),
                score_self_hash: S7ArtifactHashStatus::present_valid(hash(base + 80)),
            });
        }
    }
    rows
}

fn required_artifacts() -> Vec<S7ClosureArtifactStatus> {
    S7_REQUIRED_CLOSURE_ARTIFACTS
        .iter()
        .enumerate()
        .map(|(index, kind)| S7ClosureArtifactStatus {
            kind: *kind,
            status: S7ArtifactHashStatus::present_valid(hash(150 + index as u8)),
        })
        .collect()
}

fn dense_emulator_artifact() -> S7ClosureArtifactStatus {
    S7ClosureArtifactStatus {
        kind: S7ClosureArtifactKind::EmulatorOneTokenDense,
        status: S7ArtifactHashStatus::present_valid(hash(220)),
    }
}

fn hash(fill: u8) -> Hash256 {
    Hash256::from_bytes([fill; 32])
}
