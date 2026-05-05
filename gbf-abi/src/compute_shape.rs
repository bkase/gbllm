//! Shared compute-shape types for F-B1 bringup surfaces.
//!
//! `SquareDim` deliberately lives in `gbf-abi`, not in `gbf-verify`, so
//! production crates can share the shape contract without depending on the
//! independent verifier crate.

use core::fmt;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(try_from = "u16", into = "u16")]
pub struct SquareDim {
    n: u16,
}

impl SquareDim {
    pub const TILE_MULTIPLE: u16 = 16;
    pub const MAX_F_B1: u16 = 128;

    pub const fn new(n: u16) -> Result<Self, MatmulShapeError> {
        if n == 0 {
            return Err(MatmulShapeError::Zero);
        }
        if !n.is_multiple_of(Self::TILE_MULTIPLE) {
            return Err(MatmulShapeError::NotMultipleOf16 { n });
        }
        if n > Self::MAX_F_B1 {
            return Err(MatmulShapeError::TooLarge {
                n,
                max: Self::MAX_F_B1,
            });
        }
        Ok(Self { n })
    }

    #[must_use]
    pub const fn n(self) -> u16 {
        self.n
    }

    #[must_use]
    pub const fn elem_count(self) -> usize {
        let n = self.n as usize;
        n * n
    }

    #[must_use]
    pub const fn operand_bytes(self) -> usize {
        self.elem_count()
    }

    #[must_use]
    pub const fn output_bytes_i32(self) -> usize {
        self.elem_count() * core::mem::size_of::<i32>()
    }

    #[must_use]
    pub const fn tiles_per_axis(self) -> u16 {
        self.n / Self::TILE_MULTIPLE
    }
}

impl From<SquareDim> for u16 {
    fn from(value: SquareDim) -> Self {
        value.n
    }
}

impl TryFrom<u16> for SquareDim {
    type Error = MatmulShapeError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl<'de> Deserialize<'de> for SquareDim {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let n = u16::deserialize(deserializer)?;
        Self::new(n).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum MatmulShapeError {
    Zero,
    NotMultipleOf16 { n: u16 },
    TooLarge { n: u16, max: u16 },
}

impl fmt::Display for MatmulShapeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Zero => f.write_str("matrix dimension must be greater than zero"),
            Self::NotMultipleOf16 { n } => {
                write!(f, "matrix dimension {n} is not a multiple of 16")
            }
            Self::TooLarge { n, max } => {
                write!(f, "matrix dimension {n} exceeds F-B1 maximum {max}")
            }
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for MatmulShapeError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn square_dim_accepts_valid() {
        assert_eq!(SquareDim::new(16).expect("valid").n(), 16);
        assert_eq!(SquareDim::new(128).expect("valid").n(), 128);
    }

    #[test]
    fn square_dim_rejects_zero() {
        assert_eq!(SquareDim::new(0), Err(MatmulShapeError::Zero));
    }

    #[test]
    fn square_dim_rejects_non_multiple_of_16() {
        assert_eq!(
            SquareDim::new(17),
            Err(MatmulShapeError::NotMultipleOf16 { n: 17 })
        );
    }

    #[test]
    fn square_dim_rejects_too_large() {
        assert_eq!(
            SquareDim::new(144),
            Err(MatmulShapeError::TooLarge { n: 144, max: 128 })
        );
    }

    #[test]
    fn elem_count_matches_n_squared() {
        let dim = SquareDim::new(96).expect("valid");
        assert_eq!(dim.elem_count(), 96 * 96);
        assert_eq!(dim.output_bytes_i32(), 96 * 96 * 4);
    }

    #[test]
    fn serde_json_shape_is_plain_number() {
        let dim = SquareDim::new(64).expect("valid");
        assert_eq!(serde_json::to_value(dim).expect("serializes"), 64);
        assert_eq!(
            serde_json::from_value::<SquareDim>(serde_json::json!(64)).expect("deserializes"),
            dim
        );
    }
}
