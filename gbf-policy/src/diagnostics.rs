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
    Manifest,
    Lowering,
    Calibration,
    HintBundle,
    Workload,
    GoldenVector,
    CompileRequest,
    PolicyResolution,
    Budget,
    StoragePlanConstruction,
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
    StorageRangePlanHashMismatch,
    StorageInferIrHashMismatch,
    StorageObservationPlanHashMismatch,
    StorageQuantGraphHashMismatch,
    StoragePolicyHashMismatch,
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
            ValidationCode::StorageRangePlanHashMismatch,
            ValidationCode::StorageInferIrHashMismatch,
            ValidationCode::StorageObservationPlanHashMismatch,
            ValidationCode::StorageQuantGraphHashMismatch,
            ValidationCode::StoragePolicyHashMismatch,
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
        let default_bounds_json = serde_json::json!({
            "placement": {"max_profile": {"kind": "PackedExperts"}},
            "observation": {"max_probe_level": {"kind": "Verbose"}},
            "range": {"max_reduction_ceiling": {"kind": "Adaptive"}},
            "storage": {"max_materialization": {"kind": "SpillColdValues"}},
            "sram": {"max_page_aggression": {"kind": "MinimizeResident"}},
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
