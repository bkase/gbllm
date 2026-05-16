//! F-S3 TinyStories success experiment surface.
//!
//! The module is intentionally inert in B7. Later F-S3 beads own artifact
//! producers, verifier logic, and runner wiring.

use std::sync::Once;

pub use crate::S3_LOG_TARGET;

/// S3 exported model artifact helpers.
pub mod artifact;
/// TinyStories 5-gram baseline helpers.
pub mod baseline;
/// Reference model bundle helpers.
pub mod bundle;
/// Charset v1 normalization helpers.
pub mod charset;
/// Command-line integration for S3 workflows.
pub mod cli;
/// S3 conformance envelope helpers.
pub mod conformance;
/// Workload/train contamination checks.
pub mod contamination;
/// Reproducibility environment hashes for S3.
pub mod environment;
/// Test-only falsification harness compiled only with `falsify`.
#[cfg(feature = "falsify")]
pub mod falsify;
/// S3 run manifest loading and validation.
pub mod manifest;
/// S3 oracle backend integration helpers.
pub mod oracle;
/// Measurement-oracle re-run helpers.
pub mod oracle_re_run;
/// S3 preregistration pin loader and logging.
pub mod preregistration;
/// Deterministic report generation.
#[cfg(any(
    feature = "phase-a",
    feature = "ablation",
    feature = "s2-full",
    feature = "s2-ablation",
    feature = "falsify"
))]
pub mod report;
/// Deterministic random-number stream helpers.
pub mod rng;
/// S3 TinyStories run orchestration.
pub mod run;
/// Canonical S3 schema and notation types.
pub mod schema;
/// Reset-context scoring wrappers for S3.
pub mod score;
/// S3 workload manifest helpers.
pub mod workload;

// Keep the crate-root guard for early Cargo/build-script diagnostics and this
// module guard to preserve the B2 module-local contract for downstream S3 users.
#[cfg(all(feature = "s3-oracle-real", feature = "s3-oracle-fallback"))]
compile_error!("s3-oracle-real and s3-oracle-fallback are mutually exclusive");

// Cargo does not expose a direct `cfg(test)` signal to normal library builds
// compiled for integration tests, so debug builds remain available for the
// workspace falsify matrix while release-like builds reject this test-only flag.
#[cfg(all(feature = "falsify", not(any(test, debug_assertions))))]
compile_error!("the unified `falsify` feature must only be enabled in test builds");

static MODULE_LOADED: Once = Once::new();

/// Number of S3 schema modules wired by B7/B12/B13.
pub const SCHEMA_COUNT: u64 = 3;

/// Number of S3 public taxonomy/schema types wired by B7/B12/B13.
pub const TYPE_COUNT: u64 = 15;

/// Emit the S3 module-load tracing event once for this process.
pub fn ensure_module_loaded() {
    MODULE_LOADED.call_once(|| {
        tracing::info!(
            target: S3_LOG_TARGET,
            event_name = "s3::module_loaded",
            schema_count = SCHEMA_COUNT,
            type_count = TYPE_COUNT,
            s3_enabled = cfg!(feature = "s3"),
            s3_phase_d_enabled = cfg!(feature = "s3-phase-d"),
            s3_oracle_real_enabled = cfg!(feature = "s3-oracle-real"),
            s3_oracle_fallback_enabled = cfg!(feature = "s3-oracle-fallback"),
            "s3 module loaded"
        );
    });
}
