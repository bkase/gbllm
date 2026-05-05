//! Independent slow `i8 x i8 -> i32` matmul reference for F-B1.

use std::fmt;

use gbf_abi::compute_shape::SquareDim;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MatrixI8<'a> {
    dim: SquareDim,
    data: &'a [i8],
}

impl<'a> MatrixI8<'a> {
    pub fn new(dim: SquareDim, data: &'a [i8]) -> Result<Self, MatmulReferenceError> {
        validate_len(dim, data.len())?;
        Ok(Self { dim, data })
    }

    #[must_use]
    pub const fn dim(self) -> SquareDim {
        self.dim
    }

    #[must_use]
    pub const fn data(self) -> &'a [i8] {
        self.data
    }

    #[must_use]
    pub fn get(self, row: usize, col: usize) -> i8 {
        self.data[row * usize::from(self.dim.n()) + col]
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatrixI32 {
    dim: SquareDim,
    data: Vec<i32>,
}

impl MatrixI32 {
    pub fn new(dim: SquareDim, data: Vec<i32>) -> Result<Self, MatmulReferenceError> {
        validate_len(dim, data.len())?;
        Ok(Self { dim, data })
    }

    #[must_use]
    pub const fn dim(&self) -> SquareDim {
        self.dim
    }

    #[must_use]
    pub fn data(&self) -> &[i32] {
        &self.data
    }

    #[must_use]
    pub fn into_data(self) -> Vec<i32> {
        self.data
    }

    #[must_use]
    pub fn to_le_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.data.len() * 4);
        for value in &self.data {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum MatmulReferenceError {
    BadShape { expected: usize, found: usize },
    DimMismatch { a: u16, b: u16 },
}

impl fmt::Display for MatmulReferenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadShape { expected, found } => {
                write!(f, "slice length {found} does not match n*n = {expected}")
            }
            Self::DimMismatch { a, b } => write!(f, "matrix dimensions differ: {a} vs {b}"),
        }
    }
}

impl std::error::Error for MatmulReferenceError {}

pub fn matmul_reference_i8(
    a: MatrixI8<'_>,
    b: MatrixI8<'_>,
) -> Result<MatrixI32, MatmulReferenceError> {
    if a.dim != b.dim {
        return Err(MatmulReferenceError::DimMismatch {
            a: a.dim.n(),
            b: b.dim.n(),
        });
    }
    let dim = a.dim;
    let n = usize::from(dim.n());
    let mut out = vec![0_i32; dim.elem_count()];
    for i in 0..n {
        for j in 0..n {
            let mut acc = 0_i32;
            for k in 0..n {
                acc += i32::from(a.get(i, k)) * i32::from(b.get(k, j));
            }
            out[i * n + j] = acc;
        }
    }
    MatrixI32::new(dim, out)
}

#[must_use]
pub fn deterministic_matrix_a(dim: SquareDim) -> Vec<i8> {
    deterministic_matrix(dim, |i, j| 73 * i + 37 * j + 19)
}

#[must_use]
pub fn deterministic_matrix_b(dim: SquareDim) -> Vec<i8> {
    deterministic_matrix(dim, |i, j| 29 * i + 91 * j + 11)
}

#[must_use]
pub fn deterministic_operand_bytes_a(dim: SquareDim) -> Vec<u8> {
    deterministic_matrix_a(dim)
        .into_iter()
        .map(|value| value as u8)
        .collect()
}

#[must_use]
pub fn deterministic_operand_bytes_b(dim: SquareDim) -> Vec<u8> {
    deterministic_matrix_b(dim)
        .into_iter()
        .map(|value| value as u8)
        .collect()
}

#[must_use]
pub fn quarter_square_table_reference_i16() -> [i16; QUARTER_SQUARE_LEN] {
    let mut table = [0_i16; QUARTER_SQUARE_LEN];
    let mut x = QUARTER_SQUARE_MIN;
    while x <= QUARTER_SQUARE_MAX {
        let square = i32::from(x) * i32::from(x);
        table[quarter_square_index(x)] = (square / 4) as i16;
        x += 1;
    }
    table
}

#[must_use]
pub fn quarter_square_mul_reference_i8(a: i8, b: i8) -> i32 {
    let table = quarter_square_table_reference_i16();
    let sum = i16::from(a) + i16::from(b);
    let diff = i16::from(a) - i16::from(b);
    i32::from(table[quarter_square_index(sum)]) - i32::from(table[quarter_square_index(diff)])
}

pub const QUARTER_SQUARE_MIN: i16 = -256;
pub const QUARTER_SQUARE_MAX: i16 = 255;
pub const QUARTER_SQUARE_LEN: usize = 512;

#[must_use]
pub fn quarter_square_index(x: i16) -> usize {
    usize::try_from(i32::from(x) - i32::from(QUARTER_SQUARE_MIN))
        .expect("quarter-square index is non-negative")
}

fn deterministic_matrix(dim: SquareDim, f: impl Fn(i32, i32) -> i32) -> Vec<i8> {
    let n = usize::from(dim.n());
    let mut out = Vec::with_capacity(dim.elem_count());
    for i in 0..n {
        for j in 0..n {
            let raw = f(i as i32, j as i32).rem_euclid(256) - 128;
            out.push(i8::try_from(raw).expect("affine fixture stays in i8 range"));
        }
    }
    out
}

fn validate_len(dim: SquareDim, found: usize) -> Result<(), MatmulReferenceError> {
    let expected = dim.elem_count();
    if found == expected {
        Ok(())
    } else {
        Err(MatmulReferenceError::BadShape { expected, found })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matmul_reference_i8_known_fixture() {
        let dim = SquareDim::new(16).expect("valid");
        let a = deterministic_matrix_a(dim);
        let b = deterministic_matrix_b(dim);
        let out = matmul_reference_i8(
            MatrixI8::new(dim, &a).expect("shape"),
            MatrixI8::new(dim, &b).expect("shape"),
        )
        .expect("reference");
        assert_eq!(out.dim(), dim);
        assert_eq!(out.data()[0], 4_472);
        assert_eq!(out.data()[1], 26_320);
        assert_eq!(out.data()[15], 27_040);
        assert_eq!(out.data()[255], -56);
    }

    #[test]
    fn matmul_reference_rejects_bad_shape() {
        let dim = SquareDim::new(16).expect("valid");
        let data = vec![0_i8; dim.elem_count() - 1];
        assert_eq!(
            MatrixI8::new(dim, &data),
            Err(MatmulReferenceError::BadShape {
                expected: 256,
                found: 255,
            })
        );
    }

    #[test]
    fn fixture_generation_is_stable() {
        let dim = SquareDim::new(32).expect("valid");
        assert_eq!(deterministic_matrix_a(dim), deterministic_matrix_a(dim));
        assert_eq!(deterministic_operand_bytes_b(dim)[0], 139);
        assert_eq!(deterministic_operand_bytes_b(dim)[31], 144);
    }

    #[test]
    fn output_bytes_are_little_endian_i32() {
        let dim = SquareDim::new(16).expect("valid");
        let matrix = MatrixI32::new(dim, vec![0x0102_0304; dim.elem_count()]).expect("shape");
        assert_eq!(&matrix.to_le_bytes()[..4], &[0x04, 0x03, 0x02, 0x01]);
    }

    #[test]
    fn reference_is_deterministic_across_runs() {
        let dim = SquareDim::new(32).expect("valid");
        let a = deterministic_matrix_a(dim);
        let b = deterministic_matrix_b(dim);
        let first = matmul_reference_i8(
            MatrixI8::new(dim, &a).expect("shape"),
            MatrixI8::new(dim, &b).expect("shape"),
        )
        .expect("reference");
        let second = matmul_reference_i8(
            MatrixI8::new(dim, &a).expect("shape"),
            MatrixI8::new(dim, &b).expect("shape"),
        )
        .expect("reference");
        assert_eq!(first, second);
    }

    #[test]
    fn quarter_square_reference_has_canonical_shape() {
        let table = quarter_square_table_reference_i16();
        assert_eq!(table.len(), QUARTER_SQUARE_LEN);
        assert_eq!(table[0], 16_384);
        assert_eq!(table[quarter_square_index(0)], 0);
        assert!(table.iter().all(|value| *value >= 0));
    }

    #[test]
    fn quarter_square_reference_matches_direct_multiply() {
        for a in i8::MIN..=i8::MAX {
            for b in i8::MIN..=i8::MAX {
                assert_eq!(
                    quarter_square_mul_reference_i8(a, b),
                    i32::from(a) * i32::from(b)
                );
            }
        }
    }
}
