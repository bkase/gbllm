//! Compile-request and resolved-policy schema.

use std::collections::BTreeSet;

use gbf_foundation::{CompileProfileId, Hash256, TargetProfileId};
use gbf_hw::calibration::CalibrationSetRef;
use serde::{Deserialize, Serialize};

use crate::budget::RuntimeChromeBudget;
use crate::objective::{CompileObjective, RiskPolicy};
use crate::repair::{RepairPolicy, RepairProposalId};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum SequenceSemanticsRef {
    #[default]
    Unspecified,
    LinearState,
    BoundedKv,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FieldPath(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SelectorPath(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceRef {
    pub kind: String,
    pub reference: String,
    pub hash: Option<Hash256>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompileRequest {
    pub target: TargetProfileId,
    pub profile: CompileProfileId,
    pub objective: CompileObjective,
    pub calibration_set_ref: CalibrationSetRef,
    pub required_features: BTreeSet<CompilerFeature>,
    pub constraint_overrides: Option<CompileKnobOverrides>,
    pub requested_runtime_modes: BTreeSet<RuntimeMode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedCompilePolicy {
    pub target: TargetProfileId,
    pub profile: CompileProfileId,
    pub objective: CompileObjective,
    pub effective_constraints: EffectiveConstraints,
    pub observability: ObservabilityMode,
    pub trace_budget: TraceBudget,
    pub requested_runtime_modes: BTreeSet<RuntimeMode>,
    pub knobs: CompileKnobs,
    pub repair: RepairPolicy,
    pub provenance: PolicyProvenance,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompileProfileSpec {
    pub id: CompileProfileId,
    pub defaults_hash: Hash256,
    pub observability: ObservabilityMode,
    pub trace_budget: TraceBudget,
    pub repair_policy: RepairPolicy,
    pub risk_policy: RiskPolicy,
    pub knob_defaults: CompileKnobPartialValues,
    pub knob_bounds: CompileKnobPartialBounds,
    pub locks: KnobLockSet,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectiveConstraints {
    pub target_caps: CompileKnobBounds,
    pub required_features: BTreeSet<CompilerFeature>,
    pub requested_runtime_modes: BTreeSet<RuntimeMode>,
    pub runtime_chrome_budget: Option<RuntimeChromeBudget>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompileKnobs {
    pub global: CompileKnobValues,
    pub bounds: CompileKnobBounds,
    pub locks: KnobLockSet,
    pub overrides: CompileKnobOverrides,
    pub provenance: Vec<CompileKnobProvenanceEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompileKnobValues {
    pub placement: PlacementKnob,
    pub observation: ObservationKnob,
    pub range: RangeKnob,
    pub storage: StorageKnob,
    pub sram: SramKnob,
    pub rom_window: RomWindowKnob,
    pub overlay: OverlayKnob,
    pub schedule: ScheduleKnob,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompileKnobPartialValues {
    pub placement: Option<PlacementKnob>,
    pub observation: Option<ObservationKnob>,
    pub range: Option<RangeKnob>,
    pub storage: Option<StorageKnob>,
    pub sram: Option<SramKnob>,
    pub rom_window: Option<RomWindowKnob>,
    pub overlay: Option<OverlayKnob>,
    pub schedule: Option<ScheduleKnob>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompileKnobBounds {
    pub placement: PlacementKnobBounds,
    pub observation: ObservationKnobBounds,
    pub range: RangeKnobBounds,
    pub storage: StorageKnobBounds,
    pub sram: SramKnobBounds,
    pub rom_window: RomWindowKnobBounds,
    pub overlay: OverlayKnobBounds,
    pub schedule: ScheduleKnobBounds,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompileKnobPartialBounds {
    pub placement: Option<PlacementKnobBounds>,
    pub observation: Option<ObservationKnobBounds>,
    pub range: Option<RangeKnobBounds>,
    pub storage: Option<StorageKnobBounds>,
    pub sram: Option<SramKnobBounds>,
    pub rom_window: Option<RomWindowKnobBounds>,
    pub overlay: Option<OverlayKnobBounds>,
    pub schedule: Option<ScheduleKnobBounds>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompileKnobOverrides {
    pub values: CompileKnobPartialValues,
    pub bounds: CompileKnobPartialBounds,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KnobLockSet {
    pub locked: BTreeSet<CompileKnobId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum CompileKnobId {
    Placement,
    Observation,
    Range,
    Storage,
    Sram,
    RomWindow,
    Overlay,
    Schedule,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompileKnobPath {
    pub knob: CompileKnobId,
    pub selector: Option<SelectorPath>,
    pub field: Option<FieldPath>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompileKnobProvenanceEntry {
    pub path: CompileKnobPath,
    pub chain: Vec<ConstraintProvenance>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConstraintProvenance {
    pub source: PolicySource,
    pub operation: ConstraintOperation,
    pub evidence: Vec<EvidenceRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum PolicySource {
    TargetDefault,
    ProfileDefault,
    CompileRequestOverride,
    HintBundle,
    Calibration,
    RepairProposal { id: RepairProposalId },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ConstraintOperation {
    SeedDefault,
    TightenBound,
    ApplyPreference,
    ApplyHardConstraint,
    ApplyOverride,
    ApplyCalibration,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyProvenance {
    pub target_defaults: Hash256,
    pub profile_defaults: Hash256,
    pub hint_bundle_hash: Option<Hash256>,
    pub compile_request_hash: Hash256,
    pub calibration_hash: Option<Hash256>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum PlacementProfile {
    StrictOnePerBank = 0,
    Budgeted = 1,
    PackedExperts = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlacementKnob {
    pub profile: PlacementProfile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlacementKnobBounds {
    pub max_profile: PlacementProfile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ObservabilityMode {
    Invariant,
    Flexible,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ProbeCollectionLevel {
    RequiredOnly = 0,
    Operational = 1,
    Verbose = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationKnob {
    pub observability: ObservabilityMode,
    pub probe_level: ProbeCollectionLevel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationKnobBounds {
    pub max_probe_level: ProbeCollectionLevel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ReductionPlanCeiling {
    ExactOnly = 0,
    Conservative = 1,
    Adaptive = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RangeKnob {
    pub reduction_ceiling: ReductionPlanCeiling,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RangeKnobBounds {
    pub max_reduction_ceiling: ReductionPlanCeiling,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum StorageMaterialization {
    PreserveAll = 0,
    RecomputePureValues = 1,
    SpillColdValues = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StorageKnob {
    pub materialization: StorageMaterialization,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StorageKnobBounds {
    pub max_materialization: StorageMaterialization,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum SramPageAggression {
    Preserve = 0,
    PackCold = 1,
    MinimizeResident = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SramKnob {
    pub page_aggression: SramPageAggression,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SramKnobBounds {
    pub max_page_aggression: SramPageAggression,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
#[serde(tag = "kind", deny_unknown_fields)]
#[allow(clippy::enum_variant_names)]
pub enum RomKernelResidencyBias {
    PreferCommonBank = 0,
    PreferExpertBank = 1,
    PreferWramOverlay = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum RomKernelDuplicationBias {
    Share = 0,
    DuplicateHot = 1,
    DuplicateAllFit = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RomWindowKnob {
    pub kernel_residency_bias: RomKernelResidencyBias,
    pub kernel_duplication_bias: RomKernelDuplicationBias,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RomWindowKnobBounds {
    pub max_kernel_residency_bias: RomKernelResidencyBias,
    pub max_kernel_duplication_bias: RomKernelDuplicationBias,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum OverlayPromotion {
    Disabled = 0,
    TinyLuts = 1,
    EligibleKernels = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverlayKnob {
    pub promotion: OverlayPromotion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverlayKnobBounds {
    pub max_promotion: OverlayPromotion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ScheduleTileSearch {
    Fixed = 0,
    Local = 1,
    ProfileGuided = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ScheduleSliceCoarsening {
    Fine = 0,
    Balanced = 1,
    Coarse = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ScheduleResourcePressure {
    Conservative = 0,
    Balanced = 1,
    FitFirst = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduleKnob {
    pub tile_search: ScheduleTileSearch,
    pub slice_coarsening: ScheduleSliceCoarsening,
    pub resource_pressure: ScheduleResourcePressure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduleKnobBounds {
    pub max_tile_search: ScheduleTileSearch,
    pub max_slice_coarsening: ScheduleSliceCoarsening,
    pub max_resource_pressure: ScheduleResourcePressure,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConstraintFrame {
    pub source: PolicySource,
    pub evidence: Vec<EvidenceRef>,
    pub defaults: CompileKnobPartialValues,
    pub hard_bounds: CompileKnobPartialBounds,
    pub preferences: CompileKnobPreferences,
    pub locks: KnobLockSet,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompileKnobPreferences {
    pub preferred_values: Vec<CompileKnobPreference>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompileKnobPreference {
    pub path: CompileKnobPath,
    pub value: ConstraintValue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ConstraintValue {
    PlacementProfile { value: PlacementProfile },
    ObservabilityMode { value: ObservabilityMode },
    U16 { value: u16 },
    U32 { value: u32 },
    Bool { value: bool },
    Text { value: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum CompilerFeature {
    ArtifactValidation,
    PolicyResolution,
    QuantGraphBudgetSource,
    StaticBudgetReport,
    Ternary2Quant,
    Binary1Quant,
    SparseTernaryBitplanes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum RuntimeMode {
    Interactive,
    Steady,
    Trace,
    Safe,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum TraceDropPolicy {
    DropOldest,
    DropNewest,
    HaltAndFault,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TraceBudget {
    pub max_events_per_slice: u16,
    pub max_bytes_per_frame: u16,
    pub drop_policy: TraceDropPolicy,
}

pub trait MonotoneKnob {
    fn is_monotone_successor_of(&self, previous: &Self) -> bool;
}

macro_rules! monotone_enum {
    ($ty:ty) => {
        impl MonotoneKnob for $ty {
            fn is_monotone_successor_of(&self, previous: &Self) -> bool {
                self >= previous
            }
        }
    };
}

monotone_enum!(PlacementProfile);
monotone_enum!(ProbeCollectionLevel);
monotone_enum!(ReductionPlanCeiling);
monotone_enum!(StorageMaterialization);
monotone_enum!(SramPageAggression);
monotone_enum!(RomKernelResidencyBias);
monotone_enum!(RomKernelDuplicationBias);
monotone_enum!(OverlayPromotion);
monotone_enum!(ScheduleTileSearch);
monotone_enum!(ScheduleSliceCoarsening);
monotone_enum!(ScheduleResourcePressure);

impl MonotoneKnob for PlacementKnob {
    fn is_monotone_successor_of(&self, previous: &Self) -> bool {
        self.profile.is_monotone_successor_of(&previous.profile)
    }
}

impl MonotoneKnob for PlacementKnobBounds {
    fn is_monotone_successor_of(&self, previous: &Self) -> bool {
        self.max_profile <= previous.max_profile
    }
}

impl MonotoneKnob for ObservationKnob {
    fn is_monotone_successor_of(&self, previous: &Self) -> bool {
        self.probe_level
            .is_monotone_successor_of(&previous.probe_level)
    }
}

impl MonotoneKnob for ObservationKnobBounds {
    fn is_monotone_successor_of(&self, previous: &Self) -> bool {
        self.max_probe_level <= previous.max_probe_level
    }
}

impl MonotoneKnob for RangeKnob {
    fn is_monotone_successor_of(&self, previous: &Self) -> bool {
        self.reduction_ceiling
            .is_monotone_successor_of(&previous.reduction_ceiling)
    }
}

impl MonotoneKnob for RangeKnobBounds {
    fn is_monotone_successor_of(&self, previous: &Self) -> bool {
        self.max_reduction_ceiling <= previous.max_reduction_ceiling
    }
}

impl MonotoneKnob for StorageKnob {
    fn is_monotone_successor_of(&self, previous: &Self) -> bool {
        self.materialization
            .is_monotone_successor_of(&previous.materialization)
    }
}

impl MonotoneKnob for StorageKnobBounds {
    fn is_monotone_successor_of(&self, previous: &Self) -> bool {
        self.max_materialization <= previous.max_materialization
    }
}

impl MonotoneKnob for SramKnob {
    fn is_monotone_successor_of(&self, previous: &Self) -> bool {
        self.page_aggression
            .is_monotone_successor_of(&previous.page_aggression)
    }
}

impl MonotoneKnob for SramKnobBounds {
    fn is_monotone_successor_of(&self, previous: &Self) -> bool {
        self.max_page_aggression <= previous.max_page_aggression
    }
}

impl MonotoneKnob for RomWindowKnob {
    fn is_monotone_successor_of(&self, previous: &Self) -> bool {
        self.kernel_residency_bias
            .is_monotone_successor_of(&previous.kernel_residency_bias)
            && self
                .kernel_duplication_bias
                .is_monotone_successor_of(&previous.kernel_duplication_bias)
    }
}

impl MonotoneKnob for RomWindowKnobBounds {
    fn is_monotone_successor_of(&self, previous: &Self) -> bool {
        self.max_kernel_residency_bias <= previous.max_kernel_residency_bias
            && self.max_kernel_duplication_bias <= previous.max_kernel_duplication_bias
    }
}

impl MonotoneKnob for OverlayKnob {
    fn is_monotone_successor_of(&self, previous: &Self) -> bool {
        self.promotion.is_monotone_successor_of(&previous.promotion)
    }
}

impl MonotoneKnob for OverlayKnobBounds {
    fn is_monotone_successor_of(&self, previous: &Self) -> bool {
        self.max_promotion <= previous.max_promotion
    }
}

impl MonotoneKnob for ScheduleKnob {
    fn is_monotone_successor_of(&self, previous: &Self) -> bool {
        self.tile_search
            .is_monotone_successor_of(&previous.tile_search)
            && self
                .slice_coarsening
                .is_monotone_successor_of(&previous.slice_coarsening)
            && self
                .resource_pressure
                .is_monotone_successor_of(&previous.resource_pressure)
    }
}

impl MonotoneKnob for ScheduleKnobBounds {
    fn is_monotone_successor_of(&self, previous: &Self) -> bool {
        self.max_tile_search <= previous.max_tile_search
            && self.max_slice_coarsening <= previous.max_slice_coarsening
            && self.max_resource_pressure <= previous.max_resource_pressure
    }
}

#[must_use]
pub const fn canonical_default_bounds_fixture() -> CompileKnobBounds {
    CompileKnobBounds {
        placement: PlacementKnobBounds {
            max_profile: PlacementProfile::PackedExperts,
        },
        observation: ObservationKnobBounds {
            max_probe_level: ProbeCollectionLevel::Verbose,
        },
        range: RangeKnobBounds {
            max_reduction_ceiling: ReductionPlanCeiling::Adaptive,
        },
        storage: StorageKnobBounds {
            max_materialization: StorageMaterialization::SpillColdValues,
        },
        sram: SramKnobBounds {
            max_page_aggression: SramPageAggression::MinimizeResident,
        },
        rom_window: RomWindowKnobBounds {
            max_kernel_residency_bias: RomKernelResidencyBias::PreferWramOverlay,
            max_kernel_duplication_bias: RomKernelDuplicationBias::DuplicateAllFit,
        },
        overlay: OverlayKnobBounds {
            max_promotion: OverlayPromotion::EligibleKernels,
        },
        schedule: ScheduleKnobBounds {
            max_tile_search: ScheduleTileSearch::ProfileGuided,
            max_slice_coarsening: ScheduleSliceCoarsening::Coarse,
            max_resource_pressure: ScheduleResourcePressure::FitFirst,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::objective::{CompileObjective, RiskPolicy, ServiceLevelObjective};
    use crate::repair::RepairPolicyProfile;
    use crate::risk::{CalibrationConfidenceClass, CalibrationConfidenceRequirement};

    fn objective_fixture() -> CompileObjective {
        CompileObjective {
            service: Some(ServiceLevelObjective {
                max_first_token_cycles_p95: Some(3_000),
                max_checkpoint_gap_cycles_p95: None,
                max_resume_latency_cycles_p95: Some(1_000),
                max_ui_jitter_frames_p99: Some(1),
            }),
            max_cycles_per_token: Some(8_000),
            max_bank_switches_per_token: Some(5),
            max_sram_page_switches_per_token: Some(1),
            min_ui_headroom_pct: 9,
            max_rom_bytes: Some(512 * 1024),
            risk: RiskPolicy {
                cycle_quantile: 95,
                switch_quantile: 99,
                calibration_confidence_requirement: CalibrationConfidenceRequirement::AtLeast {
                    class: CalibrationConfidenceClass::Transferred,
                },
                fallback_profile: None,
                fallback_runtime_mode: Some(RuntimeMode::Safe),
            },
        }
    }

    fn values_fixture() -> CompileKnobValues {
        CompileKnobValues {
            placement: PlacementKnob {
                profile: PlacementProfile::Budgeted,
            },
            observation: ObservationKnob {
                observability: ObservabilityMode::Invariant,
                probe_level: ProbeCollectionLevel::Operational,
            },
            range: RangeKnob {
                reduction_ceiling: ReductionPlanCeiling::Conservative,
            },
            storage: StorageKnob {
                materialization: StorageMaterialization::RecomputePureValues,
            },
            sram: SramKnob {
                page_aggression: SramPageAggression::PackCold,
            },
            rom_window: RomWindowKnob {
                kernel_residency_bias: RomKernelResidencyBias::PreferExpertBank,
                kernel_duplication_bias: RomKernelDuplicationBias::DuplicateHot,
            },
            overlay: OverlayKnob {
                promotion: OverlayPromotion::TinyLuts,
            },
            schedule: ScheduleKnob {
                tile_search: ScheduleTileSearch::Local,
                slice_coarsening: ScheduleSliceCoarsening::Balanced,
                resource_pressure: ScheduleResourcePressure::Balanced,
            },
        }
    }

    fn compile_knobs_fixture() -> CompileKnobs {
        CompileKnobs {
            global: values_fixture(),
            bounds: canonical_default_bounds_fixture(),
            locks: KnobLockSet {
                locked: BTreeSet::from([CompileKnobId::RomWindow]),
            },
            overrides: CompileKnobOverrides {
                values: CompileKnobPartialValues {
                    placement: Some(PlacementKnob {
                        profile: PlacementProfile::Budgeted,
                    }),
                    ..CompileKnobPartialValues::default()
                },
                bounds: CompileKnobPartialBounds::default(),
            },
            provenance: vec![CompileKnobProvenanceEntry {
                path: CompileKnobPath {
                    knob: CompileKnobId::Placement,
                    selector: None,
                    field: Some(FieldPath("profile".to_owned())),
                },
                chain: vec![ConstraintProvenance {
                    source: PolicySource::ProfileDefault,
                    operation: ConstraintOperation::SeedDefault,
                    evidence: vec![EvidenceRef {
                        kind: "ProfileFile".to_owned(),
                        reference: "Bringup.toml".to_owned(),
                        hash: Some(Hash256::from_bytes([5; 32])),
                    }],
                }],
            }],
        }
    }

    fn request_fixture() -> CompileRequest {
        CompileRequest {
            target: TargetProfileId::from("dmg-mbc5"),
            profile: CompileProfileId::from("Bringup"),
            objective: objective_fixture(),
            calibration_set_ref: CalibrationSetRef {
                platform: None,
                kernel: None,
                runtime: None,
            },
            required_features: BTreeSet::from([
                CompilerFeature::ArtifactValidation,
                CompilerFeature::PolicyResolution,
            ]),
            constraint_overrides: Some(CompileKnobOverrides {
                values: CompileKnobPartialValues {
                    placement: Some(PlacementKnob {
                        profile: PlacementProfile::StrictOnePerBank,
                    }),
                    ..CompileKnobPartialValues::default()
                },
                bounds: CompileKnobPartialBounds::default(),
            }),
            requested_runtime_modes: BTreeSet::from([RuntimeMode::Interactive]),
        }
    }

    fn policy_fixture() -> ResolvedCompilePolicy {
        ResolvedCompilePolicy {
            target: TargetProfileId::from("dmg-mbc5"),
            profile: CompileProfileId::from("Bringup"),
            objective: objective_fixture(),
            effective_constraints: EffectiveConstraints {
                target_caps: canonical_default_bounds_fixture(),
                required_features: BTreeSet::from([CompilerFeature::ArtifactValidation]),
                requested_runtime_modes: BTreeSet::from([RuntimeMode::Interactive]),
                runtime_chrome_budget: None,
            },
            observability: ObservabilityMode::Invariant,
            trace_budget: TraceBudget {
                max_events_per_slice: 4,
                max_bytes_per_frame: 128,
                drop_policy: TraceDropPolicy::HaltAndFault,
            },
            requested_runtime_modes: BTreeSet::from([RuntimeMode::Interactive]),
            knobs: compile_knobs_fixture(),
            repair: RepairPolicy::for_profile(RepairPolicyProfile::Bringup),
            provenance: PolicyProvenance {
                target_defaults: Hash256::from_bytes([1; 32]),
                profile_defaults: Hash256::from_bytes([2; 32]),
                hint_bundle_hash: Some(Hash256::from_bytes([3; 32])),
                compile_request_hash: Hash256::from_bytes([4; 32]),
                calibration_hash: Some(Hash256::from_bytes([5; 32])),
            },
        }
    }

    #[test]
    fn sequence_semantics_ref_defaults_to_unspecified_until_profiles_are_defined() {
        assert_eq!(
            SequenceSemanticsRef::default(),
            SequenceSemanticsRef::Unspecified
        );
    }

    #[test]
    fn sequence_semantics_ref_round_trips_through_serde() {
        let encoded = serde_json::to_string(&SequenceSemanticsRef::BoundedKv).unwrap();
        let decoded: SequenceSemanticsRef = serde_json::from_str(&encoded).unwrap();

        assert_eq!(decoded, SequenceSemanticsRef::BoundedKv);
    }

    #[test]
    fn compile_types_round_trip() {
        let request = request_fixture();
        let encoded = serde_json::to_string(&request).expect("request serializes");
        let decoded: CompileRequest = serde_json::from_str(&encoded).expect("request deserializes");
        assert_eq!(decoded, request);

        let policy = policy_fixture();
        let encoded = serde_json::to_string(&policy).expect("policy serializes");
        let decoded: ResolvedCompilePolicy =
            serde_json::from_str(&encoded).expect("policy deserializes");
        assert_eq!(decoded, policy);
    }

    #[test]
    fn compile_profile_spec_includes_risk_policy_field() {
        let spec = CompileProfileSpec {
            id: CompileProfileId::from("Default"),
            defaults_hash: Hash256::from_bytes([9; 32]),
            observability: ObservabilityMode::Invariant,
            trace_budget: TraceBudget {
                max_events_per_slice: 1,
                max_bytes_per_frame: 32,
                drop_policy: TraceDropPolicy::DropOldest,
            },
            repair_policy: RepairPolicy::for_profile(RepairPolicyProfile::Default),
            risk_policy: objective_fixture().risk,
            knob_defaults: CompileKnobPartialValues::default(),
            knob_bounds: CompileKnobPartialBounds {
                placement: Some(PlacementKnobBounds {
                    max_profile: PlacementProfile::PackedExperts,
                }),
                ..CompileKnobPartialBounds::default()
            },
            locks: KnobLockSet::default(),
        };

        let value = serde_json::to_value(&spec).expect("profile spec serializes");
        assert!(value.get("risk_policy").is_some());
        let decoded: CompileProfileSpec =
            serde_json::from_value(value).expect("profile spec deserializes");
        assert_eq!(decoded, spec);
    }

    #[test]
    fn compile_knobs_monotone_order_per_subknob() {
        assert!(
            PlacementProfile::PackedExperts.is_monotone_successor_of(&PlacementProfile::Budgeted)
        );
        assert!(
            ProbeCollectionLevel::Verbose
                .is_monotone_successor_of(&ProbeCollectionLevel::Operational)
        );
        assert!(
            ReductionPlanCeiling::Adaptive
                .is_monotone_successor_of(&ReductionPlanCeiling::Conservative)
        );
        assert!(
            StorageMaterialization::SpillColdValues
                .is_monotone_successor_of(&StorageMaterialization::RecomputePureValues)
        );
        assert!(
            SramPageAggression::MinimizeResident
                .is_monotone_successor_of(&SramPageAggression::PackCold)
        );
        assert!(
            RomKernelResidencyBias::PreferWramOverlay
                .is_monotone_successor_of(&RomKernelResidencyBias::PreferExpertBank)
        );
        assert!(
            RomKernelDuplicationBias::DuplicateAllFit
                .is_monotone_successor_of(&RomKernelDuplicationBias::DuplicateHot)
        );
        assert!(
            OverlayPromotion::EligibleKernels.is_monotone_successor_of(&OverlayPromotion::TinyLuts)
        );
        assert!(
            ScheduleTileSearch::ProfileGuided.is_monotone_successor_of(&ScheduleTileSearch::Local)
        );
        assert!(
            ScheduleSliceCoarsening::Coarse
                .is_monotone_successor_of(&ScheduleSliceCoarsening::Balanced)
        );
        assert!(
            ScheduleResourcePressure::FitFirst
                .is_monotone_successor_of(&ScheduleResourcePressure::Balanced)
        );

        assert!(
            PlacementKnobBounds {
                max_profile: PlacementProfile::Budgeted,
            }
            .is_monotone_successor_of(&PlacementKnobBounds {
                max_profile: PlacementProfile::PackedExperts,
            })
        );
    }

    #[test]
    fn policy_source_repair_proposal_variant_round_trips_but_is_not_default() {
        let source = PolicySource::RepairProposal {
            id: RepairProposalId("future-rp-1".to_owned()),
        };
        let encoded = serde_json::to_string(&source).expect("source serializes");
        let decoded: PolicySource = serde_json::from_str(&encoded).expect("source deserializes");

        assert_eq!(decoded, source);
        assert!(
            !compile_knobs_fixture()
                .provenance
                .iter()
                .flat_map(|entry| entry.chain.iter())
                .any(|provenance| matches!(provenance.source, PolicySource::RepairProposal { .. }))
        );
    }

    #[test]
    fn constraint_operation_has_no_authorized_relaxation_variant() {
        let value = serde_json::json!({"kind": "AuthorizedRelaxation"});

        assert!(serde_json::from_value::<ConstraintOperation>(value).is_err());
    }

    #[test]
    fn compile_request_rejects_unknown_field() {
        let mut value = serde_json::to_value(request_fixture()).expect("request serializes");
        value["bringup_relaxation"] = serde_json::json!(true);

        assert!(serde_json::from_value::<CompileRequest>(value).is_err());
    }

    #[test]
    fn canonical_default_bounds_fixture_round_trips() {
        let bounds = canonical_default_bounds_fixture();
        let encoded = serde_json::to_string(&bounds).expect("bounds serializes");
        let decoded: CompileKnobBounds =
            serde_json::from_str(&encoded).expect("bounds deserializes");

        assert_eq!(decoded, bounds);
    }

    #[test]
    fn compile_knob_paths_sort_by_knob_selector_then_field() {
        let paths = BTreeSet::from([
            CompileKnobPath {
                knob: CompileKnobId::Schedule,
                selector: None,
                field: None,
            },
            CompileKnobPath {
                knob: CompileKnobId::Placement,
                selector: Some(SelectorPath("expert.1".to_owned())),
                field: Some(FieldPath("profile".to_owned())),
            },
        ]);

        let first = paths.into_iter().next().expect("path exists");
        assert_eq!(first.knob, CompileKnobId::Placement);
    }

    #[test]
    fn constraint_value_carries_placement_profile() {
        let value = ConstraintValue::PlacementProfile {
            value: PlacementProfile::Budgeted,
        };
        let encoded = serde_json::to_string(&value).expect("value serializes");
        let decoded: ConstraintValue = serde_json::from_str(&encoded).expect("value deserializes");

        assert_eq!(decoded, value);
    }

    #[test]
    fn policy_provenance_round_trips_with_optional_hashes() {
        let provenance = policy_fixture().provenance;
        let encoded = serde_json::to_string(&provenance).expect("provenance serializes");
        let decoded: PolicyProvenance =
            serde_json::from_str(&encoded).expect("provenance deserializes");

        assert_eq!(decoded, provenance);
    }

    #[test]
    fn compile_knobs_round_trip() {
        let knobs = compile_knobs_fixture();
        let encoded = serde_json::to_string(&knobs).expect("knobs serializes");
        let decoded: CompileKnobs = serde_json::from_str(&encoded).expect("knobs deserializes");

        assert_eq!(decoded, knobs);
    }

    #[test]
    fn compile_request_required_features_are_sorted() {
        let encoded = serde_json::to_string(&request_fixture()).expect("request serializes");
        let artifact = encoded
            .find("ArtifactValidation")
            .expect("artifact feature present");
        let policy = encoded
            .find("PolicyResolution")
            .expect("policy feature present");

        assert!(artifact < policy);
    }

    #[test]
    fn effective_constraints_round_trip_without_runtime_budget() {
        let constraints = policy_fixture().effective_constraints;
        let encoded = serde_json::to_string(&constraints).expect("constraints serializes");
        let decoded: EffectiveConstraints =
            serde_json::from_str(&encoded).expect("constraints deserializes");

        assert_eq!(decoded, constraints);
    }

    #[test]
    fn compile_knob_preferences_round_trip() {
        let preferences = CompileKnobPreferences {
            preferred_values: vec![CompileKnobPreference {
                path: CompileKnobPath {
                    knob: CompileKnobId::Placement,
                    selector: None,
                    field: Some(FieldPath("profile".to_owned())),
                },
                value: ConstraintValue::PlacementProfile {
                    value: PlacementProfile::Budgeted,
                },
            }],
        };

        let encoded = serde_json::to_string(&preferences).expect("preferences serializes");
        let decoded: CompileKnobPreferences =
            serde_json::from_str(&encoded).expect("preferences deserializes");

        assert_eq!(decoded, preferences);
    }
}
