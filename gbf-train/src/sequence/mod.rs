//! Burn-backed sequence-state training adapters.

pub mod bounded_kv;
pub mod linear_state;

pub use bounded_kv::{BoundedKvBurnQat, BoundedKvBurnQatError, BoundedKvBurnRun};
pub use linear_state::{LinearStateBurnQat, LinearStateBurnQatError, LinearStateBurnRun};
