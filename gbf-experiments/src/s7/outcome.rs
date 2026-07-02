//! S7 outcome algebra helpers.

/// Aggregate §11.2 per-seed parity verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AggregateParityVerdict {
    /// All per-seed parity checks passed under valid matched bytes.
    PassClean,
    /// Matched bytes were valid, but at least one per-seed bpc parity check failed.
    FailParity,
    /// Matched-deployed-byte tolerance was violated, making the comparison invalid.
    FailBytes,
}

/// S7 outcome variants from §12.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum S7Outcome {
    /// All mandatory S7 hypotheses confirmed.
    PassClean,
    /// MoE training failed through divergence or non-collapse H1 failure.
    FailMoeTrain,
    /// MoE training hit the production-lambda router-collapse halt.
    FailRouterCollapse,
    /// Dense matched baseline training failed.
    FailDenseBaseline,
    /// Valid matched-bytes parity scientifically falsified the MoE-wins claim.
    FailParity,
    /// Matched-deployed-byte tolerance was violated.
    FailBytes,
    /// H4 Pareto closure failed under valid matched bytes.
    FailPareto,
    /// H5 switch-statistics closure failed.
    FailSwitchStats,
    /// H6 router-collapse guardrail failed.
    FailRouterCollapseGuardrail,
    /// H7 gradient provenance failed.
    FailGradProvenance,
    /// H8 Burn gradient smoke failed.
    FailBurnGrad,
    /// H9 routed artifact oracle agreement failed.
    FailOracleRouted,
    /// H10 emulator routed check failed.
    FailEmulatorRouted,
    /// MoE median validation bpc was suspiciously low.
    FailSuspicious,
}

impl S7Outcome {
    /// All active S7 outcome variants; there is intentionally no pass-with-warning slot.
    pub const ALL: [Self; 14] = [
        Self::PassClean,
        Self::FailMoeTrain,
        Self::FailRouterCollapse,
        Self::FailDenseBaseline,
        Self::FailParity,
        Self::FailBytes,
        Self::FailPareto,
        Self::FailSwitchStats,
        Self::FailRouterCollapseGuardrail,
        Self::FailGradProvenance,
        Self::FailBurnGrad,
        Self::FailOracleRouted,
        Self::FailEmulatorRouted,
        Self::FailSuspicious,
    ];
}

/// §12 decision produced from one S7 outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum S7Decision {
    /// Proceed to S8 with MoE still viable.
    ProceedToS8,
    /// Proceed to S8 dense-only after valid scientific parity falsification.
    ProceedToS8DenseOnly,
    /// Investigation is required before S7 closure.
    Investigate {
        /// Stable machine-readable reason tag.
        reason: &'static str,
    },
    /// Closure must halt because the experiment or artifact is invalid.
    Halt {
        /// Stable machine-readable reason tag.
        reason: &'static str,
    },
}

/// Halt reason for bytes-mismatch invalid experiments.
pub const MATCHED_BYTES_INVALID_REASON: &str = "matched-bytes-invalid; comparison-not-scientific";

/// Error returned by S7 outcome helper validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum S7OutcomeError {
    /// Aggregate parity requires at least one per-seed parity verdict.
    MissingPerSeedParityVerdict,
}

/// Inputs to the §12 outcome dispatcher.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct S7OutcomeDispatchInput {
    /// Whether any MoE seed diverged.
    pub moe_diverged: bool,
    /// Whether any MoE seed collapsed at the production lambda.
    pub moe_collapsed: bool,
    /// Whether H1 was refuted for a non-collapse reason.
    pub h1_refuted_non_collapse: bool,
    /// Whether any dense matched-baseline seed diverged.
    pub dense_diverged: bool,
    /// Whether H2 was refuted.
    pub h2_refuted: bool,
    /// Whether H7 was refuted.
    pub h7_refuted: bool,
    /// Whether H8 was refuted.
    pub h8_refuted: bool,
    /// Whether H5 was refuted.
    pub h5_refuted: bool,
    /// Whether H6 was refuted.
    pub h6_refuted: bool,
    /// Whether the MoE median bpc suspiciousness guard fired.
    pub suspicious_moe_bpc: bool,
    /// Aggregate parity verdict from §11.2.
    pub aggregate_parity_verdict: AggregateParityVerdict,
    /// Whether H3 was refuted.
    pub h3_refuted: bool,
    /// Whether H4 was refuted.
    pub h4_refuted: bool,
    /// Whether H9 was refuted.
    pub h9_refuted: bool,
    /// Whether H10 was refuted.
    pub h10_refuted: bool,
}

impl Default for S7OutcomeDispatchInput {
    fn default() -> Self {
        Self {
            moe_diverged: false,
            moe_collapsed: false,
            h1_refuted_non_collapse: false,
            dense_diverged: false,
            h2_refuted: false,
            h7_refuted: false,
            h8_refuted: false,
            h5_refuted: false,
            h6_refuted: false,
            suspicious_moe_bpc: false,
            aggregate_parity_verdict: AggregateParityVerdict::PassClean,
            h3_refuted: false,
            h4_refuted: false,
            h9_refuted: false,
            h10_refuted: false,
        }
    }
}

/// Compute §11.2 aggregate parity. A bytes mismatch is an invalid experiment,
/// not a scientific parity falsification.
pub fn aggregate_parity_verdict(
    per_seed_passes: &[bool],
    bytes_diff: u64,
    d6_tolerance: u64,
) -> Result<AggregateParityVerdict, S7OutcomeError> {
    if bytes_diff > d6_tolerance {
        return Ok(AggregateParityVerdict::FailBytes);
    }
    if per_seed_passes.is_empty() {
        return Err(S7OutcomeError::MissingPerSeedParityVerdict);
    }
    if per_seed_passes.iter().all(|passed| *passed) {
        Ok(AggregateParityVerdict::PassClean)
    } else {
        Ok(AggregateParityVerdict::FailParity)
    }
}

/// Dispatch §12 S7Outcome from mandatory checks and aggregate verdicts.
#[must_use]
pub const fn dispatch_s7_outcome(input: S7OutcomeDispatchInput) -> S7Outcome {
    if input.moe_diverged {
        S7Outcome::FailMoeTrain
    } else if input.moe_collapsed {
        S7Outcome::FailRouterCollapse
    } else if input.h1_refuted_non_collapse {
        S7Outcome::FailMoeTrain
    } else if input.dense_diverged || input.h2_refuted {
        S7Outcome::FailDenseBaseline
    } else if input.h7_refuted {
        S7Outcome::FailGradProvenance
    } else if input.h8_refuted {
        S7Outcome::FailBurnGrad
    } else if input.h5_refuted {
        S7Outcome::FailSwitchStats
    } else if input.h6_refuted {
        S7Outcome::FailRouterCollapseGuardrail
    } else if input.suspicious_moe_bpc {
        S7Outcome::FailSuspicious
    } else if matches!(
        input.aggregate_parity_verdict,
        AggregateParityVerdict::FailBytes
    ) {
        S7Outcome::FailBytes
    } else if input.h3_refuted
        || matches!(
            input.aggregate_parity_verdict,
            AggregateParityVerdict::FailParity
        )
    {
        S7Outcome::FailParity
    } else if input.h4_refuted {
        S7Outcome::FailPareto
    } else if input.h9_refuted {
        S7Outcome::FailOracleRouted
    } else if input.h10_refuted {
        S7Outcome::FailEmulatorRouted
    } else {
        S7Outcome::PassClean
    }
}

/// Dispatch §12 decision from an S7 outcome.
#[must_use]
pub const fn decision_for_s7_outcome(outcome: S7Outcome) -> S7Decision {
    match outcome {
        S7Outcome::PassClean => S7Decision::ProceedToS8,
        S7Outcome::FailParity => S7Decision::ProceedToS8DenseOnly,
        S7Outcome::FailBytes => S7Decision::Halt {
            reason: MATCHED_BYTES_INVALID_REASON,
        },
        S7Outcome::FailPareto => S7Decision::Investigate {
            reason: "pareto-incomparable",
        },
        S7Outcome::FailMoeTrain => S7Decision::Investigate {
            reason: "burn-or-loss-substrate",
        },
        S7Outcome::FailRouterCollapse => S7Decision::Investigate {
            reason: "reduce-lambda-switch-or-tune-dropout",
        },
        S7Outcome::FailRouterCollapseGuardrail => S7Decision::Investigate {
            reason: "sweep-grid-or-thresholds",
        },
        S7Outcome::FailDenseBaseline => S7Decision::Investigate {
            reason: "dense-topology-constructor",
        },
        S7Outcome::FailSwitchStats => S7Decision::Halt {
            reason: "export-schema-broken",
        },
        S7Outcome::FailGradProvenance => S7Decision::Halt {
            reason: "loss-math-dishonest",
        },
        S7Outcome::FailBurnGrad => S7Decision::Halt {
            reason: "burn-adapter-broken",
        },
        S7Outcome::FailOracleRouted => S7Decision::Halt {
            reason: "oracle-cannot-resolve-routed-FFN",
        },
        S7Outcome::FailEmulatorRouted => S7Decision::Halt {
            reason: "routed-encoded-rom-broken",
        },
        S7Outcome::FailSuspicious => S7Decision::Halt {
            reason: "audit-split-and-bpc",
        },
    }
}

/// §21 rule 12: DenseOnly closure is valid only for true parity misses under
/// valid matched-deployed-byte accounting.
#[must_use]
pub const fn dense_only_closure_permitted(
    outcome: S7Outcome,
    valid_matched_bytes: bool,
    per_seed_bpc_parity_failed: bool,
) -> bool {
    matches!(outcome, S7Outcome::FailParity) && valid_matched_bytes && per_seed_bpc_parity_failed
}
