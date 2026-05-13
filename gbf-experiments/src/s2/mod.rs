//! F-S2 QAT Survives experiment surface.
//!
//! The submodules are intentionally skeletal in F-S2.01. Later beads own their
//! executable contracts and artifact producers.

use std::sync::Once;

use crate::S2_LOG_TARGET;

/// Runtime build comparison helpers.
pub mod ablation;
/// Public API non-drift checks for S2 closure.
pub mod api_drift;
/// Command-line integration for S2 workflows.
pub mod cli;
/// Deterministic device profile delegates for S2.
pub mod device_profile;
/// Logit distillation helpers and diagnostics.
pub mod distill;
/// Reproducibility environment hashes for S2.
pub mod environment;
/// Test-only falsification harness compiled only with `falsify`.
#[cfg(feature = "falsify")]
pub mod falsify;
/// Ternary-vs-fp gap evaluation helpers.
pub mod gap;
/// LinearState gradient smoke fixtures.
pub mod linearstate_smoke;
/// Standard loss-term gradient-flow fixtures.
pub mod loss_grad_flow;
/// S2 loss composition structured logging helpers.
pub mod loss_logging;
/// S2 run manifest loading and validation.
pub mod manifest;
/// Measurement-oracle re-run helpers.
pub mod oracle_re_run;
/// Phase transition integration checks.
pub mod phase_transition_integ;
/// Deterministic report generation.
pub mod report;
/// Deterministic random-number stream helpers.
pub mod rng;
/// A-to-D training run orchestration.
pub mod run;
/// Canonical S2 schema and notation types.
pub mod schema;
/// Reset-context scoring wrappers for S2.
pub mod score;
/// Scalar S2 hypothesis verifiers.
pub mod verifiers;

static MODULE_LOADED: Once = Once::new();

/// Number of S2 public schema/notation types wired so far.
pub const TYPE_COUNT: u64 = 23;

/// Number of S2 schema modules wired by F-S2.01.
pub const SCHEMA_COUNT: u64 = 1;

/// Emit the S2 module-load tracing event once for this process.
pub fn ensure_module_loaded() {
    MODULE_LOADED.call_once(|| {
        tracing::info!(
            target: S2_LOG_TARGET,
            event_name = "s2::module_loaded",
            schema_count = SCHEMA_COUNT,
            type_count = TYPE_COUNT,
            s2_full_enabled = cfg!(feature = "s2-full"),
            s2_ablation_enabled = cfg!(feature = "s2-ablation"),
            "s2 module loaded"
        );
    });
}
