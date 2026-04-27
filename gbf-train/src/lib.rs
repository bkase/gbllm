//! Training, evaluation, export orchestration, phased QAT, preflight, and shadow compilation.

pub mod adapter;
#[cfg(feature = "burn-adapter")]
pub mod embeddings;
pub mod logging;
pub mod loss;
pub mod phase;
pub mod preflight;
pub mod qat;
pub mod scheduler;
pub mod shadow;
