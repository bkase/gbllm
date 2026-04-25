//! Stable adapter boundary around training-framework dependencies.
//!
//! `gbf-model` owns deployable numeric semantics and must not import Burn
//! directly. Training code that needs Burn APIs should go through this module so
//! version/API drift is contained inside `gbf-train`.

#[cfg(feature = "burn-adapter")]
pub mod burn;

/// Exact Burn version pinned by the workspace.
pub const BURN_VERSION: &str = "0.21.0-pre.3";

/// Exact Cargo requirement expected for the Burn workspace dependency.
pub const BURN_VERSION_REQUIREMENT: &str = "=0.21.0-pre.3";

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT_CARGO: &str = include_str!("../../../Cargo.toml");
    const TRAIN_CARGO: &str = include_str!("../../Cargo.toml");
    const MODEL_CARGO: &str = include_str!("../../../gbf-model/Cargo.toml");

    #[test]
    fn burn_version_is_pinned_for_manual_drift_review() {
        assert!(
            ROOT_CARGO.contains(&format!(
                "burn = {{ version = \"{BURN_VERSION_REQUIREMENT}\""
            )),
            "workspace Burn dependency must stay exactly pinned"
        );
        assert!(
            TRAIN_CARGO.contains("burn = { workspace = true, optional = true }"),
            "gbf-train must consume the pinned workspace Burn dependency"
        );
    }

    #[test]
    fn gbf_model_does_not_depend_on_burn_directly() {
        let has_burn_dependency = MODEL_CARGO.lines().any(|line| {
            let line = line.trim_start();
            line.starts_with("burn ") || line.starts_with("burn=")
        });

        assert!(
            !has_burn_dependency,
            "gbf-model must not depend on Burn directly; use gbf-train::adapter instead"
        );
    }

    #[test]
    fn adapter_constants_match_pinned_requirement() {
        assert_eq!(BURN_VERSION_REQUIREMENT, format!("={BURN_VERSION}"));
    }
}
