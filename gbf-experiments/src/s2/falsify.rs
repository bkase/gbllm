//! Compile-gated S2 falsification harness.

use std::cell::RefCell;
use std::collections::BTreeMap;

use gbf_foundation::{Hash256, sha256};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::S2_LOG_TARGET;
use crate::s1::schema::{S1CanonicalJson, S1SchemaError};
use crate::s2::linearstate_smoke::{
    LinearStateSmokeRun, STATE_READOUT_OUTPUT_PROJECTION_WEIGHT, run_fixture_v1,
};
use crate::s2::loss_grad_flow::{H5_4B_SUBCHECK_NAME, h5_4_fixture_with_zero_raw_honesty};
use crate::s2::run::{RunInputs, RunProductS2, S2TrainRunError, s2_train_run};
use crate::s2::schema::{
    DiagnosticSubcheckResult, FixtureResult, HardnessTriple, LossGradFlowReport, PhaseEntry,
    PhaseLog, QuantHardness, S2_PHASE_B_END_STEP, S2_PHASE_C_END_STEP, S2BuildKind, S2SchemaError,
    TrainConfigS2Full,
};

thread_local! {
    static ACTIVE_BROKEN_KIND: RefCell<Option<BrokenKind>> = const { RefCell::new(None) };
}

/// One deliberately broken S2 implementation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrokenKind {
    /// F1-broken-S2: Phase B skips ternary and C/D stay expert-QAT Off.
    F1PhaseBSkipsTernary,
    /// F2-broken-S2: Phase D unfreezes the teacher.
    F2PhaseDUnfreezesTeacher,
    /// F3-broken-S2: distillation temperature is inverted to 0.5.
    F3DistillTempInverted,
    /// F4-broken-S2: structural fallback for illegal per-weight threshold behavior.
    ///
    /// This hand-modeled fixture only proves the H5.4 report notices a
    /// threshold-mask mismatch. It does not mutate the real zero-loss helper or
    /// Burn adapter path.
    F4ThresholdPerWeight,
    /// F5-broken-S2: diagnostic-runner fallback for a zero-loss raw helper short-circuit.
    ///
    /// This is intentionally not an end-to-end mutation of `zero_loss` or the
    /// Burn adapter. It injects the failed H5.4b diagnostic result that would
    /// be produced if the raw helper skipped computation at `lambda_zero = 0`.
    F5ZeroLossShortCircuit,
    /// F6-broken-S2: S2 H6 smoke-layer structural fallback for dead recurrence/readout flow.
    ///
    /// This is intentionally not a mutation of the public LinearState Burn
    /// adapter. The H6 wrapper supplies the smallest structural fixture that
    /// removes the recurrence/readout gradient before report validation.
    F6LinearStateGradDead,
}

impl BrokenKind {
    /// Stable log/report label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::F1PhaseBSkipsTernary => "F1_phase_b_skips_ternary",
            Self::F2PhaseDUnfreezesTeacher => "F2_phase_d_unfreezes_teacher",
            Self::F3DistillTempInverted => "F3_distill_temperature_inverted",
            Self::F4ThresholdPerWeight => "F4_threshold_per_weight_structural_mask_fixture",
            Self::F5ZeroLossShortCircuit => "F5_zero_loss_diagnostic_runner_fallback",
            Self::F6LinearStateGradDead => "F6_linearstate_structural_smoke_fallback",
        }
    }

    /// Expected verifier/config verdict.
    #[must_use]
    pub const fn expected_verdict(self) -> &'static str {
        match self {
            Self::F1PhaseBSkipsTernary => "H1 Refuted",
            Self::F2PhaseDUnfreezesTeacher => "H1 Refuted",
            Self::F3DistillTempInverted => "config-validator rejected",
            Self::F4ThresholdPerWeight => "H5.4 structural mask fixture Refuted",
            Self::F5ZeroLossShortCircuit => "H5.4b diagnostic-runner fallback Refuted",
            Self::F6LinearStateGradDead => "H6 structural smoke fallback Refuted",
        }
    }
}

/// RAII guard for an installed broken implementation.
#[derive(Debug)]
pub struct Guard {
    previous: Option<BrokenKind>,
}

/// Install one broken implementation for the current thread.
#[must_use]
pub fn install_broken_impl(kind: BrokenKind) -> Guard {
    tracing::debug!(
        target: S2_LOG_TARGET,
        event_name = "falsify_flag_active",
        kind = kind.as_str(),
        "s2 falsify flag active"
    );
    let previous = ACTIVE_BROKEN_KIND.with(|active| active.replace(Some(kind)));
    Guard { previous }
}

impl Drop for Guard {
    fn drop(&mut self) {
        ACTIVE_BROKEN_KIND.with(|active| {
            active.replace(self.previous);
        });
    }
}

/// Return the current broken implementation for this thread.
#[must_use]
pub fn active_broken_kind() -> Option<BrokenKind> {
    ACTIVE_BROKEN_KIND.with(|active| *active.borrow())
}

/// Whether a specific broken implementation is active on this thread.
#[must_use]
pub fn is_active(kind: BrokenKind) -> bool {
    active_broken_kind() == Some(kind)
}

/// Result for one falsification case.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FalsificationCaseResult {
    /// Broken implementation label.
    pub broken_kind: String,
    /// Expected verdict.
    pub expected_verdict: String,
    /// Observed verdict.
    pub observed_verdict: String,
    /// Whether the expected verifier/config rejection happened.
    pub passed: bool,
}

impl FalsificationCaseResult {
    /// Construct a case result.
    #[must_use]
    pub fn new(kind: BrokenKind, observed_verdict: impl Into<String>, passed: bool) -> Self {
        Self {
            broken_kind: kind.as_str().to_owned(),
            expected_verdict: kind.expected_verdict().to_owned(),
            observed_verdict: observed_verdict.into(),
            passed,
        }
    }
}

/// Canonical S2 falsification suite report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FalsificationSuiteReport {
    /// Schema id.
    pub schema: String,
    /// Per-source SHA-256 digests for the six falsification test files.
    ///
    /// Keys are workspace-relative paths so the suite hash carries provenance
    /// beyond ambiguous `f1.rs`...`f6.rs` basenames.
    pub source_digests: BTreeMap<String, Hash256>,
    /// Per-case results.
    pub results: Vec<FalsificationCaseResult>,
    /// AND over the six expected catches.
    pub falsification_s2_passed: bool,
    /// Canonical suite hash over source digests and results.
    pub falsification_s2_suite_hash: Hash256,
}

/// Build the canonical falsification suite report and hash.
pub fn suite_report(
    source_digests: BTreeMap<String, Hash256>,
    results: Vec<FalsificationCaseResult>,
) -> Result<FalsificationSuiteReport, S1SchemaError> {
    let falsification_s2_passed = results.iter().all(|result| result.passed);
    let preimage = json!({
        "schema": "s2_falsification_suite.v1",
        "source_digests": source_digests,
        "results": results,
        "falsification_s2_passed": falsification_s2_passed,
    });
    let falsification_s2_suite_hash = sha256(S1CanonicalJson::value_to_vec(&preimage)?);
    Ok(FalsificationSuiteReport {
        schema: "s2_falsification_suite.v1".to_owned(),
        source_digests,
        results,
        falsification_s2_passed,
        falsification_s2_suite_hash,
    })
}

/// Emit the standard falsification start log.
pub fn log_test_start(kind: BrokenKind) {
    tracing::info!(
        target: S2_LOG_TARGET,
        event_name = "falsify_test_start",
        broken_kind = kind.as_str(),
        expected_verdict = kind.expected_verdict(),
        "s2 falsify test start"
    );
}

/// Emit the standard falsification completion log.
pub fn log_test_done(result: &FalsificationCaseResult) {
    tracing::info!(
        target: S2_LOG_TARGET,
        event_name = "falsify_test_done",
        broken_kind = result.broken_kind,
        observed_verdict = result.observed_verdict,
        passed = result.passed,
        "s2 falsify test done"
    );
    if !result.passed {
        tracing::error!(
            target: S2_LOG_TARGET,
            event_name = "falsify_verifier_insensitive",
            broken_kind = result.broken_kind,
            remediation = "the corresponding verifier (H1/H4/H5/H6) is missing a check; see CLAUDE.md 'Training Loss Beads' falsification rule",
            "s2 falsify verifier insensitive"
        );
    }
}

/// Clean tiny S2 run material used by F1/F2 before injecting broken evidence.
pub struct H1FalsificationFixture {
    /// Phase-log header.
    pub phase_log: PhaseLog,
    /// Phase-log entries.
    pub entries: Vec<PhaseEntry>,
}

/// Produce an H1 fixture with the active F1 or F2 broken implementation applied.
pub fn h1_fixture_for_active_broken_kind() -> Result<H1FalsificationFixture, S2TrainRunError> {
    let active = active_broken_kind();
    let product = s2_train_run(&RunInputs::tiny_fixture(0, S2BuildKind::s2_ternary_full))?;
    let RunProductS2::Completed(product) = product else {
        return Err(S2TrainRunError::TrainLoop(
            "falsification fixture diverged".to_owned(),
        ));
    };
    let mut entries = product.phase_entries.clone();
    match active {
        Some(BrokenKind::F1PhaseBSkipsTernary) => {
            for entry in entries
                .iter_mut()
                .filter(|entry| entry.step > S2_PHASE_B_END_STEP)
            {
                entry.hardness = HardnessTriple {
                    expert_qat: QuantHardness::Off,
                    ..entry.hardness
                };
            }
        }
        Some(BrokenKind::F2PhaseDUnfreezesTeacher) => {
            for entry in entries
                .iter_mut()
                .filter(|entry| entry.step > S2_PHASE_C_END_STEP)
            {
                entry.teacher_frozen = false;
            }
        }
        _ => {}
    }
    Ok(H1FalsificationFixture {
        phase_log: product.phase_log.clone(),
        entries,
    })
}

/// Return the F3 broken config with an inverted distillation temperature.
#[must_use]
pub fn distill_temperature_inverted_config() -> TrainConfigS2Full {
    let mut config = TrainConfigS2Full::pinned();
    if is_active(BrokenKind::F3DistillTempInverted) {
        config.distill_temp = 0.5;
    }
    config
}

/// F4 threshold-per-weight evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThresholdPerWeightEvidence {
    /// Legal mask under one threshold per output row.
    pub legal_per_row_mask: Vec<bool>,
    /// Candidate mask under one threshold per individual weight.
    ///
    /// This mask is illegal/deviating only when the F4 Guard is active. In the
    /// inactive branch it intentionally matches `legal_per_row_mask` so the
    /// expected-passing fixture cannot masquerade as a refutation.
    pub illegal_per_weight_mask: Vec<bool>,
    /// Whether the illegal mask deviates from the legal D4 mask.
    pub mask_deviates: bool,
    /// Whether the per-weight shape is rejected by the per-row constructor.
    pub per_weight_shape_rejected: bool,
}

/// F4 structural fixture evidence after routing the broken mask through H5.
///
/// This evidence proves the H5 report notices a structural mismatch in the
/// active zero-loss mask. It does not claim an end-to-end mutation of the real
/// zero-loss helper or Burn adapter path.
#[derive(Debug, Clone, PartialEq)]
pub struct ThresholdPerWeightVerifierEvidence {
    /// Local threshold/mask evidence used to build the H5.4 fixture.
    pub threshold_evidence: ThresholdPerWeightEvidence,
    /// Whether the H5.4 fixture refuted its sub-hypothesis.
    pub h5_4_refuted: bool,
    /// Whether the aggregate H5 verifier/report is refuted.
    pub h5_refuted: bool,
    /// H5 loss-gradient-flow report that carried the broken fixture.
    pub report: LossGradFlowReport,
}

/// F5 fallback evidence after routing the short-circuit diagnostic through the H5 report.
///
/// This evidence proves H5 report sensitivity to a failed H5.4b diagnostic
/// result. It does not claim that the production `zero_loss` helper or Burn
/// adapter was mutated end-to-end in this falsification case.
#[derive(Debug, Clone, PartialEq)]
pub struct ZeroShortCircuitVerifierEvidence {
    /// H5.4 fixture carrying the failed raw-honesty diagnostic fallback.
    pub h5_4_fixture: FixtureResult,
    /// Whether the H5.4 fixture refuted its sub-hypothesis.
    pub h5_4_refuted: bool,
    /// Whether the aggregate H5 verifier/report is refuted.
    pub h5_refuted: bool,
    /// H5 loss-gradient-flow report that carried the broken fixture.
    pub report: LossGradFlowReport,
}

/// F6 fallback evidence after routing structural dead recurrence through the H6 smoke report.
///
/// This evidence proves H6 report sensitivity to a structurally dead
/// recurrence/readout fixture. It does not claim an end-to-end mutation of the
/// public LinearState Burn adapter.
#[derive(Debug, Clone, PartialEq)]
pub struct LinearStateGradDeadVerifierEvidence {
    /// H6 smoke run produced by the active structural fallback.
    pub run: LinearStateSmokeRun,
    /// Recurrence/readout gradient observed by the H6 report.
    pub recurrence_grad_norm: Option<f32>,
    /// Whether the aggregate H6 smoke verifier/report is refuted.
    pub h6_refuted: bool,
}

/// Produce F4 evidence for an illegal per-weight threshold plan.
pub fn threshold_per_weight_evidence() -> ThresholdPerWeightEvidence {
    let weights = [0.45_f32, 0.65, 0.55, 0.35];
    let legal_row_thresholds = [0.5_f32, 0.5];
    // The Guard controls only this structural threshold-selection fixture:
    // active Guard chooses illegal per-weight thresholds that alter the mask;
    // inactive Guard uses legal per-row-equivalent thresholds and should not
    // refute H5.4.
    let per_weight_thresholds = if is_active(BrokenKind::F4ThresholdPerWeight) {
        [0.4_f32, 0.7, 0.7, 0.4]
    } else {
        [0.5_f32, 0.5, 0.5, 0.5]
    };
    let legal_per_row_mask = weights
        .iter()
        .enumerate()
        .map(|(index, weight)| weight.abs() < legal_row_thresholds[index / 2])
        .collect::<Vec<_>>();
    // Keep the historical field name for snapshot continuity, but treat it as
    // a candidate per-weight mask: only the active Guard branch is actually
    // illegal, while the inactive branch is legal-equivalent and expected to
    // pass.
    let illegal_per_weight_mask = weights
        .iter()
        .zip(per_weight_thresholds)
        .map(|(weight, threshold)| weight.abs() < threshold)
        .collect::<Vec<_>>();
    let per_weight_shape_rejected = crate::s2::run::threshold_init::PerRowThresholds::new(
        "f4.illegal",
        2,
        per_weight_thresholds.to_vec(),
    )
    .is_err();
    ThresholdPerWeightEvidence {
        mask_deviates: legal_per_row_mask != illegal_per_weight_mask,
        legal_per_row_mask,
        illegal_per_weight_mask,
        per_weight_shape_rejected,
    }
}

/// Route F4 structural threshold-mask behavior through the H5.4 report path.
pub fn threshold_per_weight_verifier_evidence()
-> Result<ThresholdPerWeightVerifierEvidence, S1SchemaError> {
    let threshold_evidence = threshold_per_weight_evidence();
    let h5_4_fixture = h5_4_threshold_per_weight_fixture(&threshold_evidence);
    let h5_4_refuted = !h5_4_fixture.sub_passed;
    let report = LossGradFlowReport::new(vec![
        passing_loss_grad_flow_fixture("H5.1", "lambda_zrouter"),
        passing_loss_grad_flow_fixture("H5.2", "lambda_balance"),
        passing_loss_grad_flow_fixture("H5.3", "lambda_range"),
        h5_4_fixture,
        passing_loss_grad_flow_fixture("H5.5", "lambda_distill"),
    ])?;
    let h5_refuted = !report.overall_passed;

    Ok(ThresholdPerWeightVerifierEvidence {
        threshold_evidence,
        h5_4_refuted,
        h5_refuted,
        report,
    })
}

/// Route F5 diagnostic-runner fallback behavior through the H5.4 verifier path.
pub fn zero_short_circuit_verifier_evidence()
-> Result<ZeroShortCircuitVerifierEvidence, S1SchemaError> {
    let h5_4_fixture = h5_4_fixture_with_zero_raw_honesty()
        .map_err(|error| S1SchemaError::Custom(error.to_string()))?;
    let h5_4_refuted = !h5_4_fixture.sub_passed;
    let report = LossGradFlowReport::new(vec![
        passing_loss_grad_flow_fixture("H5.1", "lambda_zrouter"),
        passing_loss_grad_flow_fixture("H5.2", "lambda_balance"),
        passing_loss_grad_flow_fixture("H5.3", "lambda_range"),
        h5_4_fixture.clone(),
        passing_loss_grad_flow_fixture("H5.5", "lambda_distill"),
    ])?;
    let h5_refuted = !report.overall_passed;

    Ok(ZeroShortCircuitVerifierEvidence {
        h5_4_fixture,
        h5_4_refuted,
        h5_refuted,
        report,
    })
}

/// Route F6 structural-dead-recurrence fallback behavior through the H6 smoke report.
pub fn linearstate_grad_dead_verifier_evidence()
-> Result<LinearStateGradDeadVerifierEvidence, S1SchemaError> {
    let run = run_fixture_v1().map_err(|error| S1SchemaError::Custom(error.to_string()))?;
    let recurrence_grad_norm = run
        .report
        .param_grad_norms
        .get(STATE_READOUT_OUTPUT_PROJECTION_WEIGHT)
        .copied();
    let h6_refuted = !run.report.smoke_passed;

    Ok(LinearStateGradDeadVerifierEvidence {
        run,
        recurrence_grad_norm,
        h6_refuted,
    })
}

fn h5_4_threshold_per_weight_fixture(evidence: &ThresholdPerWeightEvidence) -> FixtureResult {
    let mut in_scope_grad_norms = BTreeMap::new();
    // F4 only claims H5.4 verifier sensitivity to the illegal mask changing
    // the active zero-loss weight set. Per-weight threshold buffers are still
    // rejected at the deployable constructor boundary, even in the no-deviation
    // branch covered by the non-Guard regression test.
    // The 0.0/0.25 values below are structural placeholders for "mask changed"
    // versus "mask unchanged"; they are not measured zero-loss helper or Burn
    // adapter gradients.
    in_scope_grad_norms.insert(
        "lambda_zero_d4_active_weights".to_owned(),
        if evidence.mask_deviates { 0.0 } else { 0.25 },
    );
    let mut stop_gradient_grad_norms = BTreeMap::new();
    stop_gradient_grad_norms.insert("lambda_zero_thresholds".to_owned(), 0.0);
    // F4 perturbs threshold shape/mask selection only. Keep H5.4b raw-honesty
    // forced passing so this fixture cannot be mistaken for an F5
    // lambda_zero raw-helper short-circuit.
    let diagnostic_subchecks = vec![
        DiagnosticSubcheckResult {
            name: H5_4B_SUBCHECK_NAME.to_owned(),
            lambda_value: 0.0,
            raw_loss_computed: true,
            raw_loss_finite: true,
            weighted_loss_value: Some(0.0),
            passed: true,
        },
        DiagnosticSubcheckResult {
            name: "per_row_threshold_shape_rejected".to_owned(),
            lambda_value: 0.5,
            raw_loss_computed: true,
            raw_loss_finite: true,
            weighted_loss_value: Some(0.0),
            passed: evidence.per_weight_shape_rejected,
        },
    ];
    let sub_passed = !evidence.mask_deviates && evidence.per_weight_shape_rejected;

    FixtureResult {
        sub_hypothesis: "H5.4".to_owned(),
        loss_term: "lambda_zero".to_owned(),
        in_scope_grad_norms,
        stop_gradient_grad_norms,
        non_default_value_used: true,
        numerical_stability_passed: true,
        diagnostic_subchecks,
        detached_grad_absence: BTreeMap::new(),
        sub_passed,
    }
}

// Generic expected-passing H5 fixture rows used to pad unrelated subchecks.
// These rows are deliberately inert context around the active falsification
// fixture in each case; they should never be read as helper/adapter fault
// injection.
fn passing_loss_grad_flow_fixture(sub_hypothesis: &str, loss_term: &str) -> FixtureResult {
    let mut in_scope_grad_norms = BTreeMap::new();
    in_scope_grad_norms.insert(format!("{loss_term}_target"), 0.25);
    let mut stop_gradient_grad_norms = BTreeMap::new();
    stop_gradient_grad_norms.insert(format!("{loss_term}_detached"), 0.0);
    if sub_hypothesis == "H5.5" {
        stop_gradient_grad_norms.insert("teacher_logits".to_owned(), 0.0);
    }
    let mut diagnostic_subchecks = vec![DiagnosticSubcheckResult {
        name: format!("{loss_term}_finite_raw"),
        lambda_value: 0.5,
        raw_loss_computed: true,
        raw_loss_finite: true,
        weighted_loss_value: Some(0.125),
        passed: true,
    }];
    if sub_hypothesis == "H5.4" {
        // H5.4 is the only generic passing fixture that also needs the H5.4b
        // raw-honesty subcheck present. Keep it explicitly passing here:
        // F4 only exercises structural threshold-mask sensitivity, while the
        // failed raw-helper short-circuit is owned by the separate F5
        // diagnostic-runner fallback.
        diagnostic_subchecks.push(DiagnosticSubcheckResult {
            name: H5_4B_SUBCHECK_NAME.to_owned(),
            lambda_value: 0.0,
            raw_loss_computed: true,
            raw_loss_finite: true,
            weighted_loss_value: Some(0.0),
            passed: true,
        });
    }

    FixtureResult {
        sub_hypothesis: sub_hypothesis.to_owned(),
        loss_term: loss_term.to_owned(),
        in_scope_grad_norms,
        stop_gradient_grad_norms,
        non_default_value_used: true,
        numerical_stability_passed: true,
        diagnostic_subchecks,
        detached_grad_absence: BTreeMap::new(),
        sub_passed: true,
    }
}

/// Return whether F3 rejection was the pinned-temperature validator.
#[must_use]
pub fn is_distill_temperature_rejection(error: &S2SchemaError) -> bool {
    error
        .to_string()
        .contains("distill_temp must be pinned to 2.0")
}
