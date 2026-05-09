//! Compile-request and resolved-policy schema.

use std::collections::BTreeSet;

use gbf_foundation::{BlobRef, CompileProfileId, Hash256, TargetProfileId};
use gbf_hw::calibration::CalibrationSetRef;
use gbf_hw::target::TargetProfile;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::budget::RuntimeChromeBudget;
use crate::objective::{CompileObjective, RiskPolicy};
use crate::repair::{RepairPolicy, RepairProposalId};

pub use gbf_foundation::{EvidenceRef, FieldPath};

pub const BRINGUP_COMPILE_PROFILE_ID: &str = "Bringup";
pub const DEFAULT_COMPILE_PROFILE_ID: &str = "Default";
pub const TRACE_COMPILE_PROFILE_ID: &str = "Trace";
pub const RECOVERY_COMPILE_PROFILE_ID: &str = "Recovery";

pub const BRINGUP_COMPILE_PROFILE_TOML: &str =
    include_str!("../fixtures/compile-profiles/bringup.profile.toml");
pub const DEFAULT_COMPILE_PROFILE_TOML: &str =
    include_str!("../fixtures/compile-profiles/default.profile.toml");
pub const TRACE_COMPILE_PROFILE_TOML: &str =
    include_str!("../fixtures/compile-profiles/trace.profile.toml");
pub const RECOVERY_COMPILE_PROFILE_TOML: &str =
    include_str!("../fixtures/compile-profiles/recovery.profile.toml");

/// RFC-shaped domain separator for the canonical `CompileProfileSpec` defaults hash.
///
/// The preimage is this byte string followed by canonical JSON for the profile
/// spec with `defaults_hash` zeroed. The NUL suffix prevents accidental
/// concatenation ambiguity with the first JSON byte.
pub const COMPILE_PROFILE_DEFAULTS_HASH_DOMAIN: &[u8] =
    b"gbf:gbf-policy:CompileProfileSpec:compile_profile_spec:1.0.0\0";

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
pub struct SelectorPath(pub String);

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

pub type ArtifactRef = BlobRef;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompileInvocationInputs {
    pub artifact_ref: ArtifactRef,
    pub compile_request: CompileRequest,
    pub target_profile: TargetProfile,
    pub runtime_chrome_budget: Option<RuntimeChromeBudget>,
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

pub fn compile_profile_defaults_hash(
    spec: &CompileProfileSpec,
) -> Result<Hash256, serde_json::Error> {
    let canonical_bytes = compile_profile_defaults_hash_canonical_json_bytes(spec)?;
    let mut hasher = Sha256::new();
    hasher.update(COMPILE_PROFILE_DEFAULTS_HASH_DOMAIN);
    hasher.update(canonical_bytes);
    let digest = hasher.finalize();
    Ok(Hash256::from_bytes(digest.into()))
}

fn compile_profile_defaults_hash_canonical_json_bytes(
    spec: &CompileProfileSpec,
) -> Result<Vec<u8>, serde_json::Error> {
    let mut hashable = spec.clone();
    hashable.defaults_hash = Hash256::ZERO;
    let value = serde_json::to_value(&hashable)?;
    let mut canonical_bytes = Vec::new();
    write_canonical_json_value(&value, &mut canonical_bytes)?;
    Ok(canonical_bytes)
}

fn write_canonical_json_value(
    value: &serde_json::Value,
    out: &mut Vec<u8>,
) -> Result<(), serde_json::Error> {
    match value {
        serde_json::Value::Null
        | serde_json::Value::Bool(_)
        | serde_json::Value::Number(_)
        | serde_json::Value::String(_) => serde_json::to_writer(out, value)?,
        serde_json::Value::Array(items) => {
            out.push(b'[');
            for (index, item) in items.iter().enumerate() {
                if index != 0 {
                    out.push(b',');
                }
                write_canonical_json_value(item, out)?;
            }
            out.push(b']');
        }
        serde_json::Value::Object(entries) => {
            out.push(b'{');
            let mut keys = entries.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            for (index, key) in keys.into_iter().enumerate() {
                if index != 0 {
                    out.push(b',');
                }
                serde_json::to_writer(&mut *out, key)?;
                out.push(b':');
                write_canonical_json_value(&entries[key], out)?;
            }
            out.push(b'}');
        }
    }
    Ok(())
}

pub fn load_compile_profile_spec(toml_source: &str) -> Result<CompileProfileSpec, toml::de::Error> {
    toml::from_str(toml_source)
}

pub fn canonical_compile_profile_specs() -> Result<[CompileProfileSpec; 4], toml::de::Error> {
    Ok([
        load_compile_profile_spec(BRINGUP_COMPILE_PROFILE_TOML)?,
        load_compile_profile_spec(DEFAULT_COMPILE_PROFILE_TOML)?,
        load_compile_profile_spec(TRACE_COMPILE_PROFILE_TOML)?,
        load_compile_profile_spec(RECOVERY_COMPILE_PROFILE_TOML)?,
    ])
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
    /// Source-schema hashes for inputs that always participate in policy resolution.
    pub target_defaults: Hash256,
    pub profile_defaults: Hash256,
    /// Optional because a source `ResolvedCompilePolicy` may be built before a
    /// hint bundle or calibration layer exists. Report sections that embed a
    /// consumed hint/calibration hash can require those fields after validation.
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
                    class: CalibrationConfidenceClass::Weak,
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
                    field: Some(FieldPath::from("profile")),
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

    fn hash_json(byte: u8) -> serde_json::Value {
        serde_json::to_value(Hash256::from_bytes([byte; 32])).expect("hash serializes")
    }

    fn risk_policy_json(class: &str) -> serde_json::Value {
        serde_json::json!({
            "cycle_quantile": 95,
            "switch_quantile": 99,
            "calibration_confidence_requirement": {
                "kind": "AtLeast",
                "class": {"kind": class}
            },
            "fallback_profile": null,
            "fallback_runtime_mode": {"kind": "Safe"}
        })
    }

    fn objective_json() -> serde_json::Value {
        serde_json::json!({
            "service": {
                "max_first_token_cycles_p95": 3000,
                "max_checkpoint_gap_cycles_p95": null,
                "max_resume_latency_cycles_p95": 1000,
                "max_ui_jitter_frames_p99": 1
            },
            "max_cycles_per_token": 8000,
            "max_bank_switches_per_token": 5,
            "max_sram_page_switches_per_token": 1,
            "min_ui_headroom_pct": 9,
            "max_rom_bytes": 524288,
            "risk": risk_policy_json("Weak")
        })
    }

    fn values_json() -> serde_json::Value {
        serde_json::json!({
            "placement": {"profile": {"kind": "Budgeted"}},
            "observation": {
                "observability": {"kind": "Invariant"},
                "probe_level": {"kind": "Operational"}
            },
            "range": {"reduction_ceiling": {"kind": "Conservative"}},
            "storage": {"materialization": {"kind": "RecomputePureValues"}},
            "sram": {"page_aggression": {"kind": "PackCold"}},
            "rom_window": {
                "kernel_residency_bias": {"kind": "PreferExpertBank"},
                "kernel_duplication_bias": {"kind": "DuplicateHot"}
            },
            "overlay": {"promotion": {"kind": "TinyLuts"}},
            "schedule": {
                "tile_search": {"kind": "Local"},
                "slice_coarsening": {"kind": "Balanced"},
                "resource_pressure": {"kind": "Balanced"}
            }
        })
    }

    fn default_bounds_json() -> serde_json::Value {
        serde_json::json!({
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
        })
    }

    fn empty_partial_values_json() -> serde_json::Value {
        serde_json::json!({
            "placement": null,
            "observation": null,
            "range": null,
            "storage": null,
            "sram": null,
            "rom_window": null,
            "overlay": null,
            "schedule": null
        })
    }

    fn empty_partial_bounds_json() -> serde_json::Value {
        serde_json::json!({
            "placement": null,
            "observation": null,
            "range": null,
            "storage": null,
            "sram": null,
            "rom_window": null,
            "overlay": null,
            "schedule": null
        })
    }

    fn knobs_json() -> serde_json::Value {
        serde_json::json!({
            "global": values_json(),
            "bounds": default_bounds_json(),
            "locks": {"locked": [{"kind": "RomWindow"}]},
            "overrides": {
                "values": {
                    "placement": {"profile": {"kind": "Budgeted"}},
                    "observation": null,
                    "range": null,
                    "storage": null,
                    "sram": null,
                    "rom_window": null,
                    "overlay": null,
                    "schedule": null
                },
                "bounds": empty_partial_bounds_json()
            },
            "provenance": [
                {
                    "path": {
                        "knob": {"kind": "Placement"},
                        "selector": null,
                        "field": "profile"
                    },
                    "chain": [
                        {
                            "source": {"kind": "ProfileDefault"},
                            "operation": {"kind": "SeedDefault"},
                            "evidence": [
                                {
                                    "kind": "ProfileFile",
                                    "reference": "Bringup.toml",
                                    "hash": hash_json(5)
                                }
                            ]
                        }
                    ]
                }
            ]
        })
    }

    fn request_json() -> serde_json::Value {
        serde_json::json!({
            "target": "dmg-mbc5",
            "profile": "Bringup",
            "objective": objective_json(),
            "calibration_set_ref": {
                "platform": null,
                "kernel": null,
                "runtime": null
            },
            "required_features": [
                {"kind": "ArtifactValidation"},
                {"kind": "PolicyResolution"}
            ],
            "constraint_overrides": {
                "values": {
                    "placement": {"profile": {"kind": "StrictOnePerBank"}},
                    "observation": null,
                    "range": null,
                    "storage": null,
                    "sram": null,
                    "rom_window": null,
                    "overlay": null,
                    "schedule": null
                },
                "bounds": empty_partial_bounds_json()
            },
            "requested_runtime_modes": [{"kind": "Interactive"}]
        })
    }

    fn policy_provenance_json() -> serde_json::Value {
        serde_json::json!({
            "target_defaults": hash_json(1),
            "profile_defaults": hash_json(2),
            "hint_bundle_hash": hash_json(3),
            "compile_request_hash": hash_json(4),
            "calibration_hash": hash_json(5)
        })
    }

    fn policy_json() -> serde_json::Value {
        serde_json::json!({
            "target": "dmg-mbc5",
            "profile": "Bringup",
            "objective": objective_json(),
            "effective_constraints": {
                "target_caps": default_bounds_json(),
                "required_features": [{"kind": "ArtifactValidation"}],
                "requested_runtime_modes": [{"kind": "Interactive"}],
                "runtime_chrome_budget": null
            },
            "observability": {"kind": "Invariant"},
            "trace_budget": {
                "max_events_per_slice": 4,
                "max_bytes_per_frame": 128,
                "drop_policy": {"kind": "HaltAndFault"}
            },
            "requested_runtime_modes": [{"kind": "Interactive"}],
            "knobs": knobs_json(),
            "repair": {
                "max_refinement_iters": 1,
                "allow_placement_profile_fallback": false,
                "allow_trace_demotion": false,
                "allow_overlay_promotion": false,
                "allow_recompute_promotion": false
            },
            "provenance": policy_provenance_json()
        })
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
        let linear_state_expected = serde_json::json!({"kind": "LinearState"});
        let bounded_kv_expected = serde_json::json!({"kind": "BoundedKv"});

        assert_eq!(
            serde_json::to_value(SequenceSemanticsRef::LinearState).unwrap(),
            linear_state_expected
        );

        assert_eq!(
            serde_json::to_value(SequenceSemanticsRef::BoundedKv).unwrap(),
            bounded_kv_expected
        );

        let encoded = serde_json::to_string(&SequenceSemanticsRef::BoundedKv).unwrap();
        let decoded: SequenceSemanticsRef = serde_json::from_str(&encoded).unwrap();
        let linear_state_from_shape: SequenceSemanticsRef =
            serde_json::from_value(linear_state_expected).unwrap();
        let bounded_kv_from_shape: SequenceSemanticsRef =
            serde_json::from_value(bounded_kv_expected).unwrap();

        assert_eq!(decoded, SequenceSemanticsRef::BoundedKv);
        assert_eq!(linear_state_from_shape, SequenceSemanticsRef::LinearState);
        assert_eq!(bounded_kv_from_shape, SequenceSemanticsRef::BoundedKv);
    }

    #[test]
    fn compile_types_round_trip() {
        let request = request_fixture();
        let encoded = serde_json::to_string(&request).expect("request serializes");
        let decoded: CompileRequest = serde_json::from_str(&encoded).expect("request deserializes");
        assert_eq!(decoded, request);
        assert_eq!(
            serde_json::to_value(&request).expect("request serializes"),
            request_json()
        );

        let policy = policy_fixture();
        let encoded = serde_json::to_string(&policy).expect("policy serializes");
        let decoded: ResolvedCompilePolicy =
            serde_json::from_str(&encoded).expect("policy deserializes");
        assert_eq!(decoded, policy);
        assert_eq!(
            serde_json::to_value(&policy).expect("policy serializes"),
            policy_json()
        );
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
        assert_eq!(
            value,
            serde_json::json!({
                "id": "Default",
                "defaults_hash": hash_json(9),
                "observability": {"kind": "Invariant"},
                "trace_budget": {
                    "max_events_per_slice": 1,
                    "max_bytes_per_frame": 32,
                    "drop_policy": {"kind": "DropOldest"}
                },
                "repair_policy": {
                    "max_refinement_iters": 4,
                    "allow_placement_profile_fallback": true,
                    "allow_trace_demotion": true,
                    "allow_overlay_promotion": true,
                    "allow_recompute_promotion": true
                },
                "risk_policy": risk_policy_json("Weak"),
                "knob_defaults": empty_partial_values_json(),
                "knob_bounds": {
                    "placement": {"max_profile": {"kind": "PackedExperts"}},
                    "observation": null,
                    "range": null,
                    "storage": null,
                    "sram": null,
                    "rom_window": null,
                    "overlay": null,
                    "schedule": null
                },
                "locks": {"locked": []}
            })
        );
        let decoded: CompileProfileSpec =
            serde_json::from_value(value).expect("profile spec deserializes");
        assert_eq!(decoded, spec);
    }

    #[test]
    fn compile_profile_spec_fixtures_round_trip() {
        let specs = canonical_compile_profile_specs().expect("canonical profiles parse");
        assert_eq!(BRINGUP_COMPILE_PROFILE_ID, "Bringup");
        assert_eq!(DEFAULT_COMPILE_PROFILE_ID, "Default");
        assert_eq!(TRACE_COMPILE_PROFILE_ID, "Trace");
        assert_eq!(RECOVERY_COMPILE_PROFILE_ID, "Recovery");
        assert_eq!(
            specs
                .iter()
                .map(|spec| spec.id.as_str())
                .collect::<Vec<_>>(),
            vec![
                BRINGUP_COMPILE_PROFILE_ID,
                DEFAULT_COMPILE_PROFILE_ID,
                TRACE_COMPILE_PROFILE_ID,
                RECOVERY_COMPILE_PROFILE_ID
            ]
        );

        for (source, spec) in [
            (BRINGUP_COMPILE_PROFILE_TOML, &specs[0]),
            (DEFAULT_COMPILE_PROFILE_TOML, &specs[1]),
            (TRACE_COMPILE_PROFILE_TOML, &specs[2]),
            (RECOVERY_COMPILE_PROFILE_TOML, &specs[3]),
        ] {
            assert!(
                !source.contains("relaxations"),
                "profile fixture must not expose profile-time relaxations"
            );
            assert_eq!(
                toml::from_str::<CompileProfileSpec>(source).expect("profile reparses"),
                *spec
            );
            assert!(
                spec.knob_defaults.placement.is_some()
                    && spec.knob_defaults.observation.is_some()
                    && spec.knob_defaults.range.is_some()
                    && spec.knob_defaults.storage.is_some()
                    && spec.knob_defaults.sram.is_some()
                    && spec.knob_defaults.rom_window.is_some()
                    && spec.knob_defaults.overlay.is_some()
                    && spec.knob_defaults.schedule.is_some()
            );
            assert!(
                spec.knob_bounds.placement.is_some()
                    && spec.knob_bounds.observation.is_some()
                    && spec.knob_bounds.range.is_some()
                    && spec.knob_bounds.storage.is_some()
                    && spec.knob_bounds.sram.is_some()
                    && spec.knob_bounds.rom_window.is_some()
                    && spec.knob_bounds.overlay.is_some()
                    && spec.knob_bounds.schedule.is_some()
            );
            assert_eq!(
                spec.risk_policy
                    .fallback_profile
                    .as_ref()
                    .map(|profile| profile.as_str()),
                Some(RECOVERY_COMPILE_PROFILE_ID),
                "{} must fall back to the canonical Recovery profile id",
                spec.id
            );
        }

        assert_eq!(
            specs[0].risk_policy.calibration_confidence_requirement,
            CalibrationConfidenceRequirement::NoMinimumConfidence
        );
        for spec in &specs[1..] {
            match spec.risk_policy.calibration_confidence_requirement {
                CalibrationConfidenceRequirement::NoMinimumConfidence => {
                    panic!("non-bringup profiles require measured confidence")
                }
                CalibrationConfidenceRequirement::AtLeast { class } => {
                    assert!(
                        class.rank() >= CalibrationConfidenceClass::Transferred.rank(),
                        "non-bringup profile {} requires {class:?}",
                        spec.id
                    );
                }
            }
        }
    }

    #[test]
    fn compile_profile_spec_defaults_hash_is_deterministic() {
        let specs = canonical_compile_profile_specs().expect("canonical profiles parse");

        for spec in specs {
            let hash = compile_profile_defaults_hash(&spec).expect("hashable profile spec");
            assert_eq!(
                spec.defaults_hash, hash,
                "defaults_hash mismatch for {}",
                spec.id
            );
            assert_eq!(
                hash,
                compile_profile_defaults_hash(&spec).expect("hashable profile spec"),
                "defaults_hash is deterministic for {}",
                spec.id
            );
        }
    }

    #[test]
    fn compile_profile_spec_defaults_hash_uses_canonical_domain_preimage() {
        assert_eq!(
            COMPILE_PROFILE_DEFAULTS_HASH_DOMAIN,
            b"gbf:gbf-policy:CompileProfileSpec:compile_profile_spec:1.0.0\0"
        );

        let spec = load_compile_profile_spec(DEFAULT_COMPILE_PROFILE_TOML)
            .expect("default profile parses");
        let canonical_bytes = compile_profile_defaults_hash_canonical_json_bytes(&spec)
            .expect("profile has canonical JSON bytes");
        let canonical_json =
            std::str::from_utf8(&canonical_bytes).expect("canonical JSON is UTF-8");

        assert!(
            canonical_json.starts_with(r#"{"defaults_hash":"sha256:0000000000000000000000000000000000000000000000000000000000000000","id":"Default""#),
            "top-level canonical JSON keys must be lexicographically ordered"
        );
        assert!(
            canonical_json.contains(r#""risk_policy":{"calibration_confidence_requirement":"#),
            "nested object keys must be lexicographically ordered"
        );

        let mut struct_order = spec.clone();
        struct_order.defaults_hash = Hash256::ZERO;
        let plain_serde_bytes =
            serde_json::to_vec(&struct_order).expect("plain profile spec serializes");
        assert_ne!(
            canonical_bytes, plain_serde_bytes,
            "canonical JSON preimage must not be serde_json's struct field order"
        );

        let hash = compile_profile_defaults_hash(&spec).expect("profile hash computes");
        let no_domain_digest = Sha256::digest(&canonical_bytes);
        assert_ne!(
            hash,
            Hash256::from_bytes(no_domain_digest.into()),
            "defaults_hash must include the explicit CompileProfileSpec domain separator"
        );
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
        assert!(
            PlacementKnobBounds {
                max_profile: PlacementProfile::Budgeted,
            }
            .is_monotone_successor_of(&PlacementKnobBounds {
                max_profile: PlacementProfile::Budgeted,
            })
        );
        assert!(
            !PlacementKnobBounds {
                max_profile: PlacementProfile::PackedExperts,
            }
            .is_monotone_successor_of(&PlacementKnobBounds {
                max_profile: PlacementProfile::Budgeted,
            })
        );
        assert!(
            !PlacementProfile::StrictOnePerBank
                .is_monotone_successor_of(&PlacementProfile::Budgeted)
        );
        assert!(
            !ObservationKnobBounds {
                max_probe_level: ProbeCollectionLevel::Verbose,
            }
            .is_monotone_successor_of(&ObservationKnobBounds {
                max_probe_level: ProbeCollectionLevel::Operational,
            })
        );
        assert!(
            ObservationKnobBounds {
                max_probe_level: ProbeCollectionLevel::Operational,
            }
            .is_monotone_successor_of(&ObservationKnobBounds {
                max_probe_level: ProbeCollectionLevel::Operational,
            })
        );
        assert!(
            !RangeKnobBounds {
                max_reduction_ceiling: ReductionPlanCeiling::Adaptive,
            }
            .is_monotone_successor_of(&RangeKnobBounds {
                max_reduction_ceiling: ReductionPlanCeiling::Conservative,
            })
        );
        assert!(
            !StorageKnobBounds {
                max_materialization: StorageMaterialization::SpillColdValues,
            }
            .is_monotone_successor_of(&StorageKnobBounds {
                max_materialization: StorageMaterialization::RecomputePureValues,
            })
        );
        assert!(
            !SramKnobBounds {
                max_page_aggression: SramPageAggression::MinimizeResident,
            }
            .is_monotone_successor_of(&SramKnobBounds {
                max_page_aggression: SramPageAggression::PackCold,
            })
        );
        assert!(
            !RomWindowKnobBounds {
                max_kernel_residency_bias: RomKernelResidencyBias::PreferWramOverlay,
                max_kernel_duplication_bias: RomKernelDuplicationBias::DuplicateHot,
            }
            .is_monotone_successor_of(&RomWindowKnobBounds {
                max_kernel_residency_bias: RomKernelResidencyBias::PreferExpertBank,
                max_kernel_duplication_bias: RomKernelDuplicationBias::DuplicateHot,
            })
        );
        assert!(
            !RomWindowKnobBounds {
                max_kernel_residency_bias: RomKernelResidencyBias::PreferExpertBank,
                max_kernel_duplication_bias: RomKernelDuplicationBias::DuplicateAllFit,
            }
            .is_monotone_successor_of(&RomWindowKnobBounds {
                max_kernel_residency_bias: RomKernelResidencyBias::PreferExpertBank,
                max_kernel_duplication_bias: RomKernelDuplicationBias::DuplicateHot,
            })
        );
        assert!(
            !OverlayKnobBounds {
                max_promotion: OverlayPromotion::EligibleKernels,
            }
            .is_monotone_successor_of(&OverlayKnobBounds {
                max_promotion: OverlayPromotion::TinyLuts,
            })
        );
        assert!(
            !ScheduleKnobBounds {
                max_tile_search: ScheduleTileSearch::ProfileGuided,
                max_slice_coarsening: ScheduleSliceCoarsening::Balanced,
                max_resource_pressure: ScheduleResourcePressure::Balanced,
            }
            .is_monotone_successor_of(&ScheduleKnobBounds {
                max_tile_search: ScheduleTileSearch::Local,
                max_slice_coarsening: ScheduleSliceCoarsening::Balanced,
                max_resource_pressure: ScheduleResourcePressure::Balanced,
            })
        );
        assert!(
            !ScheduleKnobBounds {
                max_tile_search: ScheduleTileSearch::Local,
                max_slice_coarsening: ScheduleSliceCoarsening::Coarse,
                max_resource_pressure: ScheduleResourcePressure::Balanced,
            }
            .is_monotone_successor_of(&ScheduleKnobBounds {
                max_tile_search: ScheduleTileSearch::Local,
                max_slice_coarsening: ScheduleSliceCoarsening::Balanced,
                max_resource_pressure: ScheduleResourcePressure::Balanced,
            })
        );
        assert!(
            !ScheduleKnobBounds {
                max_tile_search: ScheduleTileSearch::Local,
                max_slice_coarsening: ScheduleSliceCoarsening::Balanced,
                max_resource_pressure: ScheduleResourcePressure::FitFirst,
            }
            .is_monotone_successor_of(&ScheduleKnobBounds {
                max_tile_search: ScheduleTileSearch::Local,
                max_slice_coarsening: ScheduleSliceCoarsening::Balanced,
                max_resource_pressure: ScheduleResourcePressure::Balanced,
            })
        );
    }

    #[test]
    fn policy_source_repair_proposal_variant_round_trips_but_is_not_default() {
        let source = PolicySource::RepairProposal {
            id: RepairProposalId("future-rp-1".to_owned()),
        };
        let expected = serde_json::json!({
            "kind": "RepairProposal",
            "id": "future-rp-1"
        });

        assert_eq!(
            serde_json::to_value(&source).expect("source serializes"),
            expected
        );

        let encoded = serde_json::to_string(&source).expect("source serializes");
        let decoded: PolicySource = serde_json::from_str(&encoded).expect("source deserializes");
        let decoded_from_shape: PolicySource =
            serde_json::from_value(expected).expect("source deserializes from public shape");

        assert_eq!(decoded, source);
        assert_eq!(decoded_from_shape, source);
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
        assert_eq!(
            serde_json::to_value(bounds).expect("bounds serialize"),
            default_bounds_json()
        );
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
                field: Some(FieldPath::from("profile")),
            },
        ]);

        let first = paths.into_iter().next().expect("path exists");
        assert_eq!(first.knob, CompileKnobId::Placement);
    }

    #[test]
    fn field_path_reexports_foundation_type_and_serializes_as_string() {
        fn accepts_foundation_field_path(_: gbf_foundation::FieldPath) {}

        let field = FieldPath::from("profile");
        accepts_foundation_field_path(field.clone());

        let path = CompileKnobPath {
            knob: CompileKnobId::Placement,
            selector: None,
            field: Some(field),
        };

        assert_eq!(
            serde_json::to_value(path).expect("path serializes"),
            serde_json::json!({
                "knob": {"kind": "Placement"},
                "selector": null,
                "field": "profile"
            })
        );
    }

    #[test]
    fn constraint_value_json_shapes_are_pinned() {
        let cases = [
            (
                ConstraintValue::PlacementProfile {
                    value: PlacementProfile::Budgeted,
                },
                serde_json::json!({
                    "kind": "PlacementProfile",
                    "value": {"kind": "Budgeted"}
                }),
            ),
            (
                ConstraintValue::ObservabilityMode {
                    value: ObservabilityMode::Flexible,
                },
                serde_json::json!({
                    "kind": "ObservabilityMode",
                    "value": {"kind": "Flexible"}
                }),
            ),
            (
                ConstraintValue::U16 { value: 512 },
                serde_json::json!({
                    "kind": "U16",
                    "value": 512
                }),
            ),
            (
                ConstraintValue::U32 { value: 70_000 },
                serde_json::json!({
                    "kind": "U32",
                    "value": 70_000
                }),
            ),
            (
                ConstraintValue::Bool { value: true },
                serde_json::json!({
                    "kind": "Bool",
                    "value": true
                }),
            ),
            (
                ConstraintValue::Text {
                    value: "bank0.kernel".to_owned(),
                },
                serde_json::json!({
                    "kind": "Text",
                    "value": "bank0.kernel"
                }),
            ),
        ];

        for (value, expected_json) in cases {
            assert_eq!(
                serde_json::to_value(&value).expect("value serializes"),
                expected_json
            );
            let decoded: ConstraintValue =
                serde_json::from_value(expected_json).expect("value deserializes");
            assert_eq!(decoded, value);
        }
    }

    #[test]
    fn policy_provenance_round_trips_with_optional_hashes() {
        let provenance = policy_fixture().provenance;
        let encoded = serde_json::to_string(&provenance).expect("provenance serializes");
        let decoded: PolicyProvenance =
            serde_json::from_str(&encoded).expect("provenance deserializes");

        assert_eq!(decoded, provenance);
        assert_eq!(
            serde_json::to_value(&provenance).expect("provenance serializes"),
            policy_provenance_json()
        );
    }

    #[test]
    fn compile_knobs_round_trip() {
        let knobs = compile_knobs_fixture();
        let encoded = serde_json::to_string(&knobs).expect("knobs serializes");
        let decoded: CompileKnobs = serde_json::from_str(&encoded).expect("knobs deserializes");

        assert_eq!(decoded, knobs);
        assert_eq!(
            serde_json::to_value(&knobs).expect("knobs serializes"),
            knobs_json()
        );
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
}
