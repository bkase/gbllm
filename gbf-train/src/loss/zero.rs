//! S2 ternary zero regularizer.
//!
//! The public S2 helper uses one threshold per output row and computes the raw
//! diagnostic even when the caller's effective lambda is zero. The indicator is
//! treated as stop-gradient in the Burn path; only the `abs(weight)` factor
//! carries gradients for below-threshold entries.

use std::error::Error;
use std::fmt;

#[cfg(feature = "burn-adapter")]
use crate::adapter::burn::{
    BurnAdapterError, BurnBackend, BurnFloatTensor, float_tensor_into_vec, float_tensor_shape,
};

#[derive(Debug, Clone, PartialEq)]
pub struct PerRowThresholds {
    thresholds: Vec<f32>,
}

impl PerRowThresholds {
    pub fn new(thresholds: Vec<f32>) -> Result<Self, ZeroLossError> {
        if thresholds.is_empty() {
            return Err(ZeroLossError::EmptyThresholds);
        }
        for (index, &value) in thresholds.iter().enumerate() {
            if !value.is_finite() || value < 0.0 {
                return Err(ZeroLossError::InvalidThreshold { index, value });
            }
        }

        Ok(Self { thresholds })
    }

    pub fn for_rows(self, rows: usize) -> Result<Self, ZeroLossError> {
        if self.thresholds.len() != rows {
            tracing::error!(
                event = "threshold_shape_error",
                expected = "per_row",
                observed = ?vec![self.thresholds.len()],
            );
            return Err(ZeroLossError::LengthMismatch {
                expected: rows,
                got: self.thresholds.len(),
            });
        }

        Ok(self)
    }

    pub fn reject_matrix_shape(rows: usize, cols: usize) -> Result<Self, ZeroLossError> {
        tracing::error!(
            event = "threshold_shape_error",
            expected = "per_row",
            observed = ?vec![rows, cols],
        );
        Err(ZeroLossError::ThresholdShapeError {
            expected: "per_row",
            observed: vec![rows, cols],
        })
    }

    #[must_use]
    pub fn values(&self) -> &[f32] {
        &self.thresholds
    }
}

#[derive(Debug)]
pub enum ZeroLossError {
    EmptyWeights,
    ZeroRows,
    ZeroCols,
    WeightCountOverflow {
        rows: usize,
        cols: usize,
    },
    WeightCountMismatch {
        expected: usize,
        got: usize,
    },
    EmptyThresholds,
    LengthMismatch {
        expected: usize,
        got: usize,
    },
    ThresholdShapeError {
        expected: &'static str,
        observed: Vec<usize>,
    },
    NonFiniteWeight {
        index: usize,
        value: f32,
    },
    InvalidThreshold {
        index: usize,
        value: f32,
    },
    NonFiniteLoss {
        value: f32,
    },
    NegativeLoss {
        value: f32,
    },
    NegativeLambdaZero {
        lambda_zero: f32,
    },
    NonFiniteLambdaZero {
        lambda_zero: f32,
    },
    #[cfg(feature = "burn-adapter")]
    InvalidWeightShape {
        shape: Vec<usize>,
    },
    #[cfg(feature = "burn-adapter")]
    InvalidThresholdShape {
        expected: usize,
        got: usize,
    },
    #[cfg(feature = "burn-adapter")]
    BurnAdapter(BurnAdapterError),
}

impl PartialEq for ZeroLossError {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::EmptyWeights, Self::EmptyWeights)
            | (Self::ZeroRows, Self::ZeroRows)
            | (Self::ZeroCols, Self::ZeroCols)
            | (Self::EmptyThresholds, Self::EmptyThresholds) => true,
            (
                Self::WeightCountOverflow {
                    rows: left_rows,
                    cols: left_cols,
                },
                Self::WeightCountOverflow {
                    rows: right_rows,
                    cols: right_cols,
                },
            ) => left_rows == right_rows && left_cols == right_cols,
            (
                Self::WeightCountMismatch {
                    expected: left_expected,
                    got: left_got,
                },
                Self::WeightCountMismatch {
                    expected: right_expected,
                    got: right_got,
                },
            )
            | (
                Self::LengthMismatch {
                    expected: left_expected,
                    got: left_got,
                },
                Self::LengthMismatch {
                    expected: right_expected,
                    got: right_got,
                },
            ) => left_expected == right_expected && left_got == right_got,
            (
                Self::ThresholdShapeError {
                    expected: left_expected,
                    observed: left_observed,
                },
                Self::ThresholdShapeError {
                    expected: right_expected,
                    observed: right_observed,
                },
            ) => left_expected == right_expected && left_observed == right_observed,
            (
                Self::NonFiniteWeight {
                    index: left_index,
                    value: left_value,
                },
                Self::NonFiniteWeight {
                    index: right_index,
                    value: right_value,
                },
            )
            | (
                Self::InvalidThreshold {
                    index: left_index,
                    value: left_value,
                },
                Self::InvalidThreshold {
                    index: right_index,
                    value: right_value,
                },
            ) => left_index == right_index && float_error_value_eq(*left_value, *right_value),
            (
                Self::NonFiniteLoss { value: left_value },
                Self::NonFiniteLoss { value: right_value },
            )
            | (
                Self::NegativeLoss { value: left_value },
                Self::NegativeLoss { value: right_value },
            ) => float_error_value_eq(*left_value, *right_value),
            (
                Self::NegativeLambdaZero {
                    lambda_zero: left_lambda,
                },
                Self::NegativeLambdaZero {
                    lambda_zero: right_lambda,
                },
            )
            | (
                Self::NonFiniteLambdaZero {
                    lambda_zero: left_lambda,
                },
                Self::NonFiniteLambdaZero {
                    lambda_zero: right_lambda,
                },
            ) => float_error_value_eq(*left_lambda, *right_lambda),
            #[cfg(feature = "burn-adapter")]
            (
                Self::InvalidWeightShape { shape: left_shape },
                Self::InvalidWeightShape { shape: right_shape },
            ) => left_shape == right_shape,
            #[cfg(feature = "burn-adapter")]
            (
                Self::InvalidThresholdShape {
                    expected: left_expected,
                    got: left_got,
                },
                Self::InvalidThresholdShape {
                    expected: right_expected,
                    got: right_got,
                },
            ) => left_expected == right_expected && left_got == right_got,
            #[cfg(feature = "burn-adapter")]
            (Self::BurnAdapter(_), Self::BurnAdapter(_)) => false,
            _ => false,
        }
    }
}

impl fmt::Display for ZeroLossError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyWeights => f.write_str("zero_loss weights must not be empty"),
            Self::ZeroRows => f.write_str("zero_loss rows must be greater than 0"),
            Self::ZeroCols => f.write_str("zero_loss cols must be greater than 0"),
            Self::WeightCountOverflow { rows, cols } => {
                write!(f, "zero_loss shape {rows}x{cols} overflows usize")
            }
            Self::WeightCountMismatch { expected, got } => {
                write!(f, "zero_loss expected {expected} weights, got {got}")
            }
            Self::EmptyThresholds => f.write_str("zero_loss thresholds must not be empty"),
            Self::LengthMismatch { expected, got } => {
                write!(
                    f,
                    "zero_loss expected {expected} per-row thresholds, got {got}"
                )
            }
            Self::ThresholdShapeError { expected, observed } => {
                write!(
                    f,
                    "zero_loss expected {expected} thresholds, got shape {observed:?}"
                )
            }
            Self::NonFiniteWeight { index, value } => {
                write!(
                    f,
                    "zero_loss weight at index {index} must be finite, got {value}"
                )
            }
            Self::InvalidThreshold { index, value } => {
                write!(
                    f,
                    "zero_loss threshold at index {index} must be finite and non-negative, got {value}"
                )
            }
            Self::NonFiniteLoss { value } => {
                write!(f, "zero_loss must be finite, got {value}")
            }
            Self::NegativeLoss { value } => {
                write!(f, "zero_loss must be non-negative, got {value}")
            }
            Self::NegativeLambdaZero { lambda_zero } => {
                write!(f, "lambda_zero must be non-negative, got {lambda_zero}")
            }
            Self::NonFiniteLambdaZero { lambda_zero } => {
                write!(f, "lambda_zero must be finite, got {lambda_zero}")
            }
            #[cfg(feature = "burn-adapter")]
            Self::InvalidWeightShape { shape } => {
                write!(
                    f,
                    "zero_loss weight tensor must be rank 2 with non-zero dimensions, got {shape:?}"
                )
            }
            #[cfg(feature = "burn-adapter")]
            Self::InvalidThresholdShape { expected, got } => {
                write!(f, "zero_loss expected {expected} threshold rows, got {got}")
            }
            #[cfg(feature = "burn-adapter")]
            Self::BurnAdapter(error) => write!(f, "{error}"),
        }
    }
}

impl Error for ZeroLossError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            #[cfg(feature = "burn-adapter")]
            Self::BurnAdapter(error) => Some(error),
            _ => None,
        }
    }
}

#[cfg(feature = "burn-adapter")]
impl From<BurnAdapterError> for ZeroLossError {
    fn from(error: BurnAdapterError) -> Self {
        Self::BurnAdapter(error)
    }
}

pub fn zero_loss(
    weights: &[f32],
    rows: usize,
    cols: usize,
    thresholds: &PerRowThresholds,
) -> Result<f32, ZeroLossError> {
    validate_matrix(weights, rows, cols)?;
    thresholds.clone().for_rows(rows)?;

    let mut below_threshold_count = 0_u32;
    let mut row_mean_sum = 0.0_f64;
    for (row_index, row) in weights.chunks_exact(cols).enumerate() {
        let threshold = thresholds.values()[row_index];
        let mut row_sum = 0.0_f64;
        for &weight in row {
            let abs_weight = weight.abs();
            if abs_weight < threshold {
                below_threshold_count = below_threshold_count.saturating_add(1);
                row_sum += f64::from(abs_weight);
            }
        }
        row_mean_sum += row_sum / cols as f64;
    }

    let raw = normalize_f64_zero_loss(row_mean_sum / rows as f64)?;
    let (threshold_min, threshold_max) = threshold_min_max(thresholds.values());
    tracing::debug!(
        event = "zero_loss_call",
        rows = rows as u32,
        cols = cols as u32,
        threshold_min,
        threshold_max,
        below_threshold_count,
        raw,
    );
    Ok(raw)
}

pub fn weighted_zero_loss(raw_zero_loss: f32, lambda_zero: f32) -> Result<f32, ZeroLossError> {
    validate_lambda_zero(lambda_zero)?;
    normalize_zero_loss(raw_zero_loss)?;
    normalize_zero_loss(raw_zero_loss * lambda_zero)
}

#[cfg(feature = "burn-adapter")]
pub fn burn_zero_loss<B>(
    weights: BurnFloatTensor<B, 2>,
    thresholds: BurnFloatTensor<B, 1>,
) -> Result<BurnFloatTensor<B, 1>, ZeroLossError>
where
    B: BurnBackend,
{
    let weight_shape = float_tensor_shape(&weights);
    let threshold_shape = float_tensor_shape(&thresholds);
    validate_burn_weight_shape(weight_shape)?;
    validate_burn_threshold_shape(weight_shape[0], threshold_shape[0])?;
    validate_burn_thresholds(&thresholds)?;

    let threshold_grid = thresholds
        .detach()
        .reshape([weight_shape[0], 1])
        .repeat_dim(1, weight_shape[1]);
    let abs_weights = weights.abs();
    let below_threshold = abs_weights.clone().lower(threshold_grid);
    let selected = abs_weights
        .zeros_like()
        .mask_where(below_threshold, abs_weights);
    let row_means = selected.sum_dim(1) / weight_shape[1] as f32;
    let loss = row_means.mean();
    validate_burn_loss(&loss)?;

    Ok(loss)
}

#[cfg(feature = "burn-adapter")]
pub fn burn_weighted_zero_loss<B, const D: usize>(
    raw_zero_loss: BurnFloatTensor<B, D>,
    lambda_zero: f32,
) -> Result<BurnFloatTensor<B, D>, ZeroLossError>
where
    B: BurnBackend,
{
    validate_lambda_zero(lambda_zero)?;
    validate_burn_loss(&raw_zero_loss)?;
    let loss = raw_zero_loss * lambda_zero;
    validate_burn_loss(&loss)?;
    Ok(loss)
}

fn validate_matrix(weights: &[f32], rows: usize, cols: usize) -> Result<(), ZeroLossError> {
    if weights.is_empty() {
        return Err(ZeroLossError::EmptyWeights);
    }
    if rows == 0 {
        return Err(ZeroLossError::ZeroRows);
    }
    if cols == 0 {
        return Err(ZeroLossError::ZeroCols);
    }
    let expected = rows
        .checked_mul(cols)
        .ok_or(ZeroLossError::WeightCountOverflow { rows, cols })?;
    if weights.len() != expected {
        return Err(ZeroLossError::WeightCountMismatch {
            expected,
            got: weights.len(),
        });
    }
    for (index, &value) in weights.iter().enumerate() {
        if !value.is_finite() {
            return Err(ZeroLossError::NonFiniteWeight { index, value });
        }
    }
    Ok(())
}

fn validate_lambda_zero(lambda_zero: f32) -> Result<(), ZeroLossError> {
    if !lambda_zero.is_finite() {
        return Err(ZeroLossError::NonFiniteLambdaZero { lambda_zero });
    }
    if lambda_zero < 0.0 {
        return Err(ZeroLossError::NegativeLambdaZero { lambda_zero });
    }
    Ok(())
}

fn normalize_zero_loss(value: f32) -> Result<f32, ZeroLossError> {
    if !value.is_finite() {
        return Err(ZeroLossError::NonFiniteLoss { value });
    }
    if value < 0.0 {
        return Err(ZeroLossError::NegativeLoss { value });
    }
    Ok(value)
}

fn normalize_f64_zero_loss(value: f64) -> Result<f32, ZeroLossError> {
    if !value.is_finite() || value > f64::from(f32::MAX) {
        return Err(ZeroLossError::NonFiniteLoss {
            value: f32::INFINITY,
        });
    }
    normalize_zero_loss(value as f32)
}

fn threshold_min_max(thresholds: &[f32]) -> (f32, f32) {
    thresholds
        .iter()
        .copied()
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(min, max), value| {
            (min.min(value), max.max(value))
        })
}

fn float_error_value_eq(left: f32, right: f32) -> bool {
    left == right || left.is_nan() && right.is_nan()
}

#[cfg(feature = "burn-adapter")]
fn validate_burn_weight_shape(shape: [usize; 2]) -> Result<(), ZeroLossError> {
    if shape.contains(&0) {
        return Err(ZeroLossError::InvalidWeightShape {
            shape: shape.to_vec(),
        });
    }
    Ok(())
}

#[cfg(feature = "burn-adapter")]
fn validate_burn_threshold_shape(expected: usize, got: usize) -> Result<(), ZeroLossError> {
    if got != expected {
        tracing::error!(
            event = "threshold_shape_error",
            expected = "per_row",
            observed = ?vec![got],
        );
        return Err(ZeroLossError::InvalidThresholdShape { expected, got });
    }
    Ok(())
}

#[cfg(feature = "burn-adapter")]
fn validate_burn_thresholds<B>(thresholds: &BurnFloatTensor<B, 1>) -> Result<(), ZeroLossError>
where
    B: BurnBackend,
{
    for (index, value) in float_tensor_into_vec(thresholds.clone().detach())?
        .iter()
        .copied()
        .enumerate()
    {
        if !value.is_finite() || value < 0.0 {
            return Err(ZeroLossError::InvalidThreshold { index, value });
        }
    }
    Ok(())
}

#[cfg(feature = "burn-adapter")]
fn validate_burn_loss<B, const D: usize>(loss: &BurnFloatTensor<B, D>) -> Result<(), ZeroLossError>
where
    B: BurnBackend,
{
    for value in float_tensor_into_vec(loss.clone().detach())? {
        normalize_zero_loss(value)?;
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

    fn finite_difference_gradient(weights: &[f32], index: usize, epsilon: f32) -> f32 {
        let thresholds = PerRowThresholds::new(vec![0.5]).unwrap();
        let mut plus = weights.to_vec();
        plus[index] += epsilon;
        let mut minus = weights.to_vec();
        minus[index] -= epsilon;

        let plus_loss = zero_loss(&plus, 1, weights.len(), &thresholds).unwrap();
        let minus_loss = zero_loss(&minus, 1, weights.len(), &thresholds).unwrap();

        (plus_loss - minus_loss) / (2.0 * epsilon)
    }

    #[test]
    fn zero_loss_uses_per_row_threshold_indicator_and_l1_factor() {
        let weights = [
            -0.1, 0.1, -0.3, 0.3, -0.7, 0.7, -0.9, 0.9, -0.1, 0.1, -0.3, 0.3, -0.7, 0.7, -0.9, 0.9,
            -0.1, 0.1, -0.3, 0.3, -0.7, 0.7, -0.9, 0.9, -0.1, 0.1, -0.3, 0.3, -0.7, 0.7, -0.9, 0.9,
        ];
        let thresholds = PerRowThresholds::new(vec![0.5, 0.5, 0.5, 0.5]).unwrap();

        let loss = zero_loss(&weights, 4, 8, &thresholds).unwrap();

        assert_close(loss, 0.1, 1.0e-7);
    }

    #[test]
    fn zero_loss_raw_is_honest_when_weighted_lambda_is_zero() {
        let thresholds = PerRowThresholds::new(vec![0.5]).unwrap();
        let raw = zero_loss(&[0.1, 0.7], 1, 2, &thresholds).unwrap();

        assert_close(raw, 0.05, 1.0e-7);
        assert_close(weighted_zero_loss(raw, 0.0).unwrap(), 0.0, 1.0e-7);
        assert_eq!(
            weighted_zero_loss(f32::NAN, 0.0).unwrap_err(),
            ZeroLossError::NonFiniteLoss { value: f32::NAN }
        );
    }

    #[test]
    fn zero_loss_gradient_reference_is_nonzero_only_below_threshold() {
        let weights = [-0.3, -0.7, 0.3, 0.7];

        assert_close(
            finite_difference_gradient(&weights, 0, 1.0e-3),
            -0.25,
            1.0e-3,
        );
        assert_close(finite_difference_gradient(&weights, 1, 1.0e-3), 0.0, 1.0e-3);
        assert_close(
            finite_difference_gradient(&weights, 2, 1.0e-3),
            0.25,
            1.0e-3,
        );
        assert_close(finite_difference_gradient(&weights, 3, 1.0e-3), 0.0, 1.0e-3);
    }

    #[test]
    fn per_row_thresholds_reject_wrong_shapes() {
        let thresholds = PerRowThresholds::new(vec![0.5]).unwrap();
        assert_eq!(
            zero_loss(&[0.1, 0.2, 0.3, 0.4], 2, 2, &thresholds).unwrap_err(),
            ZeroLossError::LengthMismatch {
                expected: 2,
                got: 1,
            }
        );
        assert_eq!(
            PerRowThresholds::reject_matrix_shape(4, 8).unwrap_err(),
            ZeroLossError::ThresholdShapeError {
                expected: "per_row",
                observed: vec![4, 8],
            }
        );
    }

    #[cfg(feature = "burn-adapter")]
    mod burn_tests {
        use super::*;
        use crate::adapter::burn::{
            BurnDevice, BurnNdArrayAutodiffBackend, BurnNdArrayBackend, float_tensor_from_vec,
            float_tensor_into_vec,
        };

        #[test]
        fn burn_zero_loss_matches_scalar_oracle() {
            type B = BurnNdArrayBackend;

            let device = BurnDevice::<B>::default();
            let values = vec![-0.1, 0.7, 0.3, -0.9];
            let weights = float_tensor_from_vec::<B, 2>(values.clone(), [2, 2], &device).unwrap();
            let thresholds = float_tensor_from_vec::<B, 1>(vec![0.5, 0.5], [2], &device).unwrap();
            let scalar_thresholds = PerRowThresholds::new(vec![0.5, 0.5]).unwrap();

            let loss = burn_zero_loss(weights, thresholds).unwrap();
            let scalar = zero_loss(&values, 2, 2, &scalar_thresholds).unwrap();

            assert_close(float_tensor_into_vec(loss).unwrap()[0], scalar, 1.0e-6);
        }

        #[test]
        fn burn_zero_loss_gradients_stop_at_indicator_and_threshold() {
            type B = BurnNdArrayAutodiffBackend;

            let device = BurnDevice::<B>::default();
            let weights =
                float_tensor_from_vec::<B, 2>(vec![-0.3, -0.7, 0.3, 0.7], [2, 2], &device)
                    .unwrap()
                    .require_grad();
            let thresholds = float_tensor_from_vec::<B, 1>(vec![0.5, 0.5], [2], &device)
                .unwrap()
                .require_grad();

            let loss = burn_zero_loss(weights.clone(), thresholds.clone()).unwrap();
            let gradients = loss.backward();
            let weight_grad = weights
                .grad(&gradients)
                .expect("weights should receive zero-loss gradients");

            assert_eq!(
                float_tensor_into_vec(weight_grad).unwrap(),
                vec![-0.25, 0.0, 0.25, 0.0]
            );
            assert!(thresholds.grad(&gradients).is_none());
        }
    }
}
