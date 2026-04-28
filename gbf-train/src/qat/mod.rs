//! Burn-backed QAT adapters.

#[cfg(feature = "burn-adapter")]
pub mod activation;
#[cfg(feature = "burn-adapter")]
pub mod expert;
#[cfg(feature = "burn-adapter")]
pub mod norm;
#[cfg(feature = "burn-adapter")]
pub mod router;
#[cfg(feature = "burn-adapter")]
pub mod ternary;

#[cfg(feature = "burn-adapter")]
pub use activation::{ActFakeQuantBurnQat, ActFakeQuantBurnQatError};
#[cfg(feature = "burn-adapter")]
pub use expert::{ExpertBlockBurnQat, ExpertBlockBurnQatError};
#[cfg(feature = "burn-adapter")]
pub use norm::{NormApproxBurnPlan, NormApproxBurnQat, NormApproxBurnQatError};
#[cfg(feature = "burn-adapter")]
pub use router::{RouterBurnAuxLosses, RouterBurnForwardOutput, Top1RouterBurnQat};
#[cfg(feature = "burn-adapter")]
pub use ternary::{TernaryLinearBurnQat, TernaryLinearBurnQatError, ThresholdScheduleProgress};
