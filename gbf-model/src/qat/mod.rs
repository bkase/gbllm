//! Quantization-aware training semantics owned by the deployable model contract.
//!
//! These modules currently expose backend-independent scalar/reference cores.
//! Burn tensor wrappers and autodiff STE adapters belong in `gbf-train`.

pub mod activation;
pub mod expert;
pub mod export;
pub mod norm;
pub mod router;
pub mod ternary;

pub use activation::{
    ActFakeQuant, ActFakeQuantError, ActivationFakeQuantSpec, ActivationForwardMode,
    ActivationQuantFormat, ActivationRange, ActivationRangeMode, ActivationRangeModeKind, EmaDecay,
};
pub use expert::{
    ClippedActivation, ClippedActivationKind, DenseBranchProjection, ExpertBatchOutput,
    ExpertBlockQat, ExpertBlockQatError, ExpertForwardOptions, ExpertMlpConfig,
    ExpertMlpConfigEvent, ExpertMlpConfigEventCode, ExpertMlpConfigEventLevel, ExpertMlpVariant,
    ExpertQat, ExpertQatForwardMode, SharedDenseBranch,
};
pub use export::{
    ExportVisitor, ExportVisitorError, ExportedQatArtifact, QatModuleRef, VisitedModule,
    VisitedModuleKind,
};
pub use norm::{
    AffineParams, LutSpec, NormApproxError, NormApproxPlan, NormApproxQat, NormClip,
    NormExportData, TileRmsSpec,
};
pub use router::{
    RouterAuxLossWeights, RouterAuxLosses, RouterForwardOptions, RouterForwardOutput, RouterShape,
    RouterTrainMode, Top1RouterQat, Top1RouterQatError, default_router_rank,
};
pub use ternary::{
    MatrixShape, Q8_8Scale, TernaryLinearExport, TernaryLinearQat, TernaryLinearQatError,
    TernaryThreshold, TernaryValue, project_ternary_values,
};
