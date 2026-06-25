//! S7 MoE matched-bytes experiment helpers.

pub use crate::S7_LOG_TARGET;

/// Dense matched-bytes baseline and pin emission.
pub mod baseline_match;

/// H8 Burn ExpertBlockQat gradient smoke producer.
#[cfg(feature = "s7-burn-grad-smoke")]
pub mod burn_grad_smoke;

/// Command-line integration surface for S7 replay gates.
pub mod cli;

/// S7 lambda-switch collapse-sweep helpers.
pub mod collapse_sweep;

/// S7 derived dense-vs-MoE comparison artifact helpers.
pub mod comparison;

/// S7 closure-packet adapter for the Rust closure validator.
pub mod closure_packet;

/// H10 one-token emulator comparison helpers.
pub mod emulator_one_token;

/// S7 Pareto frontier artifact derivation.
pub mod frontier;

/// S7 falsification-suite helpers.
#[cfg(feature = "falsify")]
pub mod falsify;

/// S7 Pareto verdict helpers.
pub mod pareto;

/// S7 matched-bytes parity helpers.
pub mod parity;

/// S7 determinism and fixture replay helpers.
pub mod replay;

/// S7 closure-report validation helpers.
pub mod report;

/// S7 RouterRng counting and recomputability helpers.
pub mod rng_counting;

/// S7 train-run helper surface.
pub mod run;

/// S7 closure support artifact landing helpers.
pub mod support_artifacts;

/// S7 outcome algebra helpers.
pub mod outcome;

/// S7 telemetry schemas.
pub mod schema;

/// Deterministic tiny-fixture S7 smoke harness.
pub mod smoke;

/// S7 train-run state helpers.
pub mod state;
