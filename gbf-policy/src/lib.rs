//! Compile requests, objectives, deployment envelopes, runtime budgets, and repair policies.

pub mod budget;
pub mod compile;
pub mod envelope;
pub mod objective;
pub mod repair;
pub mod risk;

pub use budget::{BudgetSlotClass, RomBudgetSlot, RuntimeChromeBudget, RuntimeMemoryCapSection};
pub use compile::{
    CompileKnobBounds, CompileKnobId, CompileKnobOverrides, CompileKnobPartialBounds,
    CompileKnobPartialValues, CompileKnobPath, CompileKnobProvenanceEntry, CompileKnobValues,
    CompileKnobs, CompileProfileSpec, CompileRequest, CompilerFeature, ConstraintOperation,
    ConstraintProvenance, ConstraintValue, EffectiveConstraints, EvidenceRef, FieldPath,
    KnobLockSet, MonotoneKnob, ObservabilityMode, ObservationKnob, ObservationKnobBounds,
    OverlayKnob, OverlayKnobBounds, OverlayPromotion, PlacementKnob, PlacementKnobBounds,
    PlacementProfile, PolicyProvenance, PolicySource, ProbeCollectionLevel, RangeKnob,
    RangeKnobBounds, ReductionPlanCeiling, ResolvedCompilePolicy, RomKernelDuplicationBias,
    RomKernelResidencyBias, RomWindowKnob, RomWindowKnobBounds, RuntimeMode, ScheduleKnob,
    ScheduleKnobBounds, ScheduleResourcePressure, ScheduleSliceCoarsening, ScheduleTileSearch,
    SelectorPath, SequenceSemanticsRef, SramKnob, SramKnobBounds, SramPageAggression, StorageKnob,
    StorageKnobBounds, StorageMaterialization, TraceBudget, TraceDropPolicy,
    canonical_default_bounds_fixture,
};
pub use objective::{CompileObjective, RiskPolicy, ServiceLevelObjective};
pub use repair::{RepairPolicy, RepairPolicyProfile, RepairProposalId};
pub use risk::{CalibrationConfidenceClass, CalibrationConfidenceRequirement};
