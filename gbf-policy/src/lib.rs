//! Compile requests, objectives, deployment envelopes, runtime budgets, and repair policies.

pub mod budget;
pub mod calibration;
pub mod capabilities;
pub mod compile;
pub mod diagnostics;
pub mod envelope;
pub mod objective;
pub mod repair;
pub mod risk;

pub use budget::{BudgetSlotClass, RomBudgetSlot, RuntimeChromeBudget, RuntimeMemoryCapSection};
pub use calibration::{
    BootstrapCalibrationBundle, CalibrationBundle, CalibrationBundleSet, CalibrationLayer,
    CalibrationSetRef, MeasurementBlob, ValidityEnvelope, ValidityEnvelopeFuturePlaceholder,
};
pub use capabilities::{
    STAGE0_CLASS10_TARGET_CAPABILITY_OWNER, STAGE0_CLASS10_TARGET_CAPABILITY_RULES,
    STAGE0_COMPILER_FEATURE_REGISTRY_OWNER, STAGE0_COMPILER_SUPPORTED_FEATURES,
    Stage0Class10TargetCapabilities, Stage0Class10TargetCapabilityRule,
    TargetCapabilityRequirement, compiler_build_supports_feature,
    stage0_class10_lowering_profile_for_family, stage0_class10_target_family_for_profile_id,
};
pub use compile::{
    ArtifactRef, BRINGUP_COMPILE_PROFILE_ID, BRINGUP_COMPILE_PROFILE_TOML, CompileInvocationInputs,
    CompileKnobBounds, CompileKnobId, CompileKnobOverrides, CompileKnobPartialBounds,
    CompileKnobPartialValues, CompileKnobPath, CompileKnobProvenanceEntry, CompileKnobValues,
    CompileKnobs, CompileProfileSpec, CompileRequest, CompilerFeature, ConstraintOperation,
    ConstraintProvenance, ConstraintValue, DEFAULT_COMPILE_PROFILE_ID,
    DEFAULT_COMPILE_PROFILE_TOML, EffectiveConstraints, EvidenceRef, FieldPath, KnobLockSet,
    MonotoneKnob, ObservabilityMode, ObservationKnob, ObservationKnobBounds, OverlayKnob,
    OverlayKnobBounds, OverlayPromotion, PlacementKnob, PlacementKnobBounds, PlacementProfile,
    PolicyProvenance, PolicySource, ProbeCollectionLevel, RECOVERY_COMPILE_PROFILE_ID,
    RECOVERY_COMPILE_PROFILE_TOML, RangeKnob, RangeKnobBounds, ReductionPlanCeiling,
    ResolvedCompilePolicy, RomKernelDuplicationBias, RomKernelResidencyBias, RomWindowKnob,
    RomWindowKnobBounds, RuntimeMode, ScheduleKnob, ScheduleKnobBounds, ScheduleResourcePressure,
    ScheduleSliceCoarsening, ScheduleTileSearch, SelectorPath, SequenceSemanticsRef, SramKnob,
    SramKnobBounds, SramPageAggression, StorageKnob, StorageKnobBounds, StorageMaterialization,
    TRACE_COMPILE_PROFILE_ID, TRACE_COMPILE_PROFILE_TOML, TraceBudget, TraceDropPolicy,
    canonical_compile_profile_specs, canonical_default_bounds_fixture,
    compile_profile_defaults_hash, load_compile_profile_spec,
};
pub use diagnostics::{
    ArtifactFeature, ArtifactSchemaVersion, BudgetFailure, CompatibilityAdapterId, ComponentId,
    DataLoweringProfileId, DiagnosticSeverity, GoldenVectorId, KnobValueDescriptor, LineageId,
    LoweringShardId, LoweringShardRef, ManifestInvariant, ObjectiveRejection,
    PlacementInfeasibilityReason, ReductionSiteId, RiskQuantileField, ServiceLevelField,
    SidecarKind, SwitchProjectionSource, TargetIncompatibilityReason, TraceProbeId, ValidationCode,
    ValidationDetail, ValidationDiagnostic, ValidationOrigin, budget_failure_diagnostic,
    budget_failure_diagnostic_with_provenance, budget_failure_diagnostics,
    budget_failure_diagnostics_with_provenance, budget_failure_matches_diagnostic,
    budget_failure_validation_code,
};
pub use objective::{CompileObjective, RiskPolicy, ServiceLevelObjective};
pub use repair::{RepairPolicy, RepairPolicyProfile, RepairProposalId};
pub use risk::{CalibrationConfidenceClass, CalibrationConfidenceRequirement};
