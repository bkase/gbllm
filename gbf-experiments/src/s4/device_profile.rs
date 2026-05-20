//! S4 deterministic device profile surface.

#[cfg(any(
    feature = "phase-a",
    feature = "ablation",
    feature = "s2-full",
    feature = "s2-ablation",
    feature = "s3-phase-d",
    feature = "falsify"
))]
pub use crate::s1::device_profile::S1CpuDeterministic;

#[cfg(not(any(
    feature = "phase-a",
    feature = "ablation",
    feature = "s2-full",
    feature = "s2-ablation",
    feature = "s3-phase-d",
    feature = "falsify"
)))]
/// Skeleton for the inherited S1 deterministic CPU profile in S4-only builds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct S1CpuDeterministic;
