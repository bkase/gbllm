//! Burn-backed QAT adapters.

#[cfg(feature = "burn-adapter")]
pub mod activation;
#[cfg(feature = "burn-adapter")]
pub mod ternary;

#[cfg(feature = "burn-adapter")]
pub use activation::ActFakeQuantBurnQat;
#[cfg(feature = "burn-adapter")]
pub use ternary::{TernaryLinearBurnQat, TernaryLinearBurnQatError};
