//! S1 build identity selected by Cargo features.

use serde::Serialize;

/// S1 build kind for the active Cargo feature set.
#[cfg(all(feature = "phase-a", not(feature = "ablation")))]
pub const BUILD_KIND: &str = "phase_a";

/// S1 build kind for the active Cargo feature set.
#[cfg(all(feature = "ablation", not(feature = "phase-a")))]
pub const BUILD_KIND: &str = "ablation";

/// S1 build kind when compiling S2 without an active S1 build identity.
#[cfg(not(any(feature = "phase-a", feature = "ablation")))]
pub const BUILD_KIND: &str = "s1_unselected";

/// Whether QAT code paths are active for the selected S1 build.
#[cfg(all(feature = "phase-a", not(feature = "ablation")))]
pub const QAT_ACTIVE: bool = true;

/// Whether QAT code paths are active for the selected S1 build.
#[cfg(all(feature = "ablation", not(feature = "phase-a")))]
pub const QAT_ACTIVE: bool = false;

/// Whether QAT code paths are active when no S1 build identity is selected.
#[cfg(not(any(feature = "phase-a", feature = "ablation")))]
pub const QAT_ACTIVE: bool = false;

/// Whether the test-only falsification substitutes are compiled in.
pub const FALSIFY_ENABLED: bool = cfg!(feature = "falsify");

/// Build metadata embedded in S1 runtime artifacts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct BuildMetadata {
    /// Feature-selected checkpoint build identity.
    pub build_kind: &'static str,
    /// Whether the selected S1 build has QAT paths active.
    pub qat_active: bool,
    /// Workspace Git revision used to compile `gbf-experiments`.
    ///
    /// S1 crates currently share one workspace checkout, so this is the same
    /// workspace `HEAD` value as `gbf_train_sha` rather than a per-crate diff.
    pub gbf_experiments_sha: &'static str,
    /// Workspace Git revision used to compile `gbf-train`.
    ///
    /// S1 crates currently share one workspace checkout, so this is the same
    /// workspace `HEAD` value as `gbf_experiments_sha` rather than a per-crate
    /// diff.
    pub gbf_train_sha: &'static str,
}

/// Return S1 build metadata for the active binary.
#[must_use]
pub const fn build_metadata() -> BuildMetadata {
    BuildMetadata {
        build_kind: BUILD_KIND,
        qat_active: QAT_ACTIVE,
        gbf_experiments_sha: env!("GBF_EXPERIMENTS_GIT_SHA"),
        gbf_train_sha: env!("GBF_TRAIN_GIT_SHA"),
    }
}
