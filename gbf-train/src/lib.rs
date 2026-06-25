//! Training, evaluation, export orchestration, phased QAT, preflight, and shadow compilation.

#[cfg(all(feature = "qat", feature = "qat-ablation"))]
compile_error!("qat and qat-ablation are mutually exclusive");
#[cfg(all(feature = "s5-default", feature = "s5-no-log"))]
compile_error!("S5 feature mutex violated: s5-default and s5-no-log are mutually exclusive");

pub mod adapter;
pub mod ema;
#[cfg(feature = "burn-adapter")]
pub mod embeddings;
pub mod export;
pub mod export_visitor;
pub mod feedback;
pub mod logging;
pub mod loss;
pub mod phase;
pub mod preflight;
#[cfg(any(feature = "qat", feature = "qat-ablation"))]
pub mod qat;
pub mod runtime;
#[cfg(all(
    feature = "burn-adapter",
    any(feature = "qat", feature = "qat-ablation")
))]
pub mod scaffold;
pub mod scheduler;
pub mod selection;
#[cfg(all(
    feature = "burn-adapter",
    any(feature = "qat", feature = "qat-ablation")
))]
pub mod sequence;
pub mod shadow;
pub mod student;
pub mod teacher;
