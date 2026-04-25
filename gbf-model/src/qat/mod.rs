//! Quantization-aware training modules owned by the deployable model contract.

pub mod activation;
pub mod expert;
pub mod export;
pub mod norm;
pub mod router;
pub mod ternary;

pub use activation::ActFakeQuant;
pub use expert::ExpertBlockQat;
pub use export::ExportVisitor;
pub use norm::NormApproxQat;
pub use router::Top1RouterQat;
pub use ternary::{
    MatrixShape, Q8_8Scale, TernaryLinearExport, TernaryLinearQat, TernaryLinearQatError,
    TernarySteBackend, TernarySteLinear, TernaryThreshold, TernaryValue,
};
