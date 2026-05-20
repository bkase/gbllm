//! F-S4 Gutenberg promotion experiment surface.
//!
//! The module started as behavior-free F-S4.02 scaffolding. Later F-S4 beads
//! add narrow typed contracts here while leaving concrete artifact producers
//! and full runner wiring to their owning beads.

use std::sync::Once;

pub use crate::S4_LOG_TARGET;

/// Gutenberg Kneser-Ney baseline surface.
pub mod baseline;
/// Command-line integration surface for S4 workflows.
pub mod cli;
/// Cross-corpus contamination check surface.
pub mod contamination;
/// Model-free corpus-oracle fixture surface.
pub mod corpus_oracle;
/// TinyStories-to-Gutenberg corpus progression artifact surface.
pub mod corpus_progression;
/// Gutenberg corpus-quality artifact surface.
pub mod corpus_quality;
/// S4 deterministic device profile re-exports.
pub mod device_profile;
/// Test-only S4 falsification surface compiled only with `s4-falsify`.
#[cfg(feature = "s4-falsify")]
pub mod falsify;
/// Network-permitted Gutenberg fixture harvest surface.
pub mod harvest;
/// Gutenberg manifest validation wrapper.
pub mod manifest;
/// Gutenberg oracle-agreement surface.
pub mod oracle;
/// S3-to-Gutenberg promotion gate surface.
pub mod promote;
/// Deterministic S4 report surface.
pub mod report;
/// Deterministic RNG stream declarations for Gutenberg continuation training.
pub mod rng;
/// Gutenberg continuation training run surface.
pub mod run;
/// Gutenberg run-log/checkpoint/FP-reference artifact surface.
pub mod run_artifacts;
/// Canonical S4 schema and notation types.
pub mod schema;
/// Gutenberg scoring surface.
pub mod score;
/// H1-H7 verifier and outcome-dispatch surface.
pub mod verifier;

static MODULE_LOADED: Once = Once::new();

/// Number of always-on public S4 module surfaces registered by F-S4.02.
///
/// Feature-gated test-only modules, such as `s4::falsify`, are intentionally
/// excluded so this value stays stable for the base `s4` schema feature.
pub const MODULE_SURFACE_COUNT: u64 = 18;

/// Number of named public S4 skeleton contracts pinned by the base `s4` feature.
///
/// F-S4.12 adds the train config, per-seed run contract, and Phase-D
/// continuation schedule descriptor to the original five closure skeletons.
/// F-S4.13 adds run-log, checkpoint, and FP-reference artifact contracts.
/// F-S4.17 adds the corpus progression schedule artifact contract.
/// F-S4.19 adds the H1-H7 verifier contract.
pub const TYPE_COUNT: u64 = 13;

/// Emit the S4 module-load tracing event once for this process.
pub fn ensure_module_loaded() {
    MODULE_LOADED.call_once(|| {
        tracing::info!(
            target: S4_LOG_TARGET,
            event_name = "s4::module_loaded",
            module_surface_count = MODULE_SURFACE_COUNT,
            type_count = TYPE_COUNT,
            s4_enabled = cfg!(feature = "s4"),
            s4_full_enabled = cfg!(feature = "s4-full"),
            s4_falsify_enabled = cfg!(feature = "s4-falsify"),
            "s4 module loaded"
        );
    });
}
