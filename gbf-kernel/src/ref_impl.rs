//! Exact host-side reference for the ternary matvec bake-off kernels.
//!
//! This is the semantic anchor every generated kernel is diffed against; it is
//! deliberately the obvious double loop with `i32` accumulation and no packing.

use crate::spec::{TernaryKernelError, TernaryWeights};
use core::fmt;

/// Activation zero point shared by kernels and fixtures (`x = u - 128`).
pub const ACTIVATION_ZERO_POINT: i32 = 128;

/// `y[r] = sum_i w[r][i] * (u[i] - 128)` with exact `i32` accumulation.
pub fn ternary_matvec_i32(
    weights: &TernaryWeights,
    activations: &[u8],
) -> Result<Vec<i32>, RefKernelError> {
    let shape = weights.shape();
    if activations.len() != usize::from(shape.fan_in()) {
        return Err(RefKernelError::ActivationCountMismatch {
            expected: shape.fan_in(),
            actual: activations.len(),
        });
    }
    Ok((0..shape.rows())
        .map(|row| {
            weights
                .row(row)
                .iter()
                .zip(activations)
                .map(|(&weight, &raw)| i32::from(weight) * (i32::from(raw) - ACTIVATION_ZERO_POINT))
                .sum()
        })
        .collect())
}

/// Reference output as the little-endian `i16` bytes the kernels store.
pub fn expected_output_bytes_le(
    weights: &TernaryWeights,
    activations: &[u8],
) -> Result<Vec<u8>, RefKernelError> {
    let outputs = ternary_matvec_i32(weights, activations)?;
    let mut bytes = Vec::with_capacity(outputs.len() * 2);
    for value in outputs {
        let value =
            i16::try_from(value).map_err(|_| RefKernelError::OutputOutOfI16Range { value })?;
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    Ok(bytes)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefKernelError {
    ActivationCountMismatch { expected: u16, actual: usize },
    OutputOutOfI16Range { value: i32 },
    Kernel(TernaryKernelError),
}

impl fmt::Display for RefKernelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ActivationCountMismatch { expected, actual } => {
                write!(f, "expected {expected} activation bytes, got {actual}")
            }
            Self::OutputOutOfI16Range { value } => {
                write!(f, "reference output {value} does not fit i16")
            }
            Self::Kernel(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for RefKernelError {}

impl From<TernaryKernelError> for RefKernelError {
    fn from(error: TernaryKernelError) -> Self {
        Self::Kernel(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::TernaryMatvecShape;

    #[test]
    fn matvec_matches_hand_computation() {
        let shape = TernaryMatvecShape::new(4, 2).expect("valid shape");
        let weights =
            TernaryWeights::new(shape, vec![1, -1, 0, 1, -1, -1, 1, 0]).expect("valid weights");
        // Raw activations 130, 126, 200, 128 -> signed 2, -2, 72, 0.
        let activations = [130_u8, 126, 200, 128];
        let outputs = ternary_matvec_i32(&weights, &activations).expect("shapes agree");
        assert_eq!(outputs, vec![2 + 2 + 0 + 0, -2 + 2 + 72 + 0]);
    }

    #[test]
    fn expected_bytes_are_little_endian_i16() {
        let shape = TernaryMatvecShape::new(4, 1).expect("valid shape");
        let weights = TernaryWeights::new(shape, vec![-1, 0, 0, 0]).expect("valid weights");
        // Raw 0 -> signed -128, so y = 128 -> 0x0080 LE.
        let bytes = expected_output_bytes_le(&weights, &[0, 128, 128, 128]).expect("fits i16");
        assert_eq!(bytes, vec![0x80, 0x00]);
    }

    #[test]
    fn activation_length_is_validated() {
        let shape = TernaryMatvecShape::new(4, 1).expect("valid shape");
        let weights = TernaryWeights::new(shape, vec![0; 4]).expect("valid weights");
        assert!(matches!(
            ternary_matvec_i32(&weights, &[0, 1, 2]),
            Err(RefKernelError::ActivationCountMismatch {
                expected: 4,
                actual: 3
            })
        ));
    }
}
