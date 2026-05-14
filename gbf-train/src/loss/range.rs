//! Activation range penalty loss.
//!
//! This module owns the raw `lambda_range` diagnostic term:
//! `(1 / batch) * sum_b sum_axis(max(0, x - safe_hi)^2 + max(0, safe_lo - x)^2)`.
//! The checked view names both axes so callers cannot silently flatten the
//! batch/sample contract. Callers that compose total loss apply `lambda_range`
//! explicitly.

use std::error::Error;
use std::fmt;

#[cfg(feature = "burn-adapter")]
use crate::adapter::burn::{
    BurnAdapterError, BurnBackend, BurnFloatTensor, float_tensor_into_vec, float_tensor_shape,
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SafeActivationBound {
    value: f32,
}

impl SafeActivationBound {
    pub fn new(value: f32) -> Result<Self, ActivationRangeLossError> {
        if !value.is_finite() || value <= 0.0 {
            return Err(ActivationRangeLossError::InvalidSafeBound { value });
        }

        Ok(Self { value })
    }

    #[must_use]
    pub const fn value(self) -> f32 {
        self.value
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SafeActivationInterval {
    lo: f32,
    hi: f32,
}

impl SafeActivationInterval {
    pub fn new(lo: f32, hi: f32) -> Result<Self, ActivationRangeLossError> {
        if !lo.is_finite() || !hi.is_finite() || lo > hi {
            return Err(ActivationRangeLossError::InvalidSafeInterval { lo, hi });
        }

        Ok(Self { lo, hi })
    }

    #[must_use]
    pub const fn lo(self) -> f32 {
        self.lo
    }

    #[must_use]
    pub const fn hi(self) -> f32 {
        self.hi
    }
}

#[derive(Debug)]
pub enum ActivationRangeLossError {
    EmptyActivations,
    ZeroActivationSampleWidth,
    ActivationCountNotDivisibleBySampleWidth {
        activation_len: usize,
        sample_width: usize,
    },
    InvalidSafeBound {
        value: f32,
    },
    InvalidSafeInterval {
        lo: f32,
        hi: f32,
    },
    NonFiniteActivation {
        index: usize,
        value: f32,
    },
    NonFiniteLoss {
        value: f32,
    },
    NegativeLambdaRange {
        lambda_range: f32,
    },
    NonFiniteLambdaRange {
        lambda_range: f32,
    },
    InvalidActivationShape {
        shape: Vec<usize>,
    },
    #[cfg(feature = "burn-adapter")]
    InvalidBurnActivationRank {
        rank: usize,
    },
    #[cfg(feature = "burn-adapter")]
    BurnAdapter(BurnAdapterError),
}

impl PartialEq for ActivationRangeLossError {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::EmptyActivations, Self::EmptyActivations) => true,
            (Self::ZeroActivationSampleWidth, Self::ZeroActivationSampleWidth) => true,
            (
                Self::ActivationCountNotDivisibleBySampleWidth {
                    activation_len: left_activation_len,
                    sample_width: left_sample_width,
                },
                Self::ActivationCountNotDivisibleBySampleWidth {
                    activation_len: right_activation_len,
                    sample_width: right_sample_width,
                },
            ) => {
                left_activation_len == right_activation_len
                    && left_sample_width == right_sample_width
            }
            (
                Self::InvalidSafeBound { value: left_value },
                Self::InvalidSafeBound { value: right_value },
            )
            | (
                Self::NonFiniteLoss { value: left_value },
                Self::NonFiniteLoss { value: right_value },
            ) => float_error_value_eq(*left_value, *right_value),
            (
                Self::InvalidSafeInterval {
                    lo: left_lo,
                    hi: left_hi,
                },
                Self::InvalidSafeInterval {
                    lo: right_lo,
                    hi: right_hi,
                },
            ) => {
                float_error_value_eq(*left_lo, *right_lo)
                    && float_error_value_eq(*left_hi, *right_hi)
            }
            (
                Self::NonFiniteActivation {
                    index: left_index,
                    value: left_value,
                },
                Self::NonFiniteActivation {
                    index: right_index,
                    value: right_value,
                },
            ) => left_index == right_index && float_error_value_eq(*left_value, *right_value),
            (
                Self::NegativeLambdaRange {
                    lambda_range: left_lambda,
                },
                Self::NegativeLambdaRange {
                    lambda_range: right_lambda,
                },
            )
            | (
                Self::NonFiniteLambdaRange {
                    lambda_range: left_lambda,
                },
                Self::NonFiniteLambdaRange {
                    lambda_range: right_lambda,
                },
            ) => float_error_value_eq(*left_lambda, *right_lambda),
            (
                Self::InvalidActivationShape { shape: left_shape },
                Self::InvalidActivationShape { shape: right_shape },
            ) => left_shape == right_shape,
            #[cfg(feature = "burn-adapter")]
            (
                Self::InvalidBurnActivationRank { rank: left_rank },
                Self::InvalidBurnActivationRank { rank: right_rank },
            ) => left_rank == right_rank,
            #[cfg(feature = "burn-adapter")]
            (Self::BurnAdapter(_), Self::BurnAdapter(_)) => false,
            _ => false,
        }
    }
}

impl fmt::Display for ActivationRangeLossError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyActivations => {
                f.write_str("activation range penalty input must not be empty")
            }
            Self::ZeroActivationSampleWidth => {
                f.write_str("activation range sample_width must be greater than 0")
            }
            Self::ActivationCountNotDivisibleBySampleWidth {
                activation_len,
                sample_width,
            } => write!(
                f,
                "activation count {activation_len} is not divisible by sample_width {sample_width}"
            ),
            Self::InvalidSafeBound { value } => {
                write!(
                    f,
                    "safe activation bound must be finite and positive, got {value}"
                )
            }
            Self::InvalidSafeInterval { lo, hi } => {
                write!(
                    f,
                    "safe activation interval must be finite with lo <= hi, got [{lo}, {hi}]"
                )
            }
            Self::NonFiniteActivation { index, value } => {
                write!(f, "activation at index {index} must be finite, got {value}")
            }
            Self::NonFiniteLoss { value } => {
                write!(f, "activation range loss must be finite, got {value}")
            }
            Self::NegativeLambdaRange { lambda_range } => {
                write!(f, "lambda_range must be non-negative, got {lambda_range}")
            }
            Self::NonFiniteLambdaRange { lambda_range } => {
                write!(f, "lambda_range must be finite, got {lambda_range}")
            }
            Self::InvalidActivationShape { shape } => {
                write!(
                    f,
                    "activation tensor must be rank >= 1 with non-zero dimensions, got {shape:?}"
                )
            }
            #[cfg(feature = "burn-adapter")]
            Self::InvalidBurnActivationRank { rank } => {
                write!(
                    f,
                    "S2 range loss activation tensor must be rank 2, got rank {rank}"
                )
            }
            #[cfg(feature = "burn-adapter")]
            Self::BurnAdapter(error) => write!(f, "{error}"),
        }
    }
}

impl Error for ActivationRangeLossError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            #[cfg(feature = "burn-adapter")]
            Self::BurnAdapter(error) => Some(error),
            Self::EmptyActivations
            | Self::ZeroActivationSampleWidth
            | Self::ActivationCountNotDivisibleBySampleWidth { .. }
            | Self::InvalidSafeBound { .. }
            | Self::InvalidSafeInterval { .. }
            | Self::NonFiniteActivation { .. }
            | Self::NonFiniteLoss { .. }
            | Self::NegativeLambdaRange { .. }
            | Self::NonFiniteLambdaRange { .. } => None,
            Self::InvalidActivationShape { .. } => None,
            #[cfg(feature = "burn-adapter")]
            Self::InvalidBurnActivationRank { .. } => None,
        }
    }
}

#[cfg(feature = "burn-adapter")]
impl From<BurnAdapterError> for ActivationRangeLossError {
    fn from(error: BurnAdapterError) -> Self {
        Self::BurnAdapter(error)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ActivationRangeBatch<'a> {
    activations: &'a [f32],
    sample_width: usize,
}

impl<'a> ActivationRangeBatch<'a> {
    pub fn new(
        activations: &'a [f32],
        sample_width: usize,
    ) -> Result<Self, ActivationRangeLossError> {
        validate_activations(activations)?;
        validate_sample_width(activations.len(), sample_width)?;

        Ok(Self {
            activations,
            sample_width,
        })
    }

    pub fn flat_samples(activations: &'a [f32]) -> Result<Self, ActivationRangeLossError> {
        validate_activations(activations)?;

        Ok(Self {
            activations,
            sample_width: activations.len(),
        })
    }

    #[must_use]
    pub const fn activations(self) -> &'a [f32] {
        self.activations
    }

    #[must_use]
    pub const fn sample_width(self) -> usize {
        self.sample_width
    }

    #[must_use]
    pub fn batch_size(self) -> usize {
        self.activations.len() / self.sample_width
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CheckedActivationView<'a> {
    data: &'a [f32],
    batch_axis_len: usize,
    per_sample_axis_len: usize,
}

impl<'a> CheckedActivationView<'a> {
    pub fn new(
        data: &'a [f32],
        batch_axis_len: usize,
        per_sample_axis_len: usize,
    ) -> Result<Self, ActivationRangeLossError> {
        if batch_axis_len == 0 || data.is_empty() {
            return Err(ActivationRangeLossError::EmptyActivations);
        }
        if per_sample_axis_len == 0 {
            return Err(ActivationRangeLossError::ZeroActivationSampleWidth);
        }

        let expected = batch_axis_len.checked_mul(per_sample_axis_len).ok_or(
            ActivationRangeLossError::ActivationCountNotDivisibleBySampleWidth {
                activation_len: data.len(),
                sample_width: per_sample_axis_len,
            },
        )?;
        if data.len() != expected {
            return Err(
                ActivationRangeLossError::ActivationCountNotDivisibleBySampleWidth {
                    activation_len: data.len(),
                    sample_width: per_sample_axis_len,
                },
            );
        }
        validate_activations(data)?;

        Ok(Self {
            data,
            batch_axis_len,
            per_sample_axis_len,
        })
    }

    pub fn from_shape(data: &'a [f32], shape: &[usize]) -> Result<Self, ActivationRangeLossError> {
        if shape.len() != 2 {
            return Err(ActivationRangeLossError::InvalidActivationShape {
                shape: shape.to_vec(),
            });
        }
        Self::new(data, shape[0], shape[1])
    }

    #[must_use]
    pub const fn data(self) -> &'a [f32] {
        self.data
    }

    #[must_use]
    pub const fn batch_axis_len(self) -> usize {
        self.batch_axis_len
    }

    #[must_use]
    pub const fn per_sample_axis_len(self) -> usize {
        self.per_sample_axis_len
    }
}

/// Mean quadratic penalty for flattened activations outside `[-safe_bound, safe_bound]`.
///
/// Use [`ActivationRangeBatch`] when the caller wants the boundary sample width
/// to be part of the checked contract.
pub fn activation_range_penalty(
    activations: &[f32],
    safe_bound: f32,
) -> Result<f32, ActivationRangeLossError> {
    let batch = ActivationRangeBatch::flat_samples(activations)?;
    activation_range_penalty_for_batch(batch, SafeActivationBound::new(safe_bound)?)
}

pub fn activation_range_penalty_with_bound(
    activations: &[f32],
    safe_bound: SafeActivationBound,
) -> Result<f32, ActivationRangeLossError> {
    let batch = ActivationRangeBatch::flat_samples(activations)?;
    activation_range_penalty_for_batch(batch, safe_bound)
}

pub fn activation_range_penalty_for_batch(
    batch: ActivationRangeBatch<'_>,
    safe_bound: SafeActivationBound,
) -> Result<f32, ActivationRangeLossError> {
    let sample_width = batch.sample_width();
    let mut batch_loss_sum = 0.0_f64;
    for sample in batch.activations().chunks_exact(sample_width) {
        let sample_loss_sum = sample
            .iter()
            .map(|activation| f64::from(activation.abs()) - f64::from(safe_bound.value()))
            .map(|excess| excess.max(0.0))
            .map(|excess| excess * excess)
            .sum::<f64>();
        batch_loss_sum += sample_loss_sum / sample_width as f64;
    }

    normalize_f64_range_loss(batch_loss_sum / batch.batch_size() as f64)
}

pub fn range_loss(
    view: CheckedActivationView<'_>,
    safe_lo: f32,
    safe_hi: f32,
) -> Result<f32, ActivationRangeLossError> {
    range_loss_with_interval(view, SafeActivationInterval::new(safe_lo, safe_hi)?)
}

pub fn range_loss_with_interval(
    view: CheckedActivationView<'_>,
    interval: SafeActivationInterval,
) -> Result<f32, ActivationRangeLossError> {
    let mut out_of_range_count = 0_u32;
    let mut batch_loss_sum = 0.0_f64;
    for sample in view.data().chunks_exact(view.per_sample_axis_len()) {
        for &activation in sample {
            let above = (f64::from(activation) - f64::from(interval.hi())).max(0.0);
            let below = (f64::from(interval.lo()) - f64::from(activation)).max(0.0);
            if above > 0.0 || below > 0.0 {
                out_of_range_count = out_of_range_count.saturating_add(1);
            }
            batch_loss_sum += above * above + below * below;
        }
    }

    let raw = normalize_f64_range_loss(batch_loss_sum / view.batch_axis_len() as f64)?;
    tracing::debug!(
        event = "range_loss_call",
        batch = view.batch_axis_len() as u32,
        per_sample_axis = view.per_sample_axis_len() as u32,
        safe_lo = interval.lo(),
        safe_hi = interval.hi(),
        out_of_range_count,
        raw,
    );
    Ok(raw)
}

pub fn weighted_activation_range_penalty(
    raw_range_loss: f32,
    lambda_range: f32,
) -> Result<f32, ActivationRangeLossError> {
    validate_lambda_range(lambda_range)?;
    normalize_range_loss(raw_range_loss * lambda_range)
}

#[cfg(feature = "burn-adapter")]
pub fn burn_activation_range_penalty<B, const D: usize>(
    activations: BurnFloatTensor<B, D>,
    safe_bound: f32,
) -> Result<BurnFloatTensor<B, 1>, ActivationRangeLossError>
where
    B: BurnBackend,
{
    let safe_bound = SafeActivationBound::new(safe_bound)?;
    validate_burn_activation_shape(float_tensor_shape(&activations))?;

    let excess = (activations.abs() - safe_bound.value()).clamp_min(0.0);
    let loss = (excess.clone() * excess).mean();
    validate_burn_loss(&loss)?;

    Ok(loss)
}

#[cfg(feature = "burn-adapter")]
pub fn burn_range_loss<B>(
    activations: BurnFloatTensor<B, 2>,
    safe_lo: f32,
    safe_hi: f32,
) -> Result<BurnFloatTensor<B, 1>, ActivationRangeLossError>
where
    B: BurnBackend,
{
    let interval = SafeActivationInterval::new(safe_lo, safe_hi)?;
    let shape = float_tensor_shape(&activations);
    validate_burn_checked_activation_shape(shape)?;
    validate_burn_activation_shape(shape)?;

    let above = (activations.clone() - interval.hi()).clamp_min(0.0);
    let below = (interval.lo() - activations).clamp_min(0.0);
    let penalty = above.clone() * above + below.clone() * below;
    let loss = penalty.sum() / shape[0] as f32;
    validate_burn_loss(&loss)?;

    Ok(loss)
}

#[cfg(feature = "burn-adapter")]
pub fn burn_weighted_activation_range_penalty<B, const D: usize>(
    raw_range_loss: BurnFloatTensor<B, D>,
    lambda_range: f32,
) -> Result<BurnFloatTensor<B, D>, ActivationRangeLossError>
where
    B: BurnBackend,
{
    validate_lambda_range(lambda_range)?;
    let loss = raw_range_loss * lambda_range;
    validate_burn_loss(&loss)?;

    Ok(loss)
}

fn validate_activations(activations: &[f32]) -> Result<(), ActivationRangeLossError> {
    if activations.is_empty() {
        return Err(ActivationRangeLossError::EmptyActivations);
    }

    for (index, &value) in activations.iter().enumerate() {
        if !value.is_finite() {
            return Err(ActivationRangeLossError::NonFiniteActivation { index, value });
        }
    }

    Ok(())
}

fn validate_sample_width(
    activation_len: usize,
    sample_width: usize,
) -> Result<(), ActivationRangeLossError> {
    if sample_width == 0 {
        return Err(ActivationRangeLossError::ZeroActivationSampleWidth);
    }

    if !activation_len.is_multiple_of(sample_width) {
        return Err(
            ActivationRangeLossError::ActivationCountNotDivisibleBySampleWidth {
                activation_len,
                sample_width,
            },
        );
    }

    Ok(())
}

fn validate_lambda_range(lambda_range: f32) -> Result<(), ActivationRangeLossError> {
    if !lambda_range.is_finite() {
        return Err(ActivationRangeLossError::NonFiniteLambdaRange { lambda_range });
    }

    if lambda_range < 0.0 {
        return Err(ActivationRangeLossError::NegativeLambdaRange { lambda_range });
    }

    Ok(())
}

fn normalize_range_loss(value: f32) -> Result<f32, ActivationRangeLossError> {
    if !value.is_finite() {
        return Err(ActivationRangeLossError::NonFiniteLoss { value });
    }

    Ok(value)
}

fn normalize_f64_range_loss(value: f64) -> Result<f32, ActivationRangeLossError> {
    if !value.is_finite() || value > f64::from(f32::MAX) {
        return Err(ActivationRangeLossError::NonFiniteLoss {
            value: f32::INFINITY,
        });
    }

    normalize_range_loss(value as f32)
}

fn float_error_value_eq(left: f32, right: f32) -> bool {
    left == right || left.is_nan() && right.is_nan()
}

#[cfg(feature = "burn-adapter")]
fn validate_burn_activation_shape<const D: usize>(
    shape: [usize; D],
) -> Result<(), ActivationRangeLossError> {
    if shape.is_empty() || shape.contains(&0) {
        return Err(ActivationRangeLossError::InvalidActivationShape {
            shape: shape.to_vec(),
        });
    }

    shape
        .iter()
        .skip(1)
        .try_fold(1usize, |acc, dim| acc.checked_mul(*dim))
        .ok_or_else(|| ActivationRangeLossError::InvalidActivationShape {
            shape: shape.to_vec(),
        })?;

    Ok(())
}

#[cfg(feature = "burn-adapter")]
fn validate_burn_checked_activation_shape(
    shape: [usize; 2],
) -> Result<(), ActivationRangeLossError> {
    if shape.contains(&0) {
        return Err(ActivationRangeLossError::InvalidActivationShape {
            shape: shape.to_vec(),
        });
    }

    Ok(())
}

#[cfg(feature = "burn-adapter")]
fn validate_burn_loss<B, const D: usize>(
    loss: &BurnFloatTensor<B, D>,
) -> Result<(), ActivationRangeLossError>
where
    B: BurnBackend,
{
    for value in float_tensor_into_vec(loss.clone().detach())? {
        normalize_range_loss(value)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_close(actual: f32, expected: f32, tolerance: f32) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "expected {actual} to be within {tolerance} of {expected}"
        );
    }

    fn finite_difference_gradient(
        activations: &[f32],
        safe_bound: f32,
        index: usize,
        epsilon: f32,
    ) -> f32 {
        let mut plus = activations.to_vec();
        plus[index] += epsilon;
        let mut minus = activations.to_vec();
        minus[index] -= epsilon;

        let plus_loss = activation_range_penalty(&plus, safe_bound).unwrap();
        let minus_loss = activation_range_penalty(&minus, safe_bound).unwrap();

        (plus_loss - minus_loss) / (2.0 * epsilon)
    }

    #[test]
    fn activation_range_penalty_is_zero_inside_safe_bound() {
        let loss = activation_range_penalty(&[-1.0, -0.25, 0.0, 0.75, 1.0], 1.0).unwrap();

        assert_close(loss, 0.0, 1.0e-7);
    }

    #[test]
    fn activation_range_penalty_is_quadratic_outside_safe_bound() {
        let loss = activation_range_penalty(&[-1.5, -1.0, 0.25, 2.0], 1.0).unwrap();

        assert_close(loss, 0.3125, 1.0e-7);
    }

    #[test]
    fn activation_range_penalty_honors_non_default_safe_bound() {
        let loss = activation_range_penalty(&[-3.0, -2.0, 1.0, 2.5], 2.0).unwrap();

        assert_close(loss, 0.3125, 1.0e-7);
    }

    #[test]
    fn activation_range_penalty_uses_per_sample_then_batch_mean_reduction() {
        let row_a = activation_range_penalty(&[-1.5, 0.0], 1.0).unwrap();
        let row_b = activation_range_penalty(&[0.5, 2.0], 1.0).unwrap();
        let batch = ActivationRangeBatch::new(&[-1.5, 0.0, 0.5, 2.0], 2).unwrap();
        let batched =
            activation_range_penalty_for_batch(batch, SafeActivationBound::new(1.0).unwrap())
                .unwrap();

        assert_close(batched, (row_a + row_b) / 2.0, 1.0e-7);
    }

    #[test]
    fn range_loss_uses_explicit_lo_hi_and_batch_denominator() {
        let view = CheckedActivationView::new(&[-2.0, -0.5, 0.0, 2.0], 2, 2).unwrap();

        let loss = range_loss(view, -1.0, 1.0).unwrap();

        assert_close(loss, 1.0, 1.0e-7);
    }

    #[test]
    fn range_loss_does_not_double_penalize_negative_out_of_range_values() {
        let view = CheckedActivationView::new(&[-2.0], 1, 1).unwrap();

        let loss = range_loss(view, -1.0, 1.0).unwrap();

        assert_close(loss, 1.0, 1.0e-7);
    }

    #[test]
    fn checked_activation_view_rejects_non_2d_shapes() {
        assert_eq!(
            CheckedActivationView::from_shape(&[0.0], &[1]).unwrap_err(),
            ActivationRangeLossError::InvalidActivationShape { shape: vec![1] }
        );
        assert_eq!(
            CheckedActivationView::from_shape(&[0.0], &[1, 1, 1]).unwrap_err(),
            ActivationRangeLossError::InvalidActivationShape {
                shape: vec![1, 1, 1],
            }
        );
        assert_eq!(
            CheckedActivationView::from_shape(&[0.0, 1.0, 2.0], &[1, 2]).unwrap_err(),
            ActivationRangeLossError::ActivationCountNotDivisibleBySampleWidth {
                activation_len: 3,
                sample_width: 2,
            }
        );
    }

    #[test]
    fn activation_range_penalty_uses_f64_accumulation_for_finite_large_means() {
        let batch = ActivationRangeBatch::new(&[1.5e19, -1.5e19], 1).unwrap();

        let loss =
            activation_range_penalty_for_batch(batch, SafeActivationBound::new(1.0).unwrap())
                .unwrap();

        assert!(loss.is_finite());
        assert!(loss > 2.0e38);
    }

    #[test]
    fn activation_range_penalty_gradient_is_zero_inside_and_linear_outside() {
        let activations = [-1.5, -0.5, 0.5, 1.5];

        assert_close(
            finite_difference_gradient(&activations, 1.0, 0, 1.0e-3),
            -0.25,
            1.0e-3,
        );
        assert_close(
            finite_difference_gradient(&activations, 1.0, 1, 1.0e-3),
            0.0,
            1.0e-4,
        );
        assert_close(
            finite_difference_gradient(&activations, 1.0, 3, 1.0e-3),
            0.25,
            1.0e-3,
        );
    }

    #[test]
    fn weighted_activation_range_penalty_applies_explicit_lambda() {
        assert_close(
            weighted_activation_range_penalty(0.3125, 0.5).unwrap(),
            0.15625,
            1.0e-7,
        );
        assert_eq!(
            weighted_activation_range_penalty(f32::MAX, 2.0).unwrap_err(),
            ActivationRangeLossError::NonFiniteLoss {
                value: f32::INFINITY,
            }
        );
    }

    #[test]
    fn activation_range_penalty_validates_inputs() {
        assert_eq!(
            activation_range_penalty(&[], 1.0).unwrap_err(),
            ActivationRangeLossError::EmptyActivations
        );
        assert_eq!(
            activation_range_penalty(&[0.0], 0.0).unwrap_err(),
            ActivationRangeLossError::InvalidSafeBound { value: 0.0 }
        );
        assert_eq!(
            activation_range_penalty(&[0.0], f32::NAN).unwrap_err(),
            ActivationRangeLossError::InvalidSafeBound { value: f32::NAN }
        );
        assert_eq!(
            activation_range_penalty(&[f32::INFINITY], 1.0).unwrap_err(),
            ActivationRangeLossError::NonFiniteActivation {
                index: 0,
                value: f32::INFINITY,
            }
        );
        assert_eq!(
            ActivationRangeBatch::new(&[0.0], 0).unwrap_err(),
            ActivationRangeLossError::ZeroActivationSampleWidth
        );
        assert_eq!(
            ActivationRangeBatch::new(&[0.0, 1.0, 2.0], 2).unwrap_err(),
            ActivationRangeLossError::ActivationCountNotDivisibleBySampleWidth {
                activation_len: 3,
                sample_width: 2,
            }
        );
        assert_eq!(
            activation_range_penalty(&[f32::MAX], 1.0).unwrap_err(),
            ActivationRangeLossError::NonFiniteLoss {
                value: f32::INFINITY,
            }
        );
        assert_eq!(
            weighted_activation_range_penalty(1.0, -0.1).unwrap_err(),
            ActivationRangeLossError::NegativeLambdaRange { lambda_range: -0.1 }
        );
    }

    #[cfg(feature = "burn-adapter")]
    mod burn_tests {
        use super::*;
        use crate::adapter::burn::{
            BurnDevice, BurnNdArrayAutodiffBackend, float_tensor_from_vec, float_tensor_into_vec,
        };

        type B = BurnNdArrayAutodiffBackend;

        #[test]
        fn burn_activation_range_penalty_matches_scalar_oracle() {
            let device = BurnDevice::<B>::default();
            let values = vec![-3.0, -2.0, 1.0, 2.5];
            let activations =
                float_tensor_from_vec::<B, 2>(values.clone(), [2, 2], &device).unwrap();

            let burn_loss = burn_activation_range_penalty(activations, 2.0).unwrap();
            let scalar_loss = activation_range_penalty(&values, 2.0).unwrap();

            assert_close(
                float_tensor_into_vec(burn_loss).unwrap()[0],
                scalar_loss,
                1.0e-6,
            );
        }

        #[test]
        fn burn_range_loss_matches_checked_scalar_oracle() {
            let device = BurnDevice::<B>::default();
            let values = vec![-2.0, -0.5, 0.0, 2.0];
            let activations =
                float_tensor_from_vec::<B, 2>(values.clone(), [2, 2], &device).unwrap();
            let view = CheckedActivationView::new(&values, 2, 2).unwrap();

            let burn_loss = burn_range_loss(activations, -1.0, 1.0).unwrap();
            let scalar_loss = range_loss(view, -1.0, 1.0).unwrap();

            assert_close(
                float_tensor_into_vec(burn_loss).unwrap()[0],
                scalar_loss,
                1.0e-6,
            );
        }

        #[test]
        fn burn_range_loss_gradient_is_zero_inside_and_linear_outside() {
            let device = BurnDevice::<B>::default();
            let activations =
                float_tensor_from_vec::<B, 2>(vec![-2.0, -0.5, 0.5, 2.0], [2, 2], &device)
                    .unwrap()
                    .require_grad();

            let loss = burn_range_loss(activations.clone(), -1.0, 1.0).unwrap();
            let gradients = loss.backward();
            let grad = activations
                .grad(&gradients)
                .expect("activations should receive range-loss gradients");

            assert_eq!(
                float_tensor_into_vec(grad).unwrap(),
                vec![-1.0, 0.0, 0.0, 1.0]
            );
        }

        #[test]
        fn burn_activation_range_penalty_gradient_is_zero_inside_and_linear_outside() {
            let device = BurnDevice::<B>::default();
            let activations =
                float_tensor_from_vec::<B, 1>(vec![-1.5, -0.5, 0.5, 1.5], [4], &device)
                    .unwrap()
                    .require_grad();

            let loss = burn_activation_range_penalty(activations.clone(), 1.0).unwrap();
            let gradients = loss.backward();
            let grad = activations
                .grad(&gradients)
                .expect("activations should receive range-loss gradients");

            assert_eq!(
                float_tensor_into_vec(grad).unwrap(),
                vec![-0.25, 0.0, 0.0, 0.25]
            );
        }

        #[test]
        fn burn_activation_range_penalty_rejects_non_finite_result() {
            let device = BurnDevice::<B>::default();
            let activations = float_tensor_from_vec::<B, 1>(vec![f32::MAX], [1], &device).unwrap();

            assert_eq!(
                burn_activation_range_penalty(activations, 1.0).unwrap_err(),
                ActivationRangeLossError::NonFiniteLoss {
                    value: f32::INFINITY,
                }
            );
        }

        #[test]
        fn burn_weighted_activation_range_penalty_rejects_non_finite_result() {
            let device = BurnDevice::<B>::default();
            let raw_loss = float_tensor_from_vec::<B, 1>(vec![f32::MAX], [1], &device).unwrap();

            assert_eq!(
                burn_weighted_activation_range_penalty(raw_loss, 2.0).unwrap_err(),
                ActivationRangeLossError::NonFiniteLoss {
                    value: f32::INFINITY,
                }
            );
        }

        #[test]
        fn burn_weighted_activation_range_penalty_applies_lambda() {
            let device = BurnDevice::<B>::default();
            let raw_loss = float_tensor_from_vec::<B, 1>(vec![0.3125], [1], &device).unwrap();

            let weighted = burn_weighted_activation_range_penalty(raw_loss, 0.5).unwrap();

            assert_close(float_tensor_into_vec(weighted).unwrap()[0], 0.15625, 1.0e-7);
        }
    }
}
