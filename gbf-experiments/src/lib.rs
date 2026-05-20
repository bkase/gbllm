#![deny(missing_docs)]
//! F-S1 First Pulse, F-S2 QAT Survives, F-S3 TinyStories, and F-S4 Gutenberg
//! promotion experiment orchestration.
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
//! use gbf_experiments::{S1_LOG_TARGET, S2_LOG_TARGET, S3_LOG_TARGET, S4_LOG_TARGET, S5_LOG_TARGET};
//!
//! assert_eq!(S1_LOG_TARGET, "gbf_experiments::s1");
//! assert_eq!(S2_LOG_TARGET, "gbf_experiments::s2");
//! assert_eq!(S3_LOG_TARGET, "gbf_experiments::s3");
//! assert_eq!(S4_LOG_TARGET, "gbf_experiments::s4");
//! assert_eq!(S5_LOG_TARGET, "gbf_experiments::s5");
//! ```

#[cfg(all(feature = "phase-a", feature = "ablation"))]
compile_error!(
    "gbf-experiments features phase-a and ablation are mutually exclusive because qat and qat-ablation are mutually exclusive"
);
#[cfg(all(feature = "s2-full", feature = "s2-ablation"))]
compile_error!("S2 feature mutex violated");
#[cfg(all(feature = "s3-oracle-real", feature = "s3-oracle-fallback"))]
compile_error!("s3-oracle-real and s3-oracle-fallback are mutually exclusive");
#[cfg(all(feature = "s4-full", feature = "s4-falsify"))]
compile_error!("S4 feature mutex violated: s4-full and s4-falsify are mutually exclusive");
#[cfg(all(feature = "s5-default", feature = "s5-no-log"))]
compile_error!("S5 feature mutex violated: s5-default and s5-no-log are mutually exclusive");
#[cfg(not(any(
    feature = "phase-a",
    feature = "ablation",
    feature = "s2-full",
    feature = "s2-ablation",
    feature = "s3",
    feature = "s4",
    feature = "s5-default",
    feature = "s5-no-log"
)))]
compile_error!("gbf-experiments requires at least one S1, S2, S3, S4, or S5 experiment feature");

const _: () = {
    let falsifier_count = (cfg!(feature = "s5-falsify-1") as usize)
        + cfg!(feature = "s5-falsify-2") as usize
        + cfg!(feature = "s5-falsify-3") as usize
        + cfg!(feature = "s5-falsify-4") as usize
        + cfg!(feature = "s5-falsify-5") as usize
        + cfg!(feature = "s5-falsify-6") as usize
        + cfg!(feature = "s5-falsify-7") as usize
        + cfg!(feature = "s5-falsify-8") as usize
        + cfg!(feature = "s5-falsify-9") as usize
        + cfg!(feature = "s5-falsify-10") as usize
        + cfg!(feature = "s5-falsify-11") as usize
        + cfg!(feature = "s5-falsify-12") as usize
        + cfg!(feature = "s5-falsify-13") as usize
        + cfg!(feature = "s5-falsify-14") as usize
        + cfg!(feature = "s5-falsify-15") as usize;
    assert!(
        falsifier_count <= 1,
        "S5 falsifier feature mutex violated: enable at most one s5-falsify-N feature"
    );
};

/// Tracing target shared by S1 experiment logging.
pub const S1_LOG_TARGET: &str = "gbf_experiments::s1";

/// Tracing target shared by S2 experiment logging.
pub const S2_LOG_TARGET: &str = "gbf_experiments::s2";

/// Tracing target shared by S3 experiment logging.
pub const S3_LOG_TARGET: &str = "gbf_experiments::s3";

/// Tracing target shared by S4 experiment logging.
pub const S4_LOG_TARGET: &str = "gbf_experiments::s4";

/// Tracing target shared by S5 experiment logging.
pub const S5_LOG_TARGET: &str = "gbf_experiments::s5";

/// First Pulse experiment modules.
#[cfg(any(
    feature = "phase-a",
    feature = "ablation",
    feature = "s2-full",
    feature = "s2-ablation",
    feature = "s3-phase-d",
    feature = "falsify"
))]
pub mod s1;

/// QAT Survives experiment modules.
#[cfg(any(
    feature = "phase-a",
    feature = "ablation",
    feature = "s2-full",
    feature = "s2-ablation",
    feature = "s3-phase-d",
    feature = "falsify"
))]
pub mod s2;

/// TinyStories success experiment modules.
#[cfg(feature = "s3")]
pub mod s3;

/// Gutenberg promotion experiment modules.
#[cfg(feature = "s4")]
pub mod s4;

/// Pick-and-Fit experiment modules.
#[cfg(any(feature = "s5-default", feature = "s5-no-log"))]
pub mod s5;
