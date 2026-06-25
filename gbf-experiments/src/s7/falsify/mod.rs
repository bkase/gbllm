//! Test-only S7 falsification-suite helpers.
//!
//! This module does not install broken behavior into production paths. Each
//! submodule constructs an explicit §16 O5 broken-substitute fixture, evaluates
//! whether that evidence refutes the expected hypothesis, drives the real S7
//! outcome dispatcher, and emits the subscriber-captured event required by the
//! falsification suite.

use crate::S7_LOG_TARGET;

use super::outcome::{
    AggregateParityVerdict, S7Decision, S7Outcome, S7OutcomeDispatchInput, decision_for_s7_outcome,
    dispatch_s7_outcome,
};

/// Structured tracing event emitted once for each S7 falsification case.
pub const S7_FALSIFICATION_CASE_EVENT: &str = "s7.falsify.case";

/// Number of §16 O5 falsification cases.
pub const S7_FALSIFICATION_CASE_COUNT: usize = 9;

/// The nine deliberately-broken §16 O5 substitutes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum S7FalsificationCase {
    /// F1: top-2 routing silently constructs.
    RouterTopKGe2,
    /// F2: dense matched-bytes uses the MoE FFN width.
    BytesUnscaled,
    /// F3: Pareto comparison ignores D6 byte tolerance.
    ParetoUnequalBytes,
    /// F4: L_switch gradient provenance stops at routing probabilities.
    SwitchGradRouterOnly,
    /// F5: z-loss uses an uncentered baseline.
    ZUncentered,
    /// F6: load-balance dispatch provenance is not stop-gradient.
    BalanceNoStopGrad,
    /// F7: smoothness window one silently constructs.
    WindowOne,
    /// F8: lambda-switch sweep contains only the production value.
    SweepConstantLambda,
    /// F9: ExpertBlockQat Burn adapter leaves `up.weight` gradients dead.
    ExpertBlockQatGradDead,
}

impl S7FalsificationCase {
    /// All cases in RFC §16 O5 order.
    pub const ALL: [Self; S7_FALSIFICATION_CASE_COUNT] = [
        Self::RouterTopKGe2,
        Self::BytesUnscaled,
        Self::ParetoUnequalBytes,
        Self::SwitchGradRouterOnly,
        Self::ZUncentered,
        Self::BalanceNoStopGrad,
        Self::WindowOne,
        Self::SweepConstantLambda,
        Self::ExpertBlockQatGradDead,
    ];

    /// Compact case id used by the required tracing field.
    #[must_use]
    pub const fn case(self) -> &'static str {
        match self {
            Self::RouterTopKGe2 => "F1",
            Self::BytesUnscaled => "F2",
            Self::ParetoUnequalBytes => "F3",
            Self::SwitchGradRouterOnly => "F4",
            Self::ZUncentered => "F5",
            Self::BalanceNoStopGrad => "F6",
            Self::WindowOne => "F7",
            Self::SweepConstantLambda => "F8",
            Self::ExpertBlockQatGradDead => "F9",
        }
    }

    /// Full broken-substitute id from RFC §16 O5.
    #[must_use]
    pub const fn substitute_id(self) -> &'static str {
        match self {
            Self::RouterTopKGe2 => "F1-router-top-k-ge-2",
            Self::BytesUnscaled => "F2-bytes-unscaled",
            Self::ParetoUnequalBytes => "F3-pareto-unequal-bytes",
            Self::SwitchGradRouterOnly => "F4-switch-grad-router-only",
            Self::ZUncentered => "F5-z-uncentered",
            Self::BalanceNoStopGrad => "F6-balance-no-stop-grad",
            Self::WindowOne => "F7-window-one",
            Self::SweepConstantLambda => "F8-sweep-constant-lambda",
            Self::ExpertBlockQatGradDead => "F9-expert-block-qat-grad-dead",
        }
    }

    /// Target hypothesis that must be observed as Refuted.
    #[must_use]
    pub const fn hypothesis(self) -> &'static str {
        match self {
            Self::RouterTopKGe2 => "H1",
            Self::BytesUnscaled => "H3",
            Self::ParetoUnequalBytes => "H4",
            Self::SwitchGradRouterOnly | Self::ZUncentered | Self::BalanceNoStopGrad => "H7",
            Self::WindowOne => "H5",
            Self::SweepConstantLambda => "H6",
            Self::ExpertBlockQatGradDead => "H8",
        }
    }

    /// Expected verdict for every deliberately-broken substitute.
    #[must_use]
    pub const fn expected_verdict(self) -> &'static str {
        let _ = self;
        "Refuted"
    }

    /// Human-readable falsification clause from RFC §16 O5.
    #[must_use]
    pub const fn falsification_clause(self) -> &'static str {
        match self {
            Self::RouterTopKGe2 => "D3 forbids top-k >= 2",
            Self::BytesUnscaled => "matched-bytes formula uses d_ff_dense, not MoE d_ff",
            Self::ParetoUnequalBytes => "unequal byte budgets outside D6 tolerance are H4 Refuted",
            Self::SwitchGradRouterOnly => {
                "L_switch must reach LowRankRouter parameters through routing_probs"
            }
            Self::ZUncentered => "centered z-loss baseline is mu = log(n_experts)",
            Self::BalanceNoStopGrad => "dispatch_indicator is stop-gradient provenance",
            Self::WindowOne => "D10 forbids smoothness_window = 1",
            Self::SweepConstantLambda => "D11 lambda_switch grid must be a real sweep",
            Self::ExpertBlockQatGradDead => {
                "Burn ExpertBlockQat must propagate gradients into up.weight"
            }
        }
    }

    /// Diagnostic attached to the tracing event and assertion failures.
    #[must_use]
    pub const fn diagnostic(self) -> &'static str {
        match self {
            Self::RouterTopKGe2 => {
                "broken top-2 router would invalidate the mandatory top-1 MoE training claim"
            }
            Self::BytesUnscaled => {
                "broken dense control used d_ff=128 instead of the D6 solved dense width"
            }
            Self::ParetoUnequalBytes => {
                "broken Pareto verdict compared byte-unequal points as if they were equivalent"
            }
            Self::SwitchGradRouterOnly => {
                "broken L_switch provenance reached routing_probs but not router parameters"
            }
            Self::ZUncentered => "broken z-loss produced a nonzero zero-logit baseline",
            Self::BalanceNoStopGrad => {
                "broken balance loss leaked gradient through hard dispatch provenance"
            }
            Self::WindowOne => "broken switch-stat schema accepted a one-token window",
            Self::SweepConstantLambda => {
                "broken collapse sweep had no high-lambda contrast and fires FailC"
            }
            Self::ExpertBlockQatGradDead => {
                "broken Burn ExpertBlockQat adapter produced zero up.weight gradients"
            }
        }
    }

    /// Dispatcher input for the expected refuted hypothesis.
    #[must_use]
    pub const fn dispatch_input(self) -> S7OutcomeDispatchInput {
        match self {
            Self::RouterTopKGe2 => S7OutcomeDispatchInput {
                h1_refuted_non_collapse: true,
                ..S7OutcomeDispatchInput::DEFAULT
            },
            Self::BytesUnscaled => S7OutcomeDispatchInput {
                h3_refuted: true,
                ..S7OutcomeDispatchInput::DEFAULT
            },
            Self::ParetoUnequalBytes => S7OutcomeDispatchInput {
                h4_refuted: true,
                ..S7OutcomeDispatchInput::DEFAULT
            },
            Self::SwitchGradRouterOnly | Self::ZUncentered | Self::BalanceNoStopGrad => {
                S7OutcomeDispatchInput {
                    h7_refuted: true,
                    ..S7OutcomeDispatchInput::DEFAULT
                }
            }
            Self::WindowOne => S7OutcomeDispatchInput {
                h5_refuted: true,
                ..S7OutcomeDispatchInput::DEFAULT
            },
            Self::SweepConstantLambda => S7OutcomeDispatchInput {
                h6_refuted: true,
                ..S7OutcomeDispatchInput::DEFAULT
            },
            Self::ExpertBlockQatGradDead => S7OutcomeDispatchInput {
                h8_refuted: true,
                ..S7OutcomeDispatchInput::DEFAULT
            },
        }
    }

    /// Expected S7 outcome for this broken substitute.
    #[must_use]
    pub const fn expected_outcome(self) -> S7Outcome {
        dispatch_s7_outcome(self.dispatch_input())
    }

    /// Expected §12 decision for this broken substitute.
    #[must_use]
    pub const fn expected_decision(self) -> S7Decision {
        decision_for_s7_outcome(self.expected_outcome())
    }
}

/// Observed verdict for a deliberately-broken substitute.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum S7FalsificationVerdict {
    /// The broken substitute hit the expected falsification clause.
    Refuted,
    /// The broken substitute did not hit the expected falsification clause.
    NotRefuted,
}

impl S7FalsificationVerdict {
    /// Stable verdict label used in logs.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Refuted => "Refuted",
            Self::NotRefuted => "NotRefuted",
        }
    }
}

/// Test-only evidence emitted by a broken S7 substitute.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum S7FalsificationEvidence {
    /// F1 evidence: a broken router constructed a top-k path with k >= 2.
    RouterTopKGe2 {
        /// Requested top-k value.
        requested_top_k: u8,
        /// Whether the broken substitute silently constructed.
        constructed: bool,
        /// Number of non-zero hard dispatch weights produced by the substitute.
        dispatch_weight_count: u8,
    },
    /// F2 evidence: dense matched-bytes reused the MoE FFN width.
    BytesUnscaled {
        /// MoE FFN width from the canonical profile.
        moe_d_ff: u16,
        /// Broken dense FFN width.
        dense_d_ff_observed: u16,
        /// Correct D6-solved dense FFN width.
        dense_d_ff_expected: u16,
    },
    /// F3 evidence: a byte-unequal Pareto comparison was treated as comparable.
    ParetoUnequalBytes {
        /// Observed deployed-byte difference.
        bytes_diff: u64,
        /// D6 tolerance.
        d6_tolerance_bytes: u64,
        /// Whether the broken comparator treated the points as byte-equivalent.
        broken_compared_as_equivalent: bool,
    },
    /// F4 evidence: L_switch reached routing probabilities but not router params.
    SwitchGradRouterOnly {
        /// Whether gradients reached routing probabilities.
        routing_probs_grad_nonzero: bool,
        /// Whether gradients reached the LowRankRouter parameters.
        low_rank_router_grad_nonzero: bool,
    },
    /// F5 evidence: zero-logit z-loss baseline is nonzero under the substitute.
    ZUncentered {
        /// Whether D5 declared the centered `mu = log(n_experts)` baseline.
        centered_mu_declared: bool,
        /// Whether the broken zero-logit baseline is nonzero.
        zero_logit_loss_nonzero: bool,
    },
    /// F6 evidence: load-balance gradients leaked through hard dispatch.
    BalanceNoStopGrad {
        /// Whether gradients reached routing probabilities.
        routing_probs_grad_nonzero: bool,
        /// Whether gradients also leaked through hard dispatch provenance.
        dispatch_indicator_grad_leaked: bool,
    },
    /// F7 evidence: a one-token smoothness window silently constructed.
    WindowOne {
        /// Broken window value.
        smoothness_window: u16,
        /// Whether the broken substitute silently constructed.
        constructed: bool,
    },
    /// F8 evidence: a lambda-switch sweep had no high-lambda contrast.
    SweepConstantLambda {
        /// Number of grid entries.
        grid_len: usize,
        /// Whether every entry is the production lambda.
        production_only: bool,
        /// Whether the guardrail hit FailC.
        fail_c: bool,
    },
    /// F9 evidence: ExpertBlockQat Burn gradients were dead at up.weight.
    ExpertBlockQatGradDead {
        /// Whether `up.weight` received any nonzero gradient.
        up_weight_grad_nonzero: bool,
    },
}

impl S7FalsificationEvidence {
    /// Case this evidence belongs to.
    #[must_use]
    pub const fn case(self) -> S7FalsificationCase {
        match self {
            Self::RouterTopKGe2 { .. } => S7FalsificationCase::RouterTopKGe2,
            Self::BytesUnscaled { .. } => S7FalsificationCase::BytesUnscaled,
            Self::ParetoUnequalBytes { .. } => S7FalsificationCase::ParetoUnequalBytes,
            Self::SwitchGradRouterOnly { .. } => S7FalsificationCase::SwitchGradRouterOnly,
            Self::ZUncentered { .. } => S7FalsificationCase::ZUncentered,
            Self::BalanceNoStopGrad { .. } => S7FalsificationCase::BalanceNoStopGrad,
            Self::WindowOne { .. } => S7FalsificationCase::WindowOne,
            Self::SweepConstantLambda { .. } => S7FalsificationCase::SweepConstantLambda,
            Self::ExpertBlockQatGradDead { .. } => S7FalsificationCase::ExpertBlockQatGradDead,
        }
    }

    /// Whether this broken substitute hits the expected O5 falsification clause.
    #[must_use]
    pub const fn refutes_expected(self) -> bool {
        match self {
            Self::RouterTopKGe2 {
                requested_top_k,
                constructed,
                dispatch_weight_count,
            } => constructed && requested_top_k >= 2 && dispatch_weight_count >= 2,
            Self::BytesUnscaled {
                moe_d_ff,
                dense_d_ff_observed,
                dense_d_ff_expected,
            } => dense_d_ff_observed == moe_d_ff && dense_d_ff_expected != moe_d_ff,
            Self::ParetoUnequalBytes {
                bytes_diff,
                d6_tolerance_bytes,
                broken_compared_as_equivalent,
            } => bytes_diff > d6_tolerance_bytes && broken_compared_as_equivalent,
            Self::SwitchGradRouterOnly {
                routing_probs_grad_nonzero,
                low_rank_router_grad_nonzero,
            } => routing_probs_grad_nonzero && !low_rank_router_grad_nonzero,
            Self::ZUncentered {
                centered_mu_declared,
                zero_logit_loss_nonzero,
            } => centered_mu_declared && zero_logit_loss_nonzero,
            Self::BalanceNoStopGrad {
                routing_probs_grad_nonzero,
                dispatch_indicator_grad_leaked,
            } => routing_probs_grad_nonzero && dispatch_indicator_grad_leaked,
            Self::WindowOne {
                smoothness_window,
                constructed,
            } => smoothness_window == 1 && constructed,
            Self::SweepConstantLambda {
                grid_len,
                production_only,
                fail_c,
            } => grid_len == 1 && production_only && fail_c,
            Self::ExpertBlockQatGradDead {
                up_weight_grad_nonzero,
            } => !up_weight_grad_nonzero,
        }
    }
}

/// Result from one S7 falsification case observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct S7FalsificationObservation {
    case: S7FalsificationCase,
    evidence: S7FalsificationEvidence,
    observed_verdict: S7FalsificationVerdict,
    outcome: S7Outcome,
    decision: S7Decision,
}

impl S7FalsificationObservation {
    /// Case that was observed.
    #[must_use]
    pub const fn case(self) -> S7FalsificationCase {
        self.case
    }

    /// Evidence produced by the broken substitute.
    #[must_use]
    pub const fn evidence(self) -> S7FalsificationEvidence {
        self.evidence
    }

    /// Observed verdict label.
    #[must_use]
    pub const fn observed_verdict(self) -> &'static str {
        self.observed_verdict.as_str()
    }

    /// Observed S7 outcome.
    #[must_use]
    pub const fn outcome(self) -> S7Outcome {
        self.outcome
    }

    /// Observed §12 decision.
    #[must_use]
    pub const fn decision(self) -> S7Decision {
        self.decision
    }

    /// Diagnostic for this observation.
    #[must_use]
    pub const fn diagnostic(self) -> &'static str {
        match self.observed_verdict {
            S7FalsificationVerdict::Refuted => self.case.diagnostic(),
            S7FalsificationVerdict::NotRefuted => {
                "broken substitute did not trigger the expected S7 falsification clause"
            }
        }
    }

    /// True when the observed verdict/outcome/decision match the expected O5 mapping.
    #[must_use]
    pub fn matches_expected(self) -> bool {
        self.observed_verdict == S7FalsificationVerdict::Refuted
            && self.observed_verdict() == self.case.expected_verdict()
            && matches_outcome(self.outcome, self.case.expected_outcome())
            && matches_decision(self.decision, self.case.expected_decision())
    }
}

/// Observe a broken-substitute evidence packet without emitting a trace event.
#[must_use]
pub fn observe_s7_falsification_evidence(
    evidence: S7FalsificationEvidence,
) -> S7FalsificationObservation {
    let case = evidence.case();
    let observed_verdict = if evidence.refutes_expected() {
        S7FalsificationVerdict::Refuted
    } else {
        S7FalsificationVerdict::NotRefuted
    };
    let input = if matches!(observed_verdict, S7FalsificationVerdict::Refuted) {
        case.dispatch_input()
    } else {
        S7OutcomeDispatchInput::DEFAULT
    };
    let outcome = dispatch_s7_outcome(input);
    S7FalsificationObservation {
        case,
        evidence,
        observed_verdict,
        outcome,
        decision: decision_for_s7_outcome(outcome),
    }
}

/// Observe a broken-substitute evidence packet and emit the required trace event.
pub fn run_s7_falsification_evidence(
    evidence: S7FalsificationEvidence,
) -> S7FalsificationObservation {
    let observation = observe_s7_falsification_evidence(evidence);
    let case = observation.case();
    tracing::info!(
        target: S7_LOG_TARGET,
        event_name = S7_FALSIFICATION_CASE_EVENT,
        case = case.case(),
        substitute_id = case.substitute_id(),
        hypothesis = case.hypothesis(),
        expected_verdict = case.expected_verdict(),
        observed_verdict = observation.observed_verdict(),
        falsification_clause = case.falsification_clause(),
        diagnostic = observation.diagnostic(),
        evidence = ?observation.evidence(),
        outcome = ?observation.outcome(),
        decision = ?observation.decision(),
    );
    observation
}

/// Observe a case's default broken substitute and emit the required trace event.
pub fn run_s7_falsification_case(case: S7FalsificationCase) -> S7FalsificationObservation {
    run_s7_falsification_evidence(broken_substitute_for_case(case))
}

/// F1 broken substitute runner.
pub mod f1_router_top_k_ge_2 {
    //! F1: top-2 routing silently constructs.

    use super::{
        S7FalsificationEvidence, S7FalsificationObservation, run_s7_falsification_evidence,
    };

    /// Construct the F1 broken top-k router substitute.
    #[must_use]
    pub const fn broken_substitute() -> S7FalsificationEvidence {
        S7FalsificationEvidence::RouterTopKGe2 {
            requested_top_k: 2,
            constructed: true,
            dispatch_weight_count: 2,
        }
    }

    /// Run the F1 broken-substitute observation.
    pub fn run() -> S7FalsificationObservation {
        run_s7_falsification_evidence(broken_substitute())
    }
}

/// F2 broken substitute runner.
pub mod f2_bytes_unscaled {
    //! F2: dense matched-bytes uses MoE's FFN width.

    use super::{
        S7FalsificationEvidence, S7FalsificationObservation, run_s7_falsification_evidence,
    };

    /// Canonical MoeTiny FFN width from RFC D3.
    pub const MOE_TINY_D_FF: u16 = 128;
    /// D6-solved dense width for the current canonical S7 pin.
    pub const D6_SOLVED_D_FF_DENSE_FIXTURE: u16 = 572;

    /// Construct the F2 broken dense matched-bytes substitute.
    #[must_use]
    pub const fn broken_substitute() -> S7FalsificationEvidence {
        broken_substitute_with_expected_dense(D6_SOLVED_D_FF_DENSE_FIXTURE)
    }

    /// Construct the F2 broken substitute against a caller-supplied D6 width.
    #[must_use]
    pub const fn broken_substitute_with_expected_dense(
        dense_d_ff_expected: u16,
    ) -> S7FalsificationEvidence {
        S7FalsificationEvidence::BytesUnscaled {
            moe_d_ff: MOE_TINY_D_FF,
            dense_d_ff_observed: MOE_TINY_D_FF,
            dense_d_ff_expected,
        }
    }

    /// Run the F2 broken-substitute observation.
    pub fn run() -> S7FalsificationObservation {
        run_s7_falsification_evidence(broken_substitute())
    }
}

/// F3 broken substitute runner.
pub mod f3_pareto_unequal_bytes {
    //! F3: Pareto verdict compares unequal byte budgets.

    use super::{
        S7FalsificationEvidence, S7FalsificationObservation, run_s7_falsification_evidence,
    };

    /// Construct the F3 broken Pareto comparator substitute.
    #[must_use]
    pub const fn broken_substitute() -> S7FalsificationEvidence {
        S7FalsificationEvidence::ParetoUnequalBytes {
            bytes_diff: 11,
            d6_tolerance_bytes: 10,
            broken_compared_as_equivalent: true,
        }
    }

    /// Run the F3 broken-substitute observation.
    pub fn run() -> S7FalsificationObservation {
        run_s7_falsification_evidence(broken_substitute())
    }
}

/// F4 broken substitute runner.
pub mod f4_switch_grad_router_only {
    //! F4: L_switch gradient path stops before router parameters.

    use super::{
        S7FalsificationEvidence, S7FalsificationObservation, run_s7_falsification_evidence,
    };

    /// Construct the F4 broken L_switch provenance substitute.
    #[must_use]
    pub const fn broken_substitute() -> S7FalsificationEvidence {
        S7FalsificationEvidence::SwitchGradRouterOnly {
            routing_probs_grad_nonzero: true,
            low_rank_router_grad_nonzero: false,
        }
    }

    /// Run the F4 broken-substitute observation.
    pub fn run() -> S7FalsificationObservation {
        run_s7_falsification_evidence(broken_substitute())
    }
}

/// F5 broken substitute runner.
pub mod f5_z_uncentered {
    //! F5: z-loss uses an uncentered baseline.

    use super::{
        S7FalsificationEvidence, S7FalsificationObservation, run_s7_falsification_evidence,
    };

    /// Construct the F5 uncentered z-loss substitute.
    #[must_use]
    pub const fn broken_substitute() -> S7FalsificationEvidence {
        S7FalsificationEvidence::ZUncentered {
            centered_mu_declared: true,
            zero_logit_loss_nonzero: true,
        }
    }

    /// Run the F5 broken-substitute observation.
    pub fn run() -> S7FalsificationObservation {
        run_s7_falsification_evidence(broken_substitute())
    }
}

/// F6 broken substitute runner.
pub mod f6_balance_no_stop_grad {
    //! F6: load-balance dispatch provenance is differentiable.

    use super::{
        S7FalsificationEvidence, S7FalsificationObservation, run_s7_falsification_evidence,
    };

    /// Construct the F6 no-stop-gradient load-balance substitute.
    #[must_use]
    pub const fn broken_substitute() -> S7FalsificationEvidence {
        S7FalsificationEvidence::BalanceNoStopGrad {
            routing_probs_grad_nonzero: true,
            dispatch_indicator_grad_leaked: true,
        }
    }

    /// Run the F6 broken-substitute observation.
    pub fn run() -> S7FalsificationObservation {
        run_s7_falsification_evidence(broken_substitute())
    }
}

/// F7 broken substitute runner.
pub mod f7_window_one {
    //! F7: smoothness window one silently constructs.

    use super::{
        S7FalsificationEvidence, S7FalsificationObservation, run_s7_falsification_evidence,
    };

    /// Construct the F7 smoothness-window substitute.
    #[must_use]
    pub const fn broken_substitute() -> S7FalsificationEvidence {
        S7FalsificationEvidence::WindowOne {
            smoothness_window: 1,
            constructed: true,
        }
    }

    /// Run the F7 broken-substitute observation.
    pub fn run() -> S7FalsificationObservation {
        run_s7_falsification_evidence(broken_substitute())
    }
}

/// F8 broken substitute runner.
pub mod f8_sweep_constant_lambda {
    //! F8: lambda-switch sweep contains only the production value.

    use super::{
        S7FalsificationEvidence, S7FalsificationObservation, run_s7_falsification_evidence,
    };

    /// Construct the F8 constant-lambda sweep substitute.
    #[must_use]
    pub const fn broken_substitute() -> S7FalsificationEvidence {
        S7FalsificationEvidence::SweepConstantLambda {
            grid_len: 1,
            production_only: true,
            fail_c: true,
        }
    }

    /// Run the F8 broken-substitute observation.
    pub fn run() -> S7FalsificationObservation {
        run_s7_falsification_evidence(broken_substitute())
    }
}

/// F9 broken substitute runner.
pub mod f9_expert_block_qat_grad_dead {
    //! F9: ExpertBlockQat Burn gradients are dead at up.weight.

    use super::{
        S7FalsificationEvidence, S7FalsificationObservation, run_s7_falsification_evidence,
    };

    /// Construct the F9 dead-gradient ExpertBlockQat substitute.
    #[must_use]
    pub const fn broken_substitute() -> S7FalsificationEvidence {
        S7FalsificationEvidence::ExpertBlockQatGradDead {
            up_weight_grad_nonzero: false,
        }
    }

    /// Run the F9 broken-substitute observation.
    pub fn run() -> S7FalsificationObservation {
        run_s7_falsification_evidence(broken_substitute())
    }
}

const fn broken_substitute_for_case(case: S7FalsificationCase) -> S7FalsificationEvidence {
    match case {
        S7FalsificationCase::RouterTopKGe2 => f1_router_top_k_ge_2::broken_substitute(),
        S7FalsificationCase::BytesUnscaled => f2_bytes_unscaled::broken_substitute(),
        S7FalsificationCase::ParetoUnequalBytes => f3_pareto_unequal_bytes::broken_substitute(),
        S7FalsificationCase::SwitchGradRouterOnly => {
            f4_switch_grad_router_only::broken_substitute()
        }
        S7FalsificationCase::ZUncentered => f5_z_uncentered::broken_substitute(),
        S7FalsificationCase::BalanceNoStopGrad => f6_balance_no_stop_grad::broken_substitute(),
        S7FalsificationCase::WindowOne => f7_window_one::broken_substitute(),
        S7FalsificationCase::SweepConstantLambda => f8_sweep_constant_lambda::broken_substitute(),
        S7FalsificationCase::ExpertBlockQatGradDead => {
            f9_expert_block_qat_grad_dead::broken_substitute()
        }
    }
}

impl S7OutcomeDispatchInput {
    const DEFAULT: Self = Self {
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
    };
}

fn matches_outcome(left: S7Outcome, right: S7Outcome) -> bool {
    left == right
}

fn matches_decision(left: S7Decision, right: S7Decision) -> bool {
    match (left, right) {
        (S7Decision::ProceedToS8, S7Decision::ProceedToS8)
        | (S7Decision::ProceedToS8DenseOnly, S7Decision::ProceedToS8DenseOnly) => true,
        (
            S7Decision::Investigate {
                reason: left_reason,
            },
            S7Decision::Investigate {
                reason: right_reason,
            },
        )
        | (
            S7Decision::Halt {
                reason: left_reason,
            },
            S7Decision::Halt {
                reason: right_reason,
            },
        ) => str_eq(left_reason, right_reason),
        (
            S7Decision::ProceedToS8,
            S7Decision::ProceedToS8DenseOnly
            | S7Decision::Investigate { .. }
            | S7Decision::Halt { .. },
        )
        | (
            S7Decision::ProceedToS8DenseOnly,
            S7Decision::ProceedToS8 | S7Decision::Investigate { .. } | S7Decision::Halt { .. },
        )
        | (
            S7Decision::Investigate { .. },
            S7Decision::ProceedToS8 | S7Decision::ProceedToS8DenseOnly | S7Decision::Halt { .. },
        )
        | (
            S7Decision::Halt { .. },
            S7Decision::ProceedToS8
            | S7Decision::ProceedToS8DenseOnly
            | S7Decision::Investigate { .. },
        ) => false,
    }
}

fn str_eq(left: &str, right: &str) -> bool {
    left == right
}
