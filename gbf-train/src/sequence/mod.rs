//! Burn-backed sequence-state training adapters.

pub mod linear_state;

pub use linear_state::{LinearStateBurnQat, LinearStateBurnQatError, LinearStateBurnRun};
