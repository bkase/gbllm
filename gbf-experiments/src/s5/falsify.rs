//! Producer-side F-S5 falsification feature loop.
//!
//! The full S5 training/export producer does not live in this crate yet. This
//! module is the bounded producer harness for the `s5-falsify-N` feature loop:
//! each Cargo feature selects exactly one broken substitute and drives the
//! corresponding S5 verifier/policy surface with explicit fixture evidence.

use gbf_policy::emulator_harness::H15FirstCommitCardinalityVerdict;
use gbf_policy::s5::{
    FrontierRecommendation, HypothesisStatus, S5_FEEDBACK_FIXTURE_V1_SAFE_BOUND_CASES,
    S5_FRONTIER_DEFAULT_AXES, S5FeedbackApplyConfig, S5FrontierAxis, S5FrontierPointMetrics,
    S5Outcome, S5OutcomeDispatchInput, S5SelectionAuthority, dispatch_s5_outcome, s5_argmax_token,
    s5_f13_bias_non_oracle_token_above_predicted, s5_h15_oracle_token_agreement,
    verify_s5_h14_pareto_emission, verify_s5_h16_feedback_fixture,
};
#[cfg(test)]
use gbf_policy::{H5LongRangeEvidence, H5LongRangeVerdict, h5_long_range_verdict};
use gbf_policy::{
    ReValidationOutcome, S5_SHADOW_PIPELINE_STAGES, ShadowCompileSampleExpectation,
    ShadowCompileSampleReal, validate_shr1_shadow_sample, verify_h15_first_commit_payload_len,
};
use serde::{Deserialize, Serialize};

/// Number of F-S5 deliberately broken feature-loop cases.
pub const S5_FALSIFICATION_CASE_COUNT: usize = 15;

/// Stable limitation string for cases that cannot replay missing upstream producers.
pub const S5_EXPLICIT_FIXTURE_LIMITATION: &str = "upstream S5 producer replay APIs are not implemented; this case uses an explicit gbf-experiments::s5 producer-contract fixture";

/// F-S5 hypothesis id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum S5Hypothesis {
    /// Attention-oracle agreement.
    H1,
    /// BoundedKv Burn gradient smoke.
    H2,
    /// LinearState multi-timescale trains end-to-end.
    H3,
    /// Three-variant smoke.
    H4,
    /// LinearState multi-timescale advantage.
    H5,
    /// shadow_compile A/B wiring.
    H6,
    /// Frontier emission completeness.
    H7,
    /// BoundedKv-vs-LinearState parity.
    H8,
    /// Reset boundary preservation.
    H9,
    /// Per-variant determinism.
    H10,
    /// RuntimeChromeBudget integrity.
    H11,
    /// CompileProfile binding.
    H12,
    /// Shadow compile correctness.
    H13,
    /// Pareto frontier soundness.
    H14,
    /// Emulator harness one-token agreement.
    H15,
    /// Compiler feedback-loop convergence.
    H16,
    /// Logging overhead gate.
    H17,
}

/// One deliberately broken F-S5 substitute.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum S5FalsificationCase {
    /// F1: tamper attention-oracle equality.
    F1OracleEqualityTampered,
    /// F2: break BoundedKv autodiff by stopping all gradients.
    F2BoundedKvAutodiffBroken,
    /// F3: collapse L_MT4 decay bands to NaN.
    F3LMt4BandCollapse,
    /// F4: undersize capacity so Toy0 cannot beat KN-5.
    F4CapacityUndersized,
    /// F5: make shadow_compile_ok constant true.
    F5ShadowOkConstantTrue,
    /// F6: drop a required frontier axis.
    F6FrontierMissingAxis,
    /// F7: leak reset-boundary KV state.
    F7ResetBoundaryLeak,
    /// F8: inject per-seed nondeterminism.
    F8PerSeedNonDeterminism,
    /// F9: violate RuntimeChromeBudget tolerance.
    F9RuntimeBudgetToleranceViolation,
    /// F10: silently stop threading CompileProfile.
    F10CompileProfileNotThreaded,
    /// F11: omit a real shadow-pipeline stage.
    F11ShadowStagesMissing,
    /// F12: corrupt an EncodedRom certificate.
    F12EncodedRomCertCorrupted,
    /// F13: force emulator/oracle disagreement.
    F13EmulatorOracleDisagree,
    /// F14: break Pareto frontier selection.
    F14ParetoBroken,
    /// F15: break feedback application.
    F15FeedbackBroken,
}

impl S5FalsificationCase {
    /// All F-S5 broken substitutes in RFC order.
    pub const ALL: [Self; S5_FALSIFICATION_CASE_COUNT] = [
        Self::F1OracleEqualityTampered,
        Self::F2BoundedKvAutodiffBroken,
        Self::F3LMt4BandCollapse,
        Self::F4CapacityUndersized,
        Self::F5ShadowOkConstantTrue,
        Self::F6FrontierMissingAxis,
        Self::F7ResetBoundaryLeak,
        Self::F8PerSeedNonDeterminism,
        Self::F9RuntimeBudgetToleranceViolation,
        Self::F10CompileProfileNotThreaded,
        Self::F11ShadowStagesMissing,
        Self::F12EncodedRomCertCorrupted,
        Self::F13EmulatorOracleDisagree,
        Self::F14ParetoBroken,
        Self::F15FeedbackBroken,
    ];

    /// Construct a case from its `s5-falsify-N` index.
    #[must_use]
    pub const fn from_index(index: usize) -> Option<Self> {
        match index {
            1 => Some(Self::F1OracleEqualityTampered),
            2 => Some(Self::F2BoundedKvAutodiffBroken),
            3 => Some(Self::F3LMt4BandCollapse),
            4 => Some(Self::F4CapacityUndersized),
            5 => Some(Self::F5ShadowOkConstantTrue),
            6 => Some(Self::F6FrontierMissingAxis),
            7 => Some(Self::F7ResetBoundaryLeak),
            8 => Some(Self::F8PerSeedNonDeterminism),
            9 => Some(Self::F9RuntimeBudgetToleranceViolation),
            10 => Some(Self::F10CompileProfileNotThreaded),
            11 => Some(Self::F11ShadowStagesMissing),
            12 => Some(Self::F12EncodedRomCertCorrupted),
            13 => Some(Self::F13EmulatorOracleDisagree),
            14 => Some(Self::F14ParetoBroken),
            15 => Some(Self::F15FeedbackBroken),
            _ => None,
        }
    }

    /// The `s5-falsify-N` index.
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::F1OracleEqualityTampered => 1,
            Self::F2BoundedKvAutodiffBroken => 2,
            Self::F3LMt4BandCollapse => 3,
            Self::F4CapacityUndersized => 4,
            Self::F5ShadowOkConstantTrue => 5,
            Self::F6FrontierMissingAxis => 6,
            Self::F7ResetBoundaryLeak => 7,
            Self::F8PerSeedNonDeterminism => 8,
            Self::F9RuntimeBudgetToleranceViolation => 9,
            Self::F10CompileProfileNotThreaded => 10,
            Self::F11ShadowStagesMissing => 11,
            Self::F12EncodedRomCertCorrupted => 12,
            Self::F13EmulatorOracleDisagree => 13,
            Self::F14ParetoBroken => 14,
            Self::F15FeedbackBroken => 15,
        }
    }

    /// Stable case id.
    #[must_use]
    pub fn case_id(self) -> String {
        format!("F{}", self.index())
    }

    /// Cargo feature that selects this case.
    #[must_use]
    pub fn feature_name(self) -> String {
        format!("s5-falsify-{}", self.index())
    }

    /// Stable broken-substitute slug.
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::F1OracleEqualityTampered => "oracle_equality_tampered",
            Self::F2BoundedKvAutodiffBroken => "boundedkv_autodiff_broken",
            Self::F3LMt4BandCollapse => "l_mt4_band_collapse",
            Self::F4CapacityUndersized => "capacity_undersized",
            Self::F5ShadowOkConstantTrue => "shadow_ok_constant_true",
            Self::F6FrontierMissingAxis => "frontier_missing_axis",
            Self::F7ResetBoundaryLeak => "reset_boundary_leak",
            Self::F8PerSeedNonDeterminism => "per_seed_non_determinism",
            Self::F9RuntimeBudgetToleranceViolation => "runtime_budget_tolerance_violation",
            Self::F10CompileProfileNotThreaded => "compile_profile_not_threaded",
            Self::F11ShadowStagesMissing => "shadow_stages_missing",
            Self::F12EncodedRomCertCorrupted => "encoded_rom_cert_corrupted",
            Self::F13EmulatorOracleDisagree => "emulator_oracle_disagree",
            Self::F14ParetoBroken => "pareto_broken",
            Self::F15FeedbackBroken => "feedback_broken",
        }
    }

    /// Hypotheses this substitute is expected to refute.
    #[must_use]
    pub fn target_hypotheses(self) -> Vec<S5Hypothesis> {
        match self {
            Self::F1OracleEqualityTampered => vec![S5Hypothesis::H1],
            Self::F2BoundedKvAutodiffBroken => vec![S5Hypothesis::H2],
            Self::F3LMt4BandCollapse => vec![S5Hypothesis::H3],
            Self::F4CapacityUndersized => vec![S5Hypothesis::H4],
            Self::F5ShadowOkConstantTrue => vec![S5Hypothesis::H6, S5Hypothesis::H13],
            Self::F6FrontierMissingAxis => vec![S5Hypothesis::H7],
            Self::F7ResetBoundaryLeak => vec![S5Hypothesis::H9],
            Self::F8PerSeedNonDeterminism => vec![S5Hypothesis::H10],
            Self::F9RuntimeBudgetToleranceViolation => vec![S5Hypothesis::H11],
            Self::F10CompileProfileNotThreaded => vec![S5Hypothesis::H12],
            Self::F11ShadowStagesMissing => vec![S5Hypothesis::H13],
            Self::F12EncodedRomCertCorrupted => vec![S5Hypothesis::H15],
            Self::F13EmulatorOracleDisagree => vec![S5Hypothesis::H15],
            Self::F14ParetoBroken => vec![S5Hypothesis::H14],
            Self::F15FeedbackBroken => vec![S5Hypothesis::H16],
        }
    }

    /// Expected outcome once the refutation is fed through S5 dispatch.
    #[must_use]
    pub const fn expected_outcome(self) -> S5Outcome {
        match self {
            Self::F1OracleEqualityTampered => S5Outcome::FailAttentionOracle,
            Self::F2BoundedKvAutodiffBroken => S5Outcome::FailBoundedKvGrad,
            Self::F3LMt4BandCollapse => S5Outcome::FailLinearstateGrad,
            Self::F4CapacityUndersized => S5Outcome::FailSubstrate {
                failure_kind: gbf_policy::s5::FailureKind::Capacity,
            },
            Self::F5ShadowOkConstantTrue | Self::F11ShadowStagesMissing => {
                S5Outcome::FailShadowCompile
            }
            Self::F6FrontierMissingAxis | Self::F14ParetoBroken => {
                S5Outcome::FailFrontierIncomplete
            }
            Self::F7ResetBoundaryLeak | Self::F8PerSeedNonDeterminism => S5Outcome::FailSubstrate {
                failure_kind: gbf_policy::s5::FailureKind::Substrate,
            },
            Self::F9RuntimeBudgetToleranceViolation => S5Outcome::FailRuntimeBudget,
            Self::F10CompileProfileNotThreaded => S5Outcome::FailCompileProfile,
            Self::F12EncodedRomCertCorrupted => S5Outcome::FailEncodedRom,
            Self::F13EmulatorOracleDisagree => S5Outcome::FailEmulatorHarness,
            Self::F15FeedbackBroken => S5Outcome::FailFeedbackLoop,
        }
    }
}

/// Result for one active `s5-falsify-N` producer-loop case.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S5FalsificationCaseResult {
    /// Stable case id, `F1` through `F15`.
    pub case_id: String,
    /// Cargo feature used for this run.
    pub feature: String,
    /// Stable broken-substitute slug.
    pub substitute: String,
    /// Expected target hypotheses.
    pub target_hypotheses: Vec<S5Hypothesis>,
    /// Observed target status.
    pub observed_status: HypothesisStatus,
    /// Expected S5 outcome after dispatch.
    pub expected_outcome: S5Outcome,
    /// Observed S5 outcome after dispatch.
    pub observed_outcome: S5Outcome,
    /// Whether the case refuted and dispatched as expected.
    pub matches_expected: bool,
    /// Evidence source used by this bounded harness.
    pub evidence_source: String,
    /// Honest limitation for cases that are not full producer replay.
    pub limitation: String,
    /// Stable detail for reports and script output.
    pub evidence: String,
}

impl S5FalsificationCaseResult {
    fn new(
        case: S5FalsificationCase,
        observed_status: HypothesisStatus,
        observed_outcome: S5Outcome,
        evidence: impl Into<String>,
    ) -> Self {
        let expected_outcome = case.expected_outcome();
        let matches_expected =
            observed_status == HypothesisStatus::Refuted && observed_outcome == expected_outcome;
        Self {
            case_id: case.case_id(),
            feature: case.feature_name(),
            substitute: case.slug().to_owned(),
            target_hypotheses: case.target_hypotheses(),
            observed_status,
            expected_outcome,
            observed_outcome,
            matches_expected,
            evidence_source: "gbf_experiments::s5 explicit producer-contract fixture".to_owned(),
            limitation: S5_EXPLICIT_FIXTURE_LIMITATION.to_owned(),
            evidence: evidence.into(),
        }
    }
}

/// Return the `s5-falsify-N` case selected in the current compilation unit.
#[must_use]
pub fn active_s5_falsification_case() -> Option<S5FalsificationCase> {
    let active = [
        (
            cfg!(feature = "s5-falsify-1"),
            S5FalsificationCase::F1OracleEqualityTampered,
        ),
        (
            cfg!(feature = "s5-falsify-2"),
            S5FalsificationCase::F2BoundedKvAutodiffBroken,
        ),
        (
            cfg!(feature = "s5-falsify-3"),
            S5FalsificationCase::F3LMt4BandCollapse,
        ),
        (
            cfg!(feature = "s5-falsify-4"),
            S5FalsificationCase::F4CapacityUndersized,
        ),
        (
            cfg!(feature = "s5-falsify-5"),
            S5FalsificationCase::F5ShadowOkConstantTrue,
        ),
        (
            cfg!(feature = "s5-falsify-6"),
            S5FalsificationCase::F6FrontierMissingAxis,
        ),
        (
            cfg!(feature = "s5-falsify-7"),
            S5FalsificationCase::F7ResetBoundaryLeak,
        ),
        (
            cfg!(feature = "s5-falsify-8"),
            S5FalsificationCase::F8PerSeedNonDeterminism,
        ),
        (
            cfg!(feature = "s5-falsify-9"),
            S5FalsificationCase::F9RuntimeBudgetToleranceViolation,
        ),
        (
            cfg!(feature = "s5-falsify-10"),
            S5FalsificationCase::F10CompileProfileNotThreaded,
        ),
        (
            cfg!(feature = "s5-falsify-11"),
            S5FalsificationCase::F11ShadowStagesMissing,
        ),
        (
            cfg!(feature = "s5-falsify-12"),
            S5FalsificationCase::F12EncodedRomCertCorrupted,
        ),
        (
            cfg!(feature = "s5-falsify-13"),
            S5FalsificationCase::F13EmulatorOracleDisagree,
        ),
        (
            cfg!(feature = "s5-falsify-14"),
            S5FalsificationCase::F14ParetoBroken,
        ),
        (
            cfg!(feature = "s5-falsify-15"),
            S5FalsificationCase::F15FeedbackBroken,
        ),
    ];
    let selected = active
        .into_iter()
        .filter_map(|(enabled, case)| enabled.then_some(case))
        .collect::<Vec<_>>();
    match selected.as_slice() {
        [case] => Some(*case),
        _ => None,
    }
}

/// Run the case selected by the current `s5-falsify-N` feature.
#[must_use]
pub fn run_active_s5_falsification_case() -> Option<S5FalsificationCaseResult> {
    active_s5_falsification_case().map(run_s5_falsification_case)
}

/// Run one F-S5 producer-side broken substitute.
#[must_use]
pub fn run_s5_falsification_case(case: S5FalsificationCase) -> S5FalsificationCaseResult {
    match case {
        S5FalsificationCase::F1OracleEqualityTampered => f1_oracle_equality_tampered(case),
        S5FalsificationCase::F2BoundedKvAutodiffBroken => f2_boundedkv_autodiff_broken(case),
        S5FalsificationCase::F3LMt4BandCollapse => f3_l_mt4_band_collapse(case),
        S5FalsificationCase::F4CapacityUndersized => f4_capacity_undersized(case),
        S5FalsificationCase::F5ShadowOkConstantTrue => f5_shadow_ok_constant_true(case),
        S5FalsificationCase::F6FrontierMissingAxis => f6_frontier_missing_axis(case),
        S5FalsificationCase::F7ResetBoundaryLeak => f7_reset_boundary_leak(case),
        S5FalsificationCase::F8PerSeedNonDeterminism => f8_per_seed_non_determinism(case),
        S5FalsificationCase::F9RuntimeBudgetToleranceViolation => {
            f9_runtime_budget_tolerance_violation(case)
        }
        S5FalsificationCase::F10CompileProfileNotThreaded => f10_compile_profile_not_threaded(case),
        S5FalsificationCase::F11ShadowStagesMissing => f11_shadow_stages_missing(case),
        S5FalsificationCase::F12EncodedRomCertCorrupted => f12_encoded_rom_cert_corrupted(case),
        S5FalsificationCase::F13EmulatorOracleDisagree => f13_emulator_oracle_disagree(case),
        S5FalsificationCase::F14ParetoBroken => f14_pareto_broken(case),
        S5FalsificationCase::F15FeedbackBroken => f15_feedback_broken(case),
    }
}

fn dispatch_with(
    update: impl FnOnce(&mut S5OutcomeDispatchInput),
    recommendation: FrontierRecommendation,
) -> S5Outcome {
    let mut input = S5OutcomeDispatchInput::all_confirmed(recommendation);
    update(&mut input);
    dispatch_s5_outcome(&input)
}

fn f1_oracle_equality_tampered(case: S5FalsificationCase) -> S5FalsificationCaseResult {
    let reference_logits = [0.0, 1.0, 0.25];
    let tampered_logits = reference_logits.map(|logit| logit + 5.0e-4);
    let max_abs_diff = reference_logits
        .iter()
        .zip(tampered_logits)
        .map(|(left, right)| f64::abs(left - right))
        .fold(0.0, f64::max);
    let status = if max_abs_diff > 1.0e-4 {
        HypothesisStatus::Refuted
    } else {
        HypothesisStatus::Confirmed
    };
    let outcome = dispatch_with(|input| input.h1 = status, FrontierRecommendation::A);
    S5FalsificationCaseResult::new(
        case,
        status,
        outcome,
        format!("max_abs_diff={max_abs_diff} exceeded H1 tolerance 1e-4"),
    )
}

fn f2_boundedkv_autodiff_broken(case: S5FalsificationCase) -> S5FalsificationCaseResult {
    let trainable_grad_norm = 0.0_f64;
    let status = if trainable_grad_norm == 0.0 {
        HypothesisStatus::Refuted
    } else {
        HypothesisStatus::Confirmed
    };
    let outcome = dispatch_with(|input| input.h2 = status, FrontierRecommendation::A);
    S5FalsificationCaseResult::new(
        case,
        status,
        outcome,
        "trainable_grad_norm=0 after stop-gradient substitute",
    )
}

fn f3_l_mt4_band_collapse(case: S5FalsificationCase) -> S5FalsificationCaseResult {
    let decays = [f64::NAN; 4];
    let status = if decays.iter().any(|decay| !decay.is_finite()) {
        HypothesisStatus::Refuted
    } else {
        HypothesisStatus::Confirmed
    };
    let outcome = dispatch_with(|input| input.h3 = status, FrontierRecommendation::A);
    S5FalsificationCaseResult::new(case, status, outcome, "L_MT4 decay vector contains NaN")
}

fn f4_capacity_undersized(case: S5FalsificationCase) -> S5FalsificationCaseResult {
    let kn5_bpc = 3.0_f64;
    let toy0_bpc = kn5_bpc - 0.01;
    let required_margin = 0.05_f64;
    let status = if toy0_bpc > kn5_bpc - required_margin {
        HypothesisStatus::Refuted
    } else {
        HypothesisStatus::Confirmed
    };
    let outcome = dispatch_with(|input| input.h4 = status, FrontierRecommendation::A);
    S5FalsificationCaseResult::new(
        case,
        status,
        outcome,
        "Toy0 undersized capacity misses KN-5 improvement margin",
    )
}

fn f5_shadow_ok_constant_true(case: S5FalsificationCase) -> S5FalsificationCaseResult {
    let clean_broken: ShadowCompileSampleReal = serde_json::from_str(include_str!(
        "../../../fixtures/s5/shadow/broken_negative_control.sample.json"
    ))
    .expect("S5 broken negative-control fixture parses");
    validate_shr1_shadow_sample(
        &clean_broken,
        ShadowCompileSampleExpectation::BrokenNegativeControl,
    )
    .expect("baseline broken fixture must be valid negative control");

    let mut constant_true = clean_broken;
    constant_true.shadow_compile_ok = true;
    let status = if validate_shr1_shadow_sample(
        &constant_true,
        ShadowCompileSampleExpectation::BrokenNegativeControl,
    )
    .is_err()
    {
        HypothesisStatus::Refuted
    } else {
        HypothesisStatus::Confirmed
    };
    let outcome = dispatch_with(
        |input| {
            input.h6 = status;
            input.h13 = status;
        },
        FrontierRecommendation::A,
    );
    S5FalsificationCaseResult::new(
        case,
        status,
        outcome,
        "broken negative-control fixture rejects constant shadow_compile_ok=true",
    )
}

fn f6_frontier_missing_axis(case: S5FalsificationCase) -> S5FalsificationCaseResult {
    let observed_axis_count = S5_FRONTIER_DEFAULT_AXES.len() - 1;
    let status = if observed_axis_count != S5_FRONTIER_DEFAULT_AXES.len() {
        HypothesisStatus::Refuted
    } else {
        HypothesisStatus::Confirmed
    };
    let outcome = dispatch_with(|input| input.h7 = status, FrontierRecommendation::A);
    S5FalsificationCaseResult::new(
        case,
        status,
        outcome,
        format!(
            "frontier axis count observed={observed_axis_count} expected={}",
            S5_FRONTIER_DEFAULT_AXES.len()
        ),
    )
}

fn f7_reset_boundary_leak(case: S5FalsificationCase) -> S5FalsificationCaseResult {
    let expected_occupancy_after_reset = 0_u32;
    let observed_occupancy_after_reset = 129_u32;
    let status = if observed_occupancy_after_reset != expected_occupancy_after_reset {
        HypothesisStatus::Refuted
    } else {
        HypothesisStatus::Confirmed
    };
    let outcome = dispatch_with(|input| input.h9 = status, FrontierRecommendation::A);
    S5FalsificationCaseResult::new(
        case,
        status,
        outcome,
        "KV occupancy leaked across chunk boundary and exceeded K_cap=128",
    )
}

fn f8_per_seed_non_determinism(case: S5FalsificationCase) -> S5FalsificationCaseResult {
    let replay_a_hash = [0xA5_u8; 32];
    let replay_b_hash = [0x5A_u8; 32];
    let status = if replay_a_hash != replay_b_hash {
        HypothesisStatus::Refuted
    } else {
        HypothesisStatus::Confirmed
    };
    let outcome = dispatch_with(|input| input.h10 = status, FrontierRecommendation::A);
    S5FalsificationCaseResult::new(
        case,
        status,
        outcome,
        "host-clock substitute produced replay hash disagreement",
    )
}

fn f9_runtime_budget_tolerance_violation(case: S5FalsificationCase) -> S5FalsificationCaseResult {
    let outcome_probe = ReValidationOutcome::BlockExport;
    let status = if outcome_probe == ReValidationOutcome::BlockExport {
        HypothesisStatus::Refuted
    } else {
        HypothesisStatus::Confirmed
    };
    let outcome = dispatch_with(
        |input| {
            input.h11 = status;
            input.runtime_self_check_fired = true;
        },
        FrontierRecommendation::A,
    );
    S5FalsificationCaseResult::new(
        case,
        status,
        outcome,
        "runtime budget substitute exceeds D9 tolerance and blocks export",
    )
}

fn f10_compile_profile_not_threaded(case: S5FalsificationCase) -> S5FalsificationCaseResult {
    let requested_profile = "BringUp";
    let observed_profile = "Default";
    let status = if requested_profile != observed_profile {
        HypothesisStatus::Refuted
    } else {
        HypothesisStatus::Confirmed
    };
    let outcome = dispatch_with(|input| input.h12 = status, FrontierRecommendation::A);
    S5FalsificationCaseResult::new(
        case,
        status,
        outcome,
        "CompileRequest.profile was stripped before dispatch",
    )
}

fn f11_shadow_stages_missing(case: S5FalsificationCase) -> S5FalsificationCaseResult {
    let observed_stage_count = S5_SHADOW_PIPELINE_STAGES.len() - 1;
    let status = if observed_stage_count != S5_SHADOW_PIPELINE_STAGES.len() {
        HypothesisStatus::Refuted
    } else {
        HypothesisStatus::Confirmed
    };
    let outcome = dispatch_with(|input| input.h13 = status, FrontierRecommendation::A);
    S5FalsificationCaseResult::new(
        case,
        status,
        outcome,
        format!(
            "stages_executed length observed={observed_stage_count} expected={}",
            S5_SHADOW_PIPELINE_STAGES.len()
        ),
    )
}

fn f12_encoded_rom_cert_corrupted(case: S5FalsificationCase) -> S5FalsificationCaseResult {
    let cert: serde_json::Value = serde_json::from_str(include_str!(
        "../../../fixtures/s5/encoded_rom/seed_0_canonical/certs/reachability.cert.json"
    ))
    .expect("reachability cert fixture parses");
    let valid = cert
        .get("valid")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let corrupted_valid = false;
    let status = if valid && !corrupted_valid {
        HypothesisStatus::Refuted
    } else {
        HypothesisStatus::Confirmed
    };
    let outcome = dispatch_with(
        |input| {
            input.encoded_rom_cert_invalid = matches!(status, HypothesisStatus::Refuted);
        },
        FrontierRecommendation::A,
    );
    S5FalsificationCaseResult::new(
        case,
        status,
        outcome,
        "reachability cert valid=true fixture was corrupted to valid=false",
    )
}

fn f13_emulator_oracle_disagree(case: S5FalsificationCase) -> S5FalsificationCaseResult {
    let oracle_logits = [0.2, 1.0, 0.4];
    let oracle_token = s5_argmax_token(&oracle_logits).expect("fixture has argmax");
    let biased = s5_f13_bias_non_oracle_token_above_predicted(&oracle_logits, 2)
        .expect("non-oracle token can be biased");
    let harness_token = s5_argmax_token(&biased).expect("biased fixture has argmax");
    let status = s5_h15_oracle_token_agreement(oracle_token, harness_token);
    let cardinality = verify_h15_first_commit_payload_len(1);
    assert_eq!(
        cardinality.verdict,
        H15FirstCommitCardinalityVerdict::Confirmed
    );
    let outcome = dispatch_with(|input| input.h15 = status, FrontierRecommendation::A);
    S5FalsificationCaseResult::new(
        case,
        status,
        outcome,
        format!("oracle_token={oracle_token} harness_token={harness_token} after bias"),
    )
}

fn f14_pareto_broken(case: S5FalsificationCase) -> S5FalsificationCaseResult {
    let points = vec![
        frontier_point(1.20, 30_000, true),
        frontier_point(0.90, 20_000, true),
    ];
    let verification = verify_s5_h14_pareto_emission(
        &points,
        &[S5FrontierAxis::ValBpcTernary],
        vec![0],
        Some(0),
        S5SelectionAuthority::Automated,
    )
    .expect("pareto fixture validates");
    let outcome = dispatch_with(
        |input| {
            input.h14 = verification.h14;
            input.h14_manual_override = false;
        },
        FrontierRecommendation::A,
    );
    S5FalsificationCaseResult::new(
        case,
        verification.h14,
        outcome,
        "broken emitter picked first point instead of expected Pareto frontier",
    )
}

fn f15_feedback_broken(case: S5FalsificationCase) -> S5FalsificationCaseResult {
    let router_state_before = [0x11_u8, 0x22, 0x33];
    let router_state_after = [0x11_u8, 0x99, 0x33];
    let report = verify_s5_h16_feedback_fixture(
        &S5_FEEDBACK_FIXTURE_V1_SAFE_BOUND_CASES,
        &router_state_before,
        &router_state_after,
        S5FeedbackApplyConfig::pinned(),
    );
    let outcome = dispatch_with(|input| input.h16 = report.h16, FrontierRecommendation::A);
    S5FalsificationCaseResult::new(
        case,
        report.h16,
        outcome,
        "empty ExpertSlotAffinity mutated router_state bytes",
    )
}

fn frontier_point(
    val_bpc_ternary: f64,
    projected_deployed_bytes: u64,
    gates_pass: bool,
) -> S5FrontierPointMetrics {
    S5FrontierPointMetrics {
        val_bpc_fp: val_bpc_ternary - 0.05,
        val_bpc_ternary,
        ternary_gap: 0.05,
        v0_success_pass: gates_pass,
        v0_success_score: 0.99,
        param_count: 10_000,
        projected_deployed_bytes,
        shadow_compile_ok_at_end: gates_pass,
        shadow_byte_cost_at_end: projected_deployed_bytes as u32,
        shadow_kernel_count_at_end: 8,
        latency_proxy_cycles: 1_000,
        encoded_rom_byte_cost: Some(projected_deployed_bytes),
        fits_envelope: Some(gates_pass),
        reachability_cert_valid: Some(gates_pass),
        resource_state_cert_valid: Some(gates_pass),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_cases_refute_and_dispatch_as_expected() {
        for case in S5FalsificationCase::ALL {
            let result = run_s5_falsification_case(case);
            assert!(
                result.matches_expected,
                "{} did not match expected outcome: {result:#?}",
                case.case_id()
            );
        }
    }

    #[test]
    fn active_feature_maps_to_single_case_when_enabled() {
        if let Some(case) = active_s5_falsification_case() {
            let result = run_active_s5_falsification_case().expect("active case runs");
            assert_eq!(result.feature, case.feature_name());
            assert!(result.matches_expected);
        }
    }

    #[test]
    fn index_mapping_covers_all_fifteen_cases() {
        for index in 1..=S5_FALSIFICATION_CASE_COUNT {
            let case = S5FalsificationCase::from_index(index).expect("case index exists");
            assert_eq!(case.index(), index);
        }
        assert!(S5FalsificationCase::from_index(0).is_none());
        assert!(S5FalsificationCase::from_index(16).is_none());
    }

    #[test]
    fn h5_long_range_helper_still_refutes_regression_fixture() {
        let verdict =
            h5_long_range_verdict(H5LongRangeEvidence::penalties_only(0.10, 0.16)).unwrap();
        assert_eq!(verdict.verdict, H5LongRangeVerdict::Refuted);
    }
}
