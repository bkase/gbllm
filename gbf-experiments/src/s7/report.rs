//! S7 closure-report validation helpers.

use std::collections::BTreeSet;
use std::fmt;

use gbf_artifact::{S7Completion, S7Topology};
use gbf_foundation::Hash256;

use crate::s7::outcome::{
    S7Decision, S7Outcome, decision_for_s7_outcome, dense_only_closure_permitted,
};

/// The ten artifact families named by the §21 final contract.
pub const S7_REQUIRED_CLOSURE_ARTIFACTS: [S7ClosureArtifactKind; 10] = [
    S7ClosureArtifactKind::RunLog,
    S7ClosureArtifactKind::Score,
    S7ClosureArtifactKind::SwitchStats,
    S7ClosureArtifactKind::RouterCollapseSweep,
    S7ClosureArtifactKind::DenseVsMoe,
    S7ClosureArtifactKind::Frontier,
    S7ClosureArtifactKind::BurnGradSmoke,
    S7ClosureArtifactKind::OracleRouted,
    S7ClosureArtifactKind::EmulatorOneTokenMoe,
    S7ClosureArtifactKind::Report,
];

/// Top-level S7 closure artifact family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum S7ClosureArtifactKind {
    /// Per-seed `s7_run_log.v1` artifacts.
    RunLog,
    /// Per-seed `s7_score.v1` artifacts.
    Score,
    /// Aggregate `s7_switch_stats.v1` artifact.
    SwitchStats,
    /// `s7_router_collapse_sweep.v1` guardrail artifact.
    RouterCollapseSweep,
    /// `s7_dense_vs_moe.v1` comparison artifact.
    DenseVsMoe,
    /// `s7_frontier.v1` Pareto frontier artifact.
    Frontier,
    /// `s7_burn_grad_smoke.v1` H8 artifact.
    BurnGradSmoke,
    /// `s7_oracle_routed.v1` H9 artifact.
    OracleRouted,
    /// `s7_emulator_one_token.v1` MoE H10 artifact.
    EmulatorOneTokenMoe,
    /// `s7_emulator_one_token.v1` dense carry-through artifact for DenseOnly closure.
    EmulatorOneTokenDense,
    /// `s7_report.v1` closure report.
    Report,
}

impl S7ClosureArtifactKind {
    /// Stable field/family name used in validation errors.
    #[must_use]
    pub const fn field_name(self) -> &'static str {
        match self {
            Self::RunLog => "s7_run_log",
            Self::Score => "s7_score",
            Self::SwitchStats => "s7_switch_stats",
            Self::RouterCollapseSweep => "s7_router_collapse_sweep",
            Self::DenseVsMoe => "s7_dense_vs_moe",
            Self::Frontier => "s7_frontier",
            Self::BurnGradSmoke => "s7_burn_grad_smoke",
            Self::OracleRouted => "s7_oracle_routed",
            Self::EmulatorOneTokenMoe => "s7_emulator_one_token_moe",
            Self::EmulatorOneTokenDense => "s7_emulator_one_token_dense",
            Self::Report => "s7_report",
        }
    }
}

/// Presence plus self-hash validation status for one artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct S7ArtifactHashStatus {
    /// Recorded self-hash, when the artifact exists.
    pub self_hash: Option<Hash256>,
    /// Whether the recorded self-hash recomputed cleanly.
    pub self_hash_valid: bool,
}

impl S7ArtifactHashStatus {
    /// Construct a present artifact with a valid self-hash.
    #[must_use]
    pub fn present_valid(self_hash: Hash256) -> Self {
        Self {
            self_hash: Some(self_hash),
            self_hash_valid: true,
        }
    }

    /// Construct a missing artifact status.
    #[must_use]
    pub const fn missing() -> Self {
        Self {
            self_hash: None,
            self_hash_valid: false,
        }
    }

    /// Construct a present artifact whose self-hash validation failed.
    #[must_use]
    pub fn present_invalid(self_hash: Hash256) -> Self {
        Self {
            self_hash: Some(self_hash),
            self_hash_valid: false,
        }
    }

    /// Whether a self-hash value was recorded.
    #[must_use]
    pub const fn is_present(self) -> bool {
        self.self_hash.is_some()
    }

    /// Whether a self-hash value was recorded and verified.
    #[must_use]
    pub const fn is_present_and_valid(self) -> bool {
        self.self_hash.is_some() && self.self_hash_valid
    }
}

/// Status for one required artifact family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct S7ClosureArtifactStatus {
    /// Artifact family being reported.
    pub kind: S7ClosureArtifactKind,
    /// Presence and self-hash validation state for this family.
    pub status: S7ArtifactHashStatus,
}

/// Per-(topology, seed) closure row from `s7_report.v1` front matter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S7PerSeedClosureArtifacts {
    /// Training seed.
    pub seed: u64,
    /// S7 topology for this row.
    pub topology: S7Topology,
    /// Run completion state.
    pub completion: S7Completion,
    /// Final checkpoint self-hash state.
    pub checkpoint_self_hash: S7ArtifactHashStatus,
    /// Run-log self-hash state.
    pub run_log_self_hash: S7ArtifactHashStatus,
    /// Score artifact self-hash state.
    pub score_self_hash: S7ArtifactHashStatus,
}

/// Unconditional closure gates from §15.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct S7ClosureGateStatus {
    /// H5 switch statistics confirmed.
    pub h5_switch_stats_confirmed: bool,
    /// H6 router-collapse guardrail confirmed.
    pub h6_router_collapse_guardrail_confirmed: bool,
    /// H7 loss-gradient provenance confirmed.
    pub h7_loss_gradient_provenance_confirmed: bool,
    /// H8 Burn gradient smoke confirmed.
    pub h8_burn_gradient_confirmed: bool,
    /// H9 routed oracle agreement confirmed.
    pub h9_oracle_routed_confirmed: bool,
    /// H10 routed emulator check confirmed.
    pub h10_emulator_routed_confirmed: bool,
}

impl S7ClosureGateStatus {
    /// Construct the all-confirmed gate state for closure-candidate reports.
    #[must_use]
    pub const fn all_confirmed() -> Self {
        Self {
            h5_switch_stats_confirmed: true,
            h6_router_collapse_guardrail_confirmed: true,
            h7_loss_gradient_provenance_confirmed: true,
            h8_burn_gradient_confirmed: true,
            h9_oracle_routed_confirmed: true,
            h10_emulator_routed_confirmed: true,
        }
    }
}

/// Inputs needed to validate §15 closure and §21 rule 12.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S7ClosureValidationInput {
    /// S7 outcome selected by §12 dispatch.
    pub outcome: S7Outcome,
    /// Decision recorded in report front matter.
    pub decision: S7Decision,
    /// Whether matched deployed bytes are within D6 tolerance.
    pub bytes_within_tolerance: bool,
    /// Whether per-seed bpc parity failed under valid matched bytes.
    pub per_seed_bpc_parity_failed: bool,
    /// Whether pre-registration ancestry was verified.
    pub predictions_verified: bool,
    /// Unconditional closure gate statuses.
    pub gates: S7ClosureGateStatus,
    /// Ten per-(topology, seed) front-matter rows.
    pub per_seed_artifacts: Vec<S7PerSeedClosureArtifacts>,
    /// Required artifact family statuses.
    pub required_artifacts: Vec<S7ClosureArtifactStatus>,
}

/// Validate whether an S7 report may close bd-2v9r under §15/§21.
pub fn validate_s7_closure(
    input: &S7ClosureValidationInput,
) -> Result<S7Decision, S7ClosureValidationError> {
    let expected = decision_for_s7_outcome(input.outcome);
    if input.decision != expected {
        return Err(S7ClosureValidationError::DecisionMismatch {
            outcome: input.outcome,
            expected,
            observed: input.decision,
        });
    }

    match input.decision {
        S7Decision::ProceedToS8 => {}
        S7Decision::ProceedToS8DenseOnly => {
            if !dense_only_closure_permitted(
                input.outcome,
                input.bytes_within_tolerance,
                input.per_seed_bpc_parity_failed,
            ) {
                return Err(S7ClosureValidationError::DenseOnlyNotPermitted {
                    outcome: input.outcome,
                    bytes_within_tolerance: input.bytes_within_tolerance,
                    per_seed_bpc_parity_failed: input.per_seed_bpc_parity_failed,
                });
            }
        }
        S7Decision::Halt { .. } | S7Decision::Investigate { .. } => {
            return Err(S7ClosureValidationError::ClosureDecisionForbidden {
                decision: input.decision,
            });
        }
    }

    if !input.predictions_verified {
        return Err(S7ClosureValidationError::PredictionsNotVerified);
    }
    if !input.bytes_within_tolerance {
        return Err(S7ClosureValidationError::MatchedBytesOutsideTolerance);
    }

    validate_gates(input.gates)?;
    validate_per_seed_artifacts(&input.per_seed_artifacts)?;
    validate_required_artifacts(
        &input.required_artifacts,
        matches!(input.decision, S7Decision::ProceedToS8DenseOnly),
    )?;

    Ok(input.decision)
}

/// Validation failures for S7 closure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum S7ClosureValidationError {
    /// Report decision did not match the §12 outcome dispatch.
    DecisionMismatch {
        /// Outcome used for dispatch.
        outcome: S7Outcome,
        /// Expected decision from the outcome.
        expected: S7Decision,
        /// Decision recorded by the report.
        observed: S7Decision,
    },
    /// The matched decision is not a closure-permitting decision.
    ClosureDecisionForbidden {
        /// Forbidden decision.
        decision: S7Decision,
    },
    /// Dense-only closure was requested outside its narrow valid parity path.
    DenseOnlyNotPermitted {
        /// Outcome recorded by the report.
        outcome: S7Outcome,
        /// Whether matched bytes were within tolerance.
        bytes_within_tolerance: bool,
        /// Whether bpc parity failed per seed.
        per_seed_bpc_parity_failed: bool,
    },
    /// Pre-registration history was not verified.
    PredictionsNotVerified,
    /// Matched deployed byte accounting exceeded D6 tolerance.
    MatchedBytesOutsideTolerance,
    /// One of H5-H10 was refuted.
    GateRefuted {
        /// Gate name, e.g. `H7`.
        gate: &'static str,
    },
    /// Required per-seed row was absent.
    MissingPerSeedRow {
        /// Missing seed.
        seed: u64,
        /// Missing topology.
        topology: S7Topology,
    },
    /// A per-seed row did not complete successfully.
    RunNotCompleted {
        /// Seed for the incomplete run.
        seed: u64,
        /// Topology for the incomplete run.
        topology: S7Topology,
        /// Observed completion.
        completion: S7Completion,
    },
    /// A required per-seed artifact self-hash was missing.
    MissingPerSeedArtifact {
        /// Seed for the missing artifact.
        seed: u64,
        /// Topology for the missing artifact.
        topology: S7Topology,
        /// Missing field name.
        field: &'static str,
    },
    /// A required per-seed artifact self-hash failed validation.
    InvalidPerSeedArtifactSelfHash {
        /// Seed for the invalid artifact.
        seed: u64,
        /// Topology for the invalid artifact.
        topology: S7Topology,
        /// Invalid field name.
        field: &'static str,
    },
    /// A required artifact family was missing.
    MissingArtifact {
        /// Missing artifact kind.
        kind: S7ClosureArtifactKind,
    },
    /// A required artifact family was duplicated.
    DuplicateArtifact {
        /// Duplicated artifact kind.
        kind: S7ClosureArtifactKind,
    },
    /// A required artifact family self-hash failed validation.
    InvalidArtifactSelfHash {
        /// Invalid artifact kind.
        kind: S7ClosureArtifactKind,
    },
}

impl fmt::Display for S7ClosureValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DecisionMismatch {
                outcome,
                expected,
                observed,
            } => write!(
                f,
                "S7 decision mismatch for {outcome:?}: expected {expected:?}, observed {observed:?}"
            ),
            Self::ClosureDecisionForbidden { decision } => {
                write!(f, "S7 closure forbidden for decision {decision:?}")
            }
            Self::DenseOnlyNotPermitted {
                outcome,
                bytes_within_tolerance,
                per_seed_bpc_parity_failed,
            } => write!(
                f,
                "ProceedToS8DenseOnly not permitted for {outcome:?}: bytes_within_tolerance={bytes_within_tolerance}, per_seed_bpc_parity_failed={per_seed_bpc_parity_failed}"
            ),
            Self::PredictionsNotVerified => write!(f, "S7 pre-registration was not verified"),
            Self::MatchedBytesOutsideTolerance => {
                write!(f, "matched deployed bytes are outside D6 tolerance")
            }
            Self::GateRefuted { gate } => write!(f, "{gate} closure gate was refuted"),
            Self::MissingPerSeedRow { seed, topology } => {
                write!(f, "missing per-seed row for {topology:?} seed {seed}")
            }
            Self::RunNotCompleted {
                seed,
                topology,
                completion,
            } => write!(
                f,
                "{topology:?} seed {seed} is not Completed: {completion:?}"
            ),
            Self::MissingPerSeedArtifact {
                seed,
                topology,
                field,
            } => write!(f, "missing {field} for {topology:?} seed {seed}"),
            Self::InvalidPerSeedArtifactSelfHash {
                seed,
                topology,
                field,
            } => write!(f, "invalid {field} self-hash for {topology:?} seed {seed}"),
            Self::MissingArtifact { kind } => write!(f, "missing {}", kind.field_name()),
            Self::DuplicateArtifact { kind } => {
                write!(f, "duplicate {}", kind.field_name())
            }
            Self::InvalidArtifactSelfHash { kind } => {
                write!(f, "invalid {} self-hash", kind.field_name())
            }
        }
    }
}

impl std::error::Error for S7ClosureValidationError {}

fn validate_gates(gates: S7ClosureGateStatus) -> Result<(), S7ClosureValidationError> {
    let checks = [
        ("H5", gates.h5_switch_stats_confirmed),
        ("H6", gates.h6_router_collapse_guardrail_confirmed),
        ("H7", gates.h7_loss_gradient_provenance_confirmed),
        ("H8", gates.h8_burn_gradient_confirmed),
        ("H9", gates.h9_oracle_routed_confirmed),
        ("H10", gates.h10_emulator_routed_confirmed),
    ];
    for (gate, confirmed) in checks {
        if !confirmed {
            return Err(S7ClosureValidationError::GateRefuted { gate });
        }
    }
    Ok(())
}

fn validate_per_seed_artifacts(
    rows: &[S7PerSeedClosureArtifacts],
) -> Result<(), S7ClosureValidationError> {
    for seed in 0..5 {
        for topology in [S7Topology::MoeTiny, S7Topology::MoeTinyDenseMatched] {
            let Some(row) = rows
                .iter()
                .find(|row| row.seed == seed && row.topology == topology)
            else {
                return Err(S7ClosureValidationError::MissingPerSeedRow { seed, topology });
            };
            if row.completion != S7Completion::Completed {
                return Err(S7ClosureValidationError::RunNotCompleted {
                    seed,
                    topology,
                    completion: row.completion.clone(),
                });
            }
            validate_row_hash(row, "checkpoint_self_hash", row.checkpoint_self_hash)?;
            validate_row_hash(row, "run_log_self_hash", row.run_log_self_hash)?;
            validate_row_hash(row, "score_self_hash", row.score_self_hash)?;
        }
    }
    Ok(())
}

fn validate_row_hash(
    row: &S7PerSeedClosureArtifacts,
    field: &'static str,
    status: S7ArtifactHashStatus,
) -> Result<(), S7ClosureValidationError> {
    if !status.is_present() {
        return Err(S7ClosureValidationError::MissingPerSeedArtifact {
            seed: row.seed,
            topology: row.topology.clone(),
            field,
        });
    }
    if !status.is_present_and_valid() {
        return Err(S7ClosureValidationError::InvalidPerSeedArtifactSelfHash {
            seed: row.seed,
            topology: row.topology.clone(),
            field,
        });
    }
    Ok(())
}

fn validate_required_artifacts(
    artifacts: &[S7ClosureArtifactStatus],
    require_dense_emulator: bool,
) -> Result<(), S7ClosureValidationError> {
    let mut seen = BTreeSet::new();
    for artifact in artifacts {
        if !seen.insert(artifact.kind) {
            return Err(S7ClosureValidationError::DuplicateArtifact {
                kind: artifact.kind,
            });
        }
        if !artifact.status.is_present() {
            return Err(S7ClosureValidationError::MissingArtifact {
                kind: artifact.kind,
            });
        }
        if !artifact.status.is_present_and_valid() {
            return Err(S7ClosureValidationError::InvalidArtifactSelfHash {
                kind: artifact.kind,
            });
        }
    }

    for kind in S7_REQUIRED_CLOSURE_ARTIFACTS {
        if !seen.contains(&kind) {
            return Err(S7ClosureValidationError::MissingArtifact { kind });
        }
    }
    if require_dense_emulator && !seen.contains(&S7ClosureArtifactKind::EmulatorOneTokenDense) {
        return Err(S7ClosureValidationError::MissingArtifact {
            kind: S7ClosureArtifactKind::EmulatorOneTokenDense,
        });
    }

    Ok(())
}
