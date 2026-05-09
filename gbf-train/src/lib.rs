//! Training, evaluation, export orchestration, phased QAT, preflight, and shadow compilation.

#[cfg(all(feature = "qat", feature = "qat-ablation"))]
compile_error!("qat and qat-ablation are mutually exclusive");

pub mod adapter;
#[cfg(feature = "burn-adapter")]
pub mod embeddings;
pub mod logging;
pub mod loss;
pub mod phase;
pub mod preflight;
#[cfg(feature = "qat")]
pub mod qat;
pub mod scheduler;
pub mod shadow;
pub mod teacher;
