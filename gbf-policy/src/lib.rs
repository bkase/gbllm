//! Compile requests, objectives, deployment envelopes, runtime budgets, and repair policies.

#[cfg(test)]
pub(crate) static TRACE_CAPTURE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub mod budget;
pub mod calibration;
mod canonical;
pub mod capabilities;
pub mod compile;
pub mod cost;
pub mod diagnostics;
pub mod envelope;
pub mod metrics;
pub mod model_profile;
pub mod objective;
pub mod probe;
pub mod repair;
pub mod risk;
pub mod trace_event_layout;

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
    AliasClassId, ArtifactRef, BRINGUP_COMPILE_PROFILE_ID, BRINGUP_COMPILE_PROFILE_TOML,
    ByteBudget, COMPILE_PROFILE_SPEC_UNSUPPORTED_SCHEMA_CODE, COMPILE_PROFILE_SPEC_VERSION,
    CompileInvocationInputs, CompileKnobBounds, CompileKnobId, CompileKnobOverrides,
    CompileKnobPartialBounds, CompileKnobPartialValues, CompileKnobPath,
    CompileKnobProvenanceEntry, CompileKnobValues, CompileKnobs, CompileProfileSpec,
    CompileProfileSpecLoadError, CompileRequest, CompilerFeature, ConstraintDelta,
    ConstraintOperation, ConstraintProvenance, ConstraintValue, CycleBudget,
    DEFAULT_COMPILE_PROFILE_ID, DEFAULT_COMPILE_PROFILE_TOML, DeltaRejection, EffectiveConstraints,
    EvidenceRef, FieldPath, InitialKnobsResolveError, KernelResidency, KernelSelector,
    KernelSpecId, KnobDelta, KnobLockSet, LayerId, MonotoneKnob, ObservabilityMode,
    ObservationKnob, ObservationKnobBounds, ObservationProfileCaps, OverlayKnob, OverlayKnobBounds,
    OverlayPromotion, PROFILE_SPEC_V1_REJECTED_EVENT, PROFILE_SPEC_V2_INVARIANT_FAILURE_EVENT,
    PROFILE_SPEC_V2_LOADED_EVENT, PlacementKnob, PlacementKnobBounds, PlacementProfile,
    PolicyProvenance, PolicySource, PressureLimit, ProbeCollectionLevel,
    RECOVERY_COMPILE_PROFILE_ID, RECOVERY_COMPILE_PROFILE_TOML, RangeCapsSpec, RangeKnob,
    RangeKnobBounds, RecomputePurityFacts, ReductionPlanCeiling, ReductionSelector,
    RenormStrategyPolicy, ResolvedCompilePolicy, ResourcePressureThresholdResolution,
    ResourcePressureThresholds, ResourcePressureUpdate, RomKernelDuplicationBias,
    RomKernelResidencyBias, RomWindowKnob, RomWindowKnobBounds, RuntimeMode, ScheduleKnob,
    ScheduleKnobBounds, ScheduleResourcePressure, ScheduleSliceCoarsening, ScheduleTileSearch,
    SectionId, SelectorPath, SequenceSemanticsRef, SliceClass, SramKnob, SramKnobBounds,
    SramPageAggression, StorageKnob, StorageKnobBounds, StorageMaterialization,
    TRACE_COMPILE_PROFILE_ID, TRACE_COMPILE_PROFILE_TOML, TileCandidateClass, TileSelector,
    TraceBudget, TraceDropPolicy, ValueId, ValueSelector, canonical_compile_profile_specs,
    canonical_default_bounds_fixture, check_delta_admissible,
    check_delta_admissible_with_recompute_purity, compile_profile_defaults_hash,
    f_b16_profile_lock_set, f_b16_refinement_knob_ids, load_compile_profile_spec,
    resolve_initial_knobs_from_profile_spec, resolve_repair_policy_from_profile_spec,
    resolve_resource_pressure_thresholds,
};
pub use cost::{
    CostBucketTotals, CostEstimate, EstimatedCostDelta, EvidenceClass, FallbackReason,
    ModeCostBreakdown, ModeCostBreakdownEntry, ModeEstimatedCost, ObjectiveAxis,
    ObjectiveSatisfaction, ObjectiveSatisfactionMatrix, Quantile, SatisfactionEntry,
    SatisfactionKey, ScheduleCostBreakdown, ScheduleCostIdentity, ScheduleCostReport,
    SliceCostBreakdown, StaleCalibrationField, UncertaintyEnvelope,
};
pub use diagnostics::{
    ArenaPlanDiagnosticCode, ArenaPlanDiagnosticProvenance, ArtifactFeature, ArtifactSchemaVersion,
    BudgetFailure, CompatibilityAdapterId, ComponentId, DataLoweringProfileId, DiagnosticSeverity,
    GoldenVectorId, KnobValueDescriptor, LineageId, LoweringShardId, LoweringShardRef,
    ManifestInvariant, ObjectiveRejection, OverlayPlanDiagnosticCode,
    OverlayPlanDiagnosticProvenance, PlacementInfeasibilityReason, ReductionSiteId,
    ResourceStateDiagnosticCode, ResourceStateDiagnosticProvenance, RiskQuantileField,
    RomWindowPlanDiagnosticCode, RomWindowPlanDiagnosticProvenance, ScheduleCostDiagnosticCode,
    ScheduleCostDiagnosticProvenance, ServiceLevelField, SidecarKind, SramPagePlanDiagnosticCode,
    SramPagePlanDiagnosticProvenance, StaticFitInterpretation, StoragePlanDiagnosticCode,
    StoragePlanDiagnosticProvenance, SwitchProjectionSource, TargetIncompatibilityReason,
    TraceProbeId, ValidationCode, ValidationDetail, ValidationDiagnostic, ValidationOrigin,
    budget_failure_diagnostic, budget_failure_diagnostic_with_provenance,
    budget_failure_diagnostics, budget_failure_diagnostics_with_provenance,
    budget_failure_matches_diagnostic, budget_failure_validation_code,
};
pub use metrics::{
    METRIC_REGISTRY_LOADED_EVENT, METRIC_REGISTRY_VERSION, MetricAggregation, MetricId,
    MetricIdError, MetricRegistryEntry, MetricRegistryError, MetricRegistrySnapshot, MetricSource,
    emit_metric_registry_loaded, load_metric_registry_v1, metric_registry_canonical_json_bytes,
    metric_registry_hash, metric_registry_v1,
};
pub use objective::{CompileObjective, RiskPolicy, ServiceLevelObjective};
pub use probe::{
    EffectClass, InferOpTag, PROBE_REGISTRY_LOADED_EVENT, PROBE_REGISTRY_VERSION,
    ProbeImportanceClass, ProbeRegistryEntry, ProbeRegistryError, ProbeRegistrySnapshot,
    ProbeSourceSelector, ProbeTiming, TraceFrequencyBound, ValueRole, emit_probe_registry_loaded,
    load_probe_registry_v1, probe_registry_canonical_json_bytes, probe_registry_hash,
    probe_registry_v1, validate_probe_registry_event_shapes,
};
pub use repair::{RepairPolicy, RepairPolicyProfile, RepairProposalId, RepairReason};
pub use risk::{CalibrationConfidenceClass, CalibrationConfidenceRequirement};
pub use trace_event_layout::{
    ABI_TRACE_EVENT_PAYLOAD_BYTES, TRACE_EVENT_LAYOUT_REGISTRY_LOADED_EVENT,
    TRACE_EVENT_LAYOUT_REGISTRY_VERSION, TraceEventLayoutEntry, TraceEventLayoutRegistryError,
    TraceEventLayoutRegistrySnapshot, TraceEventPayloadLayout, TraceEventShape,
    TraceEventShapeError, TraceEventTupleSpecId, emit_trace_event_layout_registry_loaded,
    load_trace_event_layout_registry_v1, trace_event_layout_registry_canonical_json_bytes,
    trace_event_layout_registry_hash, trace_event_layout_registry_v1,
};
