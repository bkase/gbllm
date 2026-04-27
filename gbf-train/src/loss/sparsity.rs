//! Ternary sparsity regularization loss.
//!
//! This module owns the raw `lambda_zero` diagnostic term for full-precision
//! pre-threshold expert weights:
//! `mean(min(abs(weight), threshold))`. The term gives zero-valued weights zero
//! loss, gives sub-threshold non-zero weights an L1-style shrink-to-zero
//! gradient, and stops penalizing weights once they are already beyond the
//! ternary threshold. Callers that compose total loss apply `lambda_zero`
//! explicitly and own the config gate that avoids computing this term when
//! `lambda_zero` is zero.

use std::error::Error;
use std::fmt;

#[cfg(feature = "burn-adapter")]
use crate::adapter::burn::{
    BurnAdapterError, BurnBackend, BurnFloatTensor, float_tensor_into_vec, float_tensor_shape,
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TernaryZeroThreshold {
    value: f32,
}

impl TernaryZeroThreshold {
    pub fn new(value: f32) -> Result<Self, TernaryZeroRegularizerError> {
        if !value.is_finite() || value < 0.0 {
            return Err(TernaryZeroRegularizerError::InvalidThreshold { index: 0, value });
        }

        Ok(Self { value })
    }

    #[must_use]
    pub const fn value(self) -> f32 {
        self.value
    }
}

#[derive(Debug)]
pub enum TernaryZeroRegularizerError {
    EmptyWeights,
    ZeroOutputRows,
    ZeroInputCols,
    WeightCountOverflow {
        output_rows: usize,
        input_cols: usize,
    },
    WeightCountMismatch {
        expected: usize,
        actual: usize,
    },
    EmptyThresholds,
    ThresholdCountMismatch {
        output_rows: usize,
        threshold_count: usize,
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
    BurnAdapter(BurnAdapterError),
}

impl PartialEq for TernaryZeroRegularizerError {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::EmptyWeights, Self::EmptyWeights)
            | (Self::ZeroOutputRows, Self::ZeroOutputRows)
            | (Self::ZeroInputCols, Self::ZeroInputCols)
            | (Self::EmptyThresholds, Self::EmptyThresholds) => true,
            (
                Self::WeightCountOverflow {
                    output_rows: left_output_rows,
                    input_cols: left_input_cols,
                },
                Self::WeightCountOverflow {
                    output_rows: right_output_rows,
                    input_cols: right_input_cols,
                },
            ) => left_output_rows == right_output_rows && left_input_cols == right_input_cols,
            (
                Self::WeightCountMismatch {
                    expected: left_expected,
                    actual: left_actual,
                },
                Self::WeightCountMismatch {
                    expected: right_expected,
                    actual: right_actual,
                },
            ) => left_expected == right_expected && left_actual == right_actual,
            (
                Self::ThresholdCountMismatch {
                    output_rows: left_output_rows,
                    threshold_count: left_threshold_count,
                },
                Self::ThresholdCountMismatch {
                    output_rows: right_output_rows,
                    threshold_count: right_threshold_count,
                },
            ) => {
                left_output_rows == right_output_rows
                    && left_threshold_count == right_threshold_count
            }
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
            (Self::BurnAdapter(_), Self::BurnAdapter(_)) => false,
            _ => false,
        }
    }
}

impl fmt::Display for TernaryZeroRegularizerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyWeights => f.write_str("ternary zero regularizer weights must not be empty"),
            Self::ZeroOutputRows => {
                f.write_str("ternary zero regularizer output_rows must be greater than 0")
            }
            Self::ZeroInputCols => {
                f.write_str("ternary zero regularizer input_cols must be greater than 0")
            }
            Self::WeightCountOverflow {
                output_rows,
                input_cols,
            } => write!(
                f,
                "ternary zero regularizer matrix shape {output_rows}x{input_cols} overflows usize"
            ),
            Self::WeightCountMismatch { expected, actual } => write!(
                f,
                "ternary zero regularizer matrix expects {expected} weights, got {actual}"
            ),
            Self::EmptyThresholds => {
                f.write_str("ternary zero regularizer thresholds must not be empty")
            }
            Self::ThresholdCountMismatch {
                output_rows,
                threshold_count,
            } => write!(
                f,
                "threshold count must be 1 or match output row count {output_rows}, got {threshold_count}"
            ),
            Self::NonFiniteWeight { index, value } => {
                write!(f, "weight at index {index} must be finite, got {value}")
            }
            Self::InvalidThreshold { index, value } => write!(
                f,
                "threshold at index {index} must be finite and non-negative, got {value}"
            ),
            Self::NonFiniteLoss { value } => {
                write!(
                    f,
                    "ternary zero regularizer loss must be finite, got {value}"
                )
            }
            Self::NegativeLoss { value } => {
                write!(
                    f,
                    "ternary zero regularizer loss must be non-negative, got {value}"
                )
            }
            Self::NegativeLambdaZero { lambda_zero } => {
                write!(f, "lambda_zero must be non-negative, got {lambda_zero}")
            }
            Self::NonFiniteLambdaZero { lambda_zero } => {
                write!(f, "lambda_zero must be finite, got {lambda_zero}")
            }
            #[cfg(feature = "burn-adapter")]
            Self::InvalidWeightShape { shape } => write!(
                f,
                "ternary zero regularizer weight tensor must be rank >= 1 with non-zero dimensions, got {shape:?}"
            ),
            #[cfg(feature = "burn-adapter")]
            Self::BurnAdapter(error) => write!(f, "{error}"),
        }
    }
}

impl Error for TernaryZeroRegularizerError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            #[cfg(feature = "burn-adapter")]
            Self::BurnAdapter(error) => Some(error),
            Self::EmptyWeights
            | Self::ZeroOutputRows
            | Self::ZeroInputCols
            | Self::WeightCountOverflow { .. }
            | Self::WeightCountMismatch { .. }
            | Self::EmptyThresholds
            | Self::ThresholdCountMismatch { .. }
            | Self::NonFiniteWeight { .. }
            | Self::InvalidThreshold { .. }
            | Self::NonFiniteLoss { .. }
            | Self::NegativeLoss { .. }
            | Self::NegativeLambdaZero { .. }
            | Self::NonFiniteLambdaZero { .. } => None,
            #[cfg(feature = "burn-adapter")]
            Self::InvalidWeightShape { .. } => None,
        }
    }
}

#[cfg(feature = "burn-adapter")]
impl From<BurnAdapterError> for TernaryZeroRegularizerError {
    fn from(error: BurnAdapterError) -> Self {
        Self::BurnAdapter(error)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct TernaryZeroWeightMatrix<'a> {
    weights: &'a [f32],
    output_rows: usize,
    input_cols: usize,
}

impl<'a> TernaryZeroWeightMatrix<'a> {
    pub fn new(
        weights: &'a [f32],
        output_rows: usize,
        input_cols: usize,
    ) -> Result<Self, TernaryZeroRegularizerError> {
        if output_rows == 0 {
            return Err(TernaryZeroRegularizerError::ZeroOutputRows);
        }

        if input_cols == 0 {
            return Err(TernaryZeroRegularizerError::ZeroInputCols);
        }

        let expected = output_rows.checked_mul(input_cols).ok_or(
            TernaryZeroRegularizerError::WeightCountOverflow {
                output_rows,
                input_cols,
            },
        )?;
        if weights.len() != expected {
            return Err(TernaryZeroRegularizerError::WeightCountMismatch {
                expected,
                actual: weights.len(),
            });
        }

        validate_weights(weights)?;

        Ok(Self {
            weights,
            output_rows,
            input_cols,
        })
    }

    #[must_use]
    pub const fn weights(self) -> &'a [f32] {
        self.weights
    }

    #[must_use]
    pub const fn output_rows(self) -> usize {
        self.output_rows
    }

    #[must_use]
    pub const fn input_cols(self) -> usize {
        self.input_cols
    }
}

/// Mean capped L1 penalty for pre-threshold expert weights with one global threshold.
pub fn ternary_zero_regularizer(
    weights: &[f32],
    threshold: f32,
) -> Result<f32, TernaryZeroRegularizerError> {
    let threshold = TernaryZeroThreshold::new(threshold)?;
    validate_weights(weights)?;

    let loss_sum = weights
        .iter()
        .map(|weight| f64::from(weight.abs().min(threshold.value())))
        .sum::<f64>();

    normalize_f64_zero_loss(loss_sum / weights.len() as f64)
}

/// Mean capped L1 penalty for a row-major expert weight matrix.
///
/// `thresholds` may contain either one global threshold or one threshold per
/// output row, matching the ternary model's per-row threshold contract.
pub fn ternary_zero_regularizer_for_matrix(
    matrix: TernaryZeroWeightMatrix<'_>,
    thresholds: &[f32],
) -> Result<f32, TernaryZeroRegularizerError> {
    validate_thresholds(matrix.output_rows(), thresholds)?;

    let threshold_count = thresholds.len();
    let loss_sum = matrix
        .weights()
        .iter()
        .enumerate()
        .map(|(index, weight)| {
            let threshold = if threshold_count == 1 {
                thresholds[0]
            } else {
                thresholds[index / matrix.input_cols()]
            };
            f64::from(weight.abs().min(threshold))
        })
        .sum::<f64>();

    normalize_f64_zero_loss(loss_sum / matrix.weights().len() as f64)
}

pub fn weighted_ternary_zero_regularizer(
    raw_zero_loss: f32,
    lambda_zero: f32,
) -> Result<f32, TernaryZeroRegularizerError> {
    validate_lambda_zero(lambda_zero)?;
    normalize_zero_loss(raw_zero_loss)?;

    normalize_zero_loss(raw_zero_loss * lambda_zero)
}

#[cfg(feature = "burn-adapter")]
pub fn burn_ternary_zero_regularizer<B, const D: usize>(
    weights: BurnFloatTensor<B, D>,
    threshold: f32,
) -> Result<BurnFloatTensor<B, 1>, TernaryZeroRegularizerError>
where
    B: BurnBackend,
{
    let threshold = TernaryZeroThreshold::new(threshold)?;
    validate_burn_weight_shape(float_tensor_shape(&weights))?;

    let loss = weights.abs().clamp(0.0, threshold.value()).mean();
    validate_burn_loss(&loss)?;

    Ok(loss)
}

#[cfg(feature = "burn-adapter")]
pub fn burn_ternary_zero_regularizer_for_matrix<B>(
    weights: BurnFloatTensor<B, 2>,
    thresholds: BurnFloatTensor<B, 1>,
) -> Result<BurnFloatTensor<B, 1>, TernaryZeroRegularizerError>
where
    B: BurnBackend,
{
    let weight_shape = float_tensor_shape(&weights);
    let threshold_shape = float_tensor_shape(&thresholds);
    validate_burn_weight_shape(weight_shape)?;
    validate_burn_row_threshold_shape(weight_shape[0], threshold_shape[0])?;
    validate_burn_thresholds(&thresholds)?;

    let threshold_grid =
        expand_burn_row_thresholds(thresholds.detach(), weight_shape, threshold_shape[0]);
    let loss = capped_abs_with_thresholds(weights.abs(), threshold_grid).mean();
    validate_burn_loss(&loss)?;

    Ok(loss)
}

#[cfg(feature = "burn-adapter")]
pub fn burn_weighted_ternary_zero_regularizer<B, const D: usize>(
    raw_zero_loss: BurnFloatTensor<B, D>,
    lambda_zero: f32,
) -> Result<BurnFloatTensor<B, D>, TernaryZeroRegularizerError>
where
    B: BurnBackend,
{
    validate_lambda_zero(lambda_zero)?;
    validate_burn_loss(&raw_zero_loss)?;
    let loss = raw_zero_loss * lambda_zero;
    validate_burn_loss(&loss)?;

    Ok(loss)
}

fn validate_weights(weights: &[f32]) -> Result<(), TernaryZeroRegularizerError> {
    if weights.is_empty() {
        return Err(TernaryZeroRegularizerError::EmptyWeights);
    }

    for (index, &value) in weights.iter().enumerate() {
        if !value.is_finite() {
            return Err(TernaryZeroRegularizerError::NonFiniteWeight { index, value });
        }
    }

    Ok(())
}

fn validate_thresholds(
    output_rows: usize,
    thresholds: &[f32],
) -> Result<(), TernaryZeroRegularizerError> {
    if thresholds.is_empty() {
        return Err(TernaryZeroRegularizerError::EmptyThresholds);
    }

    if thresholds.len() != 1 && thresholds.len() != output_rows {
        return Err(TernaryZeroRegularizerError::ThresholdCountMismatch {
            output_rows,
            threshold_count: thresholds.len(),
        });
    }

    for (index, &value) in thresholds.iter().enumerate() {
        if !value.is_finite() || value < 0.0 {
            return Err(TernaryZeroRegularizerError::InvalidThreshold { index, value });
        }
    }

    Ok(())
}

fn validate_lambda_zero(lambda_zero: f32) -> Result<(), TernaryZeroRegularizerError> {
    if !lambda_zero.is_finite() {
        return Err(TernaryZeroRegularizerError::NonFiniteLambdaZero { lambda_zero });
    }

    if lambda_zero < 0.0 {
        return Err(TernaryZeroRegularizerError::NegativeLambdaZero { lambda_zero });
    }

    Ok(())
}

fn normalize_zero_loss(value: f32) -> Result<f32, TernaryZeroRegularizerError> {
    if !value.is_finite() {
        return Err(TernaryZeroRegularizerError::NonFiniteLoss { value });
    }

    if value < 0.0 {
        return Err(TernaryZeroRegularizerError::NegativeLoss { value });
    }

    Ok(value)
}

fn normalize_f64_zero_loss(value: f64) -> Result<f32, TernaryZeroRegularizerError> {
    if !value.is_finite() || value > f64::from(f32::MAX) {
        return Err(TernaryZeroRegularizerError::NonFiniteLoss {
            value: f32::INFINITY,
        });
    }

    normalize_zero_loss(value as f32)
}

fn float_error_value_eq(left: f32, right: f32) -> bool {
    left == right || left.is_nan() && right.is_nan()
}

#[cfg(feature = "burn-adapter")]
fn validate_burn_weight_shape<const D: usize>(
    shape: [usize; D],
) -> Result<(), TernaryZeroRegularizerError> {
    if shape.is_empty() || shape.contains(&0) {
        return Err(TernaryZeroRegularizerError::InvalidWeightShape {
            shape: shape.to_vec(),
        });
    }

    shape
        .iter()
        .try_fold(1usize, |acc, dim| acc.checked_mul(*dim))
        .ok_or_else(|| TernaryZeroRegularizerError::InvalidWeightShape {
            shape: shape.to_vec(),
        })?;

    Ok(())
}

#[cfg(feature = "burn-adapter")]
fn validate_burn_row_threshold_shape(
    output_rows: usize,
    threshold_count: usize,
) -> Result<(), TernaryZeroRegularizerError> {
    if threshold_count != 1 && threshold_count != output_rows {
        return Err(TernaryZeroRegularizerError::ThresholdCountMismatch {
            output_rows,
            threshold_count,
        });
    }

    Ok(())
}

#[cfg(feature = "burn-adapter")]
fn expand_burn_row_thresholds<B: BurnBackend>(
    thresholds: BurnFloatTensor<B, 1>,
    weight_shape: [usize; 2],
    threshold_count: usize,
) -> BurnFloatTensor<B, 2> {
    if threshold_count == 1 {
        thresholds
            .reshape([1, 1])
            .repeat_dim(0, weight_shape[0])
            .repeat_dim(1, weight_shape[1])
    } else {
        thresholds
            .reshape([weight_shape[0], 1])
            .repeat_dim(1, weight_shape[1])
    }
}

#[cfg(feature = "burn-adapter")]
fn capped_abs_with_thresholds<B: BurnBackend>(
    abs_weights: BurnFloatTensor<B, 2>,
    thresholds: BurnFloatTensor<B, 2>,
) -> BurnFloatTensor<B, 2> {
    let above_threshold = abs_weights.clone().greater(thresholds.clone());
    abs_weights.mask_where(above_threshold, thresholds)
}

#[cfg(feature = "burn-adapter")]
fn validate_burn_thresholds<B, const D: usize>(
    thresholds: &BurnFloatTensor<B, D>,
) -> Result<(), TernaryZeroRegularizerError>
where
    B: BurnBackend,
{
    let values = float_tensor_into_vec(thresholds.clone().detach())?;
    for (index, &value) in values.iter().enumerate() {
        if !value.is_finite() || value < 0.0 {
            return Err(TernaryZeroRegularizerError::InvalidThreshold { index, value });
        }
    }

    Ok(())
}

#[cfg(feature = "burn-adapter")]
fn validate_burn_loss<B, const D: usize>(
    loss: &BurnFloatTensor<B, D>,
) -> Result<(), TernaryZeroRegularizerError>
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

    fn finite_difference_gradient(
        weights: &[f32],
        threshold: f32,
        index: usize,
        epsilon: f32,
    ) -> f32 {
        let mut plus = weights.to_vec();
        plus[index] += epsilon;
        let mut minus = weights.to_vec();
        minus[index] -= epsilon;

        let plus_loss = ternary_zero_regularizer(&plus, threshold).unwrap();
        let minus_loss = ternary_zero_regularizer(&minus, threshold).unwrap();

        (plus_loss - minus_loss) / (2.0 * epsilon)
    }

    #[test]
    fn ternary_zero_regularizer_is_zero_for_all_zero_weights() {
        let loss = ternary_zero_regularizer(&[0.0, 0.0, 0.0, 0.0], 0.75).unwrap();

        assert_close(loss, 0.0, 1.0e-7);
    }

    #[test]
    fn ternary_zero_regularizer_penalty_increases_with_near_zero_nonzero_weights() {
        let one_near_zero = ternary_zero_regularizer(&[0.1, 0.0, 0.0, 0.0], 1.0).unwrap();
        let three_near_zero = ternary_zero_regularizer(&[0.1, -0.2, 0.3, 0.0], 1.0).unwrap();

        assert!(three_near_zero > one_near_zero);
        assert_close(one_near_zero, 0.025, 1.0e-7);
        assert_close(three_near_zero, 0.15, 1.0e-7);
    }

    #[test]
    fn ternary_zero_regularizer_caps_weights_above_threshold() {
        let loss = ternary_zero_regularizer(&[-2.0, -0.5, 0.25, 2.0], 0.75).unwrap();

        assert_close(loss, 0.5625, 1.0e-7);
    }

    #[test]
    fn ternary_zero_regularizer_supports_global_and_per_output_row_thresholds() {
        let matrix = TernaryZeroWeightMatrix::new(&[0.1, 0.4, 2.0, -0.5, 2.0, 0.2], 2, 3).unwrap();
        let per_row = ternary_zero_regularizer_for_matrix(matrix, &[0.25, 1.5]).unwrap();
        let global = ternary_zero_regularizer_for_matrix(matrix, &[0.25]).unwrap();

        assert_close(per_row, 0.466_666_67, 1.0e-7);
        assert_close(global, 0.216_666_67, 1.0e-7);
    }

    #[test]
    fn ternary_zero_regularizer_gradient_shrinks_subthreshold_weights_only() {
        let weights = [-0.5, -1.25, 0.25, 1.5];

        assert_close(
            finite_difference_gradient(&weights, 0.75, 0, 1.0e-3),
            -0.25,
            1.0e-3,
        );
        assert_close(
            finite_difference_gradient(&weights, 0.75, 1, 1.0e-3),
            0.0,
            1.0e-3,
        );
        assert_close(
            finite_difference_gradient(&weights, 0.75, 2, 1.0e-3),
            0.25,
            1.0e-3,
        );
        assert_close(
            finite_difference_gradient(&weights, 0.75, 3, 1.0e-3),
            0.0,
            1.0e-3,
        );
    }

    #[test]
    fn weighted_ternary_zero_regularizer_applies_explicit_lambda_zero() {
        assert_close(
            weighted_ternary_zero_regularizer(0.3125, 0.25).unwrap(),
            0.078_125,
            1.0e-9,
        );
        assert_close(
            weighted_ternary_zero_regularizer(0.3125, 0.0).unwrap(),
            0.0,
            1.0e-9,
        );

        assert_eq!(
            weighted_ternary_zero_regularizer(f32::NAN, 0.0).unwrap_err(),
            TernaryZeroRegularizerError::NonFiniteLoss { value: f32::NAN }
        );
        assert_eq!(
            weighted_ternary_zero_regularizer(f32::MAX, 2.0).unwrap_err(),
            TernaryZeroRegularizerError::NonFiniteLoss {
                value: f32::INFINITY,
            }
        );
        assert_eq!(
            weighted_ternary_zero_regularizer(-0.1, 1.0).unwrap_err(),
            TernaryZeroRegularizerError::NegativeLoss { value: -0.1 }
        );
    }

    #[test]
    fn ternary_zero_regularizer_validates_inputs() {
        assert_eq!(
            ternary_zero_regularizer(&[], 0.75).unwrap_err(),
            TernaryZeroRegularizerError::EmptyWeights
        );
        assert_eq!(
            ternary_zero_regularizer(&[1.0], f32::NAN).unwrap_err(),
            TernaryZeroRegularizerError::InvalidThreshold {
                index: 0,
                value: f32::NAN,
            }
        );
        assert_eq!(
            ternary_zero_regularizer(&[1.0], -0.1).unwrap_err(),
            TernaryZeroRegularizerError::InvalidThreshold {
                index: 0,
                value: -0.1,
            }
        );
        assert_eq!(
            TernaryZeroWeightMatrix::new(&[1.0], 0, 1).unwrap_err(),
            TernaryZeroRegularizerError::ZeroOutputRows
        );
        assert_eq!(
            TernaryZeroWeightMatrix::new(&[1.0], 1, 0).unwrap_err(),
            TernaryZeroRegularizerError::ZeroInputCols
        );
        assert_eq!(
            TernaryZeroWeightMatrix::new(&[1.0, 2.0], 2, 2).unwrap_err(),
            TernaryZeroRegularizerError::WeightCountMismatch {
                expected: 4,
                actual: 2,
            }
        );
        let matrix = TernaryZeroWeightMatrix::new(&[1.0, 2.0, 3.0], 3, 1).unwrap();
        assert_eq!(
            ternary_zero_regularizer_for_matrix(matrix, &[]).unwrap_err(),
            TernaryZeroRegularizerError::EmptyThresholds
        );
        assert_eq!(
            ternary_zero_regularizer_for_matrix(matrix, &[0.5, 0.75]).unwrap_err(),
            TernaryZeroRegularizerError::ThresholdCountMismatch {
                output_rows: 3,
                threshold_count: 2,
            }
        );
        assert_eq!(
            ternary_zero_regularizer(&[1.0, f32::INFINITY], 0.75).unwrap_err(),
            TernaryZeroRegularizerError::NonFiniteWeight {
                index: 1,
                value: f32::INFINITY,
            }
        );
        assert_eq!(
            weighted_ternary_zero_regularizer(1.0, -0.1).unwrap_err(),
            TernaryZeroRegularizerError::NegativeLambdaZero { lambda_zero: -0.1 }
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
        fn burn_ternary_zero_regularizer_matches_scalar_oracle() {
            type B = BurnNdArrayBackend;

            let device = BurnDevice::<B>::default();
            let values = vec![-2.0, -0.5, 0.25, 2.0];
            let weights = float_tensor_from_vec::<B, 2>(values.clone(), [2, 2], &device).unwrap();

            let burn_loss = burn_ternary_zero_regularizer(weights, 0.75).unwrap();
            let scalar_loss = ternary_zero_regularizer(&values, 0.75).unwrap();

            assert_close(
                float_tensor_into_vec(burn_loss).unwrap()[0],
                scalar_loss,
                1.0e-6,
            );
        }

        #[test]
        fn burn_ternary_zero_regularizer_preserves_large_finite_threshold_cap() {
            type B = BurnNdArrayBackend;

            let device = BurnDevice::<B>::default();
            let values = vec![1.0e8, -1.0e8, 0.25, -0.5];
            let weights = float_tensor_from_vec::<B, 2>(values.clone(), [2, 2], &device).unwrap();

            let burn_loss = burn_ternary_zero_regularizer(weights, 0.75).unwrap();
            let scalar_loss = ternary_zero_regularizer(&values, 0.75).unwrap();

            assert_close(
                float_tensor_into_vec(burn_loss).unwrap()[0],
                scalar_loss,
                1.0e-6,
            );
        }

        #[test]
        fn burn_ternary_zero_regularizer_gradient_shrinks_subthreshold_weights_only() {
            type B = BurnNdArrayAutodiffBackend;

            let device = BurnDevice::<B>::default();
            let weights = float_tensor_from_vec::<B, 1>(vec![-0.5, -1.25, 0.25, 1.5], [4], &device)
                .unwrap()
                .require_grad();

            let loss = burn_ternary_zero_regularizer(weights.clone(), 0.75).unwrap();
            let gradients = loss.backward();
            let grad = weights
                .grad(&gradients)
                .expect("weights should receive ternary zero regularizer gradients");

            assert_eq!(
                float_tensor_into_vec(grad).unwrap(),
                vec![-0.25, 0.0, 0.25, 0.0]
            );
        }

        #[test]
        fn burn_ternary_zero_regularizer_detaches_threshold_tensor() {
            type B = BurnNdArrayAutodiffBackend;

            let device = BurnDevice::<B>::default();
            let weights =
                float_tensor_from_vec::<B, 2>(vec![-0.5, -1.25, 0.25, 1.5], [2, 2], &device)
                    .unwrap()
                    .require_grad();
            let thresholds = float_tensor_from_vec::<B, 1>(vec![0.75, 0.75], [2], &device)
                .unwrap()
                .require_grad();

            let loss =
                burn_ternary_zero_regularizer_for_matrix(weights.clone(), thresholds.clone())
                    .unwrap();
            let gradients = loss.backward();
            let weight_grad = weights
                .grad(&gradients)
                .expect("weights should receive ternary zero regularizer gradients");

            assert_eq!(
                float_tensor_into_vec(weight_grad).unwrap(),
                vec![-0.25, 0.0, 0.25, 0.0]
            );
            assert!(thresholds.grad(&gradients).is_none());
        }

        #[test]
        fn burn_weighted_ternary_zero_regularizer_applies_explicit_lambda_zero() {
            type B = BurnNdArrayBackend;

            let device = BurnDevice::<B>::default();
            let raw_loss = float_tensor_from_vec::<B, 1>(vec![0.3125], [1], &device).unwrap();
            let weighted = burn_weighted_ternary_zero_regularizer(raw_loss, 0.25).unwrap();

            assert_close(
                float_tensor_into_vec(weighted).unwrap()[0],
                0.078_125,
                1.0e-7,
            );

            let disabled_raw = float_tensor_from_vec::<B, 1>(vec![0.3125], [1], &device).unwrap();
            let disabled = burn_weighted_ternary_zero_regularizer(disabled_raw, 0.0).unwrap();

            assert_eq!(float_tensor_into_vec(disabled).unwrap(), vec![0.0]);

            let invalid_disabled =
                float_tensor_from_vec::<B, 1>(vec![f32::NAN], [1], &device).unwrap();
            assert_eq!(
                burn_weighted_ternary_zero_regularizer(invalid_disabled, 0.0).unwrap_err(),
                TernaryZeroRegularizerError::NonFiniteLoss { value: f32::NAN }
            );
        }

        #[test]
        fn burn_ternary_zero_regularizer_supports_per_output_row_thresholds() {
            type B = BurnNdArrayBackend;

            let device = BurnDevice::<B>::default();
            let values = vec![1.0e8, -1.0e8, 0.25, -0.5];
            let weights = float_tensor_from_vec::<B, 2>(values.clone(), [2, 2], &device).unwrap();
            let thresholds = float_tensor_from_vec::<B, 1>(vec![0.75, 1.25], [2], &device).unwrap();
            let matrix = TernaryZeroWeightMatrix::new(&values, 2, 2).unwrap();
            let scalar_loss = ternary_zero_regularizer_for_matrix(matrix, &[0.75, 1.25]).unwrap();

            let burn_loss = burn_ternary_zero_regularizer_for_matrix(weights, thresholds).unwrap();

            assert_close(
                float_tensor_into_vec(burn_loss).unwrap()[0],
                scalar_loss,
                1.0e-6,
            );
        }

        #[test]
        fn burn_ternary_zero_regularizer_validates_threshold_tensor_contract() {
            type B = BurnNdArrayBackend;

            let device = BurnDevice::<B>::default();
            let weights =
                float_tensor_from_vec::<B, 2>(vec![0.1, 0.2, 0.3, 0.4], [2, 2], &device).unwrap();
            let wrong_count =
                float_tensor_from_vec::<B, 1>(vec![0.75, 0.75, 0.75], [3], &device).unwrap();
            let negative_threshold =
                float_tensor_from_vec::<B, 1>(vec![0.75, -0.1], [2], &device).unwrap();

            assert_eq!(
                burn_ternary_zero_regularizer_for_matrix(weights.clone(), wrong_count,)
                    .unwrap_err(),
                TernaryZeroRegularizerError::ThresholdCountMismatch {
                    output_rows: 2,
                    threshold_count: 3,
                }
            );
            assert_eq!(
                burn_ternary_zero_regularizer_for_matrix(weights, negative_threshold).unwrap_err(),
                TernaryZeroRegularizerError::InvalidThreshold {
                    index: 1,
                    value: -0.1,
                }
            );
        }

        #[test]
        fn burn_weighted_ternary_zero_regularizer_rejects_non_finite_result() {
            type B = BurnNdArrayBackend;

            let device = BurnDevice::<B>::default();
            let raw_loss = float_tensor_from_vec::<B, 1>(vec![f32::MAX], [1], &device).unwrap();

            assert_eq!(
                burn_weighted_ternary_zero_regularizer(raw_loss, 2.0).unwrap_err(),
                TernaryZeroRegularizerError::NonFiniteLoss {
                    value: f32::INFINITY,
                }
            );
        }
    }
}
