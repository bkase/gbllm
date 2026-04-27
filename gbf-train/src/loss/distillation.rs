//! Dense-teacher logit distillation loss.
//!
//! This module owns the formula and local tensor semantics:
//! `KL(softmax(teacher / T) || softmax(student / T)) * T^2`.
//! The unweighted loss is the diagnostic value for `distill_loss`; callers that
//! compose total loss apply `lambda_distill` explicitly.

use std::error::Error;
use std::fmt;

use crate::phase::TrainPhaseKind;
use crate::teacher::{DenseTeacherModel, FrozenTeacher};

#[cfg(feature = "burn-adapter")]
use crate::adapter::burn::{
    BurnAdapterError, BurnBackend, BurnFloatTensor, burn_log_softmax, float_tensor_into_vec,
    float_tensor_shape,
};

pub const DEFAULT_DISTILLATION_TEMPERATURE: f32 = 2.0;
const KL_NEGATIVE_TOLERANCE: f32 = 1.0e-6;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DistillationLogitSide {
    Student,
    Teacher,
}

impl fmt::Display for DistillationLogitSide {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Student => f.write_str("student"),
            Self::Teacher => f.write_str("teacher"),
        }
    }
}

#[derive(Debug)]
pub enum DistillationLossError {
    EmptyLogits,
    ZeroClassCount,
    LogitCountMismatch {
        student_len: usize,
        teacher_len: usize,
    },
    LogitsNotDivisibleByClassCount {
        logit_len: usize,
        class_count: usize,
    },
    NonFiniteLogit {
        side: DistillationLogitSide,
        index: usize,
        value: f32,
    },
    ScaledLogitOverflow {
        side: DistillationLogitSide,
        index: usize,
        value: f32,
        temperature: f32,
    },
    InvalidTemperature {
        temperature: f32,
    },
    NegativeLambdaDistill {
        lambda_distill: f32,
    },
    NonFiniteLambdaDistill {
        lambda_distill: f32,
    },
    NonFiniteLoss {
        value: f32,
    },
    NegativeLoss {
        value: f32,
    },
    #[cfg(feature = "burn-adapter")]
    InvalidClassDim {
        class_dim: usize,
        rank: usize,
    },
    #[cfg(feature = "burn-adapter")]
    ShapeMismatch {
        student_shape: Vec<usize>,
        teacher_shape: Vec<usize>,
    },
    #[cfg(feature = "burn-adapter")]
    BurnAdapter(BurnAdapterError),
}

impl PartialEq for DistillationLossError {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::EmptyLogits, Self::EmptyLogits)
            | (Self::ZeroClassCount, Self::ZeroClassCount) => true,
            (
                Self::LogitCountMismatch {
                    student_len: left_student_len,
                    teacher_len: left_teacher_len,
                },
                Self::LogitCountMismatch {
                    student_len: right_student_len,
                    teacher_len: right_teacher_len,
                },
            ) => left_student_len == right_student_len && left_teacher_len == right_teacher_len,
            (
                Self::LogitsNotDivisibleByClassCount {
                    logit_len: left_logit_len,
                    class_count: left_class_count,
                },
                Self::LogitsNotDivisibleByClassCount {
                    logit_len: right_logit_len,
                    class_count: right_class_count,
                },
            ) => left_logit_len == right_logit_len && left_class_count == right_class_count,
            (
                Self::NonFiniteLogit {
                    side: left_side,
                    index: left_index,
                    value: left_value,
                },
                Self::NonFiniteLogit {
                    side: right_side,
                    index: right_index,
                    value: right_value,
                },
            ) => {
                left_side == right_side
                    && left_index == right_index
                    && float_error_value_eq(*left_value, *right_value)
            }
            (
                Self::ScaledLogitOverflow {
                    side: left_side,
                    index: left_index,
                    value: left_value,
                    temperature: left_temperature,
                },
                Self::ScaledLogitOverflow {
                    side: right_side,
                    index: right_index,
                    value: right_value,
                    temperature: right_temperature,
                },
            ) => {
                left_side == right_side
                    && left_index == right_index
                    && float_error_value_eq(*left_value, *right_value)
                    && float_error_value_eq(*left_temperature, *right_temperature)
            }
            (
                Self::InvalidTemperature {
                    temperature: left_temperature,
                },
                Self::InvalidTemperature {
                    temperature: right_temperature,
                },
            ) => float_error_value_eq(*left_temperature, *right_temperature),
            (
                Self::NegativeLambdaDistill {
                    lambda_distill: left_lambda,
                },
                Self::NegativeLambdaDistill {
                    lambda_distill: right_lambda,
                },
            )
            | (
                Self::NonFiniteLambdaDistill {
                    lambda_distill: left_lambda,
                },
                Self::NonFiniteLambdaDistill {
                    lambda_distill: right_lambda,
                },
            ) => float_error_value_eq(*left_lambda, *right_lambda),
            (
                Self::NonFiniteLoss { value: left_value },
                Self::NonFiniteLoss { value: right_value },
            )
            | (
                Self::NegativeLoss { value: left_value },
                Self::NegativeLoss { value: right_value },
            ) => float_error_value_eq(*left_value, *right_value),
            #[cfg(feature = "burn-adapter")]
            (
                Self::InvalidClassDim {
                    class_dim: left_class_dim,
                    rank: left_rank,
                },
                Self::InvalidClassDim {
                    class_dim: right_class_dim,
                    rank: right_rank,
                },
            ) => left_class_dim == right_class_dim && left_rank == right_rank,
            #[cfg(feature = "burn-adapter")]
            (
                Self::ShapeMismatch {
                    student_shape: left_student_shape,
                    teacher_shape: left_teacher_shape,
                },
                Self::ShapeMismatch {
                    student_shape: right_student_shape,
                    teacher_shape: right_teacher_shape,
                },
            ) => {
                left_student_shape == right_student_shape
                    && left_teacher_shape == right_teacher_shape
            }
            #[cfg(feature = "burn-adapter")]
            (Self::BurnAdapter(_), Self::BurnAdapter(_)) => false,
            _ => false,
        }
    }
}

impl fmt::Display for DistillationLossError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyLogits => f.write_str("distillation logits must not be empty"),
            Self::ZeroClassCount => f.write_str("distillation class_count must be greater than 0"),
            Self::LogitCountMismatch {
                student_len,
                teacher_len,
            } => write!(
                f,
                "student and teacher logits must have the same length, got {student_len} and {teacher_len}"
            ),
            Self::LogitsNotDivisibleByClassCount {
                logit_len,
                class_count,
            } => write!(
                f,
                "distillation logits length {logit_len} is not divisible by class_count {class_count}"
            ),
            Self::NonFiniteLogit { side, index, value } => write!(
                f,
                "{side} logit at index {index} must be finite, got {value}"
            ),
            Self::ScaledLogitOverflow {
                side,
                index,
                value,
                temperature,
            } => write!(
                f,
                "{side} logit at index {index} overflows after temperature scaling: {value} / {temperature}"
            ),
            Self::InvalidTemperature { temperature } => write!(
                f,
                "distillation temperature must be finite and positive with finite square, got {temperature}"
            ),
            Self::NegativeLambdaDistill { lambda_distill } => write!(
                f,
                "lambda_distill must be non-negative, got {lambda_distill}"
            ),
            Self::NonFiniteLambdaDistill { lambda_distill } => {
                write!(f, "lambda_distill must be finite, got {lambda_distill}")
            }
            Self::NonFiniteLoss { value } => {
                write!(f, "distillation loss must be finite, got {value}")
            }
            Self::NegativeLoss { value } => write!(
                f,
                "distillation KL loss should be non-negative, got {value}"
            ),
            #[cfg(feature = "burn-adapter")]
            Self::InvalidClassDim { class_dim, rank } => write!(
                f,
                "distillation class_dim {class_dim} is out of range for rank {rank}"
            ),
            #[cfg(feature = "burn-adapter")]
            Self::ShapeMismatch {
                student_shape,
                teacher_shape,
            } => write!(
                f,
                "student and teacher logits must have identical shapes, got {student_shape:?} and {teacher_shape:?}"
            ),
            #[cfg(feature = "burn-adapter")]
            Self::BurnAdapter(error) => write!(f, "{error}"),
        }
    }
}

impl Error for DistillationLossError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            #[cfg(feature = "burn-adapter")]
            Self::BurnAdapter(error) => Some(error),
            Self::EmptyLogits
            | Self::ZeroClassCount
            | Self::LogitCountMismatch { .. }
            | Self::LogitsNotDivisibleByClassCount { .. }
            | Self::NonFiniteLogit { .. }
            | Self::ScaledLogitOverflow { .. }
            | Self::InvalidTemperature { .. }
            | Self::NegativeLambdaDistill { .. }
            | Self::NonFiniteLambdaDistill { .. }
            | Self::NonFiniteLoss { .. }
            | Self::NegativeLoss { .. } => None,
            #[cfg(feature = "burn-adapter")]
            Self::InvalidClassDim { .. } | Self::ShapeMismatch { .. } => None,
        }
    }
}

#[cfg(feature = "burn-adapter")]
impl From<BurnAdapterError> for DistillationLossError {
    fn from(error: BurnAdapterError) -> Self {
        Self::BurnAdapter(error)
    }
}

#[derive(Debug, PartialEq)]
pub enum FrozenTeacherDistillationError<E> {
    TeacherForward(E),
    Distillation(DistillationLossError),
}

impl<E: fmt::Display> fmt::Display for FrozenTeacherDistillationError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TeacherForward(error) => write!(f, "teacher forward failed: {error}"),
            Self::Distillation(error) => write!(f, "{error}"),
        }
    }
}

impl<E> Error for FrozenTeacherDistillationError<E>
where
    E: Error + 'static,
{
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::TeacherForward(error) => Some(error),
            Self::Distillation(error) => Some(error),
        }
    }
}

impl<E> From<DistillationLossError> for FrozenTeacherDistillationError<E> {
    fn from(error: DistillationLossError) -> Self {
        Self::Distillation(error)
    }
}

/// KL distillation for a single class distribution.
pub fn distillation_loss(
    student_logits: &[f32],
    teacher_logits: &[f32],
    temperature: f32,
) -> Result<f32, DistillationLossError> {
    validate_logits(student_logits, teacher_logits, temperature)?;
    batched_distillation_loss(
        student_logits,
        teacher_logits,
        student_logits.len(),
        temperature,
    )
}

/// KL distillation over rows of `class_count` logits, reduced as mean over rows.
pub fn batched_distillation_loss(
    student_logits: &[f32],
    teacher_logits: &[f32],
    class_count: usize,
    temperature: f32,
) -> Result<f32, DistillationLossError> {
    validate_logits(student_logits, teacher_logits, temperature)?;
    validate_class_count(student_logits.len(), class_count)?;

    let row_count = student_logits.len() / class_count;
    let mut loss_sum = 0.0;
    for row_index in 0..row_count {
        let row_start = row_index * class_count;
        let row_end = row_start + class_count;
        loss_sum += single_row_distillation_loss(
            &student_logits[row_start..row_end],
            &teacher_logits[row_start..row_end],
            temperature,
        )?;
    }

    normalize_kl_loss(loss_sum / row_count as f32)
}

pub fn weighted_distillation_loss(
    raw_distillation_loss: f32,
    lambda_distill: f32,
) -> Result<f32, DistillationLossError> {
    validate_lambda(lambda_distill)?;
    normalize_weighted_loss(raw_distillation_loss * lambda_distill)
}

pub fn distillation_loss_from_frozen_teacher<M>(
    student_logits: &[f32],
    teacher: &FrozenTeacher<M>,
    teacher_input: M::Input,
    temperature: f32,
) -> Result<f32, FrozenTeacherDistillationError<M::ForwardError>>
where
    M: DenseTeacherModel<Output = Vec<f32>>,
{
    let teacher_logits = teacher
        .forward_no_grad(teacher_input)
        .map_err(FrozenTeacherDistillationError::TeacherForward)?;

    distillation_loss(student_logits, &teacher_logits, temperature).map_err(Into::into)
}

pub fn batched_distillation_loss_from_frozen_teacher<M>(
    student_logits: &[f32],
    teacher: &FrozenTeacher<M>,
    teacher_input: M::Input,
    class_count: usize,
    temperature: f32,
) -> Result<f32, FrozenTeacherDistillationError<M::ForwardError>>
where
    M: DenseTeacherModel<Output = Vec<f32>>,
{
    let teacher_logits = teacher
        .forward_no_grad(teacher_input)
        .map_err(FrozenTeacherDistillationError::TeacherForward)?;

    batched_distillation_loss(student_logits, &teacher_logits, class_count, temperature)
        .map_err(Into::into)
}

#[must_use]
pub const fn distillation_enabled_for_phase(phase: TrainPhaseKind) -> bool {
    matches!(
        phase,
        TrainPhaseKind::ExpertTernaryQat | TrainPhaseKind::FullNumericQat
    )
}

#[must_use]
pub fn lambda_distill_for_phase(phase: TrainPhaseKind, configured_lambda_distill: f32) -> f32 {
    if distillation_enabled_for_phase(phase) {
        configured_lambda_distill
    } else {
        0.0
    }
}

#[cfg(feature = "burn-adapter")]
pub fn burn_distillation_loss<B, const D: usize>(
    student_logits: BurnFloatTensor<B, D>,
    teacher_logits: BurnFloatTensor<B, D>,
    class_dim: usize,
    temperature: f32,
) -> Result<BurnFloatTensor<B, 1>, DistillationLossError>
where
    B: BurnBackend,
{
    validate_tensor_contract(&student_logits, &teacher_logits, class_dim)?;
    validate_burn_logits(DistillationLogitSide::Student, &student_logits, temperature)?;
    validate_burn_logits(DistillationLogitSide::Teacher, &teacher_logits, temperature)?;

    let teacher_log_probs =
        burn_log_softmax(teacher_logits.detach() / temperature, class_dim).detach();
    let teacher_probs = teacher_log_probs.clone().exp();
    let student_log_probs = burn_log_softmax(student_logits / temperature, class_dim);
    let kl_terms = teacher_probs * (teacher_log_probs - student_log_probs);

    Ok(kl_terms.sum_dim(class_dim).mean() * temperature * temperature)
}

#[cfg(feature = "burn-adapter")]
pub fn burn_weighted_distillation_loss<B, const D: usize>(
    raw_distillation_loss: BurnFloatTensor<B, D>,
    lambda_distill: f32,
) -> Result<BurnFloatTensor<B, D>, DistillationLossError>
where
    B: BurnBackend,
{
    validate_lambda(lambda_distill)?;
    Ok(raw_distillation_loss * lambda_distill)
}

#[cfg(feature = "burn-adapter")]
pub fn burn_distillation_loss_from_frozen_teacher<B, M, const D: usize>(
    student_logits: BurnFloatTensor<B, D>,
    teacher: &FrozenTeacher<M>,
    teacher_input: M::Input,
    class_dim: usize,
    temperature: f32,
) -> Result<BurnFloatTensor<B, 1>, FrozenTeacherDistillationError<M::ForwardError>>
where
    B: BurnBackend,
    M: DenseTeacherModel<Output = BurnFloatTensor<B, D>>,
{
    let teacher_logits = teacher
        .forward_no_grad(teacher_input)
        .map_err(FrozenTeacherDistillationError::TeacherForward)?;

    burn_distillation_loss(student_logits, teacher_logits, class_dim, temperature)
        .map_err(Into::into)
}

fn single_row_distillation_loss(
    student_logits: &[f32],
    teacher_logits: &[f32],
    temperature: f32,
) -> Result<f32, DistillationLossError> {
    let student_log_probs = log_softmax_temperature(student_logits, temperature);
    let teacher_log_probs = log_softmax_temperature(teacher_logits, temperature);
    let teacher_probs = teacher_log_probs
        .iter()
        .map(|log_prob| log_prob.exp())
        .collect::<Vec<_>>();
    let kl = teacher_probs
        .iter()
        .zip(teacher_log_probs.iter())
        .zip(student_log_probs.iter())
        .map(|((&teacher_prob, &teacher_log_prob), &student_log_prob)| {
            teacher_prob * (teacher_log_prob - student_log_prob)
        })
        .sum::<f32>();

    normalize_kl_loss(kl * temperature * temperature)
}

fn validate_logits(
    student_logits: &[f32],
    teacher_logits: &[f32],
    temperature: f32,
) -> Result<(), DistillationLossError> {
    validate_temperature(temperature)?;

    if student_logits.is_empty() || teacher_logits.is_empty() {
        return Err(DistillationLossError::EmptyLogits);
    }

    if student_logits.len() != teacher_logits.len() {
        return Err(DistillationLossError::LogitCountMismatch {
            student_len: student_logits.len(),
            teacher_len: teacher_logits.len(),
        });
    }

    validate_finite_logits(DistillationLogitSide::Student, student_logits)?;
    validate_finite_logits(DistillationLogitSide::Teacher, teacher_logits)?;
    validate_scaled_logits(DistillationLogitSide::Student, student_logits, temperature)?;
    validate_scaled_logits(DistillationLogitSide::Teacher, teacher_logits, temperature)
}

fn validate_class_count(logit_len: usize, class_count: usize) -> Result<(), DistillationLossError> {
    if class_count == 0 {
        return Err(DistillationLossError::ZeroClassCount);
    }

    if !logit_len.is_multiple_of(class_count) {
        return Err(DistillationLossError::LogitsNotDivisibleByClassCount {
            logit_len,
            class_count,
        });
    }

    Ok(())
}

fn validate_finite_logits(
    side: DistillationLogitSide,
    logits: &[f32],
) -> Result<(), DistillationLossError> {
    for (index, &value) in logits.iter().enumerate() {
        if !value.is_finite() {
            return Err(DistillationLossError::NonFiniteLogit { side, index, value });
        }
    }

    Ok(())
}

fn validate_scaled_logits(
    side: DistillationLogitSide,
    logits: &[f32],
    temperature: f32,
) -> Result<(), DistillationLossError> {
    for (index, &value) in logits.iter().enumerate() {
        if !(value / temperature).is_finite() {
            return Err(DistillationLossError::ScaledLogitOverflow {
                side,
                index,
                value,
                temperature,
            });
        }
    }

    Ok(())
}

fn validate_temperature(temperature: f32) -> Result<(), DistillationLossError> {
    if !temperature.is_finite() || temperature <= 0.0 || !(temperature * temperature).is_finite() {
        return Err(DistillationLossError::InvalidTemperature { temperature });
    }

    Ok(())
}

fn validate_lambda(lambda_distill: f32) -> Result<(), DistillationLossError> {
    if !lambda_distill.is_finite() {
        return Err(DistillationLossError::NonFiniteLambdaDistill { lambda_distill });
    }

    if lambda_distill < 0.0 {
        return Err(DistillationLossError::NegativeLambdaDistill { lambda_distill });
    }

    Ok(())
}

fn normalize_kl_loss(loss: f32) -> Result<f32, DistillationLossError> {
    if !loss.is_finite() {
        return Err(DistillationLossError::NonFiniteLoss { value: loss });
    }

    if loss < 0.0 {
        if loss >= -KL_NEGATIVE_TOLERANCE {
            return Ok(0.0);
        }

        return Err(DistillationLossError::NegativeLoss { value: loss });
    }

    Ok(loss)
}

fn normalize_weighted_loss(loss: f32) -> Result<f32, DistillationLossError> {
    if !loss.is_finite() {
        return Err(DistillationLossError::NonFiniteLoss { value: loss });
    }

    Ok(loss)
}

fn log_softmax_temperature(logits: &[f32], temperature: f32) -> Vec<f32> {
    let max = logits
        .iter()
        .map(|logit| logit / temperature)
        .fold(f32::NEG_INFINITY, f32::max);
    let exp_sum = logits
        .iter()
        .map(|logit| (logit / temperature - max).exp())
        .sum::<f32>();
    let log_denom = max + exp_sum.ln();

    logits
        .iter()
        .map(|logit| logit / temperature - log_denom)
        .collect()
}

fn float_error_value_eq(left: f32, right: f32) -> bool {
    left == right || left.is_nan() && right.is_nan()
}

#[cfg(feature = "burn-adapter")]
fn validate_tensor_contract<B, const D: usize>(
    student_logits: &BurnFloatTensor<B, D>,
    teacher_logits: &BurnFloatTensor<B, D>,
    class_dim: usize,
) -> Result<(), DistillationLossError>
where
    B: BurnBackend,
{
    if class_dim >= D {
        return Err(DistillationLossError::InvalidClassDim { class_dim, rank: D });
    }

    let student_shape = float_tensor_shape(student_logits);
    let teacher_shape = float_tensor_shape(teacher_logits);
    if student_shape.iter().product::<usize>() == 0 || teacher_shape.iter().product::<usize>() == 0
    {
        return Err(DistillationLossError::EmptyLogits);
    }

    if student_shape != teacher_shape {
        return Err(DistillationLossError::ShapeMismatch {
            student_shape: student_shape.to_vec(),
            teacher_shape: teacher_shape.to_vec(),
        });
    }

    Ok(())
}

#[cfg(feature = "burn-adapter")]
fn validate_burn_logits<B, const D: usize>(
    side: DistillationLogitSide,
    logits: &BurnFloatTensor<B, D>,
    temperature: f32,
) -> Result<(), DistillationLossError>
where
    B: BurnBackend,
{
    let values = float_tensor_into_vec(logits.clone().detach())?;
    validate_finite_logits(side, &values)?;
    validate_scaled_logits(side, &values, temperature)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::teacher::{TeacherStorageFingerprint, TeacherWeightFingerprint, freeze_teacher};

    fn assert_close(actual: f32, expected: f32, tolerance: f32) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "expected {actual} to be within {tolerance} of {expected}"
        );
    }

    #[test]
    fn distillation_loss_is_zero_for_identical_logits() {
        let logits = [1.0, 2.0, -0.5, 0.25];

        let loss = distillation_loss(&logits, &logits, DEFAULT_DISTILLATION_TEMPERATURE).unwrap();

        assert_close(loss, 0.0, 1.0e-6);
    }

    #[test]
    fn distillation_loss_matches_manual_kl_with_temperature_scaling() {
        let student_logits = [2.0, 0.0, -1.0];
        let teacher_logits = [0.5, 1.5, -0.5];

        let loss = distillation_loss(&student_logits, &teacher_logits, 2.0).unwrap();

        assert_close(loss, 0.920_573_7, 1.0e-6);
        assert!(loss >= 0.0);
    }

    #[test]
    fn distillation_loss_orientation_is_teacher_distribution_to_student_distribution() {
        let student_logits = [2.0, 0.0, -1.0];
        let teacher_logits = [0.5, 1.5, -0.5];

        let teacher_to_student = distillation_loss(&student_logits, &teacher_logits, 2.0).unwrap();
        let student_to_teacher = distillation_loss(&teacher_logits, &student_logits, 2.0).unwrap();

        assert_close(teacher_to_student, 0.920_573_7, 1.0e-6);
        assert_close(student_to_teacher, 0.915_282_55, 1.0e-6);
    }

    #[test]
    fn batched_distillation_loss_means_rows_after_class_axis_sum() {
        let student_logits = [2.0, 0.0, -1.0, 0.5, -0.5, 1.0];
        let teacher_logits = [0.5, 1.5, -0.5, 1.0, -1.0, 0.0];

        let row_a = distillation_loss(&student_logits[..3], &teacher_logits[..3], 2.0).unwrap();
        let row_b = distillation_loss(&student_logits[3..], &teacher_logits[3..], 2.0).unwrap();
        let batched = batched_distillation_loss(&student_logits, &teacher_logits, 3, 2.0).unwrap();

        assert_close(batched, (row_a + row_b) / 2.0, 1.0e-6);
    }

    #[test]
    fn weighted_distillation_loss_applies_explicit_lambda_distill() {
        let raw = distillation_loss(&[2.0, 0.0, -1.0], &[0.5, 1.5, -0.5], 2.0).unwrap();

        let weighted = weighted_distillation_loss(raw, 0.25).unwrap();

        assert_close(weighted, raw * 0.25, 1.0e-6);
    }

    #[test]
    fn distillation_loss_can_call_frozen_teacher_forward_no_grad() {
        let source_teacher = VecTeacherModel::new(vec![0.5, 1.5, -0.5], true);
        let frozen_teacher = freeze_teacher(&source_teacher).unwrap();

        let loss =
            distillation_loss_from_frozen_teacher(&[2.0, 0.0, -1.0], &frozen_teacher, (), 2.0)
                .unwrap();

        assert_close(loss, 0.920_573_7, 1.0e-6);
    }

    #[test]
    fn distillation_loss_validates_inputs() {
        assert_eq!(
            distillation_loss(&[], &[1.0], 2.0).unwrap_err(),
            DistillationLossError::EmptyLogits
        );
        assert_eq!(
            distillation_loss(&[1.0, 2.0], &[1.0], 2.0).unwrap_err(),
            DistillationLossError::LogitCountMismatch {
                student_len: 2,
                teacher_len: 1,
            }
        );
        assert_eq!(
            batched_distillation_loss(&[1.0, 2.0, 3.0], &[1.0, 2.0, 3.0], 2, 2.0).unwrap_err(),
            DistillationLossError::LogitsNotDivisibleByClassCount {
                logit_len: 3,
                class_count: 2,
            }
        );
        assert_eq!(
            distillation_loss(&[f32::NAN], &[1.0], 2.0).unwrap_err(),
            DistillationLossError::NonFiniteLogit {
                side: DistillationLogitSide::Student,
                index: 0,
                value: f32::NAN,
            }
        );
        assert_eq!(
            distillation_loss(&[f32::MAX], &[1.0], f32::MIN_POSITIVE).unwrap_err(),
            DistillationLossError::ScaledLogitOverflow {
                side: DistillationLogitSide::Student,
                index: 0,
                value: f32::MAX,
                temperature: f32::MIN_POSITIVE,
            }
        );
        assert_eq!(
            weighted_distillation_loss(1.0, -0.1).unwrap_err(),
            DistillationLossError::NegativeLambdaDistill {
                lambda_distill: -0.1,
            }
        );
        assert_eq!(
            distillation_loss(&[1.0], &[1.0], 0.0).unwrap_err(),
            DistillationLossError::InvalidTemperature { temperature: 0.0 }
        );
    }

    #[test]
    fn distillation_is_enabled_only_for_phases_c_and_d() {
        assert!(!distillation_enabled_for_phase(
            TrainPhaseKind::DenseTeacherWarmup
        ));
        assert!(!distillation_enabled_for_phase(
            TrainPhaseKind::RouterWarmup
        ));
        assert!(distillation_enabled_for_phase(
            TrainPhaseKind::ExpertTernaryQat
        ));
        assert!(distillation_enabled_for_phase(
            TrainPhaseKind::FullNumericQat
        ));
        assert!(!distillation_enabled_for_phase(
            TrainPhaseKind::HardenAndSelect
        ));
        assert_eq!(
            lambda_distill_for_phase(TrainPhaseKind::RouterWarmup, 0.25),
            0.0
        );
        assert_eq!(
            lambda_distill_for_phase(TrainPhaseKind::FullNumericQat, 0.25),
            0.25
        );
    }

    #[derive(Debug, Clone)]
    struct VecTeacherModel {
        logits: Vec<f32>,
        requires_grad: bool,
    }

    impl VecTeacherModel {
        fn new(logits: Vec<f32>, requires_grad: bool) -> Self {
            Self {
                logits,
                requires_grad,
            }
        }
    }

    impl DenseTeacherModel for VecTeacherModel {
        type Input = ();
        type Output = Vec<f32>;
        type ForwardError = std::convert::Infallible;

        fn detach_for_teacher(&mut self) {
            self.requires_grad = false;
        }

        fn forward_no_grad(&self, (): Self::Input) -> Result<Self::Output, Self::ForwardError> {
            Ok(self.logits.clone())
        }

        fn teacher_weight_fingerprint(&self) -> TeacherWeightFingerprint {
            TeacherWeightFingerprint::new(
                self.logits
                    .iter()
                    .flat_map(|logit| logit.to_le_bytes())
                    .collect::<Vec<_>>(),
            )
            .unwrap()
        }

        fn teacher_storage_fingerprint(&self) -> TeacherStorageFingerprint {
            TeacherStorageFingerprint::new((self.logits.as_ptr() as usize).to_le_bytes()).unwrap()
        }

        fn teacher_requires_grad(&self) -> bool {
            self.requires_grad
        }
    }

    #[cfg(feature = "burn-adapter")]
    mod burn_tests {
        use super::*;
        use crate::adapter::burn::{
            BurnDevice, BurnNdArrayAutodiffBackend, float_tensor_from_vec, float_tensor_into_vec,
        };

        type B = BurnNdArrayAutodiffBackend;

        #[test]
        fn burn_distillation_loss_matches_scalar_oracle_for_batched_logits() {
            let device = BurnDevice::<B>::default();
            let student_values = vec![2.0, 0.0, -1.0, 0.5, -0.5, 1.0];
            let teacher_values = vec![0.5, 1.5, -0.5, 1.0, -1.0, 0.0];
            let student_logits =
                float_tensor_from_vec::<B, 2>(student_values.clone(), [2, 3], &device).unwrap();
            let teacher_logits =
                float_tensor_from_vec::<B, 2>(teacher_values.clone(), [2, 3], &device).unwrap();

            let burn_loss = burn_distillation_loss(student_logits, teacher_logits, 1, 3.0).unwrap();
            let scalar_loss =
                batched_distillation_loss(&student_values, &teacher_values, 3, 3.0).unwrap();

            assert_close(
                float_tensor_into_vec(burn_loss).unwrap()[0],
                scalar_loss,
                1.0e-5,
            );
        }

        #[test]
        fn burn_weighted_distillation_loss_applies_non_default_lambda() {
            let device = BurnDevice::<B>::default();
            let raw_loss = float_tensor_from_vec::<B, 1>(vec![0.75], [1], &device).unwrap();

            let weighted = burn_weighted_distillation_loss(raw_loss, 0.25).unwrap();

            assert_close(float_tensor_into_vec(weighted).unwrap()[0], 0.1875, 1.0e-6);
        }

        #[test]
        fn burn_distillation_loss_flows_gradient_only_to_student_logits() {
            let device = BurnDevice::<B>::default();
            let student_logits = float_tensor_from_vec::<B, 1>(vec![2.0, 0.0, -1.0], [3], &device)
                .unwrap()
                .require_grad();
            let teacher_logits = float_tensor_from_vec::<B, 1>(vec![0.5, 1.5, -0.5], [3], &device)
                .unwrap()
                .require_grad();

            let loss = burn_distillation_loss(
                student_logits.clone(),
                teacher_logits.clone(),
                0,
                DEFAULT_DISTILLATION_TEMPERATURE,
            )
            .unwrap();
            let gradients = loss.backward();
            let student_grad = student_logits
                .grad(&gradients)
                .expect("student logits should receive gradients");

            assert!(
                float_tensor_into_vec(student_grad)
                    .unwrap()
                    .iter()
                    .any(|value| value.abs() > 0.0)
            );
            assert!(
                teacher_logits.grad(&gradients).is_none(),
                "teacher logits must be detached from distillation gradients"
            );
        }

        #[test]
        fn burn_distillation_loss_reports_shape_not_flattened_count() {
            let device = BurnDevice::<B>::default();
            let student_logits =
                float_tensor_from_vec::<B, 2>(vec![0.0; 6], [2, 3], &device).unwrap();
            let teacher_logits =
                float_tensor_from_vec::<B, 2>(vec![0.0; 6], [3, 2], &device).unwrap();

            assert_eq!(
                burn_distillation_loss(
                    student_logits,
                    teacher_logits,
                    1,
                    DEFAULT_DISTILLATION_TEMPERATURE
                )
                .unwrap_err(),
                DistillationLossError::ShapeMismatch {
                    student_shape: vec![2, 3],
                    teacher_shape: vec![3, 2],
                }
            );
        }
    }
}
