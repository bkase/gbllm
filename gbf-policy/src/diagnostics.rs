//! Shared validation diagnostic taxonomy.

use gbf_abi::SemanticCheckpointId;
pub use gbf_foundation::{
    ArtifactFeature, ArtifactSchemaVersion, ComponentId, DataLoweringProfileId, GoldenVectorId,
    LineageId, LoweringShardId, LoweringShardRef, ManifestInvariant, SidecarKind,
};
use gbf_foundation::{
    BlobRef, BudgetSlotId, CompileProfileId, EvidenceRef, ExpertId, FieldPath, Hash256, LayerId,
    PackerVersion, SemVer, TargetProfileId, WorkloadId,
};
use serde::{Deserialize, Serialize};

use crate::calibration::CalibrationLayer;
use crate::compile::{
    CompileKnobBounds, CompileKnobId, CompilerFeature, ConstraintValue, PlacementProfile,
    RuntimeMode, SelectorPath,
};
use crate::metrics::MetricId;
use crate::probe::ProbeImportanceClass;
use crate::risk::CalibrationConfidenceClass;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidationDiagnostic {
    pub severity: DiagnosticSeverity,
    pub origin: ValidationOrigin,
    pub code: ValidationCode,
    pub detail: ValidationDetail,
    pub provenance: Vec<EvidenceRef>,
}

impl ValidationDiagnostic {
    #[must_use]
    pub fn new(
        severity: DiagnosticSeverity,
        origin: ValidationOrigin,
        code: ValidationCode,
        detail: ValidationDetail,
        provenance: Vec<EvidenceRef>,
    ) -> Self {
        Self {
            severity,
            origin,
            code,
            detail,
            provenance,
        }
    }

    #[must_use]
    pub fn hard(
        origin: ValidationOrigin,
        code: ValidationCode,
        detail: ValidationDetail,
        provenance: Vec<EvidenceRef>,
    ) -> Self {
        Self::new(DiagnosticSeverity::Hard, origin, code, detail, provenance)
    }

    #[must_use]
    pub fn soft(
        origin: ValidationOrigin,
        code: ValidationCode,
        detail: ValidationDetail,
        provenance: Vec<EvidenceRef>,
    ) -> Self {
        Self::new(DiagnosticSeverity::Soft, origin, code, detail, provenance)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum DiagnosticSeverity {
    Hard,
    Soft,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ValidationOrigin {
    Schema,
    SemanticCore,
    ObservationPlanConstruction,
    RangePlanConstruction,
    StoragePlanConstruction,
    SramPagePlanConstruction,
    RomWindowPlanConstruction,
    OverlayPlanConstruction,
    ArenaPlanConstruction,
    SchedIrConstruction,
    ResourceStateValidation,
    ScheduleCostAnalysis,
    Manifest,
    Lowering,
    Calibration,
    HintBundle,
    Workload,
    GoldenVector,
    CompileRequest,
    PolicyResolution,
    Budget,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "fields", deny_unknown_fields)]
pub enum ValidationCode {
    SchemaEpochUnsupported,
    SchemaCompatibilityAdapterMissing {
        observed: SemVer,
        target: SemVer,
    },
    SchemaCompatibilityAdapterNotLossless {
        adapter: CompatibilityAdapterId,
    },
    ReportSemanticInvariantViolated {
        field: FieldPath,
    },
    SemanticCoreHashMismatch,
    ArtifactTransportManifestMismatch,
    ManifestInvariantViolated {
        invariant: ManifestInvariant,
    },
    ArtifactPayloadMalformed {
        field: FieldPath,
    },
    ArtifactBlobDigestMismatch {
        blob: BlobRef,
        expected: Hash256,
        observed: Hash256,
    },
    ArtifactAuxMalformed {
        field: FieldPath,
    },
    ArtifactAuxSidecarMissing {
        kind: SidecarKind,
    },
    ArtifactAuxSidecarDigestMismatch {
        kind: SidecarKind,
        expected: Hash256,
        observed: Hash256,
    },
    ArtifactForbiddenBuildIdentityField {
        field: FieldPath,
    },
    ArtifactRequiredFeatureUnsupported {
        feature: ArtifactFeature,
    },
    LoweringMissingForTarget {
        target: TargetProfileId,
        lowering_profile: DataLoweringProfileId,
    },
    LoweringRoundTripFailed {
        shard: LoweringShardRef,
    },
    LoweringPackerVersionMismatch {
        artifact_version: PackerVersion,
        runtime_version: PackerVersion,
    },
    CalibrationMissing {
        class: CalibrationLayer,
    },
    CalibrationStale {
        class: CalibrationLayer,
        declared: Hash256,
        observed: Hash256,
    },
    CalibrationConfidenceTooLow {
        required: CalibrationConfidenceClass,
        observed: CalibrationConfidenceClass,
    },
    HintProvenanceInconsistent {
        fact: TraceProbeId,
    },
    WorkloadRefUnresolved {
        workload: WorkloadId,
    },
    GoldenVectorMissing {
        vector: GoldenVectorId,
    },
    GoldenVectorDigestMismatch {
        vector: GoldenVectorId,
        expected: Hash256,
        observed: Hash256,
    },
    CompileRequestUnsupportedFeature {
        feature: CompilerFeature,
    },
    CompileRequestProfileForbidsObjective {
        profile: CompileProfileId,
        reason: ObjectiveRejection,
    },
    CompileRequestRuntimeModeUnsupported {
        mode: RuntimeMode,
    },
    CompileRequestTargetIncompatible {
        target: TargetProfileId,
        reason: TargetIncompatibilityReason,
    },
    PolicyKnobOutOfBounds {
        knob: CompileKnobId,
        requested: KnobValueDescriptor,
        bounds: CompileKnobBounds,
    },
    PolicyConstraintUnsatisfiable {
        knob: CompileKnobId,
        left: CompileKnobBounds,
        right: CompileKnobBounds,
    },
    PolicyConstraintLoosened {
        knob: CompileKnobId,
        previous: CompileKnobBounds,
        requested: CompileKnobBounds,
    },
    PolicyHintConstraintUnsupported {
        knob: CompileKnobId,
        value: ConstraintValue,
    },
    PolicyKnobLockedAndOverridden {
        knob: CompileKnobId,
    },
    BudgetMissingRuntimeChromeBudget,
    BudgetQuantGraphViewMalformed {
        field: FieldPath,
    },
    BudgetExpertExceedsSlot {
        layer: LayerId,
        expert: ExpertId,
        slot: BudgetSlotId,
        payload_bytes: u32,
        cap_bytes: u32,
        excess_bytes: u32,
    },
    BudgetCommonBankExceedsCap {
        assigned_bytes: u32,
        cap_bytes: u32,
    },
    BudgetWramPeakExceeds {
        peak: u32,
        cap: u32,
    },
    BudgetSramPeakExceeds {
        peak: u32,
        cap: u32,
    },
    BudgetHramPeakExceeds {
        peak: u32,
        cap: u32,
    },
    BudgetAccumulatorOverflow {
        site: ReductionSiteId,
        projected_max_abs: u64,
    },
    InferIrRouterPresentForDenseLayer {
        layer: LayerId,
    },
    InferIrRouterMatVecMissingForRoutedLayer {
        layer: LayerId,
    },
    InferIrSequenceSemanticsUnsupportedV1 {
        field: FieldPath,
    },
    InferIrEmbeddingNotUnique {
        field: FieldPath,
    },
    InferIrDecodeNotUnique {
        field: FieldPath,
    },
    InferIrClassifyNotUnique {
        field: FieldPath,
    },
    InferIrExpertCoverageMismatch {
        layer: LayerId,
        expert: ExpertId,
    },
    InferIrRouteCoverageMismatch {
        layer: LayerId,
    },
    InferIrSemanticCheckpointEmittedHere {
        field: FieldPath,
    },
    InferIrEffectChainNotLinear {
        field: FieldPath,
    },
    InferIrEffectIdEdgeTokenViolation {
        field: FieldPath,
    },
    InferIrTopologicalOrderMismatch {
        field: FieldPath,
    },
    InferIrValueProducerMissing {
        value_id: u32,
    },
    InferIrValueFormatMismatch {
        field: FieldPath,
    },
    InferIrNormFormatMismatch {
        field: FieldPath,
    },
    InferIrExpertSectionRoleMismatch {
        layer: LayerId,
        expert: ExpertId,
    },
    InferIrNonV1RouterSemantics {
        layer: LayerId,
    },
    InferIrDenseRoutedShapeMismatch {
        layer: LayerId,
    },
    InferIrDecodePlanMismatch {
        field: FieldPath,
    },
    InferIrDecodeRngBindingMismatch {
        field: FieldPath,
    },
    InferIrUnexpectedRngEffectOnPureOp {
        field: FieldPath,
    },
    InferIrSequenceSlotCoverageMismatch {
        layer: LayerId,
    },
    InferIrOpHistogramTotalMismatch {
        field: FieldPath,
    },
    InferIrFaultBoundaryEmittedV1Forbidden {
        field: FieldPath,
    },
    InferIrResidualBoundaryMismatch {
        field: FieldPath,
    },
    InferIrTokenIngressAmbiguous {
        field: FieldPath,
    },
    InferIrSemanticEquivalenceFailed {
        field: FieldPath,
    },
    InferIrCycleDetected {
        field: FieldPath,
    },
    InferIrUnreachableNode {
        field: FieldPath,
    },
    InferIrDisconnectedComponent {
        field: FieldPath,
    },
    InferIrForbiddenStorageMetadata {
        field: FieldPath,
    },
    InferIrSemanticAnchorMissing {
        field: FieldPath,
    },
    InferIrFfnActivationMissing {
        layer: LayerId,
        expert: ExpertId,
    },
    InferIrExpertSelectionMissing {
        layer: LayerId,
    },
    InferIrGateWeightNotConsumed {
        field: FieldPath,
    },
    InferIrInputTokenValueIdMismatch {
        field: FieldPath,
    },
    InferIrReductionSiteMissing {
        field: FieldPath,
    },
    InferIrOpSignatureMismatch {
        field: FieldPath,
    },
    InferIrRouterScoreOrphaned {
        field: FieldPath,
    },
    InferIrSequenceStateNextOrphaned {
        field: FieldPath,
    },
    ObservationMandatoryCheckpointNotFeasible {
        checkpoint: SemanticCheckpointId,
    },
    ObservationWorkloadCheckpointNotFeasible {
        checkpoint: SemanticCheckpointId,
    },
    ObservationCheckpointNotInSchema {
        checkpoint: SemanticCheckpointId,
    },
    ObservationCheckpointNotAttachable {
        checkpoint: SemanticCheckpointId,
    },
    ObservationCheckpointAmbiguous {
        checkpoint: SemanticCheckpointId,
    },
    ObservationEncodingInvalidForCheckpoint {
        checkpoint: SemanticCheckpointId,
    },
    ObservationScHashMismatch {
        expected: Hash256,
        observed: Hash256,
    },
    ObservationDeterminismMismatch {
        field: FieldPath,
    },
    ObservationProbeIdUnknown {
        probe_id: TraceProbeId,
    },
    ObservationMetricIdUnknown {
        metric: MetricId,
    },
    ObservationRequiredProbeDisabled {
        probe_id: TraceProbeId,
    },
    ObservationMetricSourceReservedV1 {
        metric: MetricId,
    },
    ObservationMetricHistogramBucketCountZero {
        metric: MetricId,
    },
    ObservationProbeSourceInvalid {
        probe_id: TraceProbeId,
    },
    ObservationReservedEffectProbe {
        probe_id: TraceProbeId,
    },
    ObservationSequenceStateProbeReserved {
        probe_id: TraceProbeId,
    },
    ObservationFaultBoundaryProbeReserved {
        probe_id: TraceProbeId,
    },
    ObservationProbeClassCapExceeded {
        class: ProbeImportanceClass,
        observed: u32,
        cap: u32,
    },
    ObservationInvariantModeBudgetBusted {
        projected_max_events_per_slice: u32,
        projected_max_bytes_per_frame: u32,
        max_events_per_slice: u16,
        max_bytes_per_frame: u16,
    },
    BudgetSwitchesPerTokenOverCap {
        decision_value: u16,
        upper_bound: u16,
        cap: u16,
        source: SwitchProjectionSource,
    },
    BudgetSramPageSwitchesPerTokenOverCap {
        decision_value: u16,
        upper_bound: u16,
        cap: u16,
        source: SwitchProjectionSource,
    },
    BudgetPlacementProfileInfeasible {
        profile: PlacementProfile,
        reason: PlacementInfeasibilityReason,
    },
    StoragePlan {
        code: StoragePlanDiagnosticCode,
        provenance: StoragePlanDiagnosticProvenance,
    },
    SramPagePlan {
        code: SramPagePlanDiagnosticCode,
        provenance: SramPagePlanDiagnosticProvenance,
    },
    RomWindowPlan {
        code: RomWindowPlanDiagnosticCode,
        provenance: RomWindowPlanDiagnosticProvenance,
    },
    OverlayPlan {
        code: OverlayPlanDiagnosticCode,
        provenance: OverlayPlanDiagnosticProvenance,
    },
    ArenaPlan {
        code: ArenaPlanDiagnosticCode,
        provenance: ArenaPlanDiagnosticProvenance,
    },
    ResourceState {
        code: ResourceStateDiagnosticCode,
        provenance: ResourceStateDiagnosticProvenance,
    },
    ScheduleCost {
        code: ScheduleCostDiagnosticCode,
        provenance: ScheduleCostDiagnosticProvenance,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
#[allow(clippy::enum_variant_names)]
pub enum ScheduleCostDiagnosticCode {
    CostScheduleCostInputHashMismatch,
    CostCalibrationBundleHashMismatch,
    CostCalibrationBundleStale,
    CostCalibrationMissingForRequirement,
    CostKernelSpecNotInRegistry,
    CostPerModeMissing,
    CostPerModeUnexpected,
    CostEvidenceClassRefsInconsistent,
    CostFallbackReasonMissing,
    CostFallbackReasonPresentForCalibrated,
    CostUncertaintyEnvelopeMalformed,
    CostUncertaintyEnvelopeNegative,
    CostObjectiveSatisfactionMatrixIncomplete,
    CostObjectiveSatisfactionMatrixInconsistent,
    CostHeuristicPolicyUnknown,
    CostTransferPolicyUnknown,
    CostFloatingPointFieldDetected,
    CostScheduleCostSchemaUnknown,
    CostOptionFieldMissing,
    CostOptionFieldPresentUnexpectedly,
    CostRefsUnionInconsistent,
    CostScheduleCostReportRoundTripFailed,
    CostFinalNonNegativityViolation,
}

impl ScheduleCostDiagnosticCode {
    pub const ALL: [Self; 23] = [
        Self::CostScheduleCostInputHashMismatch,
        Self::CostCalibrationBundleHashMismatch,
        Self::CostCalibrationBundleStale,
        Self::CostCalibrationMissingForRequirement,
        Self::CostKernelSpecNotInRegistry,
        Self::CostPerModeMissing,
        Self::CostPerModeUnexpected,
        Self::CostEvidenceClassRefsInconsistent,
        Self::CostFallbackReasonMissing,
        Self::CostFallbackReasonPresentForCalibrated,
        Self::CostUncertaintyEnvelopeMalformed,
        Self::CostUncertaintyEnvelopeNegative,
        Self::CostObjectiveSatisfactionMatrixIncomplete,
        Self::CostObjectiveSatisfactionMatrixInconsistent,
        Self::CostHeuristicPolicyUnknown,
        Self::CostTransferPolicyUnknown,
        Self::CostFloatingPointFieldDetected,
        Self::CostScheduleCostSchemaUnknown,
        Self::CostOptionFieldMissing,
        Self::CostOptionFieldPresentUnexpectedly,
        Self::CostRefsUnionInconsistent,
        Self::CostScheduleCostReportRoundTripFailed,
        Self::CostFinalNonNegativityViolation,
    ];

    #[must_use]
    pub const fn number(self) -> u16 {
        match self {
            Self::CostScheduleCostInputHashMismatch => 1,
            Self::CostCalibrationBundleHashMismatch => 2,
            Self::CostCalibrationBundleStale => 3,
            Self::CostCalibrationMissingForRequirement => 4,
            Self::CostKernelSpecNotInRegistry => 5,
            Self::CostPerModeMissing => 6,
            Self::CostPerModeUnexpected => 7,
            Self::CostEvidenceClassRefsInconsistent => 8,
            Self::CostFallbackReasonMissing => 9,
            Self::CostFallbackReasonPresentForCalibrated => 10,
            Self::CostUncertaintyEnvelopeMalformed => 11,
            Self::CostUncertaintyEnvelopeNegative => 12,
            Self::CostObjectiveSatisfactionMatrixIncomplete => 13,
            Self::CostObjectiveSatisfactionMatrixInconsistent => 14,
            Self::CostHeuristicPolicyUnknown => 15,
            Self::CostTransferPolicyUnknown => 16,
            Self::CostFloatingPointFieldDetected => 17,
            Self::CostScheduleCostSchemaUnknown => 18,
            Self::CostOptionFieldMissing => 19,
            Self::CostOptionFieldPresentUnexpectedly => 20,
            Self::CostRefsUnionInconsistent => 21,
            Self::CostScheduleCostReportRoundTripFailed => 22,
            Self::CostFinalNonNegativityViolation => 23,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self.number() {
            1 => "COST-001",
            2 => "COST-002",
            3 => "COST-003",
            4 => "COST-004",
            5 => "COST-005",
            6 => "COST-006",
            7 => "COST-007",
            8 => "COST-008",
            9 => "COST-009",
            10 => "COST-010",
            11 => "COST-011",
            12 => "COST-012",
            13 => "COST-013",
            14 => "COST-014",
            15 => "COST-015",
            16 => "COST-016",
            17 => "COST-017",
            18 => "COST-018",
            19 => "COST-019",
            20 => "COST-020",
            21 => "COST-021",
            22 => "COST-022",
            23 => "COST-023",
            _ => unreachable!(),
        }
    }

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::CostScheduleCostInputHashMismatch => "CostScheduleCostInputHashMismatch",
            Self::CostCalibrationBundleHashMismatch => "CostCalibrationBundleHashMismatch",
            Self::CostCalibrationBundleStale => "CostCalibrationBundleStale",
            Self::CostCalibrationMissingForRequirement => "CostCalibrationMissingForRequirement",
            Self::CostKernelSpecNotInRegistry => "CostKernelSpecNotInRegistry",
            Self::CostPerModeMissing => "CostPerModeMissing",
            Self::CostPerModeUnexpected => "CostPerModeUnexpected",
            Self::CostEvidenceClassRefsInconsistent => "CostEvidenceClassRefsInconsistent",
            Self::CostFallbackReasonMissing => "CostFallbackReasonMissing",
            Self::CostFallbackReasonPresentForCalibrated => {
                "CostFallbackReasonPresentForCalibrated"
            }
            Self::CostUncertaintyEnvelopeMalformed => "CostUncertaintyEnvelopeMalformed",
            Self::CostUncertaintyEnvelopeNegative => "CostUncertaintyEnvelopeNegative",
            Self::CostObjectiveSatisfactionMatrixIncomplete => {
                "CostObjectiveSatisfactionMatrixIncomplete"
            }
            Self::CostObjectiveSatisfactionMatrixInconsistent => {
                "CostObjectiveSatisfactionMatrixInconsistent"
            }
            Self::CostHeuristicPolicyUnknown => "CostHeuristicPolicyUnknown",
            Self::CostTransferPolicyUnknown => "CostTransferPolicyUnknown",
            Self::CostFloatingPointFieldDetected => "CostFloatingPointFieldDetected",
            Self::CostScheduleCostSchemaUnknown => "CostScheduleCostSchemaUnknown",
            Self::CostOptionFieldMissing => "CostOptionFieldMissing",
            Self::CostOptionFieldPresentUnexpectedly => "CostOptionFieldPresentUnexpectedly",
            Self::CostRefsUnionInconsistent => "CostRefsUnionInconsistent",
            Self::CostScheduleCostReportRoundTripFailed => "CostScheduleCostReportRoundTripFailed",
            Self::CostFinalNonNegativityViolation => "CostFinalNonNegativityViolation",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ScheduleCostDiagnosticProvenance {
    HashMismatch {
        product: String,
        recorded: Hash256,
        computed: Hash256,
    },
    Mode {
        mode: RuntimeMode,
    },
    Objective {
        mode: RuntimeMode,
        axis: String,
        quantile: String,
        target_q16_16: i64,
        observed_q16_16: i64,
    },
    Calibration {
        layer: CalibrationLayer,
        declared_confidence: CalibrationConfidenceClass,
        required_confidence: String,
    },
    Estimate {
        field: String,
        invariant: String,
    },
    JsonPath {
        json_path: String,
        field_or_tag: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
#[allow(clippy::enum_variant_names)]
pub enum ResourceStateDiagnosticCode {
    SchedInputHashMismatch,
    SchedPackEmpty,
    SchedEpochCoverageGap,
    SchedEntryResidencyEpochMismatch,
    SchedOverlayInstallEpochMismatch,
    SchedArenaSlotUnknown,
    LeaseRequiredLeaseNotAcquired,
    LeaseUnbalanced,
    LeaseDoubleAcquire,
    LeaseReleaseWithoutAcquire,
    LeaseYieldCrossesNonResumable,
    LeaseKindMismatchAgainstUpstream,
    ResIsrEnabledHoldsRomWindowLease,
    ResIsrEnabledHoldsSramPageLease,
    ResIsrEnabledInExpertBank,
    ResBankSwitchUnbracketed,
    ModeRequestedModeNotEmitted,
    ModeCheckpointSchemaMismatch,
    DriftObservedNotAllNoneAtCompileTime,
    DriftConsecutiveViolationsNonZeroAtCompileTime,
    ResourceStateReportRoundTripFailed,
}

impl ResourceStateDiagnosticCode {
    pub const ALL: [Self; 21] = [
        Self::SchedInputHashMismatch,
        Self::SchedPackEmpty,
        Self::SchedEpochCoverageGap,
        Self::SchedEntryResidencyEpochMismatch,
        Self::SchedOverlayInstallEpochMismatch,
        Self::SchedArenaSlotUnknown,
        Self::LeaseRequiredLeaseNotAcquired,
        Self::LeaseUnbalanced,
        Self::LeaseDoubleAcquire,
        Self::LeaseReleaseWithoutAcquire,
        Self::LeaseYieldCrossesNonResumable,
        Self::LeaseKindMismatchAgainstUpstream,
        Self::ResIsrEnabledHoldsRomWindowLease,
        Self::ResIsrEnabledHoldsSramPageLease,
        Self::ResIsrEnabledInExpertBank,
        Self::ResBankSwitchUnbracketed,
        Self::ModeRequestedModeNotEmitted,
        Self::ModeCheckpointSchemaMismatch,
        Self::DriftObservedNotAllNoneAtCompileTime,
        Self::DriftConsecutiveViolationsNonZeroAtCompileTime,
        Self::ResourceStateReportRoundTripFailed,
    ];

    #[must_use]
    pub const fn number(self) -> u16 {
        match self {
            Self::SchedInputHashMismatch => 1,
            Self::SchedPackEmpty => 2,
            Self::SchedEpochCoverageGap => 3,
            Self::SchedEntryResidencyEpochMismatch => 4,
            Self::SchedOverlayInstallEpochMismatch => 5,
            Self::SchedArenaSlotUnknown => 6,
            Self::LeaseRequiredLeaseNotAcquired => 7,
            Self::LeaseUnbalanced => 8,
            Self::LeaseDoubleAcquire => 9,
            Self::LeaseReleaseWithoutAcquire => 10,
            Self::LeaseYieldCrossesNonResumable => 11,
            Self::LeaseKindMismatchAgainstUpstream => 12,
            Self::ResIsrEnabledHoldsRomWindowLease => 13,
            Self::ResIsrEnabledHoldsSramPageLease => 14,
            Self::ResIsrEnabledInExpertBank => 15,
            Self::ResBankSwitchUnbracketed => 16,
            Self::ModeRequestedModeNotEmitted => 17,
            Self::ModeCheckpointSchemaMismatch => 18,
            Self::DriftObservedNotAllNoneAtCompileTime => 19,
            Self::DriftConsecutiveViolationsNonZeroAtCompileTime => 20,
            Self::ResourceStateReportRoundTripFailed => 21,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self.number() {
            1 => "RSV-001",
            2 => "RSV-002",
            3 => "RSV-003",
            4 => "RSV-004",
            5 => "RSV-005",
            6 => "RSV-006",
            7 => "RSV-007",
            8 => "RSV-008",
            9 => "RSV-009",
            10 => "RSV-010",
            11 => "RSV-011",
            12 => "RSV-012",
            13 => "RSV-013",
            14 => "RSV-014",
            15 => "RSV-015",
            16 => "RSV-016",
            17 => "RSV-017",
            18 => "RSV-018",
            19 => "RSV-019",
            20 => "RSV-020",
            21 => "RSV-021",
            _ => unreachable!(),
        }
    }

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::SchedInputHashMismatch => "SchedInputHashMismatch",
            Self::SchedPackEmpty => "SchedPackEmpty",
            Self::SchedEpochCoverageGap => "SchedEpochCoverageGap",
            Self::SchedEntryResidencyEpochMismatch => "SchedEntryResidencyEpochMismatch",
            Self::SchedOverlayInstallEpochMismatch => "SchedOverlayInstallEpochMismatch",
            Self::SchedArenaSlotUnknown => "SchedArenaSlotUnknown",
            Self::LeaseRequiredLeaseNotAcquired => "LeaseRequiredLeaseNotAcquired",
            Self::LeaseUnbalanced => "LeaseUnbalanced",
            Self::LeaseDoubleAcquire => "LeaseDoubleAcquire",
            Self::LeaseReleaseWithoutAcquire => "LeaseReleaseWithoutAcquire",
            Self::LeaseYieldCrossesNonResumable => "LeaseYieldCrossesNonResumable",
            Self::LeaseKindMismatchAgainstUpstream => "LeaseKindMismatchAgainstUpstream",
            Self::ResIsrEnabledHoldsRomWindowLease => "ResIsrEnabledHoldsRomWindowLease",
            Self::ResIsrEnabledHoldsSramPageLease => "ResIsrEnabledHoldsSramPageLease",
            Self::ResIsrEnabledInExpertBank => "ResIsrEnabledInExpertBank",
            Self::ResBankSwitchUnbracketed => "ResBankSwitchUnbracketed",
            Self::ModeRequestedModeNotEmitted => "ModeRequestedModeNotEmitted",
            Self::ModeCheckpointSchemaMismatch => "ModeCheckpointSchemaMismatch",
            Self::DriftObservedNotAllNoneAtCompileTime => "DriftObservedNotAllNoneAtCompileTime",
            Self::DriftConsecutiveViolationsNonZeroAtCompileTime => {
                "DriftConsecutiveViolationsNonZeroAtCompileTime"
            }
            Self::ResourceStateReportRoundTripFailed => "ResourceStateReportRoundTripFailed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ResourceStateDiagnosticProvenance {
    HashMismatch {
        product: String,
        recorded: Hash256,
        computed: Hash256,
    },
    Mode {
        mode: RuntimeMode,
    },
    Slice {
        invariant: String,
        slice_id: u32,
    },
    Lease {
        invariant: String,
        lease_id: u32,
    },
    Epoch {
        invariant: String,
        epoch_id: u32,
    },
    ArenaSlot {
        invariant: String,
        slot_id: u32,
    },
    Drift {
        invariant: String,
        metric: String,
    },
    JsonPath {
        json_path: String,
        field_or_tag: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum StoragePlanDiagnosticCode {
    StorageNoAdmittingDecisionRule,
    StorageBindingCoverageGap,
    StorageBindingDoubleBind,
    StorageRomConstWriteViolation,
    StorageHramAdmissionInvariantViolation,
    StorageRecomputeForbiddenForObservedValue,
    StoragePersistSequenceStateUnsupportedV1,
    StoragePersistBindingKindMismatch,
    StoragePersistPageNotReferenced,
    StorageCommitGroupEmpty,
    StorageCommitGroupKindMix,
    StorageCommitGroupDurabilityMix,
    StorageAliasIntentMaterializationMismatch,
    StorageAliasClassOverlapWithoutIntent,
    StorageAliasClassMembershipFunctionalViolation,
    StorageRecomputeAliasNotIsolated,
    StorageLifetimeAdmissibilityViolation,
    StorageForbiddenSpatialEnumLeak,
    StorageDeterminismRequiresStableRules,
    StorageRangePlanHashMismatch,
    StorageInferIrHashMismatch,
    StorageObservationPlanHashMismatch,
    StorageQuantGraphHashMismatch,
    StoragePolicyHashMismatch,
    StorageIterationInputInvalid,
    StorageOverlayLensViolation,
    StorageRepairProposalIllegal,
    StorageInferIrEffectClassUnknown,
    StorageQuantGraphRoutingMismatch,
    StorageReservedShapeEmitted,
    StorageAliasMixedIntentComponent,
    StorageAliasIntentCardinalityViolation,
    StorageForcedRecomputeNotAllowed,
    StoragePolicyBudgetUnderflow,
    StorageAliasClassFingerprintCollision,
}

impl StoragePlanDiagnosticCode {
    pub const ALL: [Self; 35] = [
        Self::StorageNoAdmittingDecisionRule,
        Self::StorageBindingCoverageGap,
        Self::StorageBindingDoubleBind,
        Self::StorageRomConstWriteViolation,
        Self::StorageHramAdmissionInvariantViolation,
        Self::StorageRecomputeForbiddenForObservedValue,
        Self::StoragePersistSequenceStateUnsupportedV1,
        Self::StoragePersistBindingKindMismatch,
        Self::StoragePersistPageNotReferenced,
        Self::StorageCommitGroupEmpty,
        Self::StorageCommitGroupKindMix,
        Self::StorageCommitGroupDurabilityMix,
        Self::StorageAliasIntentMaterializationMismatch,
        Self::StorageAliasClassOverlapWithoutIntent,
        Self::StorageAliasClassMembershipFunctionalViolation,
        Self::StorageRecomputeAliasNotIsolated,
        Self::StorageLifetimeAdmissibilityViolation,
        Self::StorageForbiddenSpatialEnumLeak,
        Self::StorageDeterminismRequiresStableRules,
        Self::StorageRangePlanHashMismatch,
        Self::StorageInferIrHashMismatch,
        Self::StorageObservationPlanHashMismatch,
        Self::StorageQuantGraphHashMismatch,
        Self::StoragePolicyHashMismatch,
        Self::StorageIterationInputInvalid,
        Self::StorageOverlayLensViolation,
        Self::StorageRepairProposalIllegal,
        Self::StorageInferIrEffectClassUnknown,
        Self::StorageQuantGraphRoutingMismatch,
        Self::StorageReservedShapeEmitted,
        Self::StorageAliasMixedIntentComponent,
        Self::StorageAliasIntentCardinalityViolation,
        Self::StorageForcedRecomputeNotAllowed,
        Self::StoragePolicyBudgetUnderflow,
        Self::StorageAliasClassFingerprintCollision,
    ];

    #[must_use]
    pub const fn number(self) -> u16 {
        match self {
            Self::StorageNoAdmittingDecisionRule => 1,
            Self::StorageBindingCoverageGap => 2,
            Self::StorageBindingDoubleBind => 3,
            Self::StorageRomConstWriteViolation => 4,
            Self::StorageHramAdmissionInvariantViolation => 5,
            Self::StorageRecomputeForbiddenForObservedValue => 6,
            Self::StoragePersistSequenceStateUnsupportedV1 => 7,
            Self::StoragePersistBindingKindMismatch => 8,
            Self::StoragePersistPageNotReferenced => 9,
            Self::StorageCommitGroupEmpty => 10,
            Self::StorageCommitGroupKindMix => 11,
            Self::StorageCommitGroupDurabilityMix => 12,
            Self::StorageAliasIntentMaterializationMismatch => 13,
            Self::StorageAliasClassOverlapWithoutIntent => 14,
            Self::StorageAliasClassMembershipFunctionalViolation => 15,
            Self::StorageRecomputeAliasNotIsolated => 16,
            Self::StorageLifetimeAdmissibilityViolation => 17,
            Self::StorageForbiddenSpatialEnumLeak => 18,
            Self::StorageDeterminismRequiresStableRules => 19,
            Self::StorageRangePlanHashMismatch => 20,
            Self::StorageInferIrHashMismatch => 21,
            Self::StorageObservationPlanHashMismatch => 22,
            Self::StorageQuantGraphHashMismatch => 23,
            Self::StoragePolicyHashMismatch => 24,
            Self::StorageIterationInputInvalid => 25,
            Self::StorageOverlayLensViolation => 26,
            Self::StorageRepairProposalIllegal => 27,
            Self::StorageInferIrEffectClassUnknown => 28,
            Self::StorageQuantGraphRoutingMismatch => 29,
            Self::StorageReservedShapeEmitted => 30,
            Self::StorageAliasMixedIntentComponent => 31,
            Self::StorageAliasIntentCardinalityViolation => 32,
            Self::StorageForcedRecomputeNotAllowed => 33,
            Self::StoragePolicyBudgetUnderflow => 34,
            Self::StorageAliasClassFingerprintCollision => 35,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self.number() {
            1 => "STORE-001",
            2 => "STORE-002",
            3 => "STORE-003",
            4 => "STORE-004",
            5 => "STORE-005",
            6 => "STORE-006",
            7 => "STORE-007",
            8 => "STORE-008",
            9 => "STORE-009",
            10 => "STORE-010",
            11 => "STORE-011",
            12 => "STORE-012",
            13 => "STORE-013",
            14 => "STORE-014",
            15 => "STORE-015",
            16 => "STORE-016",
            17 => "STORE-017",
            18 => "STORE-018",
            19 => "STORE-019",
            20 => "STORE-020",
            21 => "STORE-021",
            22 => "STORE-022",
            23 => "STORE-023",
            24 => "STORE-024",
            25 => "STORE-025",
            26 => "STORE-026",
            27 => "STORE-027",
            28 => "STORE-028",
            29 => "STORE-029",
            30 => "STORE-030",
            31 => "STORE-031",
            32 => "STORE-032",
            33 => "STORE-033",
            34 => "STORE-034",
            35 => "STORE-035",
            _ => unreachable!(),
        }
    }

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::StorageNoAdmittingDecisionRule => "StorageNoAdmittingDecisionRule",
            Self::StorageBindingCoverageGap => "StorageBindingCoverageGap",
            Self::StorageBindingDoubleBind => "StorageBindingDoubleBind",
            Self::StorageRomConstWriteViolation => "StorageRomConstWriteViolation",
            Self::StorageHramAdmissionInvariantViolation => {
                "StorageHramAdmissionInvariantViolation"
            }
            Self::StorageRecomputeForbiddenForObservedValue => {
                "StorageRecomputeForbiddenForObservedValue"
            }
            Self::StoragePersistSequenceStateUnsupportedV1 => {
                "StoragePersistSequenceStateUnsupportedV1"
            }
            Self::StoragePersistBindingKindMismatch => "StoragePersistBindingKindMismatch",
            Self::StoragePersistPageNotReferenced => "StoragePersistPageNotReferenced",
            Self::StorageCommitGroupEmpty => "StorageCommitGroupEmpty",
            Self::StorageCommitGroupKindMix => "StorageCommitGroupKindMix",
            Self::StorageCommitGroupDurabilityMix => "StorageCommitGroupDurabilityMix",
            Self::StorageAliasIntentMaterializationMismatch => {
                "StorageAliasIntentMaterializationMismatch"
            }
            Self::StorageAliasClassOverlapWithoutIntent => "StorageAliasClassOverlapWithoutIntent",
            Self::StorageAliasClassMembershipFunctionalViolation => {
                "StorageAliasClassMembershipFunctionalViolation"
            }
            Self::StorageRecomputeAliasNotIsolated => "StorageRecomputeAliasNotIsolated",
            Self::StorageLifetimeAdmissibilityViolation => "StorageLifetimeAdmissibilityViolation",
            Self::StorageForbiddenSpatialEnumLeak => "StorageForbiddenSpatialEnumLeak",
            Self::StorageDeterminismRequiresStableRules => "StorageDeterminismRequiresStableRules",
            Self::StorageRangePlanHashMismatch => "StorageRangePlanHashMismatch",
            Self::StorageInferIrHashMismatch => "StorageInferIrHashMismatch",
            Self::StorageObservationPlanHashMismatch => "StorageObservationPlanHashMismatch",
            Self::StorageQuantGraphHashMismatch => "StorageQuantGraphHashMismatch",
            Self::StoragePolicyHashMismatch => "StoragePolicyHashMismatch",
            Self::StorageIterationInputInvalid => "StorageIterationInputInvalid",
            Self::StorageOverlayLensViolation => "StorageOverlayLensViolation",
            Self::StorageRepairProposalIllegal => "StorageRepairProposalIllegal",
            Self::StorageInferIrEffectClassUnknown => "StorageInferIrEffectClassUnknown",
            Self::StorageQuantGraphRoutingMismatch => "StorageQuantGraphRoutingMismatch",
            Self::StorageReservedShapeEmitted => "StorageReservedShapeEmitted",
            Self::StorageAliasMixedIntentComponent => "StorageAliasMixedIntentComponent",
            Self::StorageAliasIntentCardinalityViolation => {
                "StorageAliasIntentCardinalityViolation"
            }
            Self::StorageForcedRecomputeNotAllowed => "StorageForcedRecomputeNotAllowed",
            Self::StoragePolicyBudgetUnderflow => "StoragePolicyBudgetUnderflow",
            Self::StorageAliasClassFingerprintCollision => "StorageAliasClassFingerprintCollision",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum StoragePlanDiagnosticProvenance {
    ValueClassification {
        value_id: u32,
        producer_node: Option<u32>,
        value_role: Option<String>,
        value_format: Option<String>,
    },
    ValueProducer {
        value_id: u32,
        producer_node: u32,
    },
    BindingSet {
        value_id: u32,
        binding_count: u32,
    },
    ProducerOp {
        value_id: u32,
        producer_node: u32,
        op_tag: String,
    },
    BudgetSet {
        values: Vec<u32>,
        observed_bytes: u32,
        budget_bytes: u32,
    },
    ObservationCheckpoint {
        value_id: u32,
        semantic_anchor: String,
        checkpoint_id: u32,
    },
    SequenceState {
        value_id: u32,
        state_slot_id: u32,
        layer_id: u16,
    },
    PersistBinding {
        value_id: u32,
        persist_page_id: u32,
        commit_group_id: u32,
        persist_kind: String,
        expected: String,
    },
    PersistPage {
        invariant: String,
        persist_page_id: u32,
    },
    CommitGroup {
        invariant: String,
        commit_group_id: u32,
    },
    CommitGroupKind {
        commit_group_id: u32,
        kinds: Vec<String>,
        allowed_table: String,
    },
    CommitGroupDurability {
        commit_group_id: u32,
        durabilities: Vec<String>,
    },
    AliasMaterialization {
        alias_class_id: u32,
        members: Vec<u32>,
        intent: String,
        materializations: Vec<String>,
    },
    AliasOverlap {
        alias_class_id: u32,
        members: Vec<u32>,
    },
    AliasMembership {
        invariant: String,
        value_id: u32,
        alias_class_id: u32,
    },
    RecomputeAlias {
        value_id: u32,
        alias_class_id: u32,
    },
    LifetimeAdmissibility {
        value_id: u32,
        computed_lifetime: String,
        min_lifetime: String,
        max_lifetime: String,
        source: String,
    },
    JsonPath {
        json_path: String,
        field_or_tag: String,
    },
    RuleInstability {
        rule_id: u32,
        evidence: String,
    },
    HashMismatch {
        product: String,
        recorded: Hash256,
        computed: Hash256,
    },
    Iteration {
        iteration: u32,
        ceiling: u32,
    },
    OverlayLens {
        value_id: u32,
        materialization: String,
        forced_override: bool,
    },
    RepairProposal {
        proposal_id: String,
        delta: String,
        locks_bounds: String,
    },
    EffectClass {
        effect_id: u32,
        effect_class: String,
    },
    RoutingMismatch {
        layer_id: u16,
        expected_entry: String,
    },
    AliasMixedIntent {
        members: Vec<u32>,
        edge_count: u32,
        intents: Vec<String>,
    },
    AliasCardinality {
        alias_class_id: u32,
        intent: String,
        members: Vec<u32>,
    },
    ForcedRecompute {
        value_id: u32,
        failed_predicates: Vec<String>,
    },
    PolicyBudget {
        storage_class: String,
        soft_bytes: u32,
        reserved_bytes: u32,
    },
    FingerprintCollision {
        first_payload_hash: Hash256,
        second_payload_hash: Hash256,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
#[allow(clippy::enum_variant_names)]
pub enum SramPagePlanDiagnosticCode {
    SramInputHashMismatch,
    SramCommitGroupCrossStream,
    SramPageOverflow,
    SramPageGeometryMismatch,
    SramBudgetExceeded,
    SramResidencyUnresolved,
    SramYieldResumeResidencyViolation,
    SramCrossStreamPageSharing,
    SramCanonicalSortDrift,
    SramReportRoundTripFailed,
    SramSectionRoleLeaked,
    SramSchedulingFieldLeaked,
    SramRepairProvenanceForbidden,
    SramTargetProfileLayoutUnsupported,
    SramPolicyProjectionMismatch,
}

impl SramPagePlanDiagnosticCode {
    pub const ALL: [Self; 15] = [
        Self::SramInputHashMismatch,
        Self::SramCommitGroupCrossStream,
        Self::SramPageOverflow,
        Self::SramPageGeometryMismatch,
        Self::SramBudgetExceeded,
        Self::SramResidencyUnresolved,
        Self::SramYieldResumeResidencyViolation,
        Self::SramCrossStreamPageSharing,
        Self::SramCanonicalSortDrift,
        Self::SramReportRoundTripFailed,
        Self::SramSectionRoleLeaked,
        Self::SramSchedulingFieldLeaked,
        Self::SramRepairProvenanceForbidden,
        Self::SramTargetProfileLayoutUnsupported,
        Self::SramPolicyProjectionMismatch,
    ];

    #[must_use]
    pub const fn number(self) -> u16 {
        match self {
            Self::SramInputHashMismatch => 1,
            Self::SramCommitGroupCrossStream => 2,
            Self::SramPageOverflow => 3,
            Self::SramPageGeometryMismatch => 4,
            Self::SramBudgetExceeded => 5,
            Self::SramResidencyUnresolved => 6,
            Self::SramYieldResumeResidencyViolation => 7,
            Self::SramCrossStreamPageSharing => 8,
            Self::SramCanonicalSortDrift => 9,
            Self::SramReportRoundTripFailed => 10,
            Self::SramSectionRoleLeaked => 11,
            Self::SramSchedulingFieldLeaked => 12,
            Self::SramRepairProvenanceForbidden => 13,
            Self::SramTargetProfileLayoutUnsupported => 14,
            Self::SramPolicyProjectionMismatch => 15,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self.number() {
            1 => "SRAM-001",
            2 => "SRAM-002",
            3 => "SRAM-003",
            4 => "SRAM-004",
            5 => "SRAM-005",
            6 => "SRAM-006",
            7 => "SRAM-007",
            8 => "SRAM-008",
            9 => "SRAM-009",
            10 => "SRAM-010",
            11 => "SRAM-011",
            12 => "SRAM-012",
            13 => "SRAM-013",
            14 => "SRAM-014",
            15 => "SRAM-015",
            _ => unreachable!(),
        }
    }

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::SramInputHashMismatch => "SramInputHashMismatch",
            Self::SramCommitGroupCrossStream => "SramCommitGroupCrossStream",
            Self::SramPageOverflow => "SramPageOverflow",
            Self::SramPageGeometryMismatch => "SramPageGeometryMismatch",
            Self::SramBudgetExceeded => "SramBudgetExceeded",
            Self::SramResidencyUnresolved => "SramResidencyUnresolved",
            Self::SramYieldResumeResidencyViolation => "SramYieldResumeResidencyViolation",
            Self::SramCrossStreamPageSharing => "SramCrossStreamPageSharing",
            Self::SramCanonicalSortDrift => "SramCanonicalSortDrift",
            Self::SramReportRoundTripFailed => "SramReportRoundTripFailed",
            Self::SramSectionRoleLeaked => "SramSectionRoleLeaked",
            Self::SramSchedulingFieldLeaked => "SramSchedulingFieldLeaked",
            Self::SramRepairProvenanceForbidden => "SramRepairProvenanceForbidden",
            Self::SramTargetProfileLayoutUnsupported => "SramTargetProfileLayoutUnsupported",
            Self::SramPolicyProjectionMismatch => "SramPolicyProjectionMismatch",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum SramPagePlanDiagnosticProvenance {
    HashMismatch {
        product: String,
        recorded: Hash256,
        computed: Hash256,
    },
    Binding {
        invariant: String,
        binding_id: u32,
    },
    CommitGroup {
        invariant: String,
        commit_group_id: u32,
        sequence_streams: Vec<u32>,
    },
    Page {
        invariant: String,
        page: u8,
        observed_bytes: u32,
        cap_bytes: u32,
    },
    Budget {
        total_bytes: u32,
        cap_bytes: u32,
    },
    Residency {
        invariant: String,
        binding_id: u32,
        residency: String,
    },
    Geometry {
        observed_header_bytes: u16,
        observed_payload_bytes: u32,
        observed_commit_word_bytes: u8,
        observed_alignment: u16,
        expected_header_bytes: u16,
        expected_payload_bytes: u32,
        expected_commit_word_bytes: u8,
        expected_alignment: u16,
    },
    JsonPath {
        json_path: String,
        field_or_tag: String,
    },
    PolicyProjection {
        field: String,
        detail: String,
    },
    TargetProfileLayout {
        target_profile_hash: Hash256,
        detail: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum RomWindowPlanDiagnosticCode {
    RomInputHashMismatch,
    RomBankCapacityExceeded,
    RomBankSwitchBudgetExceeded,
    RomProfileViolation,
    RomCanonicalSortDrift,
    RomSectionRoleLeaked,
    RomSchedulingFieldLeaked,
    RomRepairProvenanceForbidden,
    RomTargetProfileLayoutUnsupported,
    RomPolicyProjectionMismatch,
    RomMultipleSwitchableBanksDemandedInPhase,
    RomBank0OverBudget,
    RomOverlayDemandExceedsWramReservation,
}

impl RomWindowPlanDiagnosticCode {
    pub const ALL: [Self; 13] = [
        Self::RomInputHashMismatch,
        Self::RomBankCapacityExceeded,
        Self::RomBankSwitchBudgetExceeded,
        Self::RomProfileViolation,
        Self::RomCanonicalSortDrift,
        Self::RomSectionRoleLeaked,
        Self::RomSchedulingFieldLeaked,
        Self::RomRepairProvenanceForbidden,
        Self::RomTargetProfileLayoutUnsupported,
        Self::RomPolicyProjectionMismatch,
        Self::RomMultipleSwitchableBanksDemandedInPhase,
        Self::RomBank0OverBudget,
        Self::RomOverlayDemandExceedsWramReservation,
    ];

    #[must_use]
    pub const fn number(self) -> u16 {
        match self {
            Self::RomInputHashMismatch => 1,
            Self::RomBankCapacityExceeded => 2,
            Self::RomBankSwitchBudgetExceeded => 3,
            Self::RomProfileViolation => 4,
            Self::RomCanonicalSortDrift => 5,
            Self::RomSectionRoleLeaked => 6,
            Self::RomSchedulingFieldLeaked => 7,
            Self::RomRepairProvenanceForbidden => 8,
            Self::RomTargetProfileLayoutUnsupported => 9,
            Self::RomPolicyProjectionMismatch => 10,
            Self::RomMultipleSwitchableBanksDemandedInPhase => 11,
            Self::RomBank0OverBudget => 12,
            Self::RomOverlayDemandExceedsWramReservation => 13,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self.number() {
            1 => "ROM-001",
            2 => "ROM-002",
            3 => "ROM-003",
            4 => "ROM-004",
            5 => "ROM-005",
            6 => "ROM-006",
            7 => "ROM-007",
            8 => "ROM-008",
            9 => "ROM-009",
            10 => "ROM-010",
            11 => "ROM-011",
            12 => "ROM-012",
            13 => "ROM-013",
            _ => unreachable!(),
        }
    }

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::RomInputHashMismatch => "RomInputHashMismatch",
            Self::RomBankCapacityExceeded => "RomBankCapacityExceeded",
            Self::RomBankSwitchBudgetExceeded => "RomBankSwitchBudgetExceeded",
            Self::RomProfileViolation => "RomProfileViolation",
            Self::RomCanonicalSortDrift => "RomCanonicalSortDrift",
            Self::RomSectionRoleLeaked => "RomSectionRoleLeaked",
            Self::RomSchedulingFieldLeaked => "RomSchedulingFieldLeaked",
            Self::RomRepairProvenanceForbidden => "RomRepairProvenanceForbidden",
            Self::RomTargetProfileLayoutUnsupported => "RomTargetProfileLayoutUnsupported",
            Self::RomPolicyProjectionMismatch => "RomPolicyProjectionMismatch",
            Self::RomMultipleSwitchableBanksDemandedInPhase => {
                "RomMultipleSwitchableBanksDemandedInPhase"
            }
            Self::RomBank0OverBudget => "RomBank0OverBudget",
            Self::RomOverlayDemandExceedsWramReservation => {
                "RomOverlayDemandExceedsWramReservation"
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum RomWindowPlanDiagnosticProvenance {
    HashMismatch {
        product: String,
        recorded: Hash256,
        computed: Hash256,
    },
    Binding {
        invariant: String,
        binding_id: u32,
    },
    Kernel {
        invariant: String,
        kernel: String,
    },
    Lut {
        invariant: String,
        lut: String,
    },
    Bank {
        bank: u16,
        observed_bytes: u32,
        cap_bytes: u32,
    },
    Budget {
        decision_value: u16,
        upper_bound: u16,
        cap: u16,
    },
    Phase {
        epoch: u32,
        demanded_banks: Vec<u16>,
    },
    Bank0Demand {
        total_kernel_bytes: u32,
        total_lut_bytes: u32,
        bank0_cap_bytes: u32,
    },
    OverlayDemand {
        declared_bytes: u32,
        wram_reserved_bytes: u32,
    },
    JsonPath {
        json_path: String,
        field_or_tag: String,
    },
    PolicyProjection {
        field: String,
        detail: String,
    },
    TargetProfileLayout {
        target_profile_hash: Hash256,
        detail: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
#[allow(clippy::enum_variant_names)]
pub enum OverlayPlanDiagnosticCode {
    OverlayInputHashMismatch,
    OverlayWramOverlayCapExceeded,
    OverlayRegionPayloadExceedsRegionCap,
    OverlayRegionEmptyButPopulated,
    OverlayRegionIdDuplicate,
    OverlayShareClassEvictionUndefined,
    OverlayInstallSourceNotVisible,
    OverlayInstallEventDefaultMissing,
    OverlayCandidateNotInstalled,
    OverlayInstallReferencesUnknownRegion,
    OverlayInstallReferencesUnknownMember,
    OverlayLeaseShapeIncomplete,
    OverlayMemberPayloadExceedsRegion,
    OverlayCanonicalSortDrift,
    OverlayReportRoundTripFailed,
    OverlaySectionRoleLeaked,
    OverlaySchedulingFieldLeaked,
    OverlayRepairProvenanceForbidden,
    OverlayResolvedPolicyProjectionMismatch,
    OverlayTargetProfileLayoutUnsupported,
    OverlayNoCandidatesButReservationDeclared,
}

impl OverlayPlanDiagnosticCode {
    pub const ALL: [Self; 21] = [
        Self::OverlayInputHashMismatch,
        Self::OverlayWramOverlayCapExceeded,
        Self::OverlayRegionPayloadExceedsRegionCap,
        Self::OverlayRegionEmptyButPopulated,
        Self::OverlayRegionIdDuplicate,
        Self::OverlayShareClassEvictionUndefined,
        Self::OverlayInstallSourceNotVisible,
        Self::OverlayInstallEventDefaultMissing,
        Self::OverlayCandidateNotInstalled,
        Self::OverlayInstallReferencesUnknownRegion,
        Self::OverlayInstallReferencesUnknownMember,
        Self::OverlayLeaseShapeIncomplete,
        Self::OverlayMemberPayloadExceedsRegion,
        Self::OverlayCanonicalSortDrift,
        Self::OverlayReportRoundTripFailed,
        Self::OverlaySectionRoleLeaked,
        Self::OverlaySchedulingFieldLeaked,
        Self::OverlayRepairProvenanceForbidden,
        Self::OverlayResolvedPolicyProjectionMismatch,
        Self::OverlayTargetProfileLayoutUnsupported,
        Self::OverlayNoCandidatesButReservationDeclared,
    ];

    #[must_use]
    pub const fn number(self) -> u16 {
        match self {
            Self::OverlayInputHashMismatch => 1,
            Self::OverlayWramOverlayCapExceeded => 2,
            Self::OverlayRegionPayloadExceedsRegionCap => 3,
            Self::OverlayRegionEmptyButPopulated => 4,
            Self::OverlayRegionIdDuplicate => 5,
            Self::OverlayShareClassEvictionUndefined => 6,
            Self::OverlayInstallSourceNotVisible => 7,
            Self::OverlayInstallEventDefaultMissing => 8,
            Self::OverlayCandidateNotInstalled => 9,
            Self::OverlayInstallReferencesUnknownRegion => 10,
            Self::OverlayInstallReferencesUnknownMember => 11,
            Self::OverlayLeaseShapeIncomplete => 12,
            Self::OverlayMemberPayloadExceedsRegion => 13,
            Self::OverlayCanonicalSortDrift => 14,
            Self::OverlayReportRoundTripFailed => 15,
            Self::OverlaySectionRoleLeaked => 16,
            Self::OverlaySchedulingFieldLeaked => 17,
            Self::OverlayRepairProvenanceForbidden => 18,
            Self::OverlayResolvedPolicyProjectionMismatch => 19,
            Self::OverlayTargetProfileLayoutUnsupported => 20,
            Self::OverlayNoCandidatesButReservationDeclared => 21,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self.number() {
            1 => "OP-001",
            2 => "OP-002",
            3 => "OP-003",
            4 => "OP-004",
            5 => "OP-005",
            6 => "OP-006",
            7 => "OP-007",
            8 => "OP-008",
            9 => "OP-009",
            10 => "OP-010",
            11 => "OP-011",
            12 => "OP-012",
            13 => "OP-013",
            14 => "OP-014",
            15 => "OP-015",
            16 => "OP-016",
            17 => "OP-017",
            18 => "OP-018",
            19 => "OP-019",
            20 => "OP-020",
            21 => "OP-021",
            _ => unreachable!(),
        }
    }

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::OverlayInputHashMismatch => "OverlayInputHashMismatch",
            Self::OverlayWramOverlayCapExceeded => "OverlayWramOverlayCapExceeded",
            Self::OverlayRegionPayloadExceedsRegionCap => "OverlayRegionPayloadExceedsRegionCap",
            Self::OverlayRegionEmptyButPopulated => "OverlayRegionEmptyButPopulated",
            Self::OverlayRegionIdDuplicate => "OverlayRegionIdDuplicate",
            Self::OverlayShareClassEvictionUndefined => "OverlayShareClassEvictionUndefined",
            Self::OverlayInstallSourceNotVisible => "OverlayInstallSourceNotVisible",
            Self::OverlayInstallEventDefaultMissing => "OverlayInstallEventDefaultMissing",
            Self::OverlayCandidateNotInstalled => "OverlayCandidateNotInstalled",
            Self::OverlayInstallReferencesUnknownRegion => "OverlayInstallReferencesUnknownRegion",
            Self::OverlayInstallReferencesUnknownMember => "OverlayInstallReferencesUnknownMember",
            Self::OverlayLeaseShapeIncomplete => "OverlayLeaseShapeIncomplete",
            Self::OverlayMemberPayloadExceedsRegion => "OverlayMemberPayloadExceedsRegion",
            Self::OverlayCanonicalSortDrift => "OverlayCanonicalSortDrift",
            Self::OverlayReportRoundTripFailed => "OverlayReportRoundTripFailed",
            Self::OverlaySectionRoleLeaked => "OverlaySectionRoleLeaked",
            Self::OverlaySchedulingFieldLeaked => "OverlaySchedulingFieldLeaked",
            Self::OverlayRepairProvenanceForbidden => "OverlayRepairProvenanceForbidden",
            Self::OverlayResolvedPolicyProjectionMismatch => {
                "OverlayResolvedPolicyProjectionMismatch"
            }
            Self::OverlayTargetProfileLayoutUnsupported => "OverlayTargetProfileLayoutUnsupported",
            Self::OverlayNoCandidatesButReservationDeclared => {
                "OverlayNoCandidatesButReservationDeclared"
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum OverlayPlanDiagnosticProvenance {
    HashMismatch {
        product: String,
        recorded: Hash256,
        computed: Hash256,
    },
    Region {
        invariant: String,
        region_id: u32,
    },
    Reservation {
        total_bytes: u32,
        cap_bytes: u32,
    },
    Member {
        invariant: String,
        member: String,
        payload_bytes: u32,
        region_bytes: u16,
    },
    Install {
        invariant: String,
        install_id: u32,
    },
    JsonPath {
        json_path: String,
        field_or_tag: String,
    },
    PolicyProjection {
        field: String,
        detail: String,
    },
    TargetProfileLayout {
        target_profile_hash: Hash256,
        detail: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
#[allow(clippy::enum_variant_names)]
pub enum ArenaPlanDiagnosticCode {
    ArenaInputHashMismatch,
    ArenaAllocationFailed,
    ArenaCapacityExceeded,
    ArenaUnmappedStorageClass,
    ArenaLifetimeClassMismatch,
    ArenaAliasClassDisagreement,
    ArenaAliasClassMustOverlapDisagreement,
    ArenaSlotIdDuplicate,
    ArenaPersistentPageGeometryMismatch,
    ArenaPersistentPageStreamMismatch,
    ArenaCrossStreamPageSharing,
    ArenaSramSpanForbidden,
    ArenaHarnessLeakDetected,
    ArenaContinuationRecordSizeMismatch,
    ArenaHramOutOfRange,
    ArenaBank0WramOverflow,
    ArenaHramUsableCapExceeded,
    ArenaOverlayReservationOverflow,
    ArenaOverlayReservationUnderflow,
    ArenaOverlayReservationOverlap,
    ArenaOverlayReservationCountMismatch,
    ArenaOverlayReservationCapDrift,
    ArenaCanonicalSortDrift,
    ArenaReportRoundTripFailed,
    ArenaSectionRoleLeaked,
    ArenaSchedulingFieldLeaked,
    ArenaRepairProvenanceForbidden,
    ArenaTargetProfileLayoutUnsupported,
    ArenaPureExpressionAllocated,
    ArenaTraceRingMisplaced,
    ArenaPolicyProjectionMismatch,
    ArenaCertAddressInvariantFailed,
}

impl ArenaPlanDiagnosticCode {
    pub const ALL: [Self; 32] = [
        Self::ArenaInputHashMismatch,
        Self::ArenaAllocationFailed,
        Self::ArenaCapacityExceeded,
        Self::ArenaUnmappedStorageClass,
        Self::ArenaLifetimeClassMismatch,
        Self::ArenaAliasClassDisagreement,
        Self::ArenaAliasClassMustOverlapDisagreement,
        Self::ArenaSlotIdDuplicate,
        Self::ArenaPersistentPageGeometryMismatch,
        Self::ArenaPersistentPageStreamMismatch,
        Self::ArenaCrossStreamPageSharing,
        Self::ArenaSramSpanForbidden,
        Self::ArenaHarnessLeakDetected,
        Self::ArenaContinuationRecordSizeMismatch,
        Self::ArenaHramOutOfRange,
        Self::ArenaBank0WramOverflow,
        Self::ArenaHramUsableCapExceeded,
        Self::ArenaOverlayReservationOverflow,
        Self::ArenaOverlayReservationUnderflow,
        Self::ArenaOverlayReservationOverlap,
        Self::ArenaOverlayReservationCountMismatch,
        Self::ArenaOverlayReservationCapDrift,
        Self::ArenaCanonicalSortDrift,
        Self::ArenaReportRoundTripFailed,
        Self::ArenaSectionRoleLeaked,
        Self::ArenaSchedulingFieldLeaked,
        Self::ArenaRepairProvenanceForbidden,
        Self::ArenaTargetProfileLayoutUnsupported,
        Self::ArenaPureExpressionAllocated,
        Self::ArenaTraceRingMisplaced,
        Self::ArenaPolicyProjectionMismatch,
        Self::ArenaCertAddressInvariantFailed,
    ];

    #[must_use]
    pub const fn number(self) -> u16 {
        match self {
            Self::ArenaInputHashMismatch => 1,
            Self::ArenaAllocationFailed => 2,
            Self::ArenaCapacityExceeded => 3,
            Self::ArenaUnmappedStorageClass => 4,
            Self::ArenaLifetimeClassMismatch => 5,
            Self::ArenaAliasClassDisagreement => 6,
            Self::ArenaAliasClassMustOverlapDisagreement => 7,
            Self::ArenaSlotIdDuplicate => 8,
            Self::ArenaPersistentPageGeometryMismatch => 9,
            Self::ArenaPersistentPageStreamMismatch => 10,
            Self::ArenaCrossStreamPageSharing => 11,
            Self::ArenaSramSpanForbidden => 12,
            Self::ArenaHarnessLeakDetected => 13,
            Self::ArenaContinuationRecordSizeMismatch => 14,
            Self::ArenaHramOutOfRange => 15,
            Self::ArenaBank0WramOverflow => 16,
            Self::ArenaHramUsableCapExceeded => 17,
            Self::ArenaOverlayReservationOverflow => 18,
            Self::ArenaOverlayReservationUnderflow => 19,
            Self::ArenaOverlayReservationOverlap => 20,
            Self::ArenaOverlayReservationCountMismatch => 21,
            Self::ArenaOverlayReservationCapDrift => 22,
            Self::ArenaCanonicalSortDrift => 23,
            Self::ArenaReportRoundTripFailed => 24,
            Self::ArenaSectionRoleLeaked => 25,
            Self::ArenaSchedulingFieldLeaked => 26,
            Self::ArenaRepairProvenanceForbidden => 27,
            Self::ArenaTargetProfileLayoutUnsupported => 28,
            Self::ArenaPureExpressionAllocated => 29,
            Self::ArenaTraceRingMisplaced => 30,
            Self::ArenaPolicyProjectionMismatch => 31,
            Self::ArenaCertAddressInvariantFailed => 32,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self.number() {
            1 => "ARENA-001",
            2 => "ARENA-002",
            3 => "ARENA-003",
            4 => "ARENA-004",
            5 => "ARENA-005",
            6 => "ARENA-006",
            7 => "ARENA-007",
            8 => "ARENA-008",
            9 => "ARENA-009",
            10 => "ARENA-010",
            11 => "ARENA-011",
            12 => "ARENA-012",
            13 => "ARENA-013",
            14 => "ARENA-014",
            15 => "ARENA-015",
            16 => "ARENA-016",
            17 => "ARENA-017",
            18 => "ARENA-018",
            19 => "ARENA-019",
            20 => "ARENA-020",
            21 => "ARENA-021",
            22 => "ARENA-022",
            23 => "ARENA-023",
            24 => "ARENA-024",
            25 => "ARENA-025",
            26 => "ARENA-026",
            27 => "ARENA-027",
            28 => "ARENA-028",
            29 => "ARENA-029",
            30 => "ARENA-030",
            31 => "ARENA-031",
            32 => "ARENA-032",
            _ => unreachable!(),
        }
    }

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::ArenaInputHashMismatch => "ArenaInputHashMismatch",
            Self::ArenaAllocationFailed => "ArenaAllocationFailed",
            Self::ArenaCapacityExceeded => "ArenaCapacityExceeded",
            Self::ArenaUnmappedStorageClass => "ArenaUnmappedStorageClass",
            Self::ArenaLifetimeClassMismatch => "ArenaLifetimeClassMismatch",
            Self::ArenaAliasClassDisagreement => "ArenaAliasClassDisagreement",
            Self::ArenaAliasClassMustOverlapDisagreement => {
                "ArenaAliasClassMustOverlapDisagreement"
            }
            Self::ArenaSlotIdDuplicate => "ArenaSlotIdDuplicate",
            Self::ArenaPersistentPageGeometryMismatch => "ArenaPersistentPageGeometryMismatch",
            Self::ArenaPersistentPageStreamMismatch => "ArenaPersistentPageStreamMismatch",
            Self::ArenaCrossStreamPageSharing => "ArenaCrossStreamPageSharing",
            Self::ArenaSramSpanForbidden => "ArenaSramSpanForbidden",
            Self::ArenaHarnessLeakDetected => "ArenaHarnessLeakDetected",
            Self::ArenaContinuationRecordSizeMismatch => "ArenaContinuationRecordSizeMismatch",
            Self::ArenaHramOutOfRange => "ArenaHramOutOfRange",
            Self::ArenaBank0WramOverflow => "ArenaBank0WramOverflow",
            Self::ArenaHramUsableCapExceeded => "ArenaHramUsableCapExceeded",
            Self::ArenaOverlayReservationOverflow => "ArenaOverlayReservationOverflow",
            Self::ArenaOverlayReservationUnderflow => "ArenaOverlayReservationUnderflow",
            Self::ArenaOverlayReservationOverlap => "ArenaOverlayReservationOverlap",
            Self::ArenaOverlayReservationCountMismatch => "ArenaOverlayReservationCountMismatch",
            Self::ArenaOverlayReservationCapDrift => "ArenaOverlayReservationCapDrift",
            Self::ArenaCanonicalSortDrift => "ArenaCanonicalSortDrift",
            Self::ArenaReportRoundTripFailed => "ArenaReportRoundTripFailed",
            Self::ArenaSectionRoleLeaked => "ArenaSectionRoleLeaked",
            Self::ArenaSchedulingFieldLeaked => "ArenaSchedulingFieldLeaked",
            Self::ArenaRepairProvenanceForbidden => "ArenaRepairProvenanceForbidden",
            Self::ArenaTargetProfileLayoutUnsupported => "ArenaTargetProfileLayoutUnsupported",
            Self::ArenaPureExpressionAllocated => "ArenaPureExpressionAllocated",
            Self::ArenaTraceRingMisplaced => "ArenaTraceRingMisplaced",
            Self::ArenaPolicyProjectionMismatch => "ArenaPolicyProjectionMismatch",
            Self::ArenaCertAddressInvariantFailed => "ArenaCertAddressInvariantFailed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ArenaPlanDiagnosticProvenance {
    HashMismatch {
        product: String,
        recorded: Hash256,
        computed: Hash256,
    },
    Binding {
        invariant: String,
        binding_id: u32,
    },
    AliasClass {
        invariant: String,
        alias_class_id: u32,
    },
    Arena {
        invariant: String,
        arena_id: u32,
        named: String,
    },
    Slot {
        invariant: String,
        slot_id: u32,
        observed_bytes: u32,
        cap_bytes: u32,
    },
    Reservation {
        invariant: String,
        total_bytes: u32,
        expected_bytes: u32,
    },
    Geometry {
        observed_header_bytes: u16,
        observed_payload_bytes: u32,
        observed_commit_word_bytes: u8,
        observed_alignment: u16,
        expected_header_bytes: u16,
        expected_payload_bytes: u32,
        expected_commit_word_bytes: u8,
        expected_alignment: u16,
    },
    JsonPath {
        json_path: String,
        field_or_tag: String,
    },
    PolicyProjection {
        field: String,
        detail: String,
    },
    TargetProfileLayout {
        target_profile_hash: Hash256,
        detail: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "fields", deny_unknown_fields)]
pub enum BudgetFailure {
    MissingRuntimeChromeBudget,
    QuantGraphBudgetViewMalformed {
        field: FieldPath,
    },
    ExpertExceedsSlot {
        layer: LayerId,
        expert: ExpertId,
        slot: BudgetSlotId,
        payload_bytes: u32,
        cap_bytes: u32,
        excess_bytes: u32,
    },
    CommonBankExceedsCap {
        assigned_bytes: u32,
        cap_bytes: u32,
        excess_bytes: u32,
    },
    WramPeakExceedsCap {
        peak: u32,
        cap: u32,
    },
    SramPeakExceedsCap {
        peak: u32,
        cap: u32,
    },
    HramPeakExceedsCap {
        peak: u32,
        cap: u32,
    },
    AccumulatorExceedsI32 {
        site: ReductionSiteId,
        projected_max_abs: u64,
    },
    BankSwitchesPerTokenOverCap {
        decision_value: u16,
        upper_bound: u16,
        cap: u16,
        source: SwitchProjectionSource,
    },
    SramPageSwitchesPerTokenOverCap {
        decision_value: u16,
        upper_bound: u16,
        cap: u16,
        source: SwitchProjectionSource,
    },
    PlacementProfileInfeasible {
        profile: PlacementProfile,
        reason: PlacementInfeasibilityReason,
    },
}

impl BudgetFailure {
    #[must_use]
    pub fn validation_code(&self) -> ValidationCode {
        match self {
            Self::MissingRuntimeChromeBudget => ValidationCode::BudgetMissingRuntimeChromeBudget,
            Self::QuantGraphBudgetViewMalformed { field } => {
                ValidationCode::BudgetQuantGraphViewMalformed {
                    field: field.clone(),
                }
            }
            Self::ExpertExceedsSlot {
                layer,
                expert,
                slot,
                payload_bytes,
                cap_bytes,
                excess_bytes,
            } => ValidationCode::BudgetExpertExceedsSlot {
                layer: *layer,
                expert: *expert,
                slot: *slot,
                payload_bytes: *payload_bytes,
                cap_bytes: *cap_bytes,
                excess_bytes: *excess_bytes,
            },
            Self::CommonBankExceedsCap {
                assigned_bytes,
                cap_bytes,
                ..
            } => ValidationCode::BudgetCommonBankExceedsCap {
                assigned_bytes: *assigned_bytes,
                cap_bytes: *cap_bytes,
            },
            Self::WramPeakExceedsCap { peak, cap } => ValidationCode::BudgetWramPeakExceeds {
                peak: *peak,
                cap: *cap,
            },
            Self::SramPeakExceedsCap { peak, cap } => ValidationCode::BudgetSramPeakExceeds {
                peak: *peak,
                cap: *cap,
            },
            Self::HramPeakExceedsCap { peak, cap } => ValidationCode::BudgetHramPeakExceeds {
                peak: *peak,
                cap: *cap,
            },
            Self::AccumulatorExceedsI32 {
                site,
                projected_max_abs,
            } => ValidationCode::BudgetAccumulatorOverflow {
                site: site.clone(),
                projected_max_abs: *projected_max_abs,
            },
            Self::BankSwitchesPerTokenOverCap {
                decision_value,
                upper_bound,
                cap,
                source,
            } => ValidationCode::BudgetSwitchesPerTokenOverCap {
                decision_value: *decision_value,
                upper_bound: *upper_bound,
                cap: *cap,
                source: *source,
            },
            Self::SramPageSwitchesPerTokenOverCap {
                decision_value,
                upper_bound,
                cap,
                source,
            } => ValidationCode::BudgetSramPageSwitchesPerTokenOverCap {
                decision_value: *decision_value,
                upper_bound: *upper_bound,
                cap: *cap,
                source: *source,
            },
            Self::PlacementProfileInfeasible { profile, reason } => {
                ValidationCode::BudgetPlacementProfileInfeasible {
                    profile: *profile,
                    reason: reason.clone(),
                }
            }
        }
    }

    #[must_use]
    pub fn diagnostic_detail(&self) -> ValidationDetail {
        match self {
            Self::MissingRuntimeChromeBudget => ValidationDetail::Field {
                field: FieldPath::from("runtime_chrome_budget"),
            },
            Self::QuantGraphBudgetViewMalformed { field } => ValidationDetail::Field {
                field: field.clone(),
            },
            _ => ValidationDetail::Selector {
                selector: self
                    .diagnostic_selector()
                    .expect("budget failure has selector"),
            },
        }
    }

    #[must_use]
    pub fn diagnostic_selector(&self) -> Option<SelectorPath> {
        let selector = match self {
            Self::MissingRuntimeChromeBudget | Self::QuantGraphBudgetViewMalformed { .. } => {
                return None;
            }
            Self::ExpertExceedsSlot {
                layer,
                expert,
                slot,
                ..
            } => format!(
                "budget.expert[layer={},expert={},slot={}]",
                layer, expert, slot
            ),
            Self::CommonBankExceedsCap { .. } => "budget.common_bank".to_owned(),
            Self::WramPeakExceedsCap { .. } => "budget.memory.wram".to_owned(),
            Self::SramPeakExceedsCap { .. } => "budget.memory.sram".to_owned(),
            Self::HramPeakExceedsCap { .. } => "budget.memory.hram".to_owned(),
            Self::AccumulatorExceedsI32 { site, .. } => {
                format!("budget.accumulator[site={}]", site.0.as_str())
            }
            Self::BankSwitchesPerTokenOverCap { .. } => "budget.switches.bank_per_token".to_owned(),
            Self::SramPageSwitchesPerTokenOverCap { .. } => {
                "budget.switches.sram_page_per_token".to_owned()
            }
            Self::PlacementProfileInfeasible { profile, reason } => {
                format!(
                    "budget.placement[profile={},reason={}]",
                    placement_profile_selector_tag(*profile),
                    placement_infeasibility_reason_selector_tag(reason)
                )
            }
        };
        Some(SelectorPath(selector))
    }

    #[must_use]
    pub fn diagnostic(&self) -> ValidationDiagnostic {
        budget_failure_diagnostic(self)
    }
}

#[must_use]
pub fn budget_failure_validation_code(failure: &BudgetFailure) -> ValidationCode {
    failure.validation_code()
}

/// Canonical taxonomy helper for Stage 2 budget producers that already have
/// typed evidence. Use this rather than rebuilding the code/detail mapping at
/// call sites, so the `BudgetFailure` <-> `ValidationDiagnostic` invariant
/// stays centralized.
#[must_use]
pub fn budget_failure_diagnostic_with_provenance(
    failure: &BudgetFailure,
    provenance: Vec<EvidenceRef>,
) -> ValidationDiagnostic {
    ValidationDiagnostic::hard(
        ValidationOrigin::Budget,
        failure.validation_code(),
        failure.diagnostic_detail(),
        provenance,
    )
}

/// Taxonomy-only convenience helper for tests and Stage 2 scaffolding that has
/// not yet attached evidence. Producer beads should prefer
/// [`budget_failure_diagnostic_with_provenance`].
#[must_use]
pub fn budget_failure_diagnostic(failure: &BudgetFailure) -> ValidationDiagnostic {
    budget_failure_diagnostic_with_provenance(failure, Vec::new())
}

#[must_use]
pub fn budget_failure_diagnostics_with_provenance(
    failures: &[BudgetFailure],
    provenance: Vec<EvidenceRef>,
) -> Vec<ValidationDiagnostic> {
    failures
        .iter()
        .map(|failure| budget_failure_diagnostic_with_provenance(failure, provenance.clone()))
        .collect()
}

#[must_use]
pub fn budget_failure_diagnostics(failures: &[BudgetFailure]) -> Vec<ValidationDiagnostic> {
    failures.iter().map(budget_failure_diagnostic).collect()
}

#[must_use]
pub fn budget_failure_matches_diagnostic(
    failure: &BudgetFailure,
    diagnostic: &ValidationDiagnostic,
) -> bool {
    diagnostic.severity == DiagnosticSeverity::Hard
        && diagnostic.origin == ValidationOrigin::Budget
        && diagnostic.code == failure.validation_code()
        && diagnostic.detail == failure.diagnostic_detail()
}

const fn placement_profile_selector_tag(profile: PlacementProfile) -> &'static str {
    match profile {
        PlacementProfile::StrictOnePerBank => "strict_one_per_bank",
        PlacementProfile::Budgeted => "budgeted",
        PlacementProfile::PackedExperts => "packed_experts",
    }
}

fn placement_infeasibility_reason_selector_tag(
    reason: &PlacementInfeasibilityReason,
) -> &'static str {
    match reason {
        PlacementInfeasibilityReason::NoSlotsForClass => "no_slots_for_class",
        PlacementInfeasibilityReason::ExpertCountExceedsSlots => "expert_count_exceeds_slots",
        PlacementInfeasibilityReason::RequiresUnavailableSlotClass => {
            "requires_unavailable_slot_class"
        }
        PlacementInfeasibilityReason::ExceedsCommonBankCap => "exceeds_common_bank_cap",
        PlacementInfeasibilityReason::ExceedsExpertBankCap => "exceeds_expert_bank_cap",
        PlacementInfeasibilityReason::ViolatesTargetLayout => "violates_target_layout",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ValidationDetail {
    None,
    HashMismatch {
        expected: Hash256,
        observed: Hash256,
    },
    Bytes {
        observed: u32,
        cap: u32,
    },
    Range {
        observed_lo: i64,
        observed_hi: i64,
        cap_lo: i64,
        cap_hi: i64,
    },
    Selector {
        selector: SelectorPath,
    },
    Field {
        field: FieldPath,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CompatibilityAdapterId(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TraceProbeId(pub u16);

impl From<TraceProbeId> for gbf_abi::trace::TraceProbeId {
    fn from(id: TraceProbeId) -> Self {
        Self(id.0)
    }
}

impl From<gbf_abi::trace::TraceProbeId> for TraceProbeId {
    fn from(id: gbf_abi::trace::TraceProbeId) -> Self {
        Self(id.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ServiceLevelField {
    MaxFirstTokenCyclesP95,
    MaxCheckpointGapCyclesP95,
    MaxResumeLatencyCyclesP95,
    MaxUiJitterFramesP99,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum RiskQuantileField {
    CycleQuantile,
    SwitchQuantile,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ObjectiveRejection {
    ServiceLevelZero { field: ServiceLevelField },
    MaxCyclesPerTokenZero,
    MaxRomBytesZero,
    MaxBankSwitchesPerTokenZero,
    MaxSramPageSwitchesPerTokenZero,
    RiskQuantileInvalid { field: RiskQuantileField, value: u8 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum TargetIncompatibilityReason {
    TargetFamilyMismatch,
    MissingLoweringProfile,
    UnsupportedRuntimeMode,
    UnsupportedCompilerFeature,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KnobValueDescriptor {
    pub value: ConstraintValue,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ReductionSiteId(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum SwitchProjectionSource {
    ConservativeStaticUpperBound,
    HintWeightedExpectedWithStaticCap,
    CalibrationClosedFormWithStaticCap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum StaticFitInterpretation {
    /// All Stage-2 necessary static checks passed. F-B10, F-B12, F-B13,
    /// and final layout remain authoritative for final deployability.
    PassesNecessaryStaticChecks,
    /// At least one Stage-2 necessary static check failed, so later passes
    /// cannot make the build valid without a policy/input change.
    FailsNecessaryStaticChecks,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum PlacementInfeasibilityReason {
    NoSlotsForClass,
    ExpertCountExceedsSlots,
    RequiresUnavailableSlotClass,
    ExceedsCommonBankCap,
    ExceedsExpertBankCap,
    ViolatesTargetLayout,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile::canonical_default_bounds_fixture;
    use gbf_foundation::BlobCodec;

    fn hash(byte: u8) -> Hash256 {
        Hash256::from_bytes([byte; 32])
    }

    fn provenance() -> Vec<EvidenceRef> {
        vec![EvidenceRef {
            kind: "Fixture".to_owned(),
            reference: "diagnostics".to_owned(),
            hash: Some(hash(9)),
        }]
    }

    fn diagnostic(code: ValidationCode) -> ValidationDiagnostic {
        ValidationDiagnostic::hard(
            ValidationOrigin::PolicyResolution,
            code,
            ValidationDetail::None,
            provenance(),
        )
    }

    fn assert_diagnostic_round_trip(diagnostic: ValidationDiagnostic) {
        let encoded = serde_json::to_string(&diagnostic).expect("diagnostic serializes");
        let decoded: ValidationDiagnostic =
            serde_json::from_str(&encoded).expect("diagnostic deserializes");

        assert_eq!(decoded, diagnostic);
    }

    fn assert_code_round_trip(code: ValidationCode) {
        assert_diagnostic_round_trip(diagnostic(code));
    }

    #[test]
    fn validation_diagnostic_round_trips_through_serde() {
        assert_diagnostic_round_trip(ValidationDiagnostic::new(
            DiagnosticSeverity::Soft,
            ValidationOrigin::Schema,
            ValidationCode::SchemaEpochUnsupported,
            ValidationDetail::Field {
                field: FieldPath::from("schema.epoch"),
            },
            provenance(),
        ));
    }

    #[test]
    fn validation_diagnostic_pins_public_json_shape() {
        let diagnostic = ValidationDiagnostic::hard(
            ValidationOrigin::Manifest,
            ValidationCode::ManifestInvariantViolated {
                invariant: ManifestInvariant::ForbiddenBuildIdentityField {
                    field: FieldPath::from("manifest.build_identity"),
                },
            },
            ValidationDetail::Field {
                field: FieldPath::from("manifest.build_identity"),
            },
            provenance(),
        );

        assert_eq!(
            serde_json::to_value(diagnostic).expect("diagnostic serializes"),
            serde_json::json!({
                "severity": { "kind": "Hard" },
                "origin": { "kind": "Manifest" },
                "code": {
                    "kind": "ManifestInvariantViolated",
                    "fields": {
                        "invariant": {
                            "kind": "ForbiddenBuildIdentityField",
                            "field": "manifest.build_identity"
                        }
                    }
                },
                "detail": {
                    "kind": "Field",
                    "field": "manifest.build_identity"
                },
                "provenance": [
                    {
                        "kind": "Fixture",
                        "reference": "diagnostics",
                        "hash": "sha256:0909090909090909090909090909090909090909090909090909090909090909"
                    }
                ]
            })
        );
    }

    #[test]
    fn trace_probe_id_converts_to_and_from_abi_runtime_id() {
        let policy_id = TraceProbeId(42);
        let abi_id: gbf_abi::trace::TraceProbeId = policy_id.into();
        let round_tripped: TraceProbeId = abi_id.into();

        assert_eq!(abi_id.0, 42);
        assert_eq!(round_tripped, policy_id);
    }

    #[test]
    fn trace_probe_id_conversion_preserves_edge_values() {
        for raw in [0, 1, u16::MAX] {
            let policy_id = TraceProbeId(raw);
            let abi_id: gbf_abi::trace::TraceProbeId = policy_id.into();
            let round_tripped: TraceProbeId = abi_id.into();

            assert_eq!(abi_id.0, raw);
            assert_eq!(round_tripped, policy_id);
        }
    }

    #[test]
    fn trace_probe_id_json_shape_is_u16() {
        assert_eq!(
            serde_json::to_value(TraceProbeId(u16::MAX)).expect("id serializes"),
            serde_json::json!(65_535_u16)
        );
        assert_eq!(
            serde_json::from_value::<TraceProbeId>(serde_json::json!(0)).expect("id deserializes"),
            TraceProbeId(0)
        );
        assert!(serde_json::from_value::<TraceProbeId>(serde_json::json!(65_536_u64)).is_err());
        assert!(serde_json::from_value::<TraceProbeId>(serde_json::json!(-1)).is_err());
    }

    #[test]
    fn validation_detail_round_trips_through_serde() {
        for detail in [
            ValidationDetail::None,
            ValidationDetail::HashMismatch {
                expected: hash(1),
                observed: hash(2),
            },
            ValidationDetail::Bytes {
                observed: 17,
                cap: 11,
            },
            ValidationDetail::Range {
                observed_lo: -3,
                observed_hi: 14,
                cap_lo: 0,
                cap_hi: 10,
            },
            ValidationDetail::Selector {
                selector: SelectorPath("experts[0]".to_owned()),
            },
            ValidationDetail::Field {
                field: FieldPath::from("manifest.lineage"),
            },
        ] {
            let encoded = serde_json::to_string(&detail).expect("detail serializes");
            let decoded: ValidationDetail =
                serde_json::from_str(&encoded).expect("detail deserializes");

            assert_eq!(decoded, detail);
        }
    }

    #[test]
    fn validation_detail_pins_selector_and_field_keys() {
        assert_eq!(
            serde_json::to_value(ValidationDetail::Selector {
                selector: SelectorPath("experts[0]".to_owned()),
            })
            .expect("selector detail serializes"),
            serde_json::json!({
                "kind": "Selector",
                "selector": "experts[0]"
            })
        );
        assert_eq!(
            serde_json::to_value(ValidationDetail::Field {
                field: FieldPath::from("manifest.lineage"),
            })
            .expect("field detail serializes"),
            serde_json::json!({
                "kind": "Field",
                "field": "manifest.lineage"
            })
        );
    }

    #[test]
    fn validation_code_round_trips_every_variant() {
        let versions = (SemVer::new(1, 2, 3), SemVer::new(2, 0, 0));
        let blob = BlobRef {
            hash: hash(3),
            len: 32,
            codec: BlobCodec::Raw,
        };
        let checkpoint = SemanticCheckpointId::from_static("layer.0.post_embedding")
            .expect("checkpoint id is valid");
        let shard = LoweringShardRef {
            id: LoweringShardId("weights.0".to_owned()),
            manifest_hash: hash(4),
        };
        let bounds = canonical_default_bounds_fixture();

        for code in [
            ValidationCode::SchemaEpochUnsupported,
            ValidationCode::SchemaCompatibilityAdapterMissing {
                observed: versions.0,
                target: versions.1,
            },
            ValidationCode::SchemaCompatibilityAdapterNotLossless {
                adapter: CompatibilityAdapterId("adapter.v1".to_owned()),
            },
            ValidationCode::ReportSemanticInvariantViolated {
                field: FieldPath::from("artifact_validation.v1.outcome"),
            },
            ValidationCode::SemanticCoreHashMismatch,
            ValidationCode::ArtifactTransportManifestMismatch,
            ValidationCode::ManifestInvariantViolated {
                invariant: ManifestInvariant::ForbiddenBuildIdentityField {
                    field: FieldPath::from("build.host"),
                },
            },
            ValidationCode::ArtifactPayloadMalformed {
                field: FieldPath::from("core.tensors"),
            },
            ValidationCode::ArtifactBlobDigestMismatch {
                blob,
                expected: hash(1),
                observed: hash(2),
            },
            ValidationCode::ArtifactAuxMalformed {
                field: FieldPath::from("aux.golden_vectors"),
            },
            ValidationCode::ArtifactAuxSidecarMissing {
                kind: SidecarKind::GoldenVector,
            },
            ValidationCode::ArtifactAuxSidecarDigestMismatch {
                kind: SidecarKind::SemanticCheckpointSchema,
                expected: hash(5),
                observed: hash(6),
            },
            ValidationCode::ArtifactForbiddenBuildIdentityField {
                field: FieldPath::from("manifest.build_identity"),
            },
            ValidationCode::ArtifactRequiredFeatureUnsupported {
                feature: ArtifactFeature::MoeRouting,
            },
            ValidationCode::LoweringMissingForTarget {
                target: TargetProfileId::from("dmg-mbc5"),
                lowering_profile: DataLoweringProfileId("dmg-default".to_owned()),
            },
            ValidationCode::LoweringRoundTripFailed {
                shard: shard.clone(),
            },
            ValidationCode::LoweringPackerVersionMismatch {
                artifact_version: PackerVersion::new(1, 0, 0),
                runtime_version: PackerVersion::new(2, 0, 0),
            },
            ValidationCode::CalibrationMissing {
                class: CalibrationLayer::Kernel,
            },
            ValidationCode::CalibrationStale {
                class: CalibrationLayer::Platform,
                declared: hash(7),
                observed: hash(8),
            },
            ValidationCode::CalibrationConfidenceTooLow {
                required: CalibrationConfidenceClass::Reasonable,
                observed: CalibrationConfidenceClass::Weak,
            },
            ValidationCode::HintProvenanceInconsistent {
                fact: TraceProbeId(2),
            },
            ValidationCode::WorkloadRefUnresolved {
                workload: WorkloadId::from("smoke"),
            },
            ValidationCode::GoldenVectorMissing {
                vector: GoldenVectorId("vec.smoke.001".to_owned()),
            },
            ValidationCode::GoldenVectorDigestMismatch {
                vector: GoldenVectorId("vec.smoke.002".to_owned()),
                expected: hash(10),
                observed: hash(11),
            },
            ValidationCode::CompileRequestUnsupportedFeature {
                feature: CompilerFeature::StaticBudgetReport,
            },
            ValidationCode::CompileRequestProfileForbidsObjective {
                profile: CompileProfileId::from("Bringup"),
                reason: ObjectiveRejection::ServiceLevelZero {
                    field: ServiceLevelField::MaxFirstTokenCyclesP95,
                },
            },
            ValidationCode::CompileRequestRuntimeModeUnsupported {
                mode: RuntimeMode::Trace,
            },
            ValidationCode::CompileRequestTargetIncompatible {
                target: TargetProfileId::from("gbc-mbc5"),
                reason: TargetIncompatibilityReason::MissingLoweringProfile,
            },
            ValidationCode::PolicyKnobOutOfBounds {
                knob: CompileKnobId::Placement,
                requested: KnobValueDescriptor {
                    value: ConstraintValue::PlacementProfile {
                        value: PlacementProfile::PackedExperts,
                    },
                },
                bounds: bounds.clone(),
            },
            ValidationCode::PolicyConstraintUnsatisfiable {
                knob: CompileKnobId::Schedule,
                left: bounds.clone(),
                right: bounds.clone(),
            },
            ValidationCode::PolicyConstraintLoosened {
                knob: CompileKnobId::Placement,
                previous: bounds.clone(),
                requested: bounds.clone(),
            },
            ValidationCode::PolicyHintConstraintUnsupported {
                knob: CompileKnobId::Schedule,
                value: ConstraintValue::U32 { value: 17 },
            },
            ValidationCode::PolicyKnobLockedAndOverridden {
                knob: CompileKnobId::RomWindow,
            },
            ValidationCode::BudgetMissingRuntimeChromeBudget,
            ValidationCode::BudgetQuantGraphViewMalformed {
                field: FieldPath::from("quant_graph.layers[0]"),
            },
            ValidationCode::BudgetExpertExceedsSlot {
                layer: LayerId::new(1),
                expert: ExpertId::new(2),
                slot: BudgetSlotId::new(3),
                payload_bytes: 9000,
                cap_bytes: 8192,
                excess_bytes: 808,
            },
            ValidationCode::BudgetCommonBankExceedsCap {
                assigned_bytes: 20_000,
                cap_bytes: 16_384,
            },
            ValidationCode::BudgetWramPeakExceeds {
                peak: 5000,
                cap: 4096,
            },
            ValidationCode::BudgetSramPeakExceeds {
                peak: 9000,
                cap: 8192,
            },
            ValidationCode::BudgetHramPeakExceeds {
                peak: 256,
                cap: 127,
            },
            ValidationCode::BudgetAccumulatorOverflow {
                site: ReductionSiteId("ffn.0.acc".to_owned()),
                projected_max_abs: i32::MAX as u64 + 1,
            },
            ValidationCode::InferIrRouterPresentForDenseLayer {
                layer: LayerId::new(1),
            },
            ValidationCode::InferIrRouterMatVecMissingForRoutedLayer {
                layer: LayerId::new(2),
            },
            ValidationCode::InferIrSequenceSemanticsUnsupportedV1 {
                field: FieldPath::from("sequence_semantics.state_slots"),
            },
            ValidationCode::InferIrEmbeddingNotUnique {
                field: FieldPath::from("nodes.embedding"),
            },
            ValidationCode::InferIrDecodeNotUnique {
                field: FieldPath::from("nodes.decode"),
            },
            ValidationCode::InferIrClassifyNotUnique {
                field: FieldPath::from("nodes.classify"),
            },
            ValidationCode::InferIrExpertCoverageMismatch {
                layer: LayerId::new(0),
                expert: ExpertId::new(1),
            },
            ValidationCode::InferIrRouteCoverageMismatch {
                layer: LayerId::new(0),
            },
            ValidationCode::InferIrSemanticCheckpointEmittedHere {
                field: FieldPath::from("anchors.semantic_checkpoint"),
            },
            ValidationCode::InferIrEffectChainNotLinear {
                field: FieldPath::from("effects"),
            },
            ValidationCode::InferIrEffectIdEdgeTokenViolation {
                field: FieldPath::from("effects"),
            },
            ValidationCode::InferIrTopologicalOrderMismatch {
                field: FieldPath::from("nodes"),
            },
            ValidationCode::InferIrValueProducerMissing { value_id: 7 },
            ValidationCode::InferIrValueFormatMismatch {
                field: FieldPath::from("values.format"),
            },
            ValidationCode::InferIrNormFormatMismatch {
                field: FieldPath::from("nodes.norm"),
            },
            ValidationCode::InferIrExpertSectionRoleMismatch {
                layer: LayerId::new(3),
                expert: ExpertId::new(4),
            },
            ValidationCode::InferIrNonV1RouterSemantics {
                layer: LayerId::new(5),
            },
            ValidationCode::InferIrDenseRoutedShapeMismatch {
                layer: LayerId::new(6),
            },
            ValidationCode::InferIrDecodePlanMismatch {
                field: FieldPath::from("nodes.decode.plan"),
            },
            ValidationCode::InferIrDecodeRngBindingMismatch {
                field: FieldPath::from("nodes.decode.effects"),
            },
            ValidationCode::InferIrUnexpectedRngEffectOnPureOp {
                field: FieldPath::from("nodes.classify.effects"),
            },
            ValidationCode::InferIrSequenceSlotCoverageMismatch {
                layer: LayerId::new(7),
            },
            ValidationCode::InferIrOpHistogramTotalMismatch {
                field: FieldPath::from("result.op_histogram"),
            },
            ValidationCode::InferIrFaultBoundaryEmittedV1Forbidden {
                field: FieldPath::from("effects"),
            },
            ValidationCode::InferIrResidualBoundaryMismatch {
                field: FieldPath::from("nodes.combine_residual"),
            },
            ValidationCode::InferIrTokenIngressAmbiguous {
                field: FieldPath::from("token_inputs"),
            },
            ValidationCode::InferIrSemanticEquivalenceFailed {
                field: FieldPath::from("semantic_equivalence.fixture"),
            },
            ValidationCode::InferIrCycleDetected {
                field: FieldPath::from("nodes"),
            },
            ValidationCode::InferIrUnreachableNode {
                field: FieldPath::from("nodes"),
            },
            ValidationCode::InferIrDisconnectedComponent {
                field: FieldPath::from("nodes"),
            },
            ValidationCode::InferIrForbiddenStorageMetadata {
                field: FieldPath::from("values.layout"),
            },
            ValidationCode::InferIrSemanticAnchorMissing {
                field: FieldPath::from("anchors"),
            },
            ValidationCode::InferIrFfnActivationMissing {
                layer: LayerId::new(0),
                expert: ExpertId::new(0),
            },
            ValidationCode::InferIrExpertSelectionMissing {
                layer: LayerId::new(0),
            },
            ValidationCode::InferIrGateWeightNotConsumed {
                field: FieldPath::from("values.gate_weight"),
            },
            ValidationCode::InferIrInputTokenValueIdMismatch {
                field: FieldPath::from("token_inputs.value_id"),
            },
            ValidationCode::InferIrReductionSiteMissing {
                field: FieldPath::from("nodes.reduction_site"),
            },
            ValidationCode::InferIrOpSignatureMismatch {
                field: FieldPath::from("nodes.signature"),
            },
            ValidationCode::InferIrRouterScoreOrphaned {
                field: FieldPath::from("values.router_score"),
            },
            ValidationCode::InferIrSequenceStateNextOrphaned {
                field: FieldPath::from("values.sequence_state_next"),
            },
            ValidationCode::ObservationMandatoryCheckpointNotFeasible {
                checkpoint: checkpoint.clone(),
            },
            ValidationCode::ObservationWorkloadCheckpointNotFeasible {
                checkpoint: checkpoint.clone(),
            },
            ValidationCode::ObservationCheckpointNotInSchema {
                checkpoint: checkpoint.clone(),
            },
            ValidationCode::ObservationCheckpointNotAttachable {
                checkpoint: checkpoint.clone(),
            },
            ValidationCode::ObservationCheckpointAmbiguous {
                checkpoint: checkpoint.clone(),
            },
            ValidationCode::ObservationEncodingInvalidForCheckpoint { checkpoint },
            ValidationCode::ObservationScHashMismatch {
                expected: hash(0x12),
                observed: hash(0x13),
            },
            ValidationCode::ObservationDeterminismMismatch {
                field: FieldPath::from("op_policy_projection.determinism_class"),
            },
            ValidationCode::ObservationProbeIdUnknown {
                probe_id: TraceProbeId(99),
            },
            ValidationCode::ObservationMetricIdUnknown {
                metric: MetricId::from_static("metric.unknown").expect("metric id"),
            },
            ValidationCode::ObservationRequiredProbeDisabled {
                probe_id: TraceProbeId(100),
            },
            ValidationCode::ObservationMetricSourceReservedV1 {
                metric: MetricId::from_static("metric.reserved").expect("metric id"),
            },
            ValidationCode::ObservationMetricHistogramBucketCountZero {
                metric: MetricId::from_static("metric.histogram").expect("metric id"),
            },
            ValidationCode::ObservationProbeSourceInvalid {
                probe_id: TraceProbeId(101),
            },
            ValidationCode::ObservationReservedEffectProbe {
                probe_id: TraceProbeId(102),
            },
            ValidationCode::ObservationSequenceStateProbeReserved {
                probe_id: TraceProbeId(103),
            },
            ValidationCode::ObservationFaultBoundaryProbeReserved {
                probe_id: TraceProbeId(104),
            },
            ValidationCode::ObservationProbeClassCapExceeded {
                class: ProbeImportanceClass::Diagnostic,
                observed: 3,
                cap: 2,
            },
            ValidationCode::ObservationInvariantModeBudgetBusted {
                projected_max_events_per_slice: 9,
                projected_max_bytes_per_frame: 128,
                max_events_per_slice: 8,
                max_bytes_per_frame: 64,
            },
            ValidationCode::BudgetSwitchesPerTokenOverCap {
                decision_value: 7,
                upper_bound: 9,
                cap: 5,
                source: SwitchProjectionSource::ConservativeStaticUpperBound,
            },
            ValidationCode::BudgetSramPageSwitchesPerTokenOverCap {
                decision_value: 3,
                upper_bound: 4,
                cap: 2,
                source: SwitchProjectionSource::CalibrationClosedFormWithStaticCap,
            },
            ValidationCode::BudgetPlacementProfileInfeasible {
                profile: PlacementProfile::PackedExperts,
                reason: PlacementInfeasibilityReason::ExceedsExpertBankCap,
            },
        ] {
            assert_code_round_trip(code);
        }
    }

    #[test]
    fn validation_code_pins_unit_and_fielded_json_shapes() {
        assert_eq!(
            serde_json::to_value(ValidationCode::BudgetMissingRuntimeChromeBudget)
                .expect("unit code serializes"),
            serde_json::json!({
                "kind": "BudgetMissingRuntimeChromeBudget"
            })
        );

        assert_eq!(
            serde_json::to_value(ValidationCode::ArtifactAuxSidecarDigestMismatch {
                kind: SidecarKind::SemanticCheckpointSchema,
                expected: hash(5),
                observed: hash(6),
            })
            .expect("fielded code serializes"),
            serde_json::json!({
                "kind": "ArtifactAuxSidecarDigestMismatch",
                "fields": {
                    "kind": { "kind": "SemanticCheckpointSchema" },
                    "expected": "sha256:0505050505050505050505050505050505050505050505050505050505050505",
                    "observed": "sha256:0606060606060606060606060606060606060606060606060606060606060606"
                }
            })
        );

        assert_eq!(
            serde_json::to_value(ValidationCode::InferIrRouterPresentForDenseLayer {
                layer: LayerId::new(7),
            })
            .expect("iir router diagnostic serializes"),
            serde_json::json!({
                "kind": "InferIrRouterPresentForDenseLayer",
                "fields": { "layer": 7 }
            })
        );

        assert_eq!(
            serde_json::to_value(ValidationCode::ReportSemanticInvariantViolated {
                field: FieldPath::from("artifact_validation.v1.compatibility.decision"),
            })
            .expect("report semantic invariant code serializes"),
            serde_json::json!({
                "kind": "ReportSemanticInvariantViolated",
                "fields": {
                    "field": "artifact_validation.v1.compatibility.decision"
                }
            })
        );

        assert_eq!(
            serde_json::to_value(ValidationCode::ObservationCheckpointNotAttachable {
                checkpoint: SemanticCheckpointId::from_static("post_decode")
                    .expect("checkpoint id is valid"),
            })
            .expect("observation diagnostic serializes"),
            serde_json::json!({
                "kind": "ObservationCheckpointNotAttachable",
                "fields": { "checkpoint": "post_decode" }
            })
        );
    }

    #[test]
    fn validation_code_pins_amendment_variant_json_shapes() {
        let bounds = canonical_default_bounds_fixture();
        let mut default_bounds_json = serde_json::json!({
            "placement": {"max_profile": {"kind": "PackedExperts"}},
            "observation": {
                "max_trace_demotion": {"kind": "RequiredOnly"},
                "max_probe_level": {"kind": "Verbose"}
            },
            "range": {"max_reduction_ceiling": {"kind": "Adaptive"}},
            "storage": {"max_materialization": {"kind": "SpillColdValues"}},
            "sram": {
                "max_page_aggression": {"kind": "MinimizeResident"},
                "max_spill_policy": {"kind": "SpillEager"}
            },
            "rom_window": {
                "max_kernel_residency_bias": {"kind": "PreferWramOverlay"},
                "max_kernel_duplication_bias": {"kind": "DuplicateAllFit"}
            },
            "overlay": {"max_promotion": {"kind": "EligibleKernels"}},
            "schedule": {
                "max_tile_search": {"kind": "ProfileGuided"},
                "max_slice_coarsening": {"kind": "Coarse"},
                "max_resource_pressure": {"kind": "FitFirst"}
            }
        });
        default_bounds_json["schedule"]["max_pressure_thresholds"] =
            serde_json::to_value(bounds.schedule.max_pressure_thresholds)
                .expect("max thresholds json");
        default_bounds_json["schedule"]["max_stage_iteration_ceilings"] =
            serde_json::to_value(bounds.schedule.max_stage_iteration_ceilings)
                .expect("max stage limits json");

        assert_eq!(
            serde_json::to_value(ValidationCode::PolicyConstraintUnsatisfiable {
                knob: CompileKnobId::Placement,
                left: bounds.clone(),
                right: bounds.clone(),
            })
            .expect("policy constraint code serializes"),
            serde_json::json!({
                "kind": "PolicyConstraintUnsatisfiable",
                "fields": {
                    "knob": { "kind": "Placement" },
                    "left": default_bounds_json,
                    "right": default_bounds_json
                }
            })
        );

        assert_eq!(
            serde_json::to_value(ValidationCode::BudgetQuantGraphViewMalformed {
                field: FieldPath::from("budget_view.per_expert_payload"),
            })
            .expect("budget view code serializes"),
            serde_json::json!({
                "kind": "BudgetQuantGraphViewMalformed",
                "fields": {
                    "field": "budget_view.per_expert_payload"
                }
            })
        );

        assert_eq!(
            serde_json::to_value(ValidationCode::PolicyConstraintLoosened {
                knob: CompileKnobId::Placement,
                previous: bounds.clone(),
                requested: bounds.clone(),
            })
            .expect("policy constraint loosened code serializes"),
            serde_json::json!({
                "kind": "PolicyConstraintLoosened",
                "fields": {
                    "knob": { "kind": "Placement" },
                    "previous": default_bounds_json,
                    "requested": default_bounds_json
                }
            })
        );

        assert_eq!(
            serde_json::to_value(ValidationCode::PolicyHintConstraintUnsupported {
                knob: CompileKnobId::Schedule,
                value: ConstraintValue::U32 { value: 17 },
            })
            .expect("unsupported policy hint constraint code serializes"),
            serde_json::json!({
                "kind": "PolicyHintConstraintUnsupported",
                "fields": {
                    "knob": { "kind": "Schedule" },
                    "value": {
                        "kind": "U32",
                        "value": 17
                    }
                }
            })
        );

        assert_eq!(
            serde_json::to_value(ValidationCode::ArtifactForbiddenBuildIdentityField {
                field: FieldPath::from("aux.build_identity.git_sha"),
            })
            .expect("forbidden build identity code serializes"),
            serde_json::json!({
                "kind": "ArtifactForbiddenBuildIdentityField",
                "fields": {
                    "field": "aux.build_identity.git_sha"
                }
            })
        );
    }

    #[test]
    fn validation_code_pins_kind_and_fields_keys_for_heavy_variant() {
        assert_eq!(
            serde_json::to_value(ValidationCode::BudgetSwitchesPerTokenOverCap {
                decision_value: 7,
                upper_bound: 9,
                cap: 5,
                source: SwitchProjectionSource::ConservativeStaticUpperBound,
            })
            .expect("heavy fielded code serializes"),
            serde_json::json!({
                "kind": "BudgetSwitchesPerTokenOverCap",
                "fields": {
                    "decision_value": 7,
                    "upper_bound": 9,
                    "cap": 5,
                    "source": { "kind": "ConservativeStaticUpperBound" }
                }
            })
        );
    }

    #[test]
    fn objective_rejection_pins_typed_payload_json_shapes() {
        assert_eq!(
            serde_json::to_value(ObjectiveRejection::ServiceLevelZero {
                field: ServiceLevelField::MaxUiJitterFramesP99,
            })
            .expect("service-level rejection serializes"),
            serde_json::json!({
                "kind": "ServiceLevelZero",
                "field": { "kind": "MaxUiJitterFramesP99" }
            })
        );

        assert_eq!(
            serde_json::to_value(ObjectiveRejection::RiskQuantileInvalid {
                field: RiskQuantileField::SwitchQuantile,
                value: 101,
            })
            .expect("risk-quantile rejection serializes"),
            serde_json::json!({
                "kind": "RiskQuantileInvalid",
                "field": { "kind": "SwitchQuantile" },
                "value": 101
            })
        );
    }

    #[test]
    fn compile_request_profile_forbids_objective_round_trips_typed_rejections() {
        for reason in [
            ObjectiveRejection::ServiceLevelZero {
                field: ServiceLevelField::MaxFirstTokenCyclesP95,
            },
            ObjectiveRejection::RiskQuantileInvalid {
                field: RiskQuantileField::CycleQuantile,
                value: 0,
            },
        ] {
            assert_code_round_trip(ValidationCode::CompileRequestProfileForbidsObjective {
                profile: CompileProfileId::from("Bringup"),
                reason,
            });
        }
    }

    #[test]
    fn policy_constraint_unsatisfiable_round_trip() {
        assert_code_round_trip(ValidationCode::PolicyConstraintUnsatisfiable {
            knob: CompileKnobId::Placement,
            left: canonical_default_bounds_fixture(),
            right: CompileKnobBounds {
                placement: crate::compile::PlacementKnobBounds {
                    max_profile: PlacementProfile::StrictOnePerBank,
                },
                ..canonical_default_bounds_fixture()
            },
        });
    }

    #[test]
    fn budget_quant_graph_view_malformed_round_trip() {
        assert_code_round_trip(ValidationCode::BudgetQuantGraphViewMalformed {
            field: FieldPath::from("budget_view.per_expert_payload"),
        });
    }

    #[test]
    fn artifact_forbidden_build_identity_field_round_trip() {
        assert_code_round_trip(ValidationCode::ArtifactForbiddenBuildIdentityField {
            field: FieldPath::from("aux.build_identity.git_sha"),
        });
    }

    #[test]
    fn validation_diagnostic_rejects_unknown_fields() {
        let mut value = serde_json::to_value(diagnostic(ValidationCode::SchemaEpochUnsupported))
            .expect("diagnostic serializes");
        value["unexpected"] = serde_json::json!(true);

        assert!(serde_json::from_value::<ValidationDiagnostic>(value).is_err());
    }

    #[test]
    fn validation_code_rejects_unknown_fields() {
        let mut value = serde_json::to_value(ValidationCode::BudgetQuantGraphViewMalformed {
            field: FieldPath::from("budget_view"),
        })
        .expect("code serializes");
        value["unexpected"] = serde_json::json!(true);

        assert!(serde_json::from_value::<ValidationCode>(value).is_err());
    }

    #[test]
    fn validation_code_rejects_nested_unknown_variant_payload_fields() {
        let mut value = serde_json::to_value(ValidationCode::BudgetSwitchesPerTokenOverCap {
            decision_value: 7,
            upper_bound: 9,
            cap: 5,
            source: SwitchProjectionSource::ConservativeStaticUpperBound,
        })
        .expect("code serializes");
        value["fields"]["unexpected"] = serde_json::json!(true);

        assert!(serde_json::from_value::<ValidationCode>(value).is_err());
    }

    #[test]
    fn manifest_invariant_carrier_values_round_trip() {
        for invariant in [
            ManifestInvariant::FeatureSetEpochInconsistent {
                epoch: ArtifactSchemaVersion { epoch: 1, minor: 0 },
                feature: ArtifactFeature::DenseI8,
            },
            ManifestInvariant::RequiredComponentMissing {
                component: ComponentId("core".to_owned()),
            },
            ManifestInvariant::ComponentDigestMismatch {
                component: ComponentId("core".to_owned()),
                expected: hash(1),
                observed: hash(2),
            },
            ManifestInvariant::LineageContradiction {
                derived: LineageId(hash(3)),
                recorded: LineageId(hash(4)),
            },
            ManifestInvariant::ManifestSelfHashMismatch {
                recomputed: hash(5),
                recorded: hash(6),
            },
            ManifestInvariant::ForbiddenBuildIdentityField {
                field: FieldPath::from("manifest.created_by"),
            },
        ] {
            let encoded = serde_json::to_string(&invariant).expect("invariant serializes");
            let decoded: ManifestInvariant =
                serde_json::from_str(&encoded).expect("invariant deserializes");

            assert_eq!(decoded, invariant);
        }
    }
}
