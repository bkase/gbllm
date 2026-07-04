//! Bake-off-level ternary matvec shapes, weights, packings, and deterministic fixtures.
//!
//! Scope: this module gives the kernel bake-off (bd-rzq5n) a typed substrate for
//! measuring cycles/MAC across kernel strategies. The full `KernelSpec` contract
//! (calling conventions, compatibility ids, tile families) is owned by F-H1
//! (bd-2f32) and F-H2 (bd-3se9) and will subsume these types.
//!
//! Numeric convention: activations are raw `u8` bytes carrying an `i8` value at
//! zero point 128 (`x = u - 128`). A row's zero-point correction
//! `-128 * sum(weights_row)` is a build-time constant folded into the row's
//! accumulator seed, so kernels never sign-extend activations on-device.

use core::fmt;

/// Trit encoding used by both packings: `0 = zero`, `1 = +1`, `2 = -1`.
const TRIT_ZERO: u8 = 0;
const TRIT_PLUS: u8 = 1;
const TRIT_MINUS: u8 = 2;

/// Row-end sentinel byte in the base-81 dispatch stream.
pub const BASE81_ROW_END: u8 = 81;
/// Matrix-end sentinel byte in the base-81 dispatch stream.
pub const BASE81_MATRIX_END: u8 = 82;
/// Number of dispatch-stream symbols: 81 weight patterns plus two sentinels.
pub const BASE81_SYMBOL_COUNT: usize = 83;

/// Shape of a ternary matvec: `rows x fan_in` weights over `fan_in` activations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TernaryMatvecShape {
    fan_in: u16,
    rows: u16,
}

impl TernaryMatvecShape {
    /// Bake-off ceiling keeps every fixture inside one WRAM page span.
    pub const MAX_FAN_IN: u16 = 128;
    pub const MAX_ROWS: u16 = 128;

    pub fn new(fan_in: u16, rows: u16) -> Result<Self, TernaryKernelError> {
        if fan_in == 0 || !fan_in.is_multiple_of(4) || fan_in > Self::MAX_FAN_IN {
            return Err(TernaryKernelError::InvalidFanIn { fan_in });
        }
        if rows == 0 || rows > Self::MAX_ROWS {
            return Err(TernaryKernelError::InvalidRows { rows });
        }
        Ok(Self { fan_in, rows })
    }

    #[must_use]
    pub const fn fan_in(self) -> u16 {
        self.fan_in
    }

    #[must_use]
    pub const fn rows(self) -> u16 {
        self.rows
    }

    #[must_use]
    pub const fn mac_count(self) -> u32 {
        self.fan_in as u32 * self.rows as u32
    }
}

/// Validated `{-1, 0, +1}` weights in row-major order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TernaryWeights {
    shape: TernaryMatvecShape,
    values: Vec<i8>,
}

impl TernaryWeights {
    pub fn new(shape: TernaryMatvecShape, values: Vec<i8>) -> Result<Self, TernaryKernelError> {
        if values.len() != shape.mac_count() as usize {
            return Err(TernaryKernelError::WeightCountMismatch {
                expected: shape.mac_count(),
                actual: values.len(),
            });
        }
        if let Some(&bad) = values.iter().find(|value| !matches!(value, -1..=1)) {
            return Err(TernaryKernelError::NonTernaryWeight { value: bad });
        }
        Ok(Self { shape, values })
    }

    /// Deterministic pseudo-random weights (SplitMix64 stream).
    ///
    /// `zero_permille` selects the expected zero fraction in thousandths, so
    /// fixtures stay float-free and bit-reproducible.
    pub fn deterministic(
        shape: TernaryMatvecShape,
        seed: u64,
        zero_permille: u16,
    ) -> Result<Self, TernaryKernelError> {
        if zero_permille > 1000 {
            return Err(TernaryKernelError::InvalidZeroPermille { zero_permille });
        }
        let mut stream = SplitMix64::new(seed);
        let values = (0..shape.mac_count())
            .map(|_| {
                let draw = stream.next_u64();
                if (draw % 1000) < u64::from(zero_permille) {
                    0
                } else if draw & (1 << 32) == 0 {
                    1
                } else {
                    -1
                }
            })
            .collect();
        Self::new(shape, values)
    }

    #[must_use]
    pub const fn shape(&self) -> TernaryMatvecShape {
        self.shape
    }

    #[must_use]
    pub fn row(&self, row: u16) -> &[i8] {
        let fan_in = usize::from(self.shape.fan_in);
        let start = usize::from(row) * fan_in;
        &self.values[start..start + fan_in]
    }

    #[must_use]
    pub fn nonzero_count(&self) -> u32 {
        self.values.iter().filter(|&&value| value != 0).count() as u32
    }

    /// Zero-point correction folded into the row's accumulator seed:
    /// `-128 * sum(row)`. Always fits `i16` for bake-off shapes.
    #[must_use]
    pub fn row_zero_point_bias(&self, row: u16) -> i16 {
        let sum: i32 = self.row(row).iter().map(|&value| i32::from(value)).sum();
        i16::try_from(-128 * sum).expect("bias fits i16 for fan_in <= 128")
    }

    /// `Ternary2`-style packing: row-major, four weights per byte, weight `k`
    /// of each byte in bits `2k+1..=2k`, trit-encoded (`00` zero, `01` +1,
    /// `10` -1).
    #[must_use]
    pub fn pack_ternary2(&self) -> Vec<u8> {
        let mut packed = Vec::with_capacity(self.values.len() / 4);
        for chunk in self.values.chunks_exact(4) {
            let mut byte = 0_u8;
            for (position, &weight) in chunk.iter().enumerate() {
                byte |= trit_of(weight) << (2 * position);
            }
            packed.push(byte);
        }
        packed
    }

    /// Dispatch stream for the threaded-dispatch kernel.
    ///
    /// Layout: `bias_0 (i16 LE) | row_0 bytes | ROW_END | bias_1 | row_1 |
    /// ROW_END | ... | bias_last | row_last | MATRIX_END`. Each row byte is
    /// `t0 + 3*t1 + 9*t2 + 27*t3` over the byte's four trits, giving a dense
    /// `0..=80` dispatch index.
    #[must_use]
    pub fn base81_stream(&self) -> Vec<u8> {
        let rows = self.shape.rows;
        let bytes_per_row = usize::from(self.shape.fan_in) / 4;
        let mut stream = Vec::with_capacity(usize::from(rows) * (bytes_per_row + 3));
        for row in 0..rows {
            let bias = self.row_zero_point_bias(row) as u16;
            stream.extend_from_slice(&bias.to_le_bytes());
            for chunk in self.row(row).chunks_exact(4) {
                let index = chunk
                    .iter()
                    .rev()
                    .fold(0_u8, |acc, &weight| acc * 3 + trit_of(weight));
                stream.push(index);
            }
            stream.push(if row + 1 == rows {
                BASE81_MATRIX_END
            } else {
                BASE81_ROW_END
            });
        }
        stream
    }
}

/// Decode a base-81 dispatch index back into four trit weights.
#[must_use]
pub fn base81_pattern(index: u8) -> [i8; 4] {
    debug_assert!(index <= 80);
    let mut rest = index;
    let mut pattern = [0_i8; 4];
    for slot in &mut pattern {
        *slot = match rest % 3 {
            TRIT_PLUS => 1,
            TRIT_MINUS => -1,
            _ => 0,
        };
        rest /= 3;
    }
    pattern
}

const fn trit_of(weight: i8) -> u8 {
    match weight {
        1 => TRIT_PLUS,
        -1 => TRIT_MINUS,
        _ => TRIT_ZERO,
    }
}

/// Deterministic activation bytes (raw `u8`, zero point 128).
#[must_use]
pub fn deterministic_activations(len: u16, seed: u64) -> Vec<u8> {
    let mut stream = SplitMix64::new(seed);
    (0..len).map(|_| (stream.next_u64() & 0xFF) as u8).collect()
}

/// SplitMix64 (Steele et al.); fixed constants, documented for reproducibility.
struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TernaryKernelError {
    InvalidFanIn { fan_in: u16 },
    InvalidRows { rows: u16 },
    InvalidZeroPermille { zero_permille: u16 },
    WeightCountMismatch { expected: u32, actual: usize },
    NonTernaryWeight { value: i8 },
}

impl fmt::Display for TernaryKernelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFanIn { fan_in } => write!(
                f,
                "fan_in {fan_in} must be a nonzero multiple of 4 at most {}",
                TernaryMatvecShape::MAX_FAN_IN
            ),
            Self::InvalidRows { rows } => write!(
                f,
                "rows {rows} must be nonzero and at most {}",
                TernaryMatvecShape::MAX_ROWS
            ),
            Self::InvalidZeroPermille { zero_permille } => {
                write!(f, "zero_permille {zero_permille} must be at most 1000")
            }
            Self::WeightCountMismatch { expected, actual } => {
                write!(f, "expected {expected} weights, got {actual}")
            }
            Self::NonTernaryWeight { value } => {
                write!(f, "weight {value} outside {{-1, 0, 1}}")
            }
        }
    }
}

impl std::error::Error for TernaryKernelError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape(fan_in: u16, rows: u16) -> TernaryMatvecShape {
        TernaryMatvecShape::new(fan_in, rows).expect("valid shape")
    }

    #[test]
    fn shape_rejects_invalid_dimensions() {
        assert!(matches!(
            TernaryMatvecShape::new(30, 4),
            Err(TernaryKernelError::InvalidFanIn { fan_in: 30 })
        ));
        assert!(matches!(
            TernaryMatvecShape::new(0, 4),
            Err(TernaryKernelError::InvalidFanIn { .. })
        ));
        assert!(matches!(
            TernaryMatvecShape::new(4, 0),
            Err(TernaryKernelError::InvalidRows { .. })
        ));
    }

    #[test]
    fn weights_reject_out_of_range_values() {
        assert!(matches!(
            TernaryWeights::new(shape(4, 1), vec![0, 1, -1, 2]),
            Err(TernaryKernelError::NonTernaryWeight { value: 2 })
        ));
    }

    #[test]
    fn pack_ternary2_places_weight_k_in_bits_2k() {
        let weights = TernaryWeights::new(shape(4, 1), vec![1, 0, -1, 1]).expect("valid");
        // w0=+1 -> 0b01, w1=0 -> 0b00, w2=-1 -> 0b10 << 4, w3=+1 -> 0b01 << 6.
        assert_eq!(weights.pack_ternary2(), vec![0b0110_0001]);
    }

    #[test]
    fn base81_stream_round_trips_patterns_and_sentinels() {
        let values = vec![1, 0, -1, 1, -1, -1, 0, 0];
        let weights = TernaryWeights::new(shape(4, 2), values).expect("valid");
        let stream = weights.base81_stream();
        // Row 0: bias = -128 * (1) = -128 -> 0xFF80 LE.
        assert_eq!(&stream[0..2], &[0x80, 0xFF]);
        let index0 = stream[2];
        assert_eq!(base81_pattern(index0), [1, 0, -1, 1]);
        assert_eq!(stream[3], BASE81_ROW_END);
        // Row 1: bias = -128 * (-2) = 256 -> 0x0100 LE.
        assert_eq!(&stream[4..6], &[0x00, 0x01]);
        assert_eq!(base81_pattern(stream[6]), [-1, -1, 0, 0]);
        assert_eq!(stream[7], BASE81_MATRIX_END);
        // All weight indices stay below the sentinels.
        assert!(index0 <= 80 && stream[6] <= 80);
    }

    #[test]
    fn deterministic_weights_hit_requested_zero_fraction() {
        let weights = TernaryWeights::deterministic(shape(128, 64), 7, 400).expect("valid");
        let zeros = weights.shape().mac_count() - weights.nonzero_count();
        let permille = zeros * 1000 / weights.shape().mac_count();
        assert!(
            (350..=450).contains(&permille),
            "zero fraction {permille} permille far from 400"
        );
        // Deterministic across calls.
        let again = TernaryWeights::deterministic(shape(128, 64), 7, 400).expect("valid");
        assert_eq!(weights, again);
    }

    #[test]
    fn row_zero_point_bias_matches_negative_row_sum() {
        let weights = TernaryWeights::new(shape(4, 1), vec![1, 1, -1, 0]).expect("valid");
        assert_eq!(weights.row_zero_point_bias(0), -128);
    }
}
