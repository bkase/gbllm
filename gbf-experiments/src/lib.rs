#![deny(missing_docs)]
//! F-S1 First Pulse and F-S2 QAT Survives experiment orchestration.
//!
//! This crate is the home for slice-specific experiment code. It depends on the
//! substrate crates through workspace-pinned dependencies and keeps experiment
//! wiring out of `gbf-train`.
//!
//! ```
//! use gbf_experiments::s1::{
//!     ablation as _, baseline as _, cli as _, device_profile as _, manifest as _, neg_test as _,
//!     oracle as _, report as _, rng as _, run as _, schema as _, score as _,
//! };
//! use gbf_experiments::{S1_LOG_TARGET, S2_LOG_TARGET};
//!
//! assert_eq!(S1_LOG_TARGET, "gbf_experiments::s1");
//! assert_eq!(S2_LOG_TARGET, "gbf_experiments::s2");
//! ```

#[cfg(all(feature = "phase-a", feature = "ablation"))]
compile_error!(
    "gbf-experiments features phase-a and ablation are mutually exclusive because qat and qat-ablation are mutually exclusive"
);
#[cfg(all(feature = "s2-full", feature = "s2-ablation"))]
compile_error!("S2 feature mutex violated");
#[cfg(not(any(
    feature = "phase-a",
    feature = "ablation",
    feature = "s2-full",
    feature = "s2-ablation"
)))]
compile_error!("gbf-experiments requires at least one S1 or S2 experiment feature");

/// Tracing target shared by S1 experiment logging.
pub const S1_LOG_TARGET: &str = "gbf_experiments::s1";

/// Tracing target shared by S2 experiment logging.
pub const S2_LOG_TARGET: &str = "gbf_experiments::s2";

/// First Pulse experiment modules.
pub mod s1;

/// QAT Survives experiment modules.
pub mod s2;
