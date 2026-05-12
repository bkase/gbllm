//! S2 loss-gradient-flow synthetic fixtures.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use gbf_train::loss::zero::{PerRowThresholds, ZeroLossError, weighted_zero_loss, zero_loss};

use crate::S2_LOG_TARGET;
use crate::s2::schema::{DiagnosticSubcheckResult, FixtureResult};

/// H5.4b zero-loss diagnostic subcheck name.
pub const H5_4B_SUBCHECK_NAME: &str = "lambda_zero_raw_honesty_at_zero_weight";

/// Non-default lambda used by the H5.5 distillation fixture.
pub const H5_5_LAMBDA_DISTILL: f32 = 0.5;

/// Non-default distillation temperature used by the H5.5 fixture.
pub const H5_5_DISTILL_TEMPERATURE: f32 = 1.0;

/// Errors from S2 loss-gradient-flow fixture construction.
#[derive(Debug)]
pub enum LossGradFlowFixtureError {
    /// Zero-loss helper rejected the synthetic H5.4b fixture.
    ZeroLoss(ZeroLossError),
    /// Burn-backed distillation fixture failed.
    #[cfg(feature = "s2-full")]
    Distillation(gbf_train::loss::distillation::DistillationLossError),
    /// Burn tensor adapter failed.
    #[cfg(feature = "s2-full")]
    BurnAdapter(gbf_train::adapter::burn::BurnAdapterError),
    /// H5.5 produced no student gradient.
    #[cfg(feature = "s2-full")]
    MissingStudentGradient,
}

impl fmt::Display for LossGradFlowFixtureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroLoss(error) => write!(f, "{error}"),
            #[cfg(feature = "s2-full")]
            Self::Distillation(error) => write!(f, "{error}"),
            #[cfg(feature = "s2-full")]
            Self::BurnAdapter(error) => write!(f, "{error}"),
            #[cfg(feature = "s2-full")]
            Self::MissingStudentGradient => {
                f.write_str("H5.5 student logits did not receive gradients")
            }
        }
    }
}

impl Error for LossGradFlowFixtureError {}

impl From<ZeroLossError> for LossGradFlowFixtureError {
    fn from(error: ZeroLossError) -> Self {
        Self::ZeroLoss(error)
    }
}

#[cfg(feature = "s2-full")]
impl From<gbf_train::loss::distillation::DistillationLossError> for LossGradFlowFixtureError {
    fn from(error: gbf_train::loss::distillation::DistillationLossError) -> Self {
        Self::Distillation(error)
    }
}

#[cfg(feature = "s2-full")]
impl From<gbf_train::adapter::burn::BurnAdapterError> for LossGradFlowFixtureError {
    fn from(error: gbf_train::adapter::burn::BurnAdapterError) -> Self {
        Self::BurnAdapter(error)
    }
}

/// Run the H5.5 distillation fixture and return its loss-grad-flow result.
#[cfg(feature = "s2-full")]
pub fn run_h5_5_distill_fixture() -> Result<FixtureResult, LossGradFlowFixtureError> {
    use gbf_train::adapter::burn::{
        BurnDevice, BurnNdArrayAutodiffBackend, float_tensor_from_vec, float_tensor_into_vec,
    };
    use gbf_train::loss::distillation::{burn_distillation_loss, burn_weighted_distillation_loss};

    type B = BurnNdArrayAutodiffBackend;

    let device = BurnDevice::<B>::default();
    let student_values = vec![0.2, -0.1, 0.4, -0.3, 0.0, 0.3, -0.2, 0.5];
    let teacher_values = vec![0.7, -0.4, 0.6, -0.7, -0.1, 0.7, -0.8, 0.8];
    let student_logits =
        float_tensor_from_vec::<B, 2>(student_values, [2, 4], &device)?.require_grad();
    let teacher_logits = float_tensor_from_vec::<B, 2>(teacher_values, [2, 4], &device)?;

    let raw_loss = burn_distillation_loss(
        student_logits.clone(),
        teacher_logits.clone(),
        1,
        H5_5_DISTILL_TEMPERATURE,
    )?;
    let weighted_loss = burn_weighted_distillation_loss(raw_loss, H5_5_LAMBDA_DISTILL)?;
    let gradients = weighted_loss.backward();
    let student_grad = student_logits
        .grad(&gradients)
        .ok_or(LossGradFlowFixtureError::MissingStudentGradient)?;
    let student_grad_values = float_tensor_into_vec(student_grad)?;
    let student_grad_norm = grad_l2_norm(&student_grad_values);
    let teacher_grad_values = match teacher_logits.grad(&gradients) {
        Some(teacher_grad) => Some(float_tensor_into_vec(teacher_grad)?),
        None => None,
    };
    let teacher_grad_absence = teacher_grad_values.is_none();
    let teacher_grad_norm = teacher_grad_values
        .as_deref()
        .map(grad_l2_norm)
        .unwrap_or(0.0);
    let numerical_stability_passed = student_grad_norm.is_finite() && teacher_grad_norm.is_finite();
    let diagnostic_subchecks = Vec::<DiagnosticSubcheckResult>::new();
    let sub_passed = numerical_stability_passed
        && diagnostic_subchecks.iter().all(|subcheck| subcheck.passed)
        && student_grad_norm.is_finite()
        && student_grad_norm > 0.0
        && teacher_grad_norm.is_finite()
        && teacher_grad_norm <= 1.0e-6;

    tracing::info!(
        target: S2_LOG_TARGET,
        event_name = "h5_5_fixture_run",
        batch = 2_u32,
        vocab = 4_u32,
        lambda_distill = H5_5_LAMBDA_DISTILL,
        distill_temperature = H5_5_DISTILL_TEMPERATURE,
        student_grad_norm = student_grad_norm,
        teacher_grad_norm = if teacher_grad_absence { None } else { Some(teacher_grad_norm) },
        teacher_grad_absence = teacher_grad_absence,
        non_default_value_used = true,
        sub_passed = sub_passed,
        "s2 h5.5 distillation fixture run"
    );

    let mut in_scope_grad_norms = BTreeMap::new();
    in_scope_grad_norms.insert("student_logits".to_owned(), student_grad_norm);
    let mut stop_gradient_grad_norms = BTreeMap::new();
    stop_gradient_grad_norms.insert("teacher_logits".to_owned(), teacher_grad_norm);
    let mut detached_grad_absence = BTreeMap::new();
    detached_grad_absence.insert("teacher_logits".to_owned(), teacher_grad_absence);

    Ok(FixtureResult {
        sub_hypothesis: "H5.5".to_owned(),
        loss_term: "lambda_distill".to_owned(),
        in_scope_grad_norms,
        stop_gradient_grad_norms,
        non_default_value_used: true,
        numerical_stability_passed,
        diagnostic_subchecks,
        detached_grad_absence,
        sub_passed,
    })
}

/// Run the H5.4b zero-loss raw-honesty diagnostic subcheck.
pub fn run_h5_4b_zero_raw_honesty_subcheck()
-> Result<DiagnosticSubcheckResult, LossGradFlowFixtureError> {
    #[cfg(feature = "falsify")]
    if crate::s2::falsify::is_active(crate::s2::falsify::BrokenKind::F5ZeroLossShortCircuit) {
        return Ok(h5_4b_short_circuit_detected_subcheck());
    }

    let weights = [
        -0.1, 0.1, -0.1, 0.1, -0.7, 0.7, -0.7, 0.7, 0.1, -0.1, 0.1, -0.1, 0.7, -0.7, 0.7, -0.7,
        -0.1, -0.1, 0.1, 0.1, -0.7, -0.7, 0.7, 0.7, 0.1, 0.1, -0.1, -0.1, 0.7, 0.7, -0.7, -0.7,
    ];
    let thresholds = PerRowThresholds::new(vec![0.5, 0.5, 0.5, 0.5])?.for_rows(4)?;
    let raw_loss = zero_loss(&weights, 4, 8, &thresholds)?;
    let weighted_loss = weighted_zero_loss(raw_loss, 0.0)?;
    let raw_loss_finite = raw_loss.is_finite();
    let passed = raw_loss_finite && weighted_loss == 0.0;

    tracing::info!(
        target: S2_LOG_TARGET,
        event_name = "h5_4b_subcheck_run",
        lambda_value = 0.0_f32,
        raw_loss_computed = true,
        raw_loss_finite = raw_loss_finite,
        weighted_loss_value = weighted_loss,
        passed = passed,
        "s2 h5.4b zero-loss raw-honesty subcheck run"
    );

    Ok(DiagnosticSubcheckResult {
        name: H5_4B_SUBCHECK_NAME.to_owned(),
        lambda_value: 0.0,
        raw_loss_computed: true,
        raw_loss_finite,
        weighted_loss_value: Some(weighted_loss),
        passed,
    })
}

/// Diagnostic result used by tests to model F5-broken short-circuit behavior.
#[must_use]
pub fn h5_4b_short_circuit_detected_subcheck() -> DiagnosticSubcheckResult {
    tracing::error!(
        target: S2_LOG_TARGET,
        event_name = "h5_4b_short_circuit_detected",
        remediation = "zero_loss helper must invoke L1 sum even at lambda=0; see CLAUDE.md 'Training Loss Beads' raw-helper rule",
        "s2 h5.4b zero-loss short circuit detected"
    );
    DiagnosticSubcheckResult {
        name: H5_4B_SUBCHECK_NAME.to_owned(),
        lambda_value: 0.0,
        raw_loss_computed: false,
        raw_loss_finite: false,
        weighted_loss_value: None,
        passed: false,
    }
}

/// Build an H5.4 fixture with the H5.4b zero-weight diagnostic attached.
pub fn h5_4_fixture_with_zero_raw_honesty() -> Result<FixtureResult, LossGradFlowFixtureError> {
    let mut in_scope_grad_norms = BTreeMap::new();
    in_scope_grad_norms.insert("lambda_zero_weights".to_owned(), 0.25);
    let mut stop_gradient_grad_norms = BTreeMap::new();
    stop_gradient_grad_norms.insert("lambda_zero_thresholds".to_owned(), 0.0);
    let diagnostic_subchecks = vec![run_h5_4b_zero_raw_honesty_subcheck()?];
    let sub_passed = diagnostic_subchecks.iter().all(|subcheck| subcheck.passed);
    Ok(FixtureResult {
        sub_hypothesis: "H5.4".to_owned(),
        loss_term: "lambda_zero".to_owned(),
        in_scope_grad_norms,
        stop_gradient_grad_norms,
        non_default_value_used: true,
        numerical_stability_passed: true,
        diagnostic_subchecks,
        detached_grad_absence: BTreeMap::new(),
        sub_passed,
    })
}

#[cfg(feature = "s2-full")]
fn grad_l2_norm(values: &[f32]) -> f32 {
    values.iter().map(|value| value * value).sum::<f32>().sqrt()
}
