//! S1 First Pulse experiment surface.
//!
//! The submodules here are intentionally skeletal in F-S1.01. Later beads own
//! their concrete contracts and acceptance tests.

/// Ablation build comparison plumbing.
pub mod ablation;
/// Baseline model and score comparison helpers.
pub mod baseline;
/// Build identity selected by S1 Cargo features.
pub mod build_metadata;
/// Command-line integration for S1 workflows.
pub mod cli;
/// Deterministic device profile enforcement.
pub mod device_profile;
/// Structured tracing contracts and subscriber setup.
pub mod logging;
/// Manifest loading and validation delegates.
pub mod manifest;
/// Negative-test and falsification harness plumbing.
pub mod neg_test;
/// Oracle integration for S1 verdicts.
pub mod oracle;
/// Deterministic report generation.
pub mod report;
/// Deterministic random-number generation primitives.
pub mod rng;
/// Phase A run orchestration.
pub mod run;
/// Canonical S1 schema types.
pub mod schema;
/// Reset-context scoring primitives.
pub mod score;
