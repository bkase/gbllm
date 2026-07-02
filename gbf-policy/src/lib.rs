//! Compile requests, objectives, deployment envelopes, runtime budgets, and repair policies.

#[cfg(test)]
pub(crate) static TRACE_CAPTURE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub mod budget;
pub mod calibration;
mod canonical;
pub mod capabilities;
pub mod compile;
pub mod cost;
pub mod dense_matched;
pub mod diagnostics;
pub mod emulator_harness;
pub mod envelope;
pub mod long_range;
pub mod matched_bytes;
pub mod metrics;
pub mod model_profile;
pub mod objective;
pub mod observable_invariants;
pub mod probe;
pub mod profile;
pub mod re_validation;
pub mod reference_shell;
pub mod repair;
pub mod risk;
pub mod s5;
pub mod shadow;
pub mod synthetic;
pub mod trace_event_layout;
pub mod wram;

pub use gbf_abi::RuntimeShellModule;

pub use budget::{
    BudgetSlotClass, RUNTIME_NUCLEUS_MODULE_BINDING_SCHEMA_VERSION, RomBudgetSlot,
    RuntimeChromeBudget, RuntimeChromeBudgetValidationError, RuntimeMemoryCapSection,
    RuntimeNucleusHash, RuntimeNucleusHashParseError, SYNTHETIC_REFERENCE_PREFIX,
};
pub use calibration::{
    BootstrapCalibrationBundle, CalibrationBundle, CalibrationBundleSet, CalibrationLayer,
    CalibrationSessionProfile, CalibrationSetRef, MeasurementBlob, ValidityEnvelope,
    ValidityEnvelopeFuturePlaceholder,
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
    SramPageAggression, SramSpillPolicy, StageIterationLimits, StorageKnob, StorageKnobBounds,
    StorageMaterialization, TRACE_COMPILE_PROFILE_ID, TRACE_COMPILE_PROFILE_TOML,
    TileCandidateClass, TileSelector, TraceBudget, TraceDemotionLevel, TraceDropPolicy, ValueId,
    ValueSelector, canonical_compile_profile_specs, canonical_default_bounds_fixture,
    check_delta_admissible, check_delta_admissible_with_recompute_purity,
    compile_profile_defaults_hash, f_b16_profile_lock_set, f_b16_refinement_knob_ids,
    load_compile_profile_spec, resolve_initial_knobs_from_profile_spec,
    resolve_repair_policy_from_profile_spec, resolve_resource_pressure_thresholds,
};
pub use cost::{
    CostBucketTotals, CostEstimate, EstimatedCostDelta, EvidenceClass, FallbackReason,
    ModeCostBreakdown, ModeCostBreakdownEntry, ModeEstimatedCost, ObjectiveAxis,
    ObjectiveSatisfaction, ObjectiveSatisfactionMatrix, Quantile, SatisfactionEntry,
    SatisfactionKey, ScheduleCostBreakdown, ScheduleCostIdentity, ScheduleCostReport,
    SliceCostBreakdown, StaleCalibrationField, UncertaintyEnvelope,
};
pub use dense_matched::{DenseMatchedBytesPolicy, DenseMatchedBytesPolicyError};
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
pub use emulator_harness::{
    H15FirstCommitCardinalityRefutation, H15FirstCommitCardinalityReport,
    H15FirstCommitCardinalityVerdict, verify_h15_first_commit_payload_len,
};
pub use long_range::{
    H5_LONG_RANGE_CONFIRM_REDUCTION_PER_TOKEN, H5_LONG_RANGE_REFUTE_REGRESSION_PER_TOKEN,
    H5_VAL_BPC_REFUTE_REGRESSION, H5LongRangeError, H5LongRangeEvidence, H5LongRangeRefutation,
    H5LongRangeVerdict, H5LongRangeVerdictResult, LONG_RANGE_REPETITION_MIN_DISTANCE,
    LongRangeRepetitionPenalty, h5_long_range_verdict, long_range_repetition_penalty,
};
pub use matched_bytes::{
    BiasPolicy, BiasPolicyParseError, LinearShape, MATCHED_BYTES_FORMULA_VERSION,
    MatchedBytesConfig, MatchedBytesError, MatchedBytesPolicy, MatchedBytesSolution,
    S7_CANONICAL_BIAS_POLICY, S7_D_FF_DENSE_MAX, S7_ONE_BANK_BYTES,
    S7_ROUTER_HIGH_PRECISION_BYTES_PER_PARAM, S7_TERNARY_METADATA_BYTES, bias_byte_cost,
    compute_dense_ffn_total, compute_linear_deployed_byte_cost, compute_moe_experts_total,
    compute_router_overhead_total, compute_weight_byte_cost, d6_tolerance_bytes, solve_d_ff_dense,
};
pub use metrics::{
    METRIC_REGISTRY_LOADED_EVENT, METRIC_REGISTRY_VERSION, MetricAggregation, MetricId,
    MetricIdError, MetricRegistryEntry, MetricRegistryError, MetricRegistrySnapshot, MetricSource,
    emit_metric_registry_loaded, load_metric_registry_v1, metric_registry_canonical_json_bytes,
    metric_registry_hash, metric_registry_v1,
};
pub use objective::{CompileObjective, RiskPolicy, ServiceLevelObjective};
pub use observable_invariants::{
    O9CrossSeedDifferenceReport, O9SafetensorsHashObservation, O9VariantResult, S5_SEEDS,
    S5VariantId, o9_cross_seed_difference_passes, o9_cross_seed_difference_report,
};
pub use probe::{
    EffectClass, InferOpTag, PROBE_REGISTRY_LOADED_EVENT, PROBE_REGISTRY_VERSION,
    ProbeImportanceClass, ProbeRegistryEntry, ProbeRegistryError, ProbeRegistrySnapshot,
    ProbeSourceSelector, ProbeTiming, TraceFrequencyBound, ValueRole, emit_probe_registry_loaded,
    load_probe_registry_v1, probe_registry_canonical_json_bytes, probe_registry_hash,
    probe_registry_v1, validate_probe_registry_event_shapes,
};
pub use profile::{
    BRINGUP_COMPILE_PROFILE, BRINGUP_MAX_BANK_SWITCHES_PER_TOKEN, BRINGUP_SWITCH_CAP_RATIONALE,
    COMPILE_PROFILE_REGISTRY, CompileProfile, CompileProfileError, SwitchCapProvenance,
    bringup_profile, profile_by_id, registry as compile_profile_registry,
};
pub use re_validation::{
    D9_RUNTIME_CHROME_BUDGET_DELTA_TOLERANCE_BYTES, ReValidationOutcome, RuntimeChromeBudgetDelta,
    RuntimeChromeBudgetReValidation, revalidate_runtime_chrome_budget,
};
pub use reference_shell::{FutureReservation, ReferenceShellSpec, pinned_reference_shell};
pub use repair::{RepairPolicy, RepairPolicyProfile, RepairProposalId, RepairReason};
pub use risk::{CalibrationConfidenceClass, CalibrationConfidenceRequirement};
pub use shadow::{
    H13_SHADOW_FINAL_STRICT_PASS_MAX_BYTES, H13_SHADOW_FINAL_WARNING_MAX_BYTES,
    H13ShadowFinalByteCostGap, H13ShadowFinalByteCostStatus, S5_SHADOW_CADENCE_STEPS,
    S5_SHADOW_COMPILE_SAMPLE_SCHEMA, S5_SHADOW_PIPELINE_STAGES, ShadowCompileSampleExpectation,
    ShadowCompileSampleReal, ShadowEmissionId, ShadowStep, Shr1ValidationError,
    h13_shadow_final_byte_cost_gap, h13_shadow_final_byte_cost_status,
    h13_shadow_sample_final_byte_cost_gap, shadow_compile_sample_real_emission_order,
    shadow_compile_sample_real_path, validate_shr1_shadow_sample,
};
pub use synthetic::{
    SYNTHETIC_BANK0_FREE_USABLE_BYTES, SYNTHETIC_BANK0_RESERVED_SLACK_BYTES,
    SYNTHETIC_COMMON_BANK_RESERVED_SLACK_BYTES, SYNTHETIC_COMMON_BANK_USABLE_BYTES,
    SYNTHETIC_EXPERT_BANK_COUNT, SYNTHETIC_EXPERT_BANK_RESERVED_SLACK_BYTES,
    SYNTHETIC_EXPERT_BANK_USABLE_BYTES, SYNTHETIC_REFERENCE_BUDGET_FIXTURE_PATH,
    SYNTHETIC_REFERENCE_RUNTIME_NUCLEUS_HASH_BYTES, SYNTHETIC_SRAM_RESERVED_BYTES,
    SYNTHETIC_WRAM_HOT_ARENA_FLOOR_BYTES, SYNTHETIC_WRAM_OVERLAY_BYTES,
    SYNTHETIC_WRAM_RESERVED_TOTAL_BYTES, synthetic_reference_future_module_slack_bytes,
    synthetic_reference_module_binding_hash, synthetic_reference_runtime_chrome_budget,
    synthetic_reference_runtime_chrome_budget_from_fixture, synthetic_reference_shell_modules,
};
pub use trace_event_layout::{
    ABI_TRACE_EVENT_PAYLOAD_BYTES, TRACE_EVENT_LAYOUT_REGISTRY_LOADED_EVENT,
    TRACE_EVENT_LAYOUT_REGISTRY_VERSION, TraceEventLayoutEntry, TraceEventLayoutRegistryError,
    TraceEventLayoutRegistrySnapshot, TraceEventPayloadLayout, TraceEventShape,
    TraceEventShapeError, TraceEventTupleSpecId, emit_trace_event_layout_registry_loaded,
    load_trace_event_layout_registry_v1, trace_event_layout_registry_canonical_json_bytes,
    trace_event_layout_registry_hash, trace_event_layout_registry_v1,
};
pub use wram::{
    DMG_WRAM_SIZE_BYTES, OverlayReloadPolicy, WramLayoutPolicy, WramPolicyError, WramReserved,
};
