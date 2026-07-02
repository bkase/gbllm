//! Temporal switch penalty loss.
//!
//! This module owns the differentiable Burn implementation of S7's raw
//! `lambda_switch` training term. Pair selection is delegated to
//! `gbf_model::loss::temporal_smoothness` so the model-level window and
//! boundary contract has one source of truth.

use std::error::Error;
use std::fmt;

use gbf_model::loss::temporal_smoothness::TemporalSmoothnessError;
#[cfg(feature = "burn-adapter")]
use gbf_model::loss::temporal_smoothness::{
    SmoothnessWindow, s7_temporal_smoothness_with_boundaries,
};

#[cfg(feature = "burn-adapter")]
use crate::adapter::burn::{
    BurnAdapterError, BurnBackend, BurnFloatTensor, float_tensor_into_vec, float_tensor_shape,
};

#[derive(Debug)]
pub enum TemporalSwitchLossError {
    InvalidRoutingProbabilityShape {
        shape: Vec<usize>,
    },
    MaskElementCountOverflow {
        batch_size: usize,
        seq_len: usize,
    },
    SequenceMaskLenMismatch {
        expected: usize,
        actual: usize,
    },
    BoundaryMaskLenMismatch {
        expected: usize,
        actual: usize,
    },
    PairSet(TemporalSmoothnessError),
    NonFiniteLoss {
        value: f32,
    },
    #[cfg(feature = "burn-adapter")]
    BurnAdapter(BurnAdapterError),
}

impl PartialEq for TemporalSwitchLossError {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (
                Self::InvalidRoutingProbabilityShape { shape: left },
                Self::InvalidRoutingProbabilityShape { shape: right },
            ) => left == right,
            (
                Self::MaskElementCountOverflow {
                    batch_size: left_batch,
                    seq_len: left_seq,
                },
                Self::MaskElementCountOverflow {
                    batch_size: right_batch,
                    seq_len: right_seq,
                },
            ) => left_batch == right_batch && left_seq == right_seq,
            (
                Self::SequenceMaskLenMismatch {
                    expected: left_expected,
                    actual: left_actual,
                },
                Self::SequenceMaskLenMismatch {
                    expected: right_expected,
                    actual: right_actual,
                },
            )
            | (
                Self::BoundaryMaskLenMismatch {
                    expected: left_expected,
                    actual: left_actual,
                },
                Self::BoundaryMaskLenMismatch {
                    expected: right_expected,
                    actual: right_actual,
                },
            ) => left_expected == right_expected && left_actual == right_actual,
            (Self::PairSet(left), Self::PairSet(right)) => left == right,
            (Self::NonFiniteLoss { value: left }, Self::NonFiniteLoss { value: right }) => {
                float_error_value_eq(*left, *right)
            }
            #[cfg(feature = "burn-adapter")]
            (Self::BurnAdapter(_), Self::BurnAdapter(_)) => false,
            _ => false,
        }
    }
}

impl fmt::Display for TemporalSwitchLossError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRoutingProbabilityShape { shape } => write!(
                f,
                "temporal switch routing_probs must have non-zero shape [batch, seq, layers, experts], got {shape:?}"
            ),
            Self::MaskElementCountOverflow {
                batch_size,
                seq_len,
            } => write!(
                f,
                "temporal switch mask element count overflowed for batch_size={batch_size}, seq_len={seq_len}"
            ),
            Self::SequenceMaskLenMismatch { expected, actual } => write!(
                f,
                "temporal switch sequence_mask length {actual} does not match expected {expected}"
            ),
            Self::BoundaryMaskLenMismatch { expected, actual } => write!(
                f,
                "temporal switch boundary mask length {actual} does not match expected {expected}"
            ),
            Self::PairSet(error) => write!(f, "{error}"),
            Self::NonFiniteLoss { value } => {
                write!(f, "temporal switch loss must be finite, got {value}")
            }
            #[cfg(feature = "burn-adapter")]
            Self::BurnAdapter(error) => write!(f, "{error}"),
        }
    }
}

impl Error for TemporalSwitchLossError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::PairSet(error) => Some(error),
            #[cfg(feature = "burn-adapter")]
            Self::BurnAdapter(error) => Some(error),
            Self::InvalidRoutingProbabilityShape { .. }
            | Self::MaskElementCountOverflow { .. }
            | Self::SequenceMaskLenMismatch { .. }
            | Self::BoundaryMaskLenMismatch { .. }
            | Self::NonFiniteLoss { .. } => None,
        }
    }
}

impl From<TemporalSmoothnessError> for TemporalSwitchLossError {
    fn from(error: TemporalSmoothnessError) -> Self {
        Self::PairSet(error)
    }
}

#[cfg(feature = "burn-adapter")]
impl From<BurnAdapterError> for TemporalSwitchLossError {
    fn from(error: BurnAdapterError) -> Self {
        Self::BurnAdapter(error)
    }
}

/// Differentiable S7 temporal switch penalty over routing probabilities.
///
/// `routing_probs` must have shape `[batch, seq, n_layers, n_experts]`.
/// `sequence_mask` is a flattened `[batch, seq]` boolean mask where `true`
/// means a valid token. `sequence_boundary_before`, when present, has the same
/// shape and marks packed-sequence boundaries before a token. The result is the
/// RFC §7.3 reduction: per-batch mean over layer/window pairs, then batch mean.
#[cfg(feature = "burn-adapter")]
pub fn burn_temporal_switch_loss<B>(
    routing_probs: BurnFloatTensor<B, 4>,
    sequence_mask: &[bool],
    sequence_boundary_before: Option<&[bool]>,
    smoothness_window: SmoothnessWindow,
) -> Result<BurnFloatTensor<B, 1>, TemporalSwitchLossError>
where
    B: BurnBackend,
{
    let shape = float_tensor_shape(&routing_probs);
    validate_routing_probability_shape(shape)?;
    let [batch_size, seq_len, n_layers, n_experts] = shape;
    let expected_mask_len = checked_mask_len(batch_size, seq_len).ok_or(
        TemporalSwitchLossError::MaskElementCountOverflow {
            batch_size,
            seq_len,
        },
    )?;
    validate_mask_len("sequence_mask", expected_mask_len, sequence_mask.len())?;
    if let Some(boundary) = sequence_boundary_before {
        validate_mask_len(
            "sequence_boundary_before",
            expected_mask_len,
            boundary.len(),
        )?;
    }
    let default_boundaries;
    let boundary_mask = if let Some(boundary) = sequence_boundary_before {
        boundary
    } else {
        default_boundaries = vec![false; expected_mask_len];
        default_boundaries.as_slice()
    };

    let zero = routing_probs.clone().sum() * 0.0;
    let mut total_loss = None;
    for batch in 0..batch_size {
        let start = batch * seq_len;
        let end = start + seq_len;
        let pairs = s7_temporal_smoothness_with_boundaries(
            &sequence_mask[start..end],
            &boundary_mask[start..end],
            smoothness_window,
        )?;
        let mut batch_total = None;
        for layer in 0..n_layers {
            for pair in pairs.iter().copied() {
                let current = routing_row(&routing_probs, batch, pair.t, layer, n_experts);
                let previous = routing_row(&routing_probs, batch, pair.u, layer, n_experts);
                let dot = (current * previous).sum();
                let penalty = dot.ones_like() - dot;
                batch_total = Some(match batch_total {
                    Some(acc) => acc + penalty,
                    None => penalty,
                });
            }
        }
        let batch_loss = if pairs.is_empty() {
            zero.clone()
        } else {
            batch_total.expect("non-empty pairs produce a batch total")
                / (pairs.len() * n_layers) as f32
        };
        total_loss = Some(match total_loss {
            Some(acc) => acc + batch_loss,
            None => batch_loss,
        });
    }

    let loss = total_loss.expect("validated batch size is non-zero") / batch_size as f32;
    validate_burn_loss(&loss)?;
    Ok(loss)
}

#[cfg(feature = "burn-adapter")]
fn routing_row<B>(
    routing_probs: &BurnFloatTensor<B, 4>,
    batch: usize,
    step: usize,
    layer: usize,
    n_experts: usize,
) -> BurnFloatTensor<B, 1>
where
    B: BurnBackend,
{
    routing_probs
        .clone()
        .slice([
            batch..batch + 1,
            step..step + 1,
            layer..layer + 1,
            0..n_experts,
        ])
        .reshape([n_experts])
}

#[cfg(feature = "burn-adapter")]
fn validate_routing_probability_shape(shape: [usize; 4]) -> Result<(), TemporalSwitchLossError> {
    if shape.contains(&0) {
        return Err(TemporalSwitchLossError::InvalidRoutingProbabilityShape {
            shape: shape.to_vec(),
        });
    }

    Ok(())
}

#[cfg(feature = "burn-adapter")]
fn checked_mask_len(batch_size: usize, seq_len: usize) -> Option<usize> {
    batch_size.checked_mul(seq_len)
}

#[cfg(feature = "burn-adapter")]
fn validate_mask_len(
    name: &'static str,
    expected: usize,
    actual: usize,
) -> Result<(), TemporalSwitchLossError> {
    if actual != expected {
        return match name {
            "sequence_mask" => {
                Err(TemporalSwitchLossError::SequenceMaskLenMismatch { expected, actual })
            }
            "sequence_boundary_before" => {
                Err(TemporalSwitchLossError::BoundaryMaskLenMismatch { expected, actual })
            }
            _ => unreachable!("unknown temporal switch mask name"),
        };
    }

    Ok(())
}

#[cfg(feature = "burn-adapter")]
fn validate_burn_loss<B>(loss: &BurnFloatTensor<B, 1>) -> Result<(), TemporalSwitchLossError>
where
    B: BurnBackend,
{
    let values = float_tensor_into_vec(loss.clone().detach())?;
    for value in values {
        if !value.is_finite() {
            return Err(TemporalSwitchLossError::NonFiniteLoss { value });
        }
    }

    Ok(())
}

fn float_error_value_eq(left: f32, right: f32) -> bool {
    left.to_bits() == right.to_bits() || (left.is_nan() && right.is_nan())
}

#[cfg(all(test, feature = "burn-adapter"))]
mod burn_tests {
    use super::*;
    use crate::adapter::burn::{
        BurnDevice, BurnFloatTensor, BurnNdArrayAutodiffBackend, float_tensor_from_vec,
        float_tensor_into_vec,
    };

    type B = BurnNdArrayAutodiffBackend;

    #[test]
    fn burn_temporal_switch_loss_matches_full_window_oracle() {
        let device = BurnDevice::<B>::default();
        let probs = routing_probs(
            vec![
                1.0, 0.0, //
                1.0, 0.0, //
                0.0, 1.0, //
                0.0, 1.0, //
            ],
            [1, 4, 1, 2],
            &device,
        );

        let loss =
            burn_temporal_switch_loss(probs, &[true; 4], None, SmoothnessWindow::new(2).unwrap())
                .unwrap();

        assert_close(float_tensor_into_vec(loss).unwrap()[0], 0.6, 1.0e-6);
    }

    #[test]
    fn burn_temporal_switch_loss_excludes_explicit_boundaries() {
        let device = BurnDevice::<B>::default();
        let probs = routing_probs(
            vec![
                1.0, 0.0, //
                1.0, 0.0, //
                0.0, 1.0, //
                0.0, 1.0, //
            ],
            [1, 4, 1, 2],
            &device,
        );

        let loss = burn_temporal_switch_loss(
            probs,
            &[true; 4],
            Some(&[false, false, true, false]),
            SmoothnessWindow::new(3).unwrap(),
        )
        .unwrap();

        assert_close(float_tensor_into_vec(loss).unwrap()[0], 0.0, 1.0e-6);
    }

    #[test]
    fn burn_temporal_switch_loss_flows_gradient_to_valid_pair_rows() {
        let device = BurnDevice::<B>::default();
        let probs = routing_probs(
            vec![
                0.9, 0.1, //
                0.8, 0.2, //
                0.2, 0.8, //
                0.1, 0.9, //
            ],
            [1, 4, 1, 2],
            &device,
        )
        .require_grad();

        let loss = burn_temporal_switch_loss(
            probs.clone(),
            &[true; 4],
            None,
            SmoothnessWindow::new(2).unwrap(),
        )
        .unwrap();
        let gradients = loss.backward();
        let grad = probs
            .grad(&gradients)
            .expect("routing probabilities should receive L_switch gradients");

        assert!(
            float_tensor_into_vec(grad)
                .unwrap()
                .iter()
                .any(|value| value.abs() > 0.0),
            "L_switch should produce non-zero gradients for valid pair rows"
        );
    }

    #[test]
    fn burn_temporal_switch_loss_rejects_invalid_masks() {
        let device = BurnDevice::<B>::default();
        let probs = routing_probs(vec![1.0, 0.0, 0.0, 1.0], [1, 2, 1, 2], &device);

        assert_eq!(
            burn_temporal_switch_loss(probs, &[true], None, SmoothnessWindow::new(2).unwrap())
                .unwrap_err(),
            TemporalSwitchLossError::SequenceMaskLenMismatch {
                expected: 2,
                actual: 1,
            }
        );
    }

    #[test]
    fn burn_temporal_switch_loss_rejects_window_one() {
        assert_eq!(
            SmoothnessWindow::new(1).unwrap_err(),
            TemporalSmoothnessError::SmoothnessWindowTooSmall { value: 1 },
        );
    }

    fn routing_probs(
        values: Vec<f32>,
        shape: [usize; 4],
        device: &BurnDevice<B>,
    ) -> BurnFloatTensor<B, 4> {
        float_tensor_from_vec(values, shape, device).unwrap()
    }

    fn assert_close(actual: f32, expected: f32, tolerance: f32) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "expected {actual} to be within {tolerance} of {expected}"
        );
    }
}
