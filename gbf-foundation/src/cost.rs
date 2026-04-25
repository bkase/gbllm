//! Byte-count cost wrappers.

use std::fmt;
use std::ops::{Add, AddAssign, Sub, SubAssign};

use serde::{Deserialize, Serialize};

/// A non-negative byte cost used for storage and budget accounting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ByteCost(u64);

impl ByteCost {
    pub const ZERO: Self = Self(0);

    #[must_use]
    pub const fn new(bytes: u64) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn as_u64(self) -> u64 {
        self.0
    }

    #[must_use]
    pub fn checked_add(self, rhs: Self) -> Option<Self> {
        self.0.checked_add(rhs.0).map(Self)
    }

    #[must_use]
    pub fn checked_sub(self, rhs: Self) -> Option<Self> {
        self.0.checked_sub(rhs.0).map(Self)
    }

    #[must_use]
    pub const fn saturating_add(self, rhs: Self) -> Self {
        Self(self.0.saturating_add(rhs.0))
    }

    #[must_use]
    pub const fn saturating_sub(self, rhs: Self) -> Self {
        Self(self.0.saturating_sub(rhs.0))
    }
}

impl From<u64> for ByteCost {
    fn from(bytes: u64) -> Self {
        Self::new(bytes)
    }
}

impl From<ByteCost> for u64 {
    fn from(cost: ByteCost) -> Self {
        cost.as_u64()
    }
}

impl Add for ByteCost {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        self.saturating_add(rhs)
    }
}

impl AddAssign for ByteCost {
    fn add_assign(&mut self, rhs: Self) {
        *self = *self + rhs;
    }
}

impl Sub for ByteCost {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        self.saturating_sub(rhs)
    }
}

impl SubAssign for ByteCost {
    fn sub_assign(&mut self, rhs: Self) {
        *self = *self - rhs;
    }
}

impl fmt::Display for ByteCost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} bytes", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_cost_adds_subtracts_and_orders() {
        let rom = ByteCost::new(16);
        let overlay = ByteCost::new(5);

        assert_eq!((rom + overlay).as_u64(), 21);
        assert_eq!((rom - overlay).as_u64(), 11);
        assert!(rom > overlay);
        assert_eq!(rom.to_string(), "16 bytes");
    }

    #[test]
    fn byte_cost_add_sub_operators_saturate() {
        assert_eq!(
            (ByteCost::new(u64::MAX) + ByteCost::new(1)).as_u64(),
            u64::MAX
        );
        assert_eq!((ByteCost::new(0) - ByteCost::new(1)).as_u64(), 0);
    }

    #[test]
    fn byte_cost_round_trips_through_serde() {
        let encoded = serde_json::to_string(&ByteCost::new(42)).expect("cost serializes");
        let decoded: ByteCost = serde_json::from_str(&encoded).expect("cost deserializes");

        assert_eq!(decoded, ByteCost::new(42));
    }
}
