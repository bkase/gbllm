#![deny(missing_docs)]
//! F-S1 First Pulse experiment orchestration.
//!
//! This crate is the home for S1-specific experiment code. It depends on the
//! substrate crates through workspace-pinned dependencies and keeps experiment
//! wiring out of `gbf-train`.
//!
//! ```
//! use gbf_experiments::s1::{
//!     ablation as _, baseline as _, cli as _, device_profile as _, manifest as _, neg_test as _,
//!     oracle as _, report as _, rng as _, run as _, schema as _, score as _,
//! };
//! use gbf_experiments::S1_LOG_TARGET;
//!
//! assert_eq!(S1_LOG_TARGET, "gbf_experiments::s1");
//! ```

#[cfg(all(feature = "phase-a", feature = "ablation"))]
compile_error!(
    "gbf-experiments features phase-a and ablation are mutually exclusive because qat and qat-ablation are mutually exclusive"
);
#[cfg(not(any(feature = "phase-a", feature = "ablation")))]
compile_error!("gbf-experiments requires exactly one of phase-a or ablation");

/// Tracing target shared by S1 experiment logging.
pub const S1_LOG_TARGET: &str = "gbf_experiments::s1";

/// First Pulse experiment modules.
pub mod s1;
