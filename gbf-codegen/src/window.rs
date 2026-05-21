//! Stage 8 `RomWindowPlan` construction, report, and cache-key surface.

use std::collections::{BTreeMap, BTreeSet};
use std::{error::Error, fmt};

use gbf_foundation::{
    CanonicalJsonError, DomainHash, EvidenceRef, Hash256, KernelSpecId, SemVer, TargetProfileId,
    canonical_json_bytes_omitting_fields, self_hash_omitting_fields,
};
use gbf_policy::{
    BudgetSlotClass, DiagnosticSeverity, PlacementProfile, ReductionSiteId, RomWindowKnob,
    RomWindowPlanDiagnosticCode, RomWindowPlanDiagnosticProvenance, RuntimeChromeBudget,
    RuntimeMode, SwitchProjectionSource, ValidationCode, ValidationDetail, ValidationDiagnostic,
    ValidationOrigin,
};
use gbf_report::{
    CanonicalJsonError as ReportCanonicalJsonError, ReportBody, ReportEnvelope,
    ReportEnvelopeError, ReportOutcome, ReportSelfHashError, canonicalize, round_trip_self_hash,
};
use serde::{Deserialize, Serialize};

use crate::s1::quant_graph::DeterminismClass;
use crate::s3::infer_ir::{GbInferIR, NodeId, ValueId, infer_ir_self_hash};
use crate::s4::observation_plan::{
    ObservationPlanCoreProduct, ObservationResidencyEpochId, ObservationRomBankIndex,
    ObservationRomReachabilityClass, ObservationRomWindowFactSource,
};
use crate::s5::range_plan::{RangePlanCoreProduct, RangeRomWindowFactSource};
use crate::sram_page_plan::SramPagePlan;
use crate::stage_cache::{
    CodegenStageCacheError, StoreBackedStageCacheKeys, StoreBackedStageCellKind,
    StoreBackedStageExpectedHashes, StoreBackedStageRunOutput, StoreBackedStageRunResult,
    crate_feature_set_hash, run_store_backed_stage_with_cache, stage8_rom_window_plan_store_key,
};
use crate::storage_plan::types::{Materialization, StorageBinding, StorageClass};
use gbf_store::stage_cache::StageCache as StoreStageCache;

pub const ROM_WINDOW_PLAN_SCHEMA_ID: &str = "rom_window_plan.v1";
pub const ROM_WINDOW_PLAN_SCHEMA_VERSION: SemVer = SemVer::new(1, 0, 0);
pub const ROM_WINDOW_PLAN_PASS_VERSION: &str = "stage8/v1";
pub const WINDOW_CERT_SCHEMA_ID: &str = "window_cert.v1";
pub const WINDOW_CERT_SCHEMA_VERSION: SemVer = SemVer::new(1, 0, 0);

pub type RomWindowPlanReportEnvelope = ReportEnvelope<RomWindowPlanReportBody>;

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RomBankIndex(pub u16);

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RomWindowBindingId(pub u32);

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ResidencyEpochId(pub u32);

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CoResidentClosureId(pub u32);

#[repr(transparent)]
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct LutInstanceId(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum RomReachabilityClass {
    HotPath,
    IsrReachable,
    YieldResumeReachable,
    FaultPathReachable,
}

impl RomReachabilityClass {
    #[must_use]
    pub const fn requires_bank0(self) -> bool {
        matches!(
            self,
            Self::IsrReachable | Self::YieldResumeReachable | Self::FaultPathReachable
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum KernelResidency {
    Bank0Fixed,
    WramOverlay,
    CoResidentSwitchable { bank: RomBankIndex },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum LutResidency {
    Bank0Inline,
    WramStaged { always_resident: bool },
    RomCoResident { bank: RomBankIndex },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RomVisibility {
    pub bank0_visible: bool,
    pub switchable: Option<RomBankIndex>,
}

impl RomVisibility {
    #[must_use]
    pub const fn bank0_only() -> Self {
        Self {
            bank0_visible: true,
            switchable: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RomWindowPlanInputIdentity {
    pub artifact_validation_self_hash: Hash256,
    pub policy_resolution_self_hash: Hash256,
    pub static_budget_self_hash: Hash256,
    pub quant_graph_self_hash: Hash256,
    pub infer_ir_self_hash: Hash256,
    pub storage_plan_self_hash: Hash256,
    pub observation_plan_self_hash: Hash256,
    pub range_plan_self_hash: Hash256,
    pub sram_page_plan_self_hash: Hash256,
    pub runtime_chrome_budget_hash: Hash256,
    pub target_profile_hash: Hash256,
    pub rom_window_plan_policy_projection_hash: Hash256,
    pub runtime_mode: RuntimeMode,
    pub determinism: DeterminismClass,
    pub target_profile_id: TargetProfileId,
    pub schema_version: SemVer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RomWindowPlanInputHashes {
    pub artifact_validation_self_hash: Hash256,
    pub policy_resolution_self_hash: Hash256,
    pub static_budget_self_hash: Hash256,
    pub quant_graph_self_hash: Hash256,
    pub infer_ir_self_hash: Hash256,
    pub storage_plan_self_hash: Hash256,
    pub observation_plan_self_hash: Hash256,
    pub range_plan_self_hash: Hash256,
    pub sram_page_plan_self_hash: Hash256,
    pub runtime_chrome_budget_hash: Hash256,
    pub target_profile_hash: Hash256,
    pub rom_window_plan_policy_projection_hash: Hash256,
}

impl RomWindowPlanInputIdentity {
    #[must_use]
    pub const fn hashes(&self) -> RomWindowPlanInputHashes {
        RomWindowPlanInputHashes {
            artifact_validation_self_hash: self.artifact_validation_self_hash,
            policy_resolution_self_hash: self.policy_resolution_self_hash,
            static_budget_self_hash: self.static_budget_self_hash,
            quant_graph_self_hash: self.quant_graph_self_hash,
            infer_ir_self_hash: self.infer_ir_self_hash,
            storage_plan_self_hash: self.storage_plan_self_hash,
            observation_plan_self_hash: self.observation_plan_self_hash,
            range_plan_self_hash: self.range_plan_self_hash,
            sram_page_plan_self_hash: self.sram_page_plan_self_hash,
            runtime_chrome_budget_hash: self.runtime_chrome_budget_hash,
            target_profile_hash: self.target_profile_hash,
            rom_window_plan_policy_projection_hash: self.rom_window_plan_policy_projection_hash,
        }
    }

    #[must_use]
    pub fn hash_for_product(&self, product: RomWindowPlanInputProduct) -> Hash256 {
        self.hashes().hash_for_product(product)
    }
}

impl RomWindowPlanInputHashes {
    #[must_use]
    pub const fn hash_for_product(&self, product: RomWindowPlanInputProduct) -> Hash256 {
        match product {
            RomWindowPlanInputProduct::ArtifactValidation => self.artifact_validation_self_hash,
            RomWindowPlanInputProduct::PolicyResolution => self.policy_resolution_self_hash,
            RomWindowPlanInputProduct::StaticBudget => self.static_budget_self_hash,
            RomWindowPlanInputProduct::QuantGraph => self.quant_graph_self_hash,
            RomWindowPlanInputProduct::InferIr => self.infer_ir_self_hash,
            RomWindowPlanInputProduct::StoragePlan => self.storage_plan_self_hash,
            RomWindowPlanInputProduct::ObservationPlan => self.observation_plan_self_hash,
            RomWindowPlanInputProduct::RangePlan => self.range_plan_self_hash,
            RomWindowPlanInputProduct::SramPagePlan => self.sram_page_plan_self_hash,
            RomWindowPlanInputProduct::RuntimeChromeBudget => self.runtime_chrome_budget_hash,
            RomWindowPlanInputProduct::TargetProfile => self.target_profile_hash,
            RomWindowPlanInputProduct::PolicyProjection => {
                self.rom_window_plan_policy_projection_hash
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum RomWindowPlanInputProduct {
    ArtifactValidation,
    PolicyResolution,
    StaticBudget,
    QuantGraph,
    InferIr,
    StoragePlan,
    ObservationPlan,
    RangePlan,
    SramPagePlan,
    RuntimeChromeBudget,
    TargetProfile,
    PolicyProjection,
}

impl RomWindowPlanInputProduct {
    #[must_use]
    pub const fn field_name(self) -> &'static str {
        match self {
            Self::ArtifactValidation => "artifact_validation_self_hash",
            Self::PolicyResolution => "policy_resolution_self_hash",
            Self::StaticBudget => "static_budget_self_hash",
            Self::QuantGraph => "quant_graph_self_hash",
            Self::InferIr => "infer_ir_self_hash",
            Self::StoragePlan => "storage_plan_self_hash",
            Self::ObservationPlan => "observation_plan_self_hash",
            Self::RangePlan => "range_plan_self_hash",
            Self::SramPagePlan => "sram_page_plan_self_hash",
            Self::RuntimeChromeBudget => "runtime_chrome_budget_hash",
            Self::TargetProfile => "target_profile_hash",
            Self::PolicyProjection => "rom_window_plan_policy_projection_hash",
        }
    }
}

const ROM_WINDOW_PLAN_INPUT_PRODUCTS: [RomWindowPlanInputProduct; 12] = [
    RomWindowPlanInputProduct::ArtifactValidation,
    RomWindowPlanInputProduct::PolicyResolution,
    RomWindowPlanInputProduct::StaticBudget,
    RomWindowPlanInputProduct::QuantGraph,
    RomWindowPlanInputProduct::InferIr,
    RomWindowPlanInputProduct::StoragePlan,
    RomWindowPlanInputProduct::ObservationPlan,
    RomWindowPlanInputProduct::RangePlan,
    RomWindowPlanInputProduct::SramPagePlan,
    RomWindowPlanInputProduct::RuntimeChromeBudget,
    RomWindowPlanInputProduct::TargetProfile,
    RomWindowPlanInputProduct::PolicyProjection,
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RomWindowPlanPolicyProjection {
    pub placement_profile: PlacementProfile,
    pub rom_window: RomWindowKnob,
    pub max_bank_switches_per_token: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RomWindowPlanInputs {
    pub input_identity: RomWindowPlanInputIdentity,
    pub expected_input_hashes: RomWindowPlanInputHashes,
    pub infer_ir: GbInferIR,
    pub runtime_chrome_budget: RuntimeChromeBudget,
    pub policy: RomWindowPlanPolicyProjection,
    pub sram_page_plan: SramPagePlan,
    pub epochs: Vec<RomWindowEpochInput>,
    pub kernels: Vec<KernelResidencyInput>,
    pub luts: Vec<LutResidencyInput>,
    pub storage_bindings: Vec<StorageBindingInput>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RomWindowPlanAuditParents {
    pub artifact_validation_self_hash: Hash256,
    pub policy_resolution_self_hash: Hash256,
    pub static_budget_self_hash: Hash256,
    pub quant_graph_self_hash: Hash256,
    pub infer_ir_self_hash: Hash256,
    pub observation_plan_self_hash: Hash256,
    pub range_plan_self_hash: Hash256,
    pub storage_plan_self_hash: Hash256,
    pub sram_page_plan_self_hash: Hash256,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RomWindowPlanRegistryInputs {
    pub base_inputs: RomWindowPlanInputs,
    pub audit_parents: RomWindowPlanAuditParents,
    pub observation_registry: RomWindowObservationRegistry,
    pub range_registry: RomWindowRangeRegistry,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RomWindowObservationRegistry {
    pub observation_plan_self_hash: Hash256,
    pub infer_ir_self_hash: Hash256,
    pub quant_graph_self_hash: Hash256,
    pub source: RomWindowRegistrySource,
    pub kernels: Vec<ObservationKernelResidencyRecord>,
    pub luts: Vec<ObservationLutResidencyRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationKernelResidencyRecord {
    pub kernel: KernelSpecId,
    pub byte_size: u32,
    pub reachability: Option<RomReachabilityClass>,
    pub overlay_eligible: bool,
    pub active_epochs: Vec<ResidencyEpochId>,
    pub requested_bank: Option<RomBankIndex>,
    pub source: RomWindowRegistrySource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationLutResidencyRecord {
    pub lut: LutInstanceId,
    pub byte_size: u32,
    pub reachability: Option<RomReachabilityClass>,
    pub overlay_eligible: bool,
    pub active_epochs: Vec<ResidencyEpochId>,
    pub requested_bank: Option<RomBankIndex>,
    pub source: RomWindowRegistrySource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum RomWindowRegistrySource {
    Available {
        registry_key: String,
    },
    Missing {
        registry_key: String,
    },
    SourceImpossible {
        registry_key: String,
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RomWindowRangeRegistry {
    pub range_plan_self_hash: Hash256,
    pub infer_ir_self_hash: Hash256,
    pub quant_graph_self_hash: Hash256,
    pub static_budget_self_hash: Hash256,
    pub source: RomWindowRegistrySource,
    pub reduction_subordinates: Vec<RangeReductionSubordinate>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RangeReductionSubordinate {
    pub site: ReductionSiteId,
    pub main_kernel: KernelSpecId,
    pub subordinate_kernel: KernelSpecId,
    pub source: RomWindowRegistrySource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RomWindowEpochInput {
    pub id: ResidencyEpochId,
    pub op_range: NodeAnchorRange,
    pub sram_page_binding: Option<ValueId>,
    pub yield_kind: YieldKindHint,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeAnchorRange {
    pub first_node: NodeId,
    pub last_node: NodeId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum YieldKindHint {
    NoYieldsExpected,
    YieldsAtCommitBoundaries,
    YieldsAtTokenBoundary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KernelResidencyInput {
    pub kernel: KernelSpecId,
    pub byte_size: u32,
    pub reachability: RomReachabilityClass,
    pub overlay_eligible: bool,
    pub active_epochs: Vec<ResidencyEpochId>,
    pub requested_bank: Option<RomBankIndex>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LutResidencyInput {
    pub lut: LutInstanceId,
    pub byte_size: u32,
    pub reachability: RomReachabilityClass,
    pub overlay_eligible: bool,
    pub active_epochs: Vec<ResidencyEpochId>,
    pub requested_bank: Option<RomBankIndex>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StorageBindingInput {
    pub binding: StorageBinding,
    pub payload_bytes: u32,
    pub active_epochs: Vec<ResidencyEpochId>,
    pub requested_bank: Option<RomBankIndex>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RomWindowPlan {
    pub identity: RomWindowPlanInputIdentity,
    pub kernel_residency: BTreeMap<KernelSpecId, KernelResidency>,
    pub lut_residency: BTreeMap<LutInstanceId, LutResidency>,
    pub rom_window_bindings: Vec<RomWindowBinding>,
    pub banks: Vec<BankAssignment>,
    pub residency_epochs: Vec<ResidencyEpoch>,
    pub co_resident_closures: Vec<CoResidentClosure>,
    pub overlay_demand: WramOverlayDemand,
    pub bank0_demand: Bank0Demand,
    pub projections: RomSwitchProjections,
    pub profile: PlacementProfile,
    pub provenance: RomWindowPlanProvenance,
    pub rom_window_plan_self_hash: Hash256,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RomWindowBinding {
    pub id: RomWindowBindingId,
    pub epoch: ResidencyEpochId,
    pub visibility: RomVisibility,
    pub assigned_kernels: Vec<KernelSpecId>,
    pub assigned_luts: Vec<LutInstanceId>,
    pub assigned_tensors: Vec<ValueId>,
    pub closure: Option<CoResidentClosureId>,
    pub provenance: Vec<EvidenceRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BankAssignment {
    pub bank: RomBankIndex,
    pub occupants: Vec<RomBankOccupant>,
    pub total_bytes: u32,
    pub cap_bytes: u32,
    pub slack_bytes: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum RomBankOccupant {
    Kernel { kernel: KernelSpecId },
    Lut { lut: LutInstanceId },
    Tensor { value: ValueId },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResidencyEpoch {
    pub id: ResidencyEpochId,
    pub op_range: NodeAnchorRange,
    pub rom_window_binding: RomWindowBindingId,
    pub sram_page_binding: Option<ValueId>,
    pub overlay_state: OverlayState,
    pub yield_kind: YieldKindHint,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum OverlayState {
    NoOverlayActive,
    OverlayActive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoResidentClosure {
    pub id: CoResidentClosureId,
    pub bank: RomBankIndex,
    pub kernels: Vec<KernelSpecId>,
    pub luts: Vec<LutInstanceId>,
    pub tensors: Vec<ValueId>,
    pub total_bytes: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WramOverlayDemand {
    pub kernels: Vec<WramOverlayKernelDemand>,
    pub luts: Vec<WramOverlayLutDemand>,
    pub install_source_visibility: Vec<OverlayInstallSourceVisibility>,
    pub total_overlay_bytes: u32,
    pub total_install_count_per_token_upper_bound: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WramOverlayKernelDemand {
    pub kernel: KernelSpecId,
    pub byte_size: u32,
    pub reachability: RomReachabilityClass,
    pub source_bank: RomBankIndex,
    pub active_epochs: Vec<ResidencyEpochId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WramOverlayLutDemand {
    pub lut: LutInstanceId,
    pub byte_size: u32,
    pub reachability: RomReachabilityClass,
    pub source_bank: RomBankIndex,
    pub active_epochs: Vec<ResidencyEpochId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverlayInstallSourceVisibility {
    pub epoch: ResidencyEpochId,
    pub visibility: RomVisibility,
    pub kernels: Vec<OverlayKernelInstallSource>,
    pub luts: Vec<OverlayLutInstallSource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverlayKernelInstallSource {
    pub kernel: KernelSpecId,
    pub source_bank: RomBankIndex,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverlayLutInstallSource {
    pub lut: LutInstanceId,
    pub source_bank: RomBankIndex,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bank0Demand {
    pub kernels: Vec<Bank0KernelDemand>,
    pub luts: Vec<Bank0LutDemand>,
    pub total_kernel_bytes: u32,
    pub total_lut_bytes: u32,
    pub remaining_slack_bytes: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bank0KernelDemand {
    pub kernel: KernelSpecId,
    pub byte_size: u32,
    pub reachability: RomReachabilityClass,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bank0LutDemand {
    pub lut: LutInstanceId,
    pub byte_size: u32,
    pub reachability: RomReachabilityClass,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RomSwitchProjections {
    pub projected_bank_switches_per_token: u16,
    pub upper_bound_per_token: u16,
    pub per_phase: Vec<PerPhaseSwitchCount>,
    pub source: SwitchProjectionSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PerPhaseSwitchCount {
    pub epoch: ResidencyEpochId,
    pub switch_count: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RomWindowPlanProvenance {
    pub kernel_to_reachability: BTreeMap<KernelSpecId, RomReachabilityClass>,
    pub lut_to_reachability: BTreeMap<LutInstanceId, RomReachabilityClass>,
    pub tensor_to_bank_assignment: Vec<TensorBankAssignment>,
    pub epoch_to_node_range: Vec<EpochNodeRange>,
    pub closure_to_kernels: Vec<ClosureKernelSet>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TensorBankAssignment {
    pub tensor: ValueId,
    pub bank: RomBankIndex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EpochNodeRange {
    pub epoch: ResidencyEpochId,
    pub op_range: NodeAnchorRange,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClosureKernelSet {
    pub closure: CoResidentClosureId,
    pub kernels: Vec<KernelSpecId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RomWindowPlanSummary {
    pub bank_count_used: u32,
    pub switch_count_per_token: u16,
    pub overlay_candidate_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RomWindowPlanOutput {
    pub input_identity: RomWindowPlanInputIdentity,
    pub outcome: RomWindowPlanOutcome,
    pub result: Option<RomWindowPlan>,
    pub summary: Option<RomWindowPlanSummary>,
    pub diagnostics: Vec<ValidationDiagnostic>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RomWindowPlanOutcome {
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RomWindowPlanReportBody {
    pub input_identity: RomWindowPlanInputIdentity,
    pub result: Option<RomWindowPlan>,
    pub summary: Option<RomWindowPlanSummary>,
    pub diagnostics: Vec<ValidationDiagnostic>,
}

impl ReportBody for RomWindowPlanReportBody {
    const REPORT_TYPE: &'static str = "RomWindowPlanReport";
    const SCHEMA_ID: &'static str = ROM_WINDOW_PLAN_SCHEMA_ID;
    const SCHEMA_VERSION: &'static str = "1.0.0";

    fn validate_semantics(&self, outcome: ReportOutcome) -> Result<(), Vec<ValidationDiagnostic>> {
        validate_rom_window_plan_report_body(self, outcome)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowCertBody {
    pub schema: String,
    pub schema_version: SemVer,
    pub cert_outcome: WindowCertOutcome,
    pub report_self_hash: Hash256,
    pub claim: WindowCertClaim,
    pub evidence: WindowCertEvidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum WindowCertOutcome {
    Passed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowCertClaim {
    pub rom_window_plan_self_hash: Hash256,
    pub single_window_invariant_holds: bool,
    pub overlay_install_sources_visible: bool,
    pub isr_kernels_in_bank0: bool,
    pub isr_luts_in_bank0_or_always_resident: bool,
    pub all_kernels_have_residency: bool,
    pub all_luts_have_residency: bool,
    pub co_residency_closures_well_formed: bool,
    pub epoch_bindings_cover_plan: bool,
    pub bank0_demand_within_slack: bool,
    pub overlay_demand_within_wram_reservation: bool,
    pub bank_switches_per_token: u16,
    pub bank_switches_cap: u16,
    pub bank_switches_per_token_within_cap: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowCertEvidence {
    pub kernel_residency_distribution: KernelResidencyDistribution,
    pub lut_residency_distribution: LutResidencyDistribution,
    pub overlay_install_source_epoch_count: u32,
    pub co_resident_closure_count: u32,
    pub residency_epoch_count: u32,
    pub bank0_kernel_bytes: u32,
    pub bank0_lut_bytes: u32,
    pub wram_overlay_bytes: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KernelResidencyDistribution {
    pub bank0_fixed: u32,
    pub wram_overlay: u32,
    pub co_resident_switchable: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LutResidencyDistribution {
    pub bank0_inline: u32,
    pub wram_staged: u32,
    pub rom_co_resident: u32,
}

pub fn build_rom_window_plan_from_registry(
    input: &RomWindowPlanRegistryInputs,
) -> RomWindowPlanOutput {
    if let Some(diagnostic) = validate_registry_audit_parents(input) {
        return failed_output(input.base_inputs.input_identity.clone(), vec![diagnostic]);
    }

    let mut contracted = input.base_inputs.clone();
    match enumerate_observation_registry(&input.observation_registry) {
        Ok((kernels, luts)) => {
            contracted.kernels = kernels;
            contracted.luts = luts;
        }
        Err(diagnostic) => {
            return failed_output(input.base_inputs.input_identity.clone(), vec![diagnostic]);
        }
    }

    let output = build_rom_window_plan(&contracted);
    if output.outcome != RomWindowPlanOutcome::Succeeded {
        return output;
    }
    let Some(plan) = output.result.as_ref() else {
        return failed_output(
            contracted.input_identity,
            vec![registry_diagnostic(
                "registry.output",
                "registry-backed RomWindowPlan succeeded without a product",
            )],
        );
    };
    if let Some(diagnostic) = validate_range_registry_subordinates(&input.range_registry, plan) {
        return failed_output(output.input_identity, vec![diagnostic]);
    }
    output
}

#[must_use]
pub fn rom_window_plan_registry_inputs_from_stage_facts(
    base_inputs: RomWindowPlanInputs,
    audit_parents: RomWindowPlanAuditParents,
    observation_product: &ObservationPlanCoreProduct,
    range_product: &RangePlanCoreProduct,
) -> RomWindowPlanRegistryInputs {
    RomWindowPlanRegistryInputs {
        observation_registry: rom_window_observation_registry_from_stage4(observation_product),
        range_registry: rom_window_range_registry_from_stage5(range_product),
        audit_parents,
        base_inputs,
    }
}

#[must_use]
pub fn rom_window_observation_registry_from_stage4(
    product: &ObservationPlanCoreProduct,
) -> RomWindowObservationRegistry {
    RomWindowObservationRegistry {
        observation_plan_self_hash: product.observation_plan_self_hash,
        infer_ir_self_hash: product.observation_plan.identity.infer_ir_self_hash,
        quant_graph_self_hash: product.observation_plan.identity.quant_graph_self_hash,
        source: observation_source_to_registry_source(&product.rom_window_facts.source),
        kernels: product
            .rom_window_facts
            .kernels
            .iter()
            .map(|row| ObservationKernelResidencyRecord {
                kernel: row.kernel.clone(),
                byte_size: row.byte_size,
                reachability: row.reachability.map(observation_reachability_to_rom),
                overlay_eligible: row.overlay_eligible,
                active_epochs: row
                    .active_epochs
                    .iter()
                    .copied()
                    .map(observation_epoch_to_rom)
                    .collect(),
                requested_bank: row.requested_bank.map(observation_bank_to_rom),
                source: observation_source_to_registry_source(&row.source),
            })
            .collect(),
        luts: product
            .rom_window_facts
            .luts
            .iter()
            .map(|row| ObservationLutResidencyRecord {
                lut: LutInstanceId(row.lut.clone()),
                byte_size: row.byte_size,
                reachability: row.reachability.map(observation_reachability_to_rom),
                overlay_eligible: row.overlay_eligible,
                active_epochs: row
                    .active_epochs
                    .iter()
                    .copied()
                    .map(observation_epoch_to_rom)
                    .collect(),
                requested_bank: row.requested_bank.map(observation_bank_to_rom),
                source: observation_source_to_registry_source(&row.source),
            })
            .collect(),
    }
}

#[must_use]
pub fn rom_window_range_registry_from_stage5(
    product: &RangePlanCoreProduct,
) -> RomWindowRangeRegistry {
    RomWindowRangeRegistry {
        range_plan_self_hash: product.range_plan_self_hash,
        infer_ir_self_hash: product.range_plan.identity.infer_ir_self_hash,
        quant_graph_self_hash: product.range_plan.identity.quant_graph_self_hash,
        static_budget_self_hash: product.range_plan.identity.static_budget_self_hash,
        source: range_source_to_registry_source(&product.rom_window_facts.source),
        reduction_subordinates: product
            .rom_window_facts
            .reduction_subordinates
            .iter()
            .map(|row| RangeReductionSubordinate {
                site: row.site.clone(),
                main_kernel: row.main_kernel.clone(),
                subordinate_kernel: row.subordinate_kernel.clone(),
                source: range_source_to_registry_source(&row.source),
            })
            .collect(),
    }
}

pub fn build_rom_window_plan(input: &RomWindowPlanInputs) -> RomWindowPlanOutput {
    let hash_diagnostics = input_hash_mismatch_diagnostics(input);
    if !hash_diagnostics.is_empty() {
        return failed_output(input.input_identity.clone(), hash_diagnostics);
    }

    match infer_ir_self_hash(&input.infer_ir) {
        Ok(computed) if computed == input.input_identity.infer_ir_self_hash => {}
        Ok(computed) => {
            return failed_output(
                input.input_identity.clone(),
                vec![diagnostic(
                    RomWindowPlanDiagnosticCode::RomInputHashMismatch,
                    RomWindowPlanDiagnosticProvenance::HashMismatch {
                        product: "infer_ir_self_hash".to_owned(),
                        recorded: input.input_identity.infer_ir_self_hash,
                        computed,
                    },
                )],
            );
        }
        Err(error) => {
            return failed_output(
                input.input_identity.clone(),
                vec![diagnostic(
                    RomWindowPlanDiagnosticCode::RomCanonicalSortDrift,
                    RomWindowPlanDiagnosticProvenance::PolicyProjection {
                        field: "infer_ir".to_owned(),
                        detail: error.to_string(),
                    },
                )],
            );
        }
    }

    if input.input_identity.sram_page_plan_self_hash
        != input.sram_page_plan.sram_page_plan_self_hash
    {
        return failed_output(
            input.input_identity.clone(),
            vec![diagnostic(
                RomWindowPlanDiagnosticCode::RomInputHashMismatch,
                RomWindowPlanDiagnosticProvenance::HashMismatch {
                    product: "sram_page_plan_self_hash".to_owned(),
                    recorded: input.input_identity.sram_page_plan_self_hash,
                    computed: input.sram_page_plan.sram_page_plan_self_hash,
                },
            )],
        );
    }

    let Some(bank0_cap_bytes) = bank0_cap_bytes(&input.runtime_chrome_budget) else {
        return failed_output(
            input.input_identity.clone(),
            vec![diagnostic(
                RomWindowPlanDiagnosticCode::RomTargetProfileLayoutUnsupported,
                RomWindowPlanDiagnosticProvenance::TargetProfileLayout {
                    target_profile_hash: input.input_identity.target_profile_hash,
                    detail: "runtime chrome budget has no Bank0Free ROM slot".to_owned(),
                },
            )],
        );
    };
    let switchable_caps =
        switchable_bank_caps(&input.runtime_chrome_budget, input.policy.placement_profile);
    if switchable_caps.is_empty() && needs_switchable_rom(input) {
        return failed_output(
            input.input_identity.clone(),
            vec![diagnostic(
                RomWindowPlanDiagnosticCode::RomTargetProfileLayoutUnsupported,
                RomWindowPlanDiagnosticProvenance::TargetProfileLayout {
                    target_profile_hash: input.input_identity.target_profile_hash,
                    detail:
                        "runtime chrome budget has no switchable ROM slot for placement profile"
                            .to_owned(),
                },
            )],
        );
    }

    let sram_binding_ids = input
        .sram_page_plan
        .bindings
        .iter()
        .map(|binding| binding.binding_id)
        .collect::<BTreeSet<_>>();
    for epoch in &input.epochs {
        if let Some(binding) = epoch.sram_page_binding
            && !sram_binding_ids.contains(&binding)
        {
            return failed_output(
                input.input_identity.clone(),
                vec![diagnostic(
                    RomWindowPlanDiagnosticCode::RomPolicyProjectionMismatch,
                    RomWindowPlanDiagnosticProvenance::PolicyProjection {
                        field: "epochs.sram_page_binding".to_owned(),
                        detail: format!(
                            "epoch {} references unknown SRAM binding {}",
                            epoch.id.0,
                            binding.get()
                        ),
                    },
                )],
            );
        }
    }

    if input.epochs.is_empty() {
        return failed_output(
            input.input_identity.clone(),
            vec![diagnostic(
                RomWindowPlanDiagnosticCode::RomPolicyProjectionMismatch,
                RomWindowPlanDiagnosticProvenance::PolicyProjection {
                    field: "epochs".to_owned(),
                    detail: "RomWindowPlan requires at least one residency epoch".to_owned(),
                },
            )],
        );
    }
    if let Some(diagnostic) = validate_epoch_inputs(input) {
        return failed_output(input.input_identity.clone(), vec![diagnostic]);
    }

    let mut kernels = input.kernels.clone();
    kernels.sort_by_key(|kernel| kernel.kernel.clone());
    let mut luts = input.luts.clone();
    luts.sort_by_key(|lut| lut.lut.clone());
    let mut storage_bindings = input
        .storage_bindings
        .iter()
        .filter(|binding| is_rom_const(&binding.binding.materialization))
        .cloned()
        .collect::<Vec<_>>();
    storage_bindings.sort_by_key(|binding| binding.binding.value);

    let mut allocator = BankAllocator::new(input.policy.placement_profile, switchable_caps);
    let mut kernel_residency = BTreeMap::new();
    let mut lut_residency = BTreeMap::new();
    let mut bank_occupants: BTreeMap<RomBankIndex, Vec<(RomBankOccupant, u32)>> = BTreeMap::new();
    let mut overlay_kernels = Vec::new();
    let mut overlay_luts = Vec::new();
    let mut bank0_kernels = Vec::new();
    let mut bank0_luts = Vec::new();
    let mut tensor_to_bank = BTreeMap::new();

    for kernel in &kernels {
        let residency = if kernel.reachability.requires_bank0() {
            KernelResidency::Bank0Fixed
        } else if kernel.overlay_eligible
            && matches!(
                input.policy.rom_window.kernel_residency_bias,
                gbf_policy::RomKernelResidencyBias::PreferWramOverlay
            )
        {
            KernelResidency::WramOverlay
        } else {
            let Some(bank) = allocator.assign(kernel.requested_bank) else {
                return failed_output(
                    input.input_identity.clone(),
                    vec![diagnostic(
                        RomWindowPlanDiagnosticCode::RomProfileViolation,
                        RomWindowPlanDiagnosticProvenance::Kernel {
                            invariant: "RWP-SC-11".to_owned(),
                            kernel: kernel.kernel.to_string(),
                        },
                    )],
                );
            };
            KernelResidency::CoResidentSwitchable { bank }
        };
        match residency {
            KernelResidency::Bank0Fixed => bank0_kernels.push(Bank0KernelDemand {
                kernel: kernel.kernel.clone(),
                byte_size: kernel.byte_size,
                reachability: kernel.reachability,
            }),
            KernelResidency::WramOverlay => {
                let Some(source_bank) = allocator.assign(kernel.requested_bank) else {
                    return failed_output(
                        input.input_identity.clone(),
                        vec![diagnostic(
                            RomWindowPlanDiagnosticCode::RomProfileViolation,
                            RomWindowPlanDiagnosticProvenance::Kernel {
                                invariant: "RWP-OVERLAY-SOURCE-VISIBLE".to_owned(),
                                kernel: kernel.kernel.to_string(),
                            },
                        )],
                    );
                };
                bank_occupants.entry(source_bank).or_default().push((
                    RomBankOccupant::Kernel {
                        kernel: kernel.kernel.clone(),
                    },
                    kernel.byte_size,
                ));
                overlay_kernels.push(WramOverlayKernelDemand {
                    kernel: kernel.kernel.clone(),
                    byte_size: kernel.byte_size,
                    reachability: kernel.reachability,
                    source_bank,
                    active_epochs: kernel.active_epochs.clone(),
                });
            }
            KernelResidency::CoResidentSwitchable { bank } => {
                bank_occupants.entry(bank).or_default().push((
                    RomBankOccupant::Kernel {
                        kernel: kernel.kernel.clone(),
                    },
                    kernel.byte_size,
                ))
            }
        }
        kernel_residency.insert(kernel.kernel.clone(), residency);
    }

    for lut in &luts {
        let residency = if lut.reachability.requires_bank0() {
            LutResidency::Bank0Inline
        } else if lut.overlay_eligible
            && matches!(
                input.policy.rom_window.kernel_residency_bias,
                gbf_policy::RomKernelResidencyBias::PreferWramOverlay
            )
        {
            LutResidency::WramStaged {
                always_resident: false,
            }
        } else {
            let Some(bank) = allocator.assign(lut.requested_bank) else {
                return failed_output(
                    input.input_identity.clone(),
                    vec![diagnostic(
                        RomWindowPlanDiagnosticCode::RomProfileViolation,
                        RomWindowPlanDiagnosticProvenance::Lut {
                            invariant: "RWP-SC-11".to_owned(),
                            lut: lut.lut.0.clone(),
                        },
                    )],
                );
            };
            LutResidency::RomCoResident { bank }
        };
        match residency {
            LutResidency::Bank0Inline => bank0_luts.push(Bank0LutDemand {
                lut: lut.lut.clone(),
                byte_size: lut.byte_size,
                reachability: lut.reachability,
            }),
            LutResidency::WramStaged { .. } => {
                let Some(source_bank) = allocator.assign(lut.requested_bank) else {
                    return failed_output(
                        input.input_identity.clone(),
                        vec![diagnostic(
                            RomWindowPlanDiagnosticCode::RomProfileViolation,
                            RomWindowPlanDiagnosticProvenance::Lut {
                                invariant: "RWP-OVERLAY-SOURCE-VISIBLE".to_owned(),
                                lut: lut.lut.0.clone(),
                            },
                        )],
                    );
                };
                bank_occupants.entry(source_bank).or_default().push((
                    RomBankOccupant::Lut {
                        lut: lut.lut.clone(),
                    },
                    lut.byte_size,
                ));
                overlay_luts.push(WramOverlayLutDemand {
                    lut: lut.lut.clone(),
                    byte_size: lut.byte_size,
                    reachability: lut.reachability,
                    source_bank,
                    active_epochs: lut.active_epochs.clone(),
                });
            }
            LutResidency::RomCoResident { bank } => bank_occupants.entry(bank).or_default().push((
                RomBankOccupant::Lut {
                    lut: lut.lut.clone(),
                },
                lut.byte_size,
            )),
        }
        lut_residency.insert(lut.lut.clone(), residency);
    }

    for binding in &storage_bindings {
        if binding.payload_bytes == 0 {
            return failed_output(
                input.input_identity.clone(),
                vec![diagnostic(
                    RomWindowPlanDiagnosticCode::RomPolicyProjectionMismatch,
                    RomWindowPlanDiagnosticProvenance::PolicyProjection {
                        field: "storage_bindings.payload_bytes".to_owned(),
                        detail: format!(
                            "ROM const binding {} has zero payload bytes",
                            binding.binding.value.get()
                        ),
                    },
                )],
            );
        }
        let Some(bank) = allocator.assign(binding.requested_bank) else {
            return failed_output(
                input.input_identity.clone(),
                vec![diagnostic(
                    RomWindowPlanDiagnosticCode::RomProfileViolation,
                    RomWindowPlanDiagnosticProvenance::Binding {
                        invariant: "RWP-SC-11".to_owned(),
                        binding_id: binding.binding.value.get(),
                    },
                )],
            );
        };
        tensor_to_bank.insert(binding.binding.value, bank);
        bank_occupants.entry(bank).or_default().push((
            RomBankOccupant::Tensor {
                value: binding.binding.value,
            },
            binding.payload_bytes,
        ));
    }

    let banks = build_bank_assignments(&bank_occupants, &allocator.cap_by_bank);
    if let Some(overfull) = banks.iter().find(|bank| bank.slack_bytes < 0) {
        return failed_output(
            input.input_identity.clone(),
            vec![diagnostic(
                RomWindowPlanDiagnosticCode::RomBankCapacityExceeded,
                RomWindowPlanDiagnosticProvenance::Bank {
                    bank: overfull.bank.0,
                    observed_bytes: overfull.total_bytes,
                    cap_bytes: overfull.cap_bytes,
                },
            )],
        );
    }

    let bank0_total_kernel_bytes = bank0_kernels
        .iter()
        .map(|kernel| kernel.byte_size)
        .sum::<u32>();
    let bank0_total_lut_bytes = bank0_luts.iter().map(|lut| lut.byte_size).sum::<u32>();
    let bank0_total = bank0_total_kernel_bytes.saturating_add(bank0_total_lut_bytes);
    let bank0_demand = Bank0Demand {
        kernels: bank0_kernels,
        luts: bank0_luts,
        total_kernel_bytes: bank0_total_kernel_bytes,
        total_lut_bytes: bank0_total_lut_bytes,
        remaining_slack_bytes: i64::from(bank0_cap_bytes) - i64::from(bank0_total),
    };
    if bank0_demand.remaining_slack_bytes < 0 {
        return failed_output(
            input.input_identity.clone(),
            vec![diagnostic(
                RomWindowPlanDiagnosticCode::RomBank0OverBudget,
                RomWindowPlanDiagnosticProvenance::Bank0Demand {
                    total_kernel_bytes: bank0_demand.total_kernel_bytes,
                    total_lut_bytes: bank0_demand.total_lut_bytes,
                    bank0_cap_bytes,
                },
            )],
        );
    }

    let overlay_total = overlay_kernels
        .iter()
        .map(|kernel| kernel.byte_size)
        .chain(overlay_luts.iter().map(|lut| lut.byte_size))
        .sum::<u32>();
    let overlay_count = overlay_kernels.len().saturating_add(overlay_luts.len());
    let mut overlay_demand = WramOverlayDemand {
        kernels: overlay_kernels,
        luts: overlay_luts,
        install_source_visibility: Vec::new(),
        total_overlay_bytes: overlay_total,
        total_install_count_per_token_upper_bound: saturating_u16(overlay_count),
    };
    let overlay_cap = u32::from(input.runtime_chrome_budget.wram_reserved);
    if overlay_demand.total_overlay_bytes > overlay_cap {
        return failed_output(
            input.input_identity.clone(),
            vec![diagnostic(
                RomWindowPlanDiagnosticCode::RomOverlayDemandExceedsWramReservation,
                RomWindowPlanDiagnosticProvenance::OverlayDemand {
                    declared_bytes: overlay_demand.total_overlay_bytes,
                    wram_reserved_bytes: overlay_cap,
                },
            )],
        );
    }

    let bindings = build_window_bindings(
        input,
        &kernels,
        &luts,
        &storage_bindings,
        &kernel_residency,
        &lut_residency,
        &tensor_to_bank,
        &overlay_demand,
    );
    overlay_demand.install_source_visibility =
        build_overlay_install_source_visibility(&overlay_demand, &bindings);
    if let Some((epoch, demanded)) = bindings.iter().find_map(|binding| {
        demanded_banks_for_binding(
            binding,
            &kernel_residency,
            &lut_residency,
            &tensor_to_bank,
            &overlay_demand,
        )
        .map(|banks| (binding.epoch, banks))
    }) {
        return failed_output(
            input.input_identity.clone(),
            vec![diagnostic(
                RomWindowPlanDiagnosticCode::RomMultipleSwitchableBanksDemandedInPhase,
                RomWindowPlanDiagnosticProvenance::Phase {
                    epoch: epoch.0,
                    demanded_banks: demanded.into_iter().map(|bank| bank.0).collect(),
                },
            )],
        );
    }
    if let Some(diagnostic) = validate_overlay_install_source_visibility(&overlay_demand, &bindings)
    {
        return failed_output(input.input_identity.clone(), vec![diagnostic]);
    }

    let projections = switch_projections(&bindings);
    if let Some(cap) = input.policy.max_bank_switches_per_token
        && projections.projected_bank_switches_per_token > cap
    {
        return failed_output(
            input.input_identity.clone(),
            vec![diagnostic(
                RomWindowPlanDiagnosticCode::RomBankSwitchBudgetExceeded,
                RomWindowPlanDiagnosticProvenance::Budget {
                    decision_value: projections.projected_bank_switches_per_token,
                    upper_bound: projections.upper_bound_per_token,
                    cap,
                },
            )],
        );
    }

    let residency_epochs = input
        .epochs
        .iter()
        .enumerate()
        .map(|(index, epoch)| ResidencyEpoch {
            id: epoch.id,
            op_range: epoch.op_range,
            rom_window_binding: RomWindowBindingId(index as u32),
            sram_page_binding: epoch.sram_page_binding,
            overlay_state: if overlay_demand.total_overlay_bytes == 0 {
                OverlayState::NoOverlayActive
            } else {
                OverlayState::OverlayActive
            },
            yield_kind: epoch.yield_kind,
        })
        .collect::<Vec<_>>();

    let co_resident_closures = build_co_resident_closures(&banks);
    let closure_to_kernels = co_resident_closures
        .iter()
        .map(|closure| ClosureKernelSet {
            closure: closure.id,
            kernels: closure.kernels.clone(),
        })
        .collect::<Vec<_>>();
    let summary = RomWindowPlanSummary {
        bank_count_used: banks.len() as u32,
        switch_count_per_token: projections.projected_bank_switches_per_token,
        overlay_candidate_count: overlay_count as u32,
    };
    let mut plan = RomWindowPlan {
        identity: input.input_identity.clone(),
        kernel_residency,
        lut_residency,
        rom_window_bindings: bindings,
        banks,
        residency_epochs,
        co_resident_closures,
        overlay_demand,
        bank0_demand,
        projections,
        profile: input.policy.placement_profile,
        provenance: RomWindowPlanProvenance {
            kernel_to_reachability: kernels
                .iter()
                .map(|kernel| (kernel.kernel.clone(), kernel.reachability))
                .collect(),
            lut_to_reachability: luts
                .iter()
                .map(|lut| (lut.lut.clone(), lut.reachability))
                .collect(),
            tensor_to_bank_assignment: tensor_to_bank
                .iter()
                .map(|(tensor, bank)| TensorBankAssignment {
                    tensor: *tensor,
                    bank: *bank,
                })
                .collect(),
            epoch_to_node_range: input
                .epochs
                .iter()
                .map(|epoch| EpochNodeRange {
                    epoch: epoch.id,
                    op_range: epoch.op_range,
                })
                .collect(),
            closure_to_kernels,
        },
        rom_window_plan_self_hash: Hash256::ZERO,
    };

    if let Some(diagnostic) = validate_rom_window_plan_product_surface(&plan) {
        return failed_output(input.input_identity.clone(), vec![diagnostic]);
    }
    if let Some(diagnostic) = validate_rom_window_plan_epoch_surface(&plan) {
        return failed_output(input.input_identity.clone(), vec![diagnostic]);
    }

    let self_hash = match rom_window_plan_self_hash(&plan) {
        Ok(hash) => hash,
        Err(error) => {
            return failed_output(
                input.input_identity.clone(),
                vec![diagnostic(
                    RomWindowPlanDiagnosticCode::RomCanonicalSortDrift,
                    RomWindowPlanDiagnosticProvenance::PolicyProjection {
                        field: "rom_window_plan_self_hash".to_owned(),
                        detail: error.to_string(),
                    },
                )],
            );
        }
    };
    plan.rom_window_plan_self_hash = self_hash;

    RomWindowPlanOutput {
        input_identity: input.input_identity.clone(),
        outcome: RomWindowPlanOutcome::Succeeded,
        result: Some(plan),
        summary: Some(summary),
        diagnostics: Vec::new(),
    }
}

pub fn emit_rom_window_plan_report(
    output: &RomWindowPlanOutput,
) -> Result<RomWindowPlanReportEnvelope, RomWindowPlanEmitError> {
    let outcome = match output.outcome {
        RomWindowPlanOutcome::Succeeded => ReportOutcome::Passed,
        RomWindowPlanOutcome::Failed => ReportOutcome::Failed,
    };
    let body = RomWindowPlanReportBody {
        input_identity: output.input_identity.clone(),
        result: output.result.clone(),
        summary: output.summary,
        diagnostics: output.diagnostics.clone(),
    };
    let envelope = ReportEnvelope::new(outcome, body)?.with_computed_self_hash()?;
    round_trip_self_hash(&envelope)?;
    Ok(envelope)
}

pub fn emit_rom_window_plan_json_bytes(
    output: &RomWindowPlanOutput,
) -> Result<Vec<u8>, RomWindowPlanEmitError> {
    Ok(canonicalize(&emit_rom_window_plan_report(output)?)?)
}

pub fn emit_window_cert_report(
    output: &RomWindowPlanOutput,
    report_self_hash: Hash256,
) -> Result<Option<WindowCertBody>, RomWindowPlanEmitError> {
    let Some(body) = build_window_cert_body(output, report_self_hash) else {
        return Ok(None);
    };
    if let Err(diagnostics) = validate_window_cert_body(&body, ReportOutcome::Passed) {
        return Err(RomWindowPlanEmitError::CertificateInvariant(diagnostics));
    }
    Ok(Some(body))
}

pub fn emit_window_cert_json_bytes(
    output: &RomWindowPlanOutput,
    report_self_hash: Hash256,
) -> Result<Option<Vec<u8>>, RomWindowPlanEmitError> {
    emit_window_cert_report(output, report_self_hash)?
        .map(|body| {
            canonical_json_bytes_omitting_fields(&body, &[])
                .map_err(RomWindowPlanEmitError::ProductCanonical)
        })
        .transpose()
}

pub fn parse_rom_window_plan_report_bytes(
    bytes: &[u8],
) -> Result<RomWindowPlanReportEnvelope, ReportSelfHashError> {
    serde_json::from_slice(bytes).map_err(|error| ReportSelfHashError::Json {
        reason: error.to_string(),
    })
}

pub fn parse_window_cert_report_bytes(bytes: &[u8]) -> Result<WindowCertBody, ReportSelfHashError> {
    serde_json::from_slice(bytes).map_err(|error| ReportSelfHashError::Json {
        reason: error.to_string(),
    })
}

pub fn rom_window_plan_self_hash(plan: &RomWindowPlan) -> Result<Hash256, CanonicalJsonError> {
    self_hash_omitting_fields(
        DomainHash::new(
            "gbf-codegen",
            "RomWindowPlan",
            ROM_WINDOW_PLAN_SCHEMA_ID,
            "1.0.0",
        ),
        plan,
        "rom_window_plan_self_hash",
        &[],
    )
}

pub fn rom_window_plan_policy_projection_hash(
    policy: &RomWindowPlanPolicyProjection,
) -> Result<Hash256, CanonicalJsonError> {
    DomainHash::new(
        "gbf-codegen",
        "RomWindowPlanPolicyProjection",
        ROM_WINDOW_PLAN_SCHEMA_ID,
        "1.0.0",
    )
    .hash(policy)
}

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RomWindowPlanCacheKey(pub Hash256);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RomWindowPlanCacheKeyInputs {
    pub artifact_validation_self_hash: Hash256,
    pub policy_resolution_self_hash: Hash256,
    pub static_budget_self_hash: Hash256,
    pub quant_graph_self_hash: Hash256,
    pub infer_ir_self_hash: Hash256,
    pub storage_plan_self_hash: Hash256,
    pub observation_plan_self_hash: Hash256,
    pub range_plan_self_hash: Hash256,
    pub sram_page_plan_self_hash: Hash256,
    pub runtime_chrome_budget_hash: Hash256,
    pub target_profile_hash: Hash256,
    pub rom_window_plan_policy_projection_hash: Hash256,
    pub runtime_mode: RuntimeMode,
    pub pass_version: String,
    pub crate_feature_set_hash: Hash256,
}

impl RomWindowPlanCacheKeyInputs {
    #[must_use]
    pub fn from_input_identity(
        identity: &RomWindowPlanInputIdentity,
        crate_feature_set_hash: Hash256,
    ) -> Self {
        Self {
            artifact_validation_self_hash: identity.artifact_validation_self_hash,
            policy_resolution_self_hash: identity.policy_resolution_self_hash,
            static_budget_self_hash: identity.static_budget_self_hash,
            quant_graph_self_hash: identity.quant_graph_self_hash,
            infer_ir_self_hash: identity.infer_ir_self_hash,
            storage_plan_self_hash: identity.storage_plan_self_hash,
            observation_plan_self_hash: identity.observation_plan_self_hash,
            range_plan_self_hash: identity.range_plan_self_hash,
            sram_page_plan_self_hash: identity.sram_page_plan_self_hash,
            runtime_chrome_budget_hash: identity.runtime_chrome_budget_hash,
            target_profile_hash: identity.target_profile_hash,
            rom_window_plan_policy_projection_hash: identity.rom_window_plan_policy_projection_hash,
            runtime_mode: identity.runtime_mode,
            pass_version: ROM_WINDOW_PLAN_PASS_VERSION.to_owned(),
            crate_feature_set_hash,
        }
    }

    pub fn cache_key(&self) -> Result<RomWindowPlanCacheKey, CanonicalJsonError> {
        rom_window_plan_cache_key(self)
    }
}

pub fn rom_window_plan_cache_key(
    inputs: &RomWindowPlanCacheKeyInputs,
) -> Result<RomWindowPlanCacheKey, CanonicalJsonError> {
    let canonical = canonical_json_bytes_omitting_fields(inputs, &[])?;
    DomainHash::new("gbf-codegen", "StageCacheKey", "rom_window_plan", "v1")
        .hash_canonical_bytes(&canonical)
        .map(RomWindowPlanCacheKey)
}

pub fn run_rom_window_plan_with_cache(
    cache: &StoreStageCache<'_>,
    input: &RomWindowPlanInputs,
    expected_hashes: StoreBackedStageExpectedHashes,
) -> Result<StoreBackedStageRunOutput<RomWindowPlan>, CodegenStageCacheError> {
    let cache_key = RomWindowPlanCacheKeyInputs::from_input_identity(
        &input.input_identity,
        crate_feature_set_hash(),
    )
    .cache_key()
    .map_err(|error| CodegenStageCacheError::StageCacheKey {
        stage_id: "8",
        message: error.to_string(),
    })?;
    let keys = StoreBackedStageCacheKeys::new(
        "8",
        stage8_rom_window_plan_store_key(cache_key, StoreBackedStageCellKind::Success),
        stage8_rom_window_plan_store_key(cache_key, StoreBackedStageCellKind::FailureMemo),
    );
    run_store_backed_stage_with_cache(cache, &keys, cache_key.0, expected_hashes, || {
        let output = build_rom_window_plan(input);
        let report = emit_rom_window_plan_report(&output).map_err(|error| {
            CodegenStageCacheError::StageEmit {
                stage_id: "8",
                message: error.to_string(),
            }
        })?;
        let report_self_hash = report.report_self_hash;
        match output.outcome {
            RomWindowPlanOutcome::Succeeded => {
                let product =
                    output
                        .result
                        .ok_or_else(|| CodegenStageCacheError::StageOutputInvariant {
                            stage_id: "8",
                            message: "succeeded output is missing RomWindowPlan product".to_owned(),
                        })?;
                Ok(StoreBackedStageRunResult::Success {
                    product_self_hash: product.rom_window_plan_self_hash,
                    product,
                    report_self_hash,
                })
            }
            RomWindowPlanOutcome::Failed => Ok(StoreBackedStageRunResult::FailureMemo {
                diagnostics: output.diagnostics,
                report_self_hash,
            }),
        }
    })
}

#[derive(Debug)]
pub enum RomWindowPlanEmitError {
    Envelope(ReportEnvelopeError),
    SelfHash(ReportSelfHashError),
    Canonical(ReportCanonicalJsonError),
    ProductCanonical(CanonicalJsonError),
    CertificateInvariant(Vec<ValidationDiagnostic>),
}

impl fmt::Display for RomWindowPlanEmitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Envelope(error) => write!(f, "rom window report envelope failed: {error}"),
            Self::SelfHash(error) => write!(f, "rom window report self hash failed: {error}"),
            Self::Canonical(error) => {
                write!(f, "rom window report canonicalization failed: {error}")
            }
            Self::ProductCanonical(error) => {
                write!(f, "rom window product canonicalization failed: {error}")
            }
            Self::CertificateInvariant(diagnostics) => {
                write!(
                    f,
                    "rom window certificate invariant failed with {} diagnostics",
                    diagnostics.len()
                )
            }
        }
    }
}

impl Error for RomWindowPlanEmitError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Envelope(error) => Some(error),
            Self::SelfHash(error) => Some(error),
            Self::Canonical(error) => Some(error),
            Self::ProductCanonical(error) => Some(error),
            Self::CertificateInvariant(_) => None,
        }
    }
}

impl From<ReportEnvelopeError> for RomWindowPlanEmitError {
    fn from(error: ReportEnvelopeError) -> Self {
        Self::Envelope(error)
    }
}

impl From<ReportSelfHashError> for RomWindowPlanEmitError {
    fn from(error: ReportSelfHashError) -> Self {
        Self::SelfHash(error)
    }
}

impl From<ReportCanonicalJsonError> for RomWindowPlanEmitError {
    fn from(error: ReportCanonicalJsonError) -> Self {
        Self::Canonical(error)
    }
}

fn validate_registry_audit_parents(
    input: &RomWindowPlanRegistryInputs,
) -> Option<ValidationDiagnostic> {
    let identity = &input.base_inputs.input_identity;
    let audit = input.audit_parents;
    let observation = &input.observation_registry;
    let range = &input.range_registry;
    for (field, recorded, computed) in [
        (
            "audit_parents.artifact_validation_self_hash",
            audit.artifact_validation_self_hash,
            identity.artifact_validation_self_hash,
        ),
        (
            "audit_parents.policy_resolution_self_hash",
            audit.policy_resolution_self_hash,
            identity.policy_resolution_self_hash,
        ),
        (
            "audit_parents.static_budget_self_hash",
            audit.static_budget_self_hash,
            identity.static_budget_self_hash,
        ),
        (
            "audit_parents.quant_graph_self_hash",
            audit.quant_graph_self_hash,
            identity.quant_graph_self_hash,
        ),
        (
            "audit_parents.infer_ir_self_hash",
            audit.infer_ir_self_hash,
            identity.infer_ir_self_hash,
        ),
        (
            "audit_parents.observation_plan_self_hash",
            audit.observation_plan_self_hash,
            identity.observation_plan_self_hash,
        ),
        (
            "audit_parents.range_plan_self_hash",
            audit.range_plan_self_hash,
            identity.range_plan_self_hash,
        ),
        (
            "audit_parents.storage_plan_self_hash",
            audit.storage_plan_self_hash,
            identity.storage_plan_self_hash,
        ),
        (
            "audit_parents.sram_page_plan_self_hash",
            audit.sram_page_plan_self_hash,
            identity.sram_page_plan_self_hash,
        ),
        (
            "observation_registry.observation_plan_self_hash",
            observation.observation_plan_self_hash,
            identity.observation_plan_self_hash,
        ),
        (
            "range_registry.range_plan_self_hash",
            range.range_plan_self_hash,
            identity.range_plan_self_hash,
        ),
        (
            "observation_registry.infer_ir_self_hash",
            observation.infer_ir_self_hash,
            audit.infer_ir_self_hash,
        ),
        (
            "observation_registry.quant_graph_self_hash",
            observation.quant_graph_self_hash,
            audit.quant_graph_self_hash,
        ),
        (
            "range_registry.infer_ir_self_hash",
            range.infer_ir_self_hash,
            audit.infer_ir_self_hash,
        ),
        (
            "range_registry.quant_graph_self_hash",
            range.quant_graph_self_hash,
            audit.quant_graph_self_hash,
        ),
        (
            "range_registry.static_budget_self_hash",
            range.static_budget_self_hash,
            audit.static_budget_self_hash,
        ),
    ] {
        if recorded != computed {
            return Some(diagnostic(
                RomWindowPlanDiagnosticCode::RomInputHashMismatch,
                RomWindowPlanDiagnosticProvenance::HashMismatch {
                    product: field.to_owned(),
                    recorded,
                    computed,
                },
            ));
        }
    }
    None
}

fn observation_source_to_registry_source(
    source: &ObservationRomWindowFactSource,
) -> RomWindowRegistrySource {
    match source {
        ObservationRomWindowFactSource::Available { registry_key } => {
            RomWindowRegistrySource::Available {
                registry_key: registry_key.clone(),
            }
        }
        ObservationRomWindowFactSource::Missing { registry_key } => {
            RomWindowRegistrySource::Missing {
                registry_key: registry_key.clone(),
            }
        }
        ObservationRomWindowFactSource::SourceImpossible {
            registry_key,
            reason,
        } => RomWindowRegistrySource::SourceImpossible {
            registry_key: registry_key.clone(),
            reason: reason.clone(),
        },
    }
}

fn range_source_to_registry_source(source: &RangeRomWindowFactSource) -> RomWindowRegistrySource {
    match source {
        RangeRomWindowFactSource::Available { registry_key } => {
            RomWindowRegistrySource::Available {
                registry_key: registry_key.clone(),
            }
        }
        RangeRomWindowFactSource::Missing { registry_key } => RomWindowRegistrySource::Missing {
            registry_key: registry_key.clone(),
        },
        RangeRomWindowFactSource::SourceImpossible {
            registry_key,
            reason,
        } => RomWindowRegistrySource::SourceImpossible {
            registry_key: registry_key.clone(),
            reason: reason.clone(),
        },
    }
}

const fn observation_reachability_to_rom(
    reachability: ObservationRomReachabilityClass,
) -> RomReachabilityClass {
    match reachability {
        ObservationRomReachabilityClass::HotPath => RomReachabilityClass::HotPath,
        ObservationRomReachabilityClass::IsrReachable => RomReachabilityClass::IsrReachable,
        ObservationRomReachabilityClass::YieldResumeReachable => {
            RomReachabilityClass::YieldResumeReachable
        }
        ObservationRomReachabilityClass::FaultPathReachable => {
            RomReachabilityClass::FaultPathReachable
        }
    }
}

const fn observation_epoch_to_rom(epoch: ObservationResidencyEpochId) -> ResidencyEpochId {
    ResidencyEpochId(epoch.0)
}

const fn observation_bank_to_rom(bank: ObservationRomBankIndex) -> RomBankIndex {
    RomBankIndex(bank.0)
}

#[allow(clippy::result_large_err)]
fn enumerate_observation_registry(
    registry: &RomWindowObservationRegistry,
) -> Result<(Vec<KernelResidencyInput>, Vec<LutResidencyInput>), ValidationDiagnostic> {
    validate_registry_collection_source("observation_registry.source", &registry.source)?;

    let mut seen_kernels = BTreeSet::new();
    let mut kernels = Vec::with_capacity(registry.kernels.len());
    for row in &registry.kernels {
        validate_registry_source(
            "observation_registry.kernels",
            &row.source,
            row.reachability,
        )?;
        if row.byte_size == 0 {
            return Err(registry_diagnostic(
                "observation_registry.kernels.byte_size",
                format!("kernel {} has zero byte size", row.kernel),
            ));
        }
        if !seen_kernels.insert(row.kernel.clone()) {
            return Err(registry_diagnostic(
                "observation_registry.kernels.kernel",
                format!("duplicate kernel {}", row.kernel),
            ));
        }
        let Some(reachability) = row.reachability else {
            return Err(registry_diagnostic(
                "observation_registry.kernels.reachability",
                format!("kernel {} is available but has no reachability", row.kernel),
            ));
        };
        kernels.push(KernelResidencyInput {
            kernel: row.kernel.clone(),
            byte_size: row.byte_size,
            reachability,
            overlay_eligible: row.overlay_eligible,
            active_epochs: row.active_epochs.clone(),
            requested_bank: row.requested_bank,
        });
    }

    let mut seen_luts = BTreeSet::new();
    let mut luts = Vec::with_capacity(registry.luts.len());
    for row in &registry.luts {
        validate_registry_source("observation_registry.luts", &row.source, row.reachability)?;
        if row.byte_size == 0 {
            return Err(registry_diagnostic(
                "observation_registry.luts.byte_size",
                format!("LUT {} has zero byte size", row.lut.0),
            ));
        }
        if !seen_luts.insert(row.lut.clone()) {
            return Err(registry_diagnostic(
                "observation_registry.luts.lut",
                format!("duplicate LUT {}", row.lut.0),
            ));
        }
        let Some(reachability) = row.reachability else {
            return Err(registry_diagnostic(
                "observation_registry.luts.reachability",
                format!("LUT {} is available but has no reachability", row.lut.0),
            ));
        };
        luts.push(LutResidencyInput {
            lut: row.lut.clone(),
            byte_size: row.byte_size,
            reachability,
            overlay_eligible: row.overlay_eligible,
            active_epochs: row.active_epochs.clone(),
            requested_bank: row.requested_bank,
        });
    }
    Ok((kernels, luts))
}

#[allow(clippy::result_large_err)]
fn validate_registry_source(
    field: &str,
    source: &RomWindowRegistrySource,
    reachability: Option<RomReachabilityClass>,
) -> Result<(), ValidationDiagnostic> {
    match source {
        RomWindowRegistrySource::Available { registry_key } => {
            if reachability.is_some() {
                Ok(())
            } else {
                Err(registry_diagnostic(
                    field,
                    format!("registry key {registry_key} is available but has no reachability"),
                ))
            }
        }
        RomWindowRegistrySource::Missing { registry_key } => Err(registry_diagnostic(
            field,
            format!("registry missing source for key {registry_key}"),
        )),
        RomWindowRegistrySource::SourceImpossible {
            registry_key,
            reason,
        } => Err(registry_diagnostic(
            field,
            format!("registry source impossible for key {registry_key}: {reason}"),
        )),
    }
}

#[allow(clippy::result_large_err)]
fn validate_registry_collection_source(
    field: &str,
    source: &RomWindowRegistrySource,
) -> Result<(), ValidationDiagnostic> {
    match source {
        RomWindowRegistrySource::Available { .. } => Ok(()),
        RomWindowRegistrySource::Missing { registry_key } => Err(registry_diagnostic(
            field,
            format!("registry missing source for key {registry_key}"),
        )),
        RomWindowRegistrySource::SourceImpossible {
            registry_key,
            reason,
        } => Err(registry_diagnostic(
            field,
            format!("registry source impossible for key {registry_key}: {reason}"),
        )),
    }
}

fn validate_range_registry_subordinates(
    registry: &RomWindowRangeRegistry,
    plan: &RomWindowPlan,
) -> Option<ValidationDiagnostic> {
    if let Err(diagnostic) =
        validate_registry_collection_source("range_registry.source", &registry.source)
    {
        return Some(diagnostic);
    }

    for subordinate in &registry.reduction_subordinates {
        if let Err(diagnostic) = validate_registry_collection_source(
            "range_registry.reduction_subordinates",
            &subordinate.source,
        ) {
            return Some(diagnostic);
        }
        let Some(main) = plan.kernel_residency.get(&subordinate.main_kernel) else {
            return Some(registry_diagnostic(
                "range_registry.reduction_subordinates.main_kernel",
                format!(
                    "reduction site {} references kernel {} missing from ObservationPlan registry",
                    subordinate.site.0, subordinate.main_kernel
                ),
            ));
        };
        let Some(child) = plan.kernel_residency.get(&subordinate.subordinate_kernel) else {
            return Some(registry_diagnostic(
                "range_registry.reduction_subordinates.subordinate_kernel",
                format!(
                    "reduction site {} references subordinate kernel {} missing from ObservationPlan registry",
                    subordinate.site.0, subordinate.subordinate_kernel
                ),
            ));
        };
        if let Some((main_bank, child_bank)) = split_switchable_banks(main, child) {
            return Some(registry_diagnostic(
                "range_registry.reduction_subordinates",
                format!(
                    "reduction site {} splits main kernel {} in bank {} from subordinate kernel {} in bank {}",
                    subordinate.site.0,
                    subordinate.main_kernel,
                    main_bank.0,
                    subordinate.subordinate_kernel,
                    child_bank.0
                ),
            ));
        }
    }
    None
}

fn split_switchable_banks(
    main: &KernelResidency,
    child: &KernelResidency,
) -> Option<(RomBankIndex, RomBankIndex)> {
    match (main, child) {
        (
            KernelResidency::CoResidentSwitchable { bank: main_bank },
            KernelResidency::CoResidentSwitchable { bank: child_bank },
        ) if main_bank != child_bank => Some((*main_bank, *child_bank)),
        _ => None,
    }
}

fn registry_diagnostic(
    field: impl Into<String>,
    detail: impl Into<String>,
) -> ValidationDiagnostic {
    diagnostic(
        RomWindowPlanDiagnosticCode::RomPolicyProjectionMismatch,
        RomWindowPlanDiagnosticProvenance::PolicyProjection {
            field: field.into(),
            detail: detail.into(),
        },
    )
}

fn input_hash_mismatch_diagnostics(input: &RomWindowPlanInputs) -> Vec<ValidationDiagnostic> {
    ROM_WINDOW_PLAN_INPUT_PRODUCTS
        .iter()
        .copied()
        .filter_map(|product| {
            let recorded = input.input_identity.hash_for_product(product);
            let computed = input.expected_input_hashes.hash_for_product(product);
            (recorded != computed).then(|| {
                diagnostic(
                    RomWindowPlanDiagnosticCode::RomInputHashMismatch,
                    RomWindowPlanDiagnosticProvenance::HashMismatch {
                        product: product.field_name().to_owned(),
                        recorded,
                        computed,
                    },
                )
            })
        })
        .collect()
}

pub fn validate_rom_window_plan_product_surface(
    plan: &RomWindowPlan,
) -> Option<ValidationDiagnostic> {
    let value = serde_json::to_value(plan).expect("rom window plan serializes");
    validate_rom_window_plan_json_surface(&value)
}

pub fn validate_rom_window_plan_json_surface(
    value: &serde_json::Value,
) -> Option<ValidationDiagnostic> {
    let text = value.to_string();
    for forbidden in ["AsmIR", "SectionRole"] {
        if text.contains(forbidden) {
            return Some(diagnostic(
                RomWindowPlanDiagnosticCode::RomSectionRoleLeaked,
                RomWindowPlanDiagnosticProvenance::JsonPath {
                    json_path: "$".to_owned(),
                    field_or_tag: forbidden.to_owned(),
                },
            ));
        }
    }
    for forbidden in ["SliceId", "LeaseId"] {
        if text.contains(forbidden) {
            return Some(diagnostic(
                RomWindowPlanDiagnosticCode::RomSchedulingFieldLeaked,
                RomWindowPlanDiagnosticProvenance::JsonPath {
                    json_path: "$".to_owned(),
                    field_or_tag: forbidden.to_owned(),
                },
            ));
        }
    }
    if text.contains("RepairProposal") || text.contains("repair_proposals") {
        return Some(diagnostic(
            RomWindowPlanDiagnosticCode::RomRepairProvenanceForbidden,
            RomWindowPlanDiagnosticProvenance::JsonPath {
                json_path: "$".to_owned(),
                field_or_tag: "repair".to_owned(),
            },
        ));
    }
    None
}

fn validate_rom_window_plan_report_body(
    body: &RomWindowPlanReportBody,
    outcome: ReportOutcome,
) -> Result<(), Vec<ValidationDiagnostic>> {
    let has_hard = body
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Hard);
    let has_soft = body
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Soft);
    let mut diagnostics = Vec::new();
    match outcome {
        ReportOutcome::Passed => {
            if body.result.is_none() || body.summary.is_none() || has_hard || has_soft {
                diagnostics.push(report_invariant("rom_window_plan.passed"));
            }
        }
        ReportOutcome::Failed => {
            if body.result.is_some() || body.summary.is_some() || !has_hard || has_soft {
                diagnostics.push(report_invariant("rom_window_plan.failed"));
            }
        }
    }
    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(diagnostics)
    }
}

fn validate_window_cert_body(
    body: &WindowCertBody,
    outcome: ReportOutcome,
) -> Result<(), Vec<ValidationDiagnostic>> {
    let claim = body.claim;
    let valid = matches!(outcome, ReportOutcome::Passed)
        && body.schema == WINDOW_CERT_SCHEMA_ID
        && body.schema_version == WINDOW_CERT_SCHEMA_VERSION
        && matches!(body.cert_outcome, WindowCertOutcome::Passed)
        && claim.single_window_invariant_holds
        && claim.overlay_install_sources_visible
        && claim.isr_kernels_in_bank0
        && claim.isr_luts_in_bank0_or_always_resident
        && claim.all_kernels_have_residency
        && claim.all_luts_have_residency
        && claim.co_residency_closures_well_formed
        && claim.epoch_bindings_cover_plan
        && claim.bank0_demand_within_slack
        && claim.overlay_demand_within_wram_reservation
        && claim.bank_switches_per_token_within_cap;
    if valid {
        Ok(())
    } else {
        Err(vec![report_invariant("window_cert.claim")])
    }
}

fn validate_epoch_inputs(input: &RomWindowPlanInputs) -> Option<ValidationDiagnostic> {
    let mut epochs = BTreeSet::new();
    let mut previous_last = None;
    for epoch in &input.epochs {
        if !epochs.insert(epoch.id) {
            return Some(diagnostic(
                RomWindowPlanDiagnosticCode::RomPolicyProjectionMismatch,
                RomWindowPlanDiagnosticProvenance::PolicyProjection {
                    field: "epochs.id".to_owned(),
                    detail: format!("duplicate residency epoch {}", epoch.id.0),
                },
            ));
        }
        if epoch.op_range.first_node > epoch.op_range.last_node {
            return Some(diagnostic(
                RomWindowPlanDiagnosticCode::RomPolicyProjectionMismatch,
                RomWindowPlanDiagnosticProvenance::PolicyProjection {
                    field: "epochs.op_range".to_owned(),
                    detail: format!("epoch {} has an inverted node range", epoch.id.0),
                },
            ));
        }
        if let Some(last) = previous_last
            && epoch.op_range.first_node.get() != last
        {
            return Some(diagnostic(
                RomWindowPlanDiagnosticCode::RomPolicyProjectionMismatch,
                RomWindowPlanDiagnosticProvenance::PolicyProjection {
                    field: "epochs.op_range".to_owned(),
                    detail: format!(
                        "epoch {} starts at node {}, expected {}",
                        epoch.id.0,
                        epoch.op_range.first_node.get(),
                        last
                    ),
                },
            ));
        }
        previous_last = Some(epoch.op_range.last_node.get());
    }

    if let Some(diagnostic) = validate_sram_epoch_alignment(input) {
        return Some(diagnostic);
    }
    if let Some(diagnostic) = validate_infer_ir_hot_epoch_coverage(input) {
        return Some(diagnostic);
    }

    let references_epoch = |active_epochs: &[ResidencyEpochId]| {
        active_epochs
            .iter()
            .copied()
            .find(|epoch| !epochs.contains(epoch))
    };
    if let Some((kernel, epoch)) = input.kernels.iter().find_map(|kernel| {
        references_epoch(&kernel.active_epochs).map(|epoch| (kernel.kernel.to_string(), epoch))
    }) {
        return Some(diagnostic(
            RomWindowPlanDiagnosticCode::RomPolicyProjectionMismatch,
            RomWindowPlanDiagnosticProvenance::PolicyProjection {
                field: "kernels.active_epochs".to_owned(),
                detail: format!("kernel {kernel} references unknown epoch {}", epoch.0),
            },
        ));
    }
    if let Some((lut, epoch)) = input.luts.iter().find_map(|lut| {
        references_epoch(&lut.active_epochs).map(|epoch| (lut.lut.0.clone(), epoch))
    }) {
        return Some(diagnostic(
            RomWindowPlanDiagnosticCode::RomPolicyProjectionMismatch,
            RomWindowPlanDiagnosticProvenance::PolicyProjection {
                field: "luts.active_epochs".to_owned(),
                detail: format!("LUT {lut} references unknown epoch {}", epoch.0),
            },
        ));
    }
    input.storage_bindings.iter().find_map(|binding| {
        references_epoch(&binding.active_epochs).map(|epoch| {
            diagnostic(
                RomWindowPlanDiagnosticCode::RomPolicyProjectionMismatch,
                RomWindowPlanDiagnosticProvenance::PolicyProjection {
                    field: "storage_bindings.active_epochs".to_owned(),
                    detail: format!(
                        "ROM const binding {} references unknown epoch {}",
                        binding.binding.value.get(),
                        epoch.0
                    ),
                },
            )
        })
    })
}

fn validate_sram_epoch_alignment(input: &RomWindowPlanInputs) -> Option<ValidationDiagnostic> {
    let epoch_ranges = input
        .epochs
        .iter()
        .map(|epoch| (epoch.id, epoch.op_range))
        .collect::<BTreeMap<_, _>>();
    let epoch_order = input
        .epochs
        .iter()
        .enumerate()
        .map(|(index, epoch)| (epoch.id, index))
        .collect::<BTreeMap<_, _>>();
    for active_set in &input.sram_page_plan.active_sets {
        let id = ResidencyEpochId(active_set.epoch.0);
        let Some(op_range) = epoch_ranges.get(&id) else {
            return Some(epoch_coverage_diagnostic(format!(
                "F-B9 SRAM epoch {} is missing from RomWindowPlan epochs",
                active_set.epoch.0
            )));
        };
        if *op_range != active_set.op_range {
            return Some(epoch_coverage_diagnostic(format!(
                "F-B9 SRAM epoch {} has op range {}..{}, but RomWindowPlan has {}..{}",
                active_set.epoch.0,
                active_set.op_range.first_node.get(),
                active_set.op_range.last_node.get(),
                op_range.first_node.get(),
                op_range.last_node.get()
            )));
        }
    }
    for rotation in &input.sram_page_plan.page_rotations {
        let before = ResidencyEpochId(rotation.at_epoch_boundary.0.0);
        let after = ResidencyEpochId(rotation.at_epoch_boundary.1.0);
        if let Some(diagnostic) =
            validate_f_b9_boundary_pair(&epoch_order, "page rotation", before, after)
        {
            return Some(diagnostic);
        }
    }
    for boundary in &input.sram_page_plan.commit_boundaries {
        let before = ResidencyEpochId(boundary.before_epoch.0);
        let after = ResidencyEpochId(boundary.after_epoch.0);
        if let Some(diagnostic) =
            validate_f_b9_boundary_pair(&epoch_order, "commit boundary", before, after)
        {
            return Some(diagnostic);
        }
    }
    None
}

fn validate_f_b9_boundary_pair(
    epoch_order: &BTreeMap<ResidencyEpochId, usize>,
    kind: &'static str,
    before: ResidencyEpochId,
    after: ResidencyEpochId,
) -> Option<ValidationDiagnostic> {
    let Some(before_index) = epoch_order.get(&before).copied() else {
        return Some(epoch_coverage_diagnostic(format!(
            "F-B9 {kind} references epoch {} missing from RomWindowPlan epochs",
            before.0
        )));
    };
    let Some(after_index) = epoch_order.get(&after).copied() else {
        return Some(epoch_coverage_diagnostic(format!(
            "F-B9 {kind} references epoch {} missing from RomWindowPlan epochs",
            after.0
        )));
    };
    if after_index != before_index.saturating_add(1) {
        return Some(epoch_coverage_diagnostic(format!(
            "F-B9 {kind} boundary {} -> {} is not adjacent in RomWindowPlan epoch order",
            before.0, after.0
        )));
    }
    None
}

fn validate_infer_ir_hot_epoch_coverage(
    input: &RomWindowPlanInputs,
) -> Option<ValidationDiagnostic> {
    let actual_nodes = input
        .infer_ir
        .nodes
        .iter()
        .map(|node| node.node_id)
        .collect::<BTreeSet<_>>();
    let hot_nodes = infer_ir_hot_nodes(input);
    let Some(max_node) = actual_nodes.iter().next_back().copied() else {
        if input
            .epochs
            .iter()
            .all(|epoch| epoch.op_range.first_node == epoch.op_range.last_node)
        {
            return None;
        }
        return Some(epoch_coverage_diagnostic(
            "RomWindowPlan epochs cite node ranges but cited GbInferIR has no nodes",
        ));
    };
    let terminal_boundary = max_node.get().checked_add(1).map(NodeId::new);
    for epoch in &input.epochs {
        if !actual_nodes.contains(&epoch.op_range.first_node) {
            return Some(epoch_coverage_diagnostic(format!(
                "epoch {} starts at node {}, which is not present in cited GbInferIR",
                epoch.id.0,
                epoch.op_range.first_node.get()
            )));
        }
        if terminal_boundary != Some(epoch.op_range.last_node)
            && !actual_nodes.contains(&epoch.op_range.last_node)
        {
            return Some(epoch_coverage_diagnostic(format!(
                "epoch {} ends at node {}, which is neither a cited GbInferIR node nor the terminal boundary",
                epoch.id.0,
                epoch.op_range.last_node.get()
            )));
        }
        if actual_nodes
            .iter()
            .all(|node| !node_in_epoch_range(*node, epoch.op_range))
        {
            return Some(epoch_coverage_diagnostic(format!(
                "epoch {} op range {}..{} contains no cited GbInferIR nodes",
                epoch.id.0,
                epoch.op_range.first_node.get(),
                epoch.op_range.last_node.get()
            )));
        }
    }
    if hot_nodes.is_empty() {
        return None;
    }

    for hot_node in hot_nodes {
        let covering_epochs = input
            .epochs
            .iter()
            .filter(|epoch| node_in_epoch_range(hot_node, epoch.op_range))
            .map(|epoch| epoch.id.0)
            .collect::<Vec<_>>();
        match covering_epochs.len() {
            1 => {}
            0 => {
                return Some(epoch_coverage_diagnostic(format!(
                    "hot GbInferIR node {} is not covered by any residency epoch",
                    hot_node.get()
                )));
            }
            _ => {
                return Some(epoch_coverage_diagnostic(format!(
                    "hot GbInferIR node {} is covered by overlapping residency epochs {:?}",
                    hot_node.get(),
                    covering_epochs
                )));
            }
        }
    }
    None
}

fn infer_ir_hot_nodes(input: &RomWindowPlanInputs) -> BTreeSet<NodeId> {
    let hot_values = input
        .storage_bindings
        .iter()
        .filter(|binding| is_hot_operation_materialization(&binding.binding.materialization))
        .map(|binding| binding.binding.value)
        .collect::<BTreeSet<_>>();
    input
        .infer_ir
        .nodes
        .iter()
        .filter(|node| {
            node.inputs
                .iter()
                .chain(node.outputs.iter())
                .any(|value| hot_values.contains(value))
        })
        .map(|node| node.node_id)
        .collect()
}

fn node_in_epoch_range(node: NodeId, range: NodeAnchorRange) -> bool {
    range.first_node <= node && node < range.last_node
}

fn is_hot_operation_materialization(materialization: &Materialization) -> bool {
    match materialization {
        Materialization::Materialize { class, .. } => {
            matches!(class, StorageClass::RomConst | StorageClass::SramPaged)
        }
        Materialization::Persist { .. } => true,
        Materialization::Recompute => false,
    }
}

fn epoch_coverage_diagnostic(detail: impl Into<String>) -> ValidationDiagnostic {
    diagnostic(
        RomWindowPlanDiagnosticCode::RomPolicyProjectionMismatch,
        RomWindowPlanDiagnosticProvenance::PolicyProjection {
            field: "epochs.op_range".to_owned(),
            detail: detail.into(),
        },
    )
}

fn validate_rom_window_plan_epoch_surface(plan: &RomWindowPlan) -> Option<ValidationDiagnostic> {
    if !epoch_bindings_cover_plan(plan) {
        return Some(diagnostic(
            RomWindowPlanDiagnosticCode::RomPolicyProjectionMismatch,
            RomWindowPlanDiagnosticProvenance::PolicyProjection {
                field: "residency_epochs".to_owned(),
                detail:
                    "residency epochs and rom window bindings are not a bijective contiguous cover"
                        .to_owned(),
            },
        ));
    }
    None
}

fn build_window_cert_body(
    output: &RomWindowPlanOutput,
    report_self_hash: Hash256,
) -> Option<WindowCertBody> {
    if output.outcome != RomWindowPlanOutcome::Succeeded || !output.diagnostics.is_empty() {
        return None;
    }
    let plan = output.result.as_ref()?;
    let switch_cap = output
        .summary
        .map(|summary| summary.switch_count_per_token)
        .unwrap_or(plan.projections.upper_bound_per_token)
        .max(plan.projections.upper_bound_per_token);
    let claim = WindowCertClaim {
        rom_window_plan_self_hash: plan.rom_window_plan_self_hash,
        single_window_invariant_holds: single_window_invariant_holds(plan),
        overlay_install_sources_visible: overlay_install_sources_visible(plan),
        isr_kernels_in_bank0: isr_kernels_in_bank0(plan),
        isr_luts_in_bank0_or_always_resident: isr_luts_in_bank0_or_always_resident(plan),
        all_kernels_have_residency: all_kernels_have_residency(plan),
        all_luts_have_residency: all_luts_have_residency(plan),
        co_residency_closures_well_formed: co_residency_closures_well_formed(plan),
        epoch_bindings_cover_plan: epoch_bindings_cover_plan(plan),
        bank0_demand_within_slack: plan.bank0_demand.remaining_slack_bytes >= 0,
        overlay_demand_within_wram_reservation: true,
        bank_switches_per_token: plan.projections.projected_bank_switches_per_token,
        bank_switches_cap: switch_cap,
        bank_switches_per_token_within_cap: plan.projections.projected_bank_switches_per_token
            <= switch_cap,
    };
    Some(WindowCertBody {
        schema: WINDOW_CERT_SCHEMA_ID.to_owned(),
        schema_version: WINDOW_CERT_SCHEMA_VERSION,
        cert_outcome: WindowCertOutcome::Passed,
        report_self_hash,
        claim,
        evidence: WindowCertEvidence {
            kernel_residency_distribution: kernel_residency_distribution(plan),
            lut_residency_distribution: lut_residency_distribution(plan),
            overlay_install_source_epoch_count: plan.overlay_demand.install_source_visibility.len()
                as u32,
            co_resident_closure_count: plan.co_resident_closures.len() as u32,
            residency_epoch_count: plan.residency_epochs.len() as u32,
            bank0_kernel_bytes: plan.bank0_demand.total_kernel_bytes,
            bank0_lut_bytes: plan.bank0_demand.total_lut_bytes,
            wram_overlay_bytes: plan.overlay_demand.total_overlay_bytes,
        },
    })
}

fn single_window_invariant_holds(plan: &RomWindowPlan) -> bool {
    plan.rom_window_bindings
        .iter()
        .all(|binding| binding.visibility.bank0_visible)
}

fn overlay_install_sources_visible(plan: &RomWindowPlan) -> bool {
    validate_overlay_install_source_visibility(&plan.overlay_demand, &plan.rom_window_bindings)
        .is_none()
}

fn isr_kernels_in_bank0(plan: &RomWindowPlan) -> bool {
    plan.provenance
        .kernel_to_reachability
        .iter()
        .all(|(kernel, reachability)| {
            !reachability.requires_bank0()
                || matches!(
                    plan.kernel_residency.get(kernel),
                    Some(KernelResidency::Bank0Fixed)
                )
        })
}

fn isr_luts_in_bank0_or_always_resident(plan: &RomWindowPlan) -> bool {
    plan.provenance
        .lut_to_reachability
        .iter()
        .all(|(lut, reachability)| {
            !reachability.requires_bank0()
                || matches!(
                    plan.lut_residency.get(lut),
                    Some(LutResidency::Bank0Inline)
                        | Some(LutResidency::WramStaged {
                            always_resident: true
                        })
                )
        })
}

fn all_kernels_have_residency(plan: &RomWindowPlan) -> bool {
    plan.provenance
        .kernel_to_reachability
        .keys()
        .all(|kernel| plan.kernel_residency.contains_key(kernel))
        && plan.kernel_residency.len() == plan.provenance.kernel_to_reachability.len()
}

fn all_luts_have_residency(plan: &RomWindowPlan) -> bool {
    plan.provenance
        .lut_to_reachability
        .keys()
        .all(|lut| plan.lut_residency.contains_key(lut))
        && plan.lut_residency.len() == plan.provenance.lut_to_reachability.len()
}

fn co_residency_closures_well_formed(plan: &RomWindowPlan) -> bool {
    let closure_banks = plan
        .co_resident_closures
        .iter()
        .map(|closure| (closure.id, closure.bank))
        .collect::<BTreeMap<_, _>>();
    plan.rom_window_bindings.iter().all(|binding| {
        binding.closure.is_none_or(|closure| {
            closure_banks.get(&closure) == binding.visibility.switchable.as_ref()
        })
    })
}

fn epoch_bindings_cover_plan(plan: &RomWindowPlan) -> bool {
    if plan.residency_epochs.len() != plan.rom_window_bindings.len()
        || plan.residency_epochs.len() != plan.provenance.epoch_to_node_range.len()
    {
        return false;
    }
    let binding_by_id = plan
        .rom_window_bindings
        .iter()
        .map(|binding| (binding.id, binding))
        .collect::<BTreeMap<_, _>>();
    let mut previous_last = None;
    for epoch in &plan.residency_epochs {
        let Some(binding) = binding_by_id.get(&epoch.rom_window_binding) else {
            return false;
        };
        if binding.epoch != epoch.id || epoch.op_range.first_node > epoch.op_range.last_node {
            return false;
        }
        if let Some(last) = previous_last
            && epoch.op_range.first_node.get() != last
        {
            return false;
        }
        previous_last = Some(epoch.op_range.last_node.get());
        if !plan
            .provenance
            .epoch_to_node_range
            .iter()
            .any(|entry| entry.epoch == epoch.id && entry.op_range == epoch.op_range)
        {
            return false;
        }
    }
    true
}

fn kernel_residency_distribution(plan: &RomWindowPlan) -> KernelResidencyDistribution {
    let mut distribution = KernelResidencyDistribution {
        bank0_fixed: 0,
        wram_overlay: 0,
        co_resident_switchable: 0,
    };
    for residency in plan.kernel_residency.values() {
        match residency {
            KernelResidency::Bank0Fixed => distribution.bank0_fixed += 1,
            KernelResidency::WramOverlay => distribution.wram_overlay += 1,
            KernelResidency::CoResidentSwitchable { .. } => {
                distribution.co_resident_switchable += 1
            }
        }
    }
    distribution
}

fn lut_residency_distribution(plan: &RomWindowPlan) -> LutResidencyDistribution {
    let mut distribution = LutResidencyDistribution {
        bank0_inline: 0,
        wram_staged: 0,
        rom_co_resident: 0,
    };
    for residency in plan.lut_residency.values() {
        match residency {
            LutResidency::Bank0Inline => distribution.bank0_inline += 1,
            LutResidency::WramStaged { .. } => distribution.wram_staged += 1,
            LutResidency::RomCoResident { .. } => distribution.rom_co_resident += 1,
        }
    }
    distribution
}

#[allow(clippy::too_many_arguments)]
fn build_window_bindings(
    input: &RomWindowPlanInputs,
    kernels: &[KernelResidencyInput],
    luts: &[LutResidencyInput],
    storage_bindings: &[StorageBindingInput],
    kernel_residency: &BTreeMap<KernelSpecId, KernelResidency>,
    lut_residency: &BTreeMap<LutInstanceId, LutResidency>,
    tensor_to_bank: &BTreeMap<ValueId, RomBankIndex>,
    overlay_demand: &WramOverlayDemand,
) -> Vec<RomWindowBinding> {
    input
        .epochs
        .iter()
        .enumerate()
        .map(|(index, epoch)| {
            let assigned_kernels = kernels
                .iter()
                .filter(|kernel| kernel.active_epochs.contains(&epoch.id))
                .map(|kernel| kernel.kernel.clone())
                .collect::<Vec<_>>();
            let assigned_luts = luts
                .iter()
                .filter(|lut| lut.active_epochs.contains(&epoch.id))
                .map(|lut| lut.lut.clone())
                .collect::<Vec<_>>();
            let assigned_tensors = storage_bindings
                .iter()
                .filter(|binding| binding.active_epochs.contains(&epoch.id))
                .map(|binding| binding.binding.value)
                .collect::<Vec<_>>();
            let demanded_bank = unique_demanded_bank(
                &assigned_kernels,
                &assigned_luts,
                &assigned_tensors,
                kernel_residency,
                lut_residency,
                tensor_to_bank,
                overlay_demand,
                epoch.id,
            );
            RomWindowBinding {
                id: RomWindowBindingId(index as u32),
                epoch: epoch.id,
                visibility: RomVisibility {
                    bank0_visible: true,
                    switchable: demanded_bank,
                },
                assigned_kernels,
                assigned_luts,
                assigned_tensors,
                closure: demanded_bank.map(|bank| CoResidentClosureId(u32::from(bank.0))),
                provenance: vec![EvidenceRef {
                    kind: "RomWindowPlanConstruction".to_owned(),
                    reference: format!("epoch:{}", epoch.id.0),
                    hash: Some(Hash256::ZERO),
                }],
            }
        })
        .collect()
}

fn demanded_banks_for_binding(
    binding: &RomWindowBinding,
    kernel_residency: &BTreeMap<KernelSpecId, KernelResidency>,
    lut_residency: &BTreeMap<LutInstanceId, LutResidency>,
    tensor_to_bank: &BTreeMap<ValueId, RomBankIndex>,
    overlay_demand: &WramOverlayDemand,
) -> Option<BTreeSet<RomBankIndex>> {
    let mut banks = BTreeSet::new();
    for kernel in &binding.assigned_kernels {
        if let Some(KernelResidency::CoResidentSwitchable { bank }) = kernel_residency.get(kernel) {
            banks.insert(*bank);
        }
    }
    for lut in &binding.assigned_luts {
        if let Some(LutResidency::RomCoResident { bank }) = lut_residency.get(lut) {
            banks.insert(*bank);
        }
    }
    for tensor in &binding.assigned_tensors {
        if let Some(bank) = tensor_to_bank.get(tensor) {
            banks.insert(*bank);
        }
    }
    banks.extend(overlay_source_banks_for_epoch(
        overlay_demand,
        binding.epoch,
    ));
    (banks.len() > 1).then_some(banks)
}

#[allow(clippy::too_many_arguments)]
fn unique_demanded_bank(
    kernels: &[KernelSpecId],
    luts: &[LutInstanceId],
    tensors: &[ValueId],
    kernel_residency: &BTreeMap<KernelSpecId, KernelResidency>,
    lut_residency: &BTreeMap<LutInstanceId, LutResidency>,
    tensor_to_bank: &BTreeMap<ValueId, RomBankIndex>,
    overlay_demand: &WramOverlayDemand,
    epoch: ResidencyEpochId,
) -> Option<RomBankIndex> {
    let mut banks = BTreeSet::new();
    for kernel in kernels {
        if let Some(KernelResidency::CoResidentSwitchable { bank }) = kernel_residency.get(kernel) {
            banks.insert(*bank);
        }
    }
    for lut in luts {
        if let Some(LutResidency::RomCoResident { bank }) = lut_residency.get(lut) {
            banks.insert(*bank);
        }
    }
    for tensor in tensors {
        if let Some(bank) = tensor_to_bank.get(tensor) {
            banks.insert(*bank);
        }
    }
    banks.extend(overlay_source_banks_for_epoch(overlay_demand, epoch));
    (banks.len() == 1).then(|| *banks.iter().next().expect("one bank"))
}

fn overlay_source_banks_for_epoch(
    overlay_demand: &WramOverlayDemand,
    epoch: ResidencyEpochId,
) -> BTreeSet<RomBankIndex> {
    overlay_demand
        .kernels
        .iter()
        .filter(|kernel| kernel.active_epochs.contains(&epoch))
        .map(|kernel| kernel.source_bank)
        .chain(
            overlay_demand
                .luts
                .iter()
                .filter(|lut| lut.active_epochs.contains(&epoch))
                .map(|lut| lut.source_bank),
        )
        .collect()
}

fn build_overlay_install_source_visibility(
    overlay_demand: &WramOverlayDemand,
    bindings: &[RomWindowBinding],
) -> Vec<OverlayInstallSourceVisibility> {
    bindings
        .iter()
        .filter_map(|binding| {
            let kernels = overlay_demand
                .kernels
                .iter()
                .filter(|kernel| kernel.active_epochs.contains(&binding.epoch))
                .map(|kernel| OverlayKernelInstallSource {
                    kernel: kernel.kernel.clone(),
                    source_bank: kernel.source_bank,
                })
                .collect::<Vec<_>>();
            let luts = overlay_demand
                .luts
                .iter()
                .filter(|lut| lut.active_epochs.contains(&binding.epoch))
                .map(|lut| OverlayLutInstallSource {
                    lut: lut.lut.clone(),
                    source_bank: lut.source_bank,
                })
                .collect::<Vec<_>>();
            (!kernels.is_empty() || !luts.is_empty()).then_some(OverlayInstallSourceVisibility {
                epoch: binding.epoch,
                visibility: binding.visibility,
                kernels,
                luts,
            })
        })
        .collect()
}

fn validate_overlay_install_source_visibility(
    overlay_demand: &WramOverlayDemand,
    bindings: &[RomWindowBinding],
) -> Option<ValidationDiagnostic> {
    let visibility_by_epoch = bindings
        .iter()
        .map(|binding| (binding.epoch, binding.visibility))
        .collect::<BTreeMap<_, _>>();
    for kernel in &overlay_demand.kernels {
        for epoch in &kernel.active_epochs {
            let visible = visibility_by_epoch
                .get(epoch)
                .and_then(|visibility| visibility.switchable);
            if visible != Some(kernel.source_bank) {
                return Some(registry_diagnostic(
                    "overlay_demand.kernels.source_bank",
                    format!(
                        "overlay kernel {} source bank {} is not visible in epoch {}",
                        kernel.kernel, kernel.source_bank.0, epoch.0
                    ),
                ));
            }
        }
    }
    for lut in &overlay_demand.luts {
        for epoch in &lut.active_epochs {
            let visible = visibility_by_epoch
                .get(epoch)
                .and_then(|visibility| visibility.switchable);
            if visible != Some(lut.source_bank) {
                return Some(registry_diagnostic(
                    "overlay_demand.luts.source_bank",
                    format!(
                        "overlay LUT {} source bank {} is not visible in epoch {}",
                        lut.lut.0, lut.source_bank.0, epoch.0
                    ),
                ));
            }
        }
    }
    let manifest_epochs = overlay_demand
        .install_source_visibility
        .iter()
        .map(|entry| entry.epoch)
        .collect::<BTreeSet<_>>();
    for epoch in overlay_demand
        .kernels
        .iter()
        .flat_map(|kernel| kernel.active_epochs.iter().copied())
        .chain(
            overlay_demand
                .luts
                .iter()
                .flat_map(|lut| lut.active_epochs.iter().copied()),
        )
    {
        if !manifest_epochs.contains(&epoch) {
            return Some(registry_diagnostic(
                "overlay_demand.install_source_visibility",
                format!("overlay install-source manifest omits epoch {}", epoch.0),
            ));
        }
    }
    None
}

fn switch_projections(bindings: &[RomWindowBinding]) -> RomSwitchProjections {
    let mut previous = RomVisibility::bank0_only();
    let mut count = 0u16;
    let mut per_phase = Vec::new();
    for binding in bindings {
        let changed = binding.visibility.switchable != previous.switchable;
        if changed {
            count = count.saturating_add(1);
        }
        per_phase.push(PerPhaseSwitchCount {
            epoch: binding.epoch,
            switch_count: u16::from(changed),
        });
        previous = binding.visibility;
    }
    RomSwitchProjections {
        projected_bank_switches_per_token: count,
        upper_bound_per_token: count,
        per_phase,
        source: SwitchProjectionSource::ConservativeStaticUpperBound,
    }
}

fn build_bank_assignments(
    bank_occupants: &BTreeMap<RomBankIndex, Vec<(RomBankOccupant, u32)>>,
    cap_by_bank: &BTreeMap<RomBankIndex, u32>,
) -> Vec<BankAssignment> {
    bank_occupants
        .iter()
        .map(|(bank, occupants)| {
            let total_bytes = occupants.iter().map(|(_, bytes)| *bytes).sum::<u32>();
            let cap_bytes = cap_by_bank.get(bank).copied().unwrap_or(0);
            let mut occupant_ids = occupants
                .iter()
                .map(|(occupant, _)| occupant.clone())
                .collect::<Vec<_>>();
            occupant_ids.sort();
            BankAssignment {
                bank: *bank,
                occupants: occupant_ids,
                total_bytes,
                cap_bytes,
                slack_bytes: i64::from(cap_bytes) - i64::from(total_bytes),
            }
        })
        .collect()
}

fn build_co_resident_closures(banks: &[BankAssignment]) -> Vec<CoResidentClosure> {
    banks
        .iter()
        .map(|bank| {
            let mut kernels = Vec::new();
            let mut luts = Vec::new();
            let mut tensors = Vec::new();
            for occupant in &bank.occupants {
                match occupant {
                    RomBankOccupant::Kernel { kernel } => kernels.push(kernel.clone()),
                    RomBankOccupant::Lut { lut } => luts.push(lut.clone()),
                    RomBankOccupant::Tensor { value } => tensors.push(*value),
                }
            }
            CoResidentClosure {
                id: CoResidentClosureId(u32::from(bank.bank.0)),
                bank: bank.bank,
                kernels,
                luts,
                tensors,
                total_bytes: bank.total_bytes,
            }
        })
        .collect()
}

fn bank0_cap_bytes(budget: &RuntimeChromeBudget) -> Option<u32> {
    budget
        .rom_slots
        .iter()
        .filter(|slot| slot.class == BudgetSlotClass::Bank0Free)
        .map(|slot| {
            slot.usable_bytes
                .saturating_sub(u32::from(slot.reserved_slack))
        })
        .max()
}

fn switchable_bank_caps(
    budget: &RuntimeChromeBudget,
    profile: PlacementProfile,
) -> BTreeMap<RomBankIndex, u32> {
    budget
        .rom_slots
        .iter()
        .filter(|slot| {
            matches!(
                slot.class,
                BudgetSlotClass::CommonBank | BudgetSlotClass::ExpertBank
            ) && slot.placement_caps.contains(&profile)
        })
        .map(|slot| {
            (
                RomBankIndex(slot.id.get()),
                slot.usable_bytes
                    .saturating_sub(u32::from(slot.reserved_slack)),
            )
        })
        .collect()
}

fn needs_switchable_rom(input: &RomWindowPlanInputs) -> bool {
    input
        .storage_bindings
        .iter()
        .any(|binding| is_rom_const(&binding.binding.materialization))
        || input
            .kernels
            .iter()
            .any(|kernel| !kernel.reachability.requires_bank0() && !kernel.overlay_eligible)
        || input
            .luts
            .iter()
            .any(|lut| !lut.reachability.requires_bank0() && !lut.overlay_eligible)
}

fn is_rom_const(materialization: &Materialization) -> bool {
    matches!(
        materialization,
        Materialization::Materialize {
            class: StorageClass::RomConst,
            ..
        }
    )
}

fn saturating_u16(value: usize) -> u16 {
    u16::try_from(value).unwrap_or(u16::MAX)
}

struct BankAllocator {
    profile: PlacementProfile,
    cap_by_bank: BTreeMap<RomBankIndex, u32>,
    ordered_banks: Vec<RomBankIndex>,
    next: usize,
}

impl BankAllocator {
    fn new(profile: PlacementProfile, cap_by_bank: BTreeMap<RomBankIndex, u32>) -> Self {
        let ordered_banks = cap_by_bank.keys().copied().collect::<Vec<_>>();
        Self {
            profile,
            cap_by_bank,
            ordered_banks,
            next: 0,
        }
    }

    fn assign(&mut self, requested: Option<RomBankIndex>) -> Option<RomBankIndex> {
        if let Some(bank) = requested {
            return self.cap_by_bank.contains_key(&bank).then_some(bank);
        }
        match self.profile {
            PlacementProfile::StrictOnePerBank => {
                let bank = self.ordered_banks.get(self.next).copied();
                self.next = self.next.saturating_add(1);
                bank
            }
            PlacementProfile::Budgeted | PlacementProfile::PackedExperts => {
                self.ordered_banks.first().copied()
            }
        }
    }
}

fn diagnostic(
    code: RomWindowPlanDiagnosticCode,
    provenance: RomWindowPlanDiagnosticProvenance,
) -> ValidationDiagnostic {
    ValidationDiagnostic::hard(
        ValidationOrigin::RomWindowPlanConstruction,
        ValidationCode::RomWindowPlan { code, provenance },
        ValidationDetail::Field {
            field: format!(
                "rom_window_plan.diagnostics.{}.{}.detail_template.v1",
                code.as_str(),
                code.name()
            )
            .into(),
        },
        vec![EvidenceRef {
            kind: "RomWindowPlanConstruction".to_owned(),
            reference: code.as_str().to_owned(),
            hash: Some(Hash256::ZERO),
        }],
    )
}

fn report_invariant(field: &str) -> ValidationDiagnostic {
    ValidationDiagnostic::hard(
        ValidationOrigin::RomWindowPlanConstruction,
        ValidationCode::ReportSemanticInvariantViolated {
            field: field.to_owned().into(),
        },
        ValidationDetail::Field {
            field: field.to_owned().into(),
        },
        vec![],
    )
}

fn failed_output(
    input_identity: RomWindowPlanInputIdentity,
    diagnostics: Vec<ValidationDiagnostic>,
) -> RomWindowPlanOutput {
    RomWindowPlanOutput {
        input_identity,
        outcome: RomWindowPlanOutcome::Failed,
        result: None,
        summary: None,
        diagnostics,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use gbf_abi::{TraceBudget, TraceDropPolicy};
    use gbf_foundation::{BudgetSlotId, CompileProfileId, WorkloadId};
    use gbf_policy::{
        RomBudgetSlot, RomKernelDuplicationBias, RomKernelResidencyBias, RuntimeMemoryCapSection,
    };

    use super::*;
    use crate::s1::quant_graph::QuantFormat;
    use crate::s3::infer_ir::{
        GbNode, InferIrIdentity, InferIrProvenance, InferOp, ValueDecl, ValueFormat, ValueKind,
        ValueLayout,
    };
    use crate::s4::observation_plan as s4;
    use crate::s5::range_plan as s5;
    use crate::sram_page_plan::{
        CommitBoundary, CommitBoundaryId, PageId, PageResidency, PersistentPage,
        PersistentPageGeometry, SramBudgetTally, SramPageBinding, SramPagePlanInputIdentity,
        SramResidencyRole, SramStreamIndexEntry, YieldSafetyClass,
    };
    use crate::storage_plan::types::{
        AbstractLiveRange, AliasClassId, BindingJustification, DecisionRuleId, LifetimeClass,
    };

    #[test]
    fn pass_budgeted_plan_constructs_bindings_report_and_cache_key() {
        let output = build_rom_window_plan(&fixture_inputs(
            PlacementProfile::Budgeted,
            vec![kernel(
                "kernel.shared",
                512,
                RomReachabilityClass::HotPath,
                false,
                None,
            )],
            vec![lut(
                "lut.norm",
                128,
                RomReachabilityClass::HotPath,
                false,
                None,
            )],
            vec![rom_const(7, 1024, None)],
        ));
        assert_eq!(
            output.outcome,
            RomWindowPlanOutcome::Succeeded,
            "{:?}",
            output.diagnostics
        );
        let plan = output.result.as_ref().expect("plan emitted");
        assert_eq!(plan.summary_bank_count(), 1);
        assert_eq!(
            plan.rom_window_bindings[0].visibility.switchable,
            Some(RomBankIndex(1))
        );
        assert_ne!(plan.rom_window_plan_self_hash, Hash256::ZERO);

        let bytes = emit_rom_window_plan_json_bytes(&output).expect("report emits");
        let repeat_bytes = emit_rom_window_plan_json_bytes(&output).expect("repeat report emits");
        assert_eq!(bytes, repeat_bytes);
        let parsed = parse_rom_window_plan_report_bytes(&bytes).expect("report parses");
        round_trip_self_hash(&parsed).expect("report self hash round trips");
        assert_eq!(canonicalize(&parsed).expect("canonical"), bytes);
        let cert_bytes = emit_window_cert_json_bytes(&output, parsed.report_self_hash)
            .expect("cert emits")
            .expect("success cert");
        let repeat_cert_bytes = emit_window_cert_json_bytes(&output, parsed.report_self_hash)
            .expect("repeat cert emits")
            .expect("repeat success cert");
        assert_eq!(cert_bytes, repeat_cert_bytes);
        let cert = parse_window_cert_report_bytes(&cert_bytes).expect("cert parses");
        assert_eq!(
            canonical_json_bytes_omitting_fields(&cert, &[]).expect("canonical cert"),
            cert_bytes
        );
        assert_eq!(cert.schema, WINDOW_CERT_SCHEMA_ID);
        assert!(cert.claim.single_window_invariant_holds);
        assert!(cert.claim.epoch_bindings_cover_plan);
        assert_eq!(
            cert.evidence
                .kernel_residency_distribution
                .co_resident_switchable,
            1
        );

        let key =
            RomWindowPlanCacheKeyInputs::from_input_identity(&output.input_identity, hash(42))
                .cache_key()
                .expect("cache key");
        let mut changed = output.input_identity.clone();
        changed.sram_page_plan_self_hash = hash(43);
        let changed_key = RomWindowPlanCacheKeyInputs::from_input_identity(&changed, hash(42))
            .cache_key()
            .expect("changed cache key");
        assert_ne!(key, changed_key);

        let mut changed = output.input_identity.clone();
        changed.runtime_mode = RuntimeMode::Trace;
        let changed_key = RomWindowPlanCacheKeyInputs::from_input_identity(&changed, hash(42))
            .cache_key()
            .expect("runtime-mode cache key");
        assert_ne!(key, changed_key);
    }

    #[test]
    fn pass_registry_inputs_contract_observation_rows_into_rom_plan() {
        let output = build_rom_window_plan_from_registry(&registry_inputs(
            vec![observation_kernel(
                "kernel.registry",
                256,
                RomReachabilityClass::HotPath,
                false,
                Some(RomBankIndex(1)),
                vec![ResidencyEpochId(0)],
            )],
            vec![observation_lut(
                "lut.registry",
                64,
                RomReachabilityClass::IsrReachable,
                false,
                None,
                vec![ResidencyEpochId(1)],
            )],
            vec![],
        ));

        assert_eq!(
            output.outcome,
            RomWindowPlanOutcome::Succeeded,
            "{:?}",
            output.diagnostics
        );
        let plan = output.result.as_ref().expect("registry plan emitted");
        assert_eq!(
            plan.kernel_residency[&KernelSpecId::from("kernel.registry")],
            KernelResidency::CoResidentSwitchable {
                bank: RomBankIndex(1)
            }
        );
        assert_eq!(
            plan.lut_residency[&LutInstanceId("lut.registry".to_owned())],
            LutResidency::Bank0Inline
        );
        assert!(
            plan.provenance
                .kernel_to_reachability
                .contains_key(&KernelSpecId::from("kernel.registry"))
        );
        assert!(
            plan.provenance
                .lut_to_reachability
                .contains_key(&LutInstanceId("lut.registry".to_owned()))
        );
    }

    #[test]
    fn stage_fact_registry_extraction_preserves_row_fields() {
        let base_inputs = fixture_inputs(PlacementProfile::Budgeted, vec![], vec![], vec![]);
        let audit_parents = registry_audit_parents(&base_inputs);
        let observation_product = stage4_product_with_rom_window_facts(
            &base_inputs.input_identity,
            s4::ObservationRomWindowFacts {
                source: s4::ObservationRomWindowFactSource::Available {
                    registry_key: "observation_plan.rom_window_facts".to_owned(),
                },
                kernels: vec![s4::ObservationRomKernelWindowFact {
                    kernel: KernelSpecId::from("kernel.stage4"),
                    byte_size: 321,
                    reachability: Some(s4::ObservationRomReachabilityClass::YieldResumeReachable),
                    overlay_eligible: true,
                    active_epochs: vec![
                        s4::ObservationResidencyEpochId(3),
                        s4::ObservationResidencyEpochId(5),
                    ],
                    requested_bank: Some(s4::ObservationRomBankIndex(7)),
                    source: s4::ObservationRomWindowFactSource::Missing {
                        registry_key: "kernel.stage4".to_owned(),
                    },
                }],
                luts: vec![s4::ObservationRomLutWindowFact {
                    lut: "lut.stage4".to_owned(),
                    byte_size: 45,
                    reachability: Some(s4::ObservationRomReachabilityClass::FaultPathReachable),
                    overlay_eligible: false,
                    active_epochs: vec![s4::ObservationResidencyEpochId(9)],
                    requested_bank: Some(s4::ObservationRomBankIndex(2)),
                    source: s4::ObservationRomWindowFactSource::SourceImpossible {
                        registry_key: "lut.stage4".to_owned(),
                        reason: "fixture probes pass-through".to_owned(),
                    },
                }],
            },
        );
        let range_product = stage5_product_with_rom_window_facts(
            &base_inputs.input_identity,
            s5::RangeRomWindowFacts {
                source: s5::RangeRomWindowFactSource::Available {
                    registry_key: "range_plan.rom_window_facts".to_owned(),
                },
                reduction_subordinates: vec![s5::RangeRomWindowReductionSubordinateFact {
                    site: ReductionSiteId("site.stage5".to_owned()),
                    main_kernel: KernelSpecId::from("kernel.main"),
                    subordinate_kernel: KernelSpecId::from("kernel.tail"),
                    source: s5::RangeRomWindowFactSource::Missing {
                        registry_key: "site.stage5".to_owned(),
                    },
                }],
            },
        );

        let inputs = rom_window_plan_registry_inputs_from_stage_facts(
            base_inputs,
            audit_parents,
            &observation_product,
            &range_product,
        );

        assert_eq!(
            inputs.observation_registry.observation_plan_self_hash,
            observation_product.observation_plan_self_hash
        );
        assert_eq!(
            inputs.observation_registry.infer_ir_self_hash,
            observation_product
                .observation_plan
                .identity
                .infer_ir_self_hash
        );
        assert_eq!(
            inputs.observation_registry.quant_graph_self_hash,
            observation_product
                .observation_plan
                .identity
                .quant_graph_self_hash
        );
        assert!(matches!(
            inputs.observation_registry.source,
            RomWindowRegistrySource::Available { ref registry_key }
                if registry_key == "observation_plan.rom_window_facts"
        ));
        let kernel = &inputs.observation_registry.kernels[0];
        assert_eq!(kernel.kernel, KernelSpecId::from("kernel.stage4"));
        assert_eq!(kernel.byte_size, 321);
        assert_eq!(
            kernel.reachability,
            Some(RomReachabilityClass::YieldResumeReachable)
        );
        assert!(kernel.overlay_eligible);
        assert_eq!(
            kernel.active_epochs,
            vec![ResidencyEpochId(3), ResidencyEpochId(5)]
        );
        assert_eq!(kernel.requested_bank, Some(RomBankIndex(7)));
        assert!(matches!(
            kernel.source,
            RomWindowRegistrySource::Missing { ref registry_key }
                if registry_key == "kernel.stage4"
        ));

        let lut = &inputs.observation_registry.luts[0];
        assert_eq!(lut.lut, LutInstanceId("lut.stage4".to_owned()));
        assert_eq!(lut.byte_size, 45);
        assert_eq!(
            lut.reachability,
            Some(RomReachabilityClass::FaultPathReachable)
        );
        assert!(!lut.overlay_eligible);
        assert_eq!(lut.active_epochs, vec![ResidencyEpochId(9)]);
        assert_eq!(lut.requested_bank, Some(RomBankIndex(2)));
        assert!(matches!(
            lut.source,
            RomWindowRegistrySource::SourceImpossible {
                ref registry_key,
                ref reason,
            } if registry_key == "lut.stage4" && reason == "fixture probes pass-through"
        ));

        assert_eq!(
            inputs.range_registry.range_plan_self_hash,
            range_product.range_plan_self_hash
        );
        assert_eq!(
            inputs.range_registry.static_budget_self_hash,
            range_product.range_plan.identity.static_budget_self_hash
        );
        assert!(matches!(
            inputs.range_registry.source,
            RomWindowRegistrySource::Available { ref registry_key }
                if registry_key == "range_plan.rom_window_facts"
        ));
        let subordinate = &inputs.range_registry.reduction_subordinates[0];
        assert_eq!(subordinate.site, ReductionSiteId("site.stage5".to_owned()));
        assert_eq!(subordinate.main_kernel, KernelSpecId::from("kernel.main"));
        assert_eq!(
            subordinate.subordinate_kernel,
            KernelSpecId::from("kernel.tail")
        );
        assert!(matches!(
            subordinate.source,
            RomWindowRegistrySource::Missing { ref registry_key }
                if registry_key == "site.stage5"
        ));
    }

    #[test]
    fn reject_registry_missing_and_source_impossible_rows() {
        let mut missing = registry_inputs(
            vec![ObservationKernelResidencyRecord {
                kernel: KernelSpecId::from("kernel.missing"),
                byte_size: 64,
                reachability: None,
                overlay_eligible: false,
                active_epochs: vec![ResidencyEpochId(0)],
                requested_bank: None,
                source: RomWindowRegistrySource::Missing {
                    registry_key: "kernel.missing".to_owned(),
                },
            }],
            vec![],
            vec![],
        );
        let output = build_rom_window_plan_from_registry(&missing);
        assert_has_code(
            &output,
            RomWindowPlanDiagnosticCode::RomPolicyProjectionMismatch,
        );
        assert_has_detail(&output, "registry missing source");

        missing.observation_registry.kernels[0].source =
            RomWindowRegistrySource::SourceImpossible {
                registry_key: "kernel.missing".to_owned(),
                reason: "ObservationPlan row has no reachable IR anchor".to_owned(),
            };
        let output = build_rom_window_plan_from_registry(&missing);
        assert_has_code(
            &output,
            RomWindowPlanDiagnosticCode::RomPolicyProjectionMismatch,
        );
        assert_has_detail(&output, "registry source impossible");
    }

    #[test]
    fn reject_registry_source_impossible_when_upstream_fact_product_absent() {
        let mut inputs = registry_inputs(vec![], vec![], vec![]);
        inputs.observation_registry.source = RomWindowRegistrySource::SourceImpossible {
            registry_key: "observation_plan.rom_window_facts".to_owned(),
            reason: "Stage 4 ObservationPlan did not publish ROM-window kernel/LUT facts"
                .to_owned(),
        };
        let output = build_rom_window_plan_from_registry(&inputs);

        assert_has_code(
            &output,
            RomWindowPlanDiagnosticCode::RomPolicyProjectionMismatch,
        );
        assert_has_detail(&output, "registry source impossible");

        let mut inputs = registry_inputs(vec![], vec![], vec![]);
        inputs.range_registry.source = RomWindowRegistrySource::SourceImpossible {
            registry_key: "range_plan.rom_window_facts".to_owned(),
            reason: "Stage 5 RangePlan did not publish ROM-window reduction subordinate facts"
                .to_owned(),
        };
        let output = build_rom_window_plan_from_registry(&inputs);

        assert_has_code(
            &output,
            RomWindowPlanDiagnosticCode::RomPolicyProjectionMismatch,
        );
        assert_has_detail(&output, "registry source impossible");
    }

    #[test]
    fn reject_registry_audit_parent_mismatches_for_all_transitive_parents() {
        let cases: [(&str, fn(&mut RomWindowPlanRegistryInputs, Hash256)); 9] = [
            (
                "audit_parents.artifact_validation_self_hash",
                |inputs, value| inputs.audit_parents.artifact_validation_self_hash = value,
            ),
            (
                "audit_parents.policy_resolution_self_hash",
                |inputs, value| inputs.audit_parents.policy_resolution_self_hash = value,
            ),
            ("audit_parents.static_budget_self_hash", |inputs, value| {
                inputs.audit_parents.static_budget_self_hash = value;
                inputs.range_registry.static_budget_self_hash = value;
            }),
            ("audit_parents.quant_graph_self_hash", |inputs, value| {
                inputs.audit_parents.quant_graph_self_hash = value;
                inputs.observation_registry.quant_graph_self_hash = value;
                inputs.range_registry.quant_graph_self_hash = value;
            }),
            ("audit_parents.infer_ir_self_hash", |inputs, value| {
                inputs.audit_parents.infer_ir_self_hash = value
            }),
            (
                "audit_parents.observation_plan_self_hash",
                |inputs, value| inputs.audit_parents.observation_plan_self_hash = value,
            ),
            ("audit_parents.range_plan_self_hash", |inputs, value| {
                inputs.audit_parents.range_plan_self_hash = value
            }),
            ("audit_parents.storage_plan_self_hash", |inputs, value| {
                inputs.audit_parents.storage_plan_self_hash = value
            }),
            ("audit_parents.sram_page_plan_self_hash", |inputs, value| {
                inputs.audit_parents.sram_page_plan_self_hash = value
            }),
        ];

        for (index, (product, mutate)) in cases.into_iter().enumerate() {
            let mut inputs = registry_inputs(vec![], vec![], vec![]);
            mutate(&mut inputs, hash(0xa0 + index as u8));

            let output = build_rom_window_plan_from_registry(&inputs);

            assert_has_code(&output, RomWindowPlanDiagnosticCode::RomInputHashMismatch);
            assert_has_hash_mismatch_product(&output, product);
        }
    }

    fn stage4_product_with_rom_window_facts(
        identity: &RomWindowPlanInputIdentity,
        rom_window_facts: s4::ObservationRomWindowFacts,
    ) -> ObservationPlanCoreProduct {
        let observation_plan = s4::ObservationPlan {
            identity: s4::ObservationPlanIdentity {
                infer_ir_self_hash: identity.infer_ir_self_hash,
                quant_graph_self_hash: identity.quant_graph_self_hash,
                semantic_checkpoint_schema_hash: hash(0xd1),
                observation_policy_projection_hash: hash(0xd2),
                determinism: DeterminismClass::BitExact,
                observability_mode: gbf_policy::ObservabilityMode::Invariant,
                trace_budget: TraceBudget::new(8, 128, TraceDropPolicy::DropOldest)
                    .expect("trace budget"),
                workload_id: WorkloadId::from("stage4.window.fixture"),
                probe_registry_hash: hash(0xd3),
                metric_registry_hash: hash(0xd4),
                trace_event_layout_registry_hash: hash(0xd5),
            },
            semantic: Vec::new(),
            probes: Vec::new(),
            metrics: Vec::new(),
            anchor_table: s4::AnchorAttachmentTable {
                semantic: BTreeMap::new(),
                probes: BTreeMap::new(),
                metrics: BTreeMap::new(),
            },
            provenance: s4::ObservationProvenance {
                semantic_provenance: BTreeMap::new(),
                probe_provenance: BTreeMap::new(),
                metric_provenance: BTreeMap::new(),
            },
            trace_budget_projection: s4::TraceBudgetProjection {
                projected_max_events_per_slice: 0,
                projected_max_bytes_per_frame: 0,
                fits_declared_budget: true,
            },
        };
        let build_active_checkpoint_schema = s4::BuildActiveCheckpointSchema {
            checkpoints: Vec::new(),
            build_active_count: 0,
            mandatory_count: 0,
            optional_count: 0,
        };
        let operational_probe_schema = s4::OperationalProbeSchema {
            probes: Vec::new(),
            metrics: Vec::new(),
            probe_count: 0,
            metric_count: 0,
            per_class_probe_weight_total: s4::PerClassWeightTotal::default(),
            per_class_metric_weight_total: s4::PerClassWeightTotal::default(),
            per_class_total_weight: s4::PerClassWeightTotal::default(),
        };

        ObservationPlanCoreProduct {
            observation_plan_self_hash: s4::observation_plan_self_hash(&observation_plan)
                .expect("observation plan hashes"),
            build_active_checkpoint_schema_hash: s4::build_active_checkpoint_schema_hash(
                &build_active_checkpoint_schema,
            )
            .expect("checkpoint schema hashes"),
            operational_probe_schema_hash: s4::operational_probe_schema_hash(
                &operational_probe_schema,
            )
            .expect("probe schema hashes"),
            observation_plan,
            build_active_checkpoint_schema,
            operational_probe_schema,
            rom_window_facts,
        }
    }

    fn stage5_product_with_rom_window_facts(
        identity: &RomWindowPlanInputIdentity,
        rom_window_facts: s5::RangeRomWindowFacts,
    ) -> RangePlanCoreProduct {
        let range_plan = s5::RangePlan {
            identity: s5::RangePlanIdentity {
                infer_ir_self_hash: identity.infer_ir_self_hash,
                quant_graph_self_hash: identity.quant_graph_self_hash,
                static_budget_self_hash: identity.static_budget_self_hash,
                range_policy_projection_hash: hash(0xe1),
                determinism: DeterminismClass::BitExact,
            },
            entries: Vec::new(),
            provenance: s5::RangePlanProvenance {
                site_to_node: BTreeMap::new(),
                site_to_qg: BTreeMap::new(),
            },
        };
        let range_plan_self_hash = s5::range_plan_self_hash(&range_plan).expect("range hashes");
        let range_cert = s5::RangeCertBody {
            identity: s5::RangeCertIdentity {
                range_plan_self_hash: Some(range_plan_self_hash),
                infer_ir_self_hash: range_plan.identity.infer_ir_self_hash,
                quant_graph_self_hash: range_plan.identity.quant_graph_self_hash,
                static_budget_self_hash: range_plan.identity.static_budget_self_hash,
                determinism: range_plan.identity.determinism,
            },
            cert_outcome: s5::CertOutcome::Verified,
            certificates: Vec::new(),
            site_to_certificate_index: BTreeMap::new(),
            diagnostics: Vec::new(),
        };

        RangePlanCoreProduct {
            range_plan_self_hash,
            range_cert_body_hash: s5::range_cert_body_hash(&range_cert).expect("cert hashes"),
            range_plan,
            range_cert,
            rom_window_facts,
        }
    }

    #[test]
    fn reject_range_subordinate_split_across_switchable_banks() {
        let output = build_rom_window_plan_from_registry(&registry_inputs(
            vec![
                observation_kernel(
                    "kernel.main",
                    256,
                    RomReachabilityClass::HotPath,
                    false,
                    Some(RomBankIndex(1)),
                    vec![ResidencyEpochId(0)],
                ),
                observation_kernel(
                    "kernel.tail",
                    128,
                    RomReachabilityClass::HotPath,
                    false,
                    Some(RomBankIndex(2)),
                    vec![ResidencyEpochId(1)],
                ),
            ],
            vec![],
            vec![RangeReductionSubordinate {
                site: ReductionSiteId("site.softmax".to_owned()),
                main_kernel: KernelSpecId::from("kernel.main"),
                subordinate_kernel: KernelSpecId::from("kernel.tail"),
                source: RomWindowRegistrySource::Available {
                    registry_key: "site.softmax".to_owned(),
                },
            }],
        ));

        assert_has_code(
            &output,
            RomWindowPlanDiagnosticCode::RomPolicyProjectionMismatch,
        );
        assert_has_detail(&output, "splits main kernel");
    }

    #[test]
    fn pass_isr_kernel_is_forced_to_bank0() {
        let output = build_rom_window_plan(&fixture_inputs(
            PlacementProfile::Budgeted,
            vec![kernel(
                "kernel.tick",
                256,
                RomReachabilityClass::IsrReachable,
                false,
                Some(RomBankIndex(1)),
            )],
            vec![],
            vec![],
        ));
        let plan = output.result.as_ref().expect("plan emitted");
        assert_eq!(
            plan.kernel_residency[&KernelSpecId::from("kernel.tick")],
            KernelResidency::Bank0Fixed
        );
        assert_eq!(plan.bank0_demand.total_kernel_bytes, 256);
        assert!(plan.banks.is_empty());
    }

    #[test]
    fn pass_bank0_only_plan_is_well_typed() {
        let output = build_rom_window_plan(&fixture_inputs(
            PlacementProfile::Budgeted,
            vec![kernel(
                "kernel.fault",
                256,
                RomReachabilityClass::FaultPathReachable,
                false,
                None,
            )],
            vec![lut(
                "lut.isr",
                64,
                RomReachabilityClass::IsrReachable,
                false,
                None,
            )],
            vec![],
        ));
        let plan = output.result.as_ref().expect("bank0-only plan emitted");

        assert!(plan.banks.is_empty());
        assert!(plan.co_resident_closures.is_empty());
        assert_eq!(plan.bank0_demand.total_kernel_bytes, 256);
        assert_eq!(plan.bank0_demand.total_lut_bytes, 64);
        assert!(plan.overlay_demand.kernels.is_empty());
        assert!(plan.overlay_demand.luts.is_empty());
        assert!(plan.rom_window_bindings.iter().all(|binding| {
            binding.visibility.bank0_visible
                && binding.visibility.switchable.is_none()
                && binding.closure.is_none()
        }));
    }

    #[test]
    fn pass_wram_overlay_plan_records_overlay_demand() {
        let output = build_rom_window_plan(
            &fixture_inputs(
                PlacementProfile::Budgeted,
                vec![kernel(
                    "kernel.overlay",
                    64,
                    RomReachabilityClass::HotPath,
                    true,
                    None,
                )],
                vec![lut(
                    "lut.overlay",
                    32,
                    RomReachabilityClass::HotPath,
                    true,
                    None,
                )],
                vec![],
            )
            .with_wram_overlay_bias(),
        );
        let plan = output.result.as_ref().expect("overlay plan emitted");

        assert_eq!(
            plan.kernel_residency[&KernelSpecId::from("kernel.overlay")],
            KernelResidency::WramOverlay
        );
        assert!(matches!(
            plan.lut_residency[&LutInstanceId("lut.overlay".to_owned())],
            LutResidency::WramStaged {
                always_resident: false
            }
        ));
        assert_eq!(plan.overlay_demand.total_overlay_bytes, 96);
        assert_eq!(plan.overlay_demand.kernels[0].source_bank, RomBankIndex(1));
        assert_eq!(plan.overlay_demand.luts[0].source_bank, RomBankIndex(1));
        assert_eq!(plan.overlay_demand.install_source_visibility.len(), 1);
        assert_eq!(
            plan.overlay_demand.install_source_visibility[0]
                .visibility
                .switchable,
            Some(RomBankIndex(1))
        );
        assert_eq!(plan.banks.len(), 1);
        assert_eq!(plan.banks[0].total_bytes, 96);
    }

    #[test]
    fn pass_co_resident_closure_contains_multiple_occupants_in_one_bank() {
        let output = build_rom_window_plan(&fixture_inputs(
            PlacementProfile::Budgeted,
            vec![kernel(
                "kernel.shared",
                128,
                RomReachabilityClass::HotPath,
                false,
                Some(RomBankIndex(1)),
            )],
            vec![lut(
                "lut.shared",
                64,
                RomReachabilityClass::HotPath,
                false,
                Some(RomBankIndex(1)),
            )],
            vec![rom_const(7, 256, Some(RomBankIndex(1)))],
        ));
        let plan = output.result.as_ref().expect("co-resident plan emitted");
        let closure = plan
            .co_resident_closures
            .iter()
            .find(|closure| closure.bank == RomBankIndex(1))
            .expect("bank 1 closure");

        assert_eq!(closure.kernels, vec![KernelSpecId::from("kernel.shared")]);
        assert_eq!(closure.luts, vec![LutInstanceId("lut.shared".to_owned())]);
        assert_eq!(closure.tensors, vec![ValueId::new(7)]);
        assert_eq!(closure.total_bytes, 448);
        assert!(plan.rom_window_bindings.iter().any(|binding| {
            binding.closure == Some(closure.id)
                && binding.visibility.switchable == Some(RomBankIndex(1))
        }));
    }

    #[test]
    fn reject_multiple_switchable_banks_in_one_epoch() {
        let output = build_rom_window_plan(&fixture_inputs(
            PlacementProfile::Budgeted,
            vec![kernel(
                "kernel.a",
                128,
                RomReachabilityClass::HotPath,
                false,
                Some(RomBankIndex(1)),
            )],
            vec![],
            vec![rom_const(7, 128, Some(RomBankIndex(2)))],
        ));
        assert_has_code(
            &output,
            RomWindowPlanDiagnosticCode::RomMultipleSwitchableBanksDemandedInPhase,
        );
    }

    #[test]
    fn reject_overlay_install_source_bank_not_visible_with_epoch_demand() {
        let output = build_rom_window_plan(
            &fixture_inputs(
                PlacementProfile::Budgeted,
                vec![kernel(
                    "kernel.overlay",
                    128,
                    RomReachabilityClass::HotPath,
                    true,
                    Some(RomBankIndex(1)),
                )],
                vec![],
                vec![rom_const(7, 128, Some(RomBankIndex(2)))],
            )
            .with_wram_overlay_bias(),
        );

        assert_has_code(
            &output,
            RomWindowPlanDiagnosticCode::RomMultipleSwitchableBanksDemandedInPhase,
        );
        assert_has_phase_banks(&output, &[1, 2]);
    }

    #[test]
    fn reject_bank0_overlay_and_switch_budget_overflows() {
        let mut bank0 = fixture_inputs(
            PlacementProfile::Budgeted,
            vec![kernel(
                "kernel.tick",
                20_000,
                RomReachabilityClass::IsrReachable,
                false,
                None,
            )],
            vec![],
            vec![],
        );
        assert_has_code(
            &build_rom_window_plan(&bank0),
            RomWindowPlanDiagnosticCode::RomBank0OverBudget,
        );

        let mut overlay = fixture_inputs(
            PlacementProfile::Budgeted,
            vec![kernel(
                "kernel.overlay",
                512,
                RomReachabilityClass::HotPath,
                true,
                None,
            )],
            vec![],
            vec![],
        );
        overlay.policy.rom_window.kernel_residency_bias = RomKernelResidencyBias::PreferWramOverlay;
        assert_has_code(
            &build_rom_window_plan(&overlay),
            RomWindowPlanDiagnosticCode::RomOverlayDemandExceedsWramReservation,
        );

        bank0 = fixture_inputs(
            PlacementProfile::StrictOnePerBank,
            vec![],
            vec![],
            vec![
                rom_const_with_epochs(7, 128, None, vec![ResidencyEpochId(0)]),
                rom_const_with_epochs(8, 128, None, vec![ResidencyEpochId(1)]),
            ],
        );
        bank0.policy.max_bank_switches_per_token = Some(1);
        assert_has_code(
            &build_rom_window_plan(&bank0),
            RomWindowPlanDiagnosticCode::RomBankSwitchBudgetExceeded,
        );
    }

    #[test]
    fn reject_hash_mismatch_and_bank_capacity_overflow() {
        let mut inputs = fixture_inputs(PlacementProfile::Budgeted, vec![], vec![], vec![]);
        inputs.expected_input_hashes.storage_plan_self_hash = hash(99);
        assert_has_code(
            &build_rom_window_plan(&inputs),
            RomWindowPlanDiagnosticCode::RomInputHashMismatch,
        );

        let overflow = fixture_inputs(
            PlacementProfile::Budgeted,
            vec![],
            vec![],
            vec![rom_const(7, 20_000, None)],
        );
        assert_has_code(
            &build_rom_window_plan(&overflow),
            RomWindowPlanDiagnosticCode::RomBankCapacityExceeded,
        );

        let mut infer_hash_mismatch =
            fixture_inputs(PlacementProfile::Budgeted, vec![], vec![], vec![]);
        infer_hash_mismatch
            .infer_ir
            .identity
            .static_budget_self_hash = hash(0xfe);
        assert_has_code(
            &build_rom_window_plan(&infer_hash_mismatch),
            RomWindowPlanDiagnosticCode::RomInputHashMismatch,
        );
    }

    #[test]
    fn reject_unknown_epoch_reference_and_noncontiguous_epoch_cover() {
        let mut unknown = fixture_inputs(
            PlacementProfile::Budgeted,
            vec![KernelResidencyInput {
                active_epochs: vec![ResidencyEpochId(99)],
                ..kernel(
                    "kernel.unknown_epoch",
                    128,
                    RomReachabilityClass::HotPath,
                    false,
                    None,
                )
            }],
            vec![],
            vec![],
        );
        assert_has_code(
            &build_rom_window_plan(&unknown),
            RomWindowPlanDiagnosticCode::RomPolicyProjectionMismatch,
        );

        unknown = fixture_inputs(PlacementProfile::Budgeted, vec![], vec![], vec![]);
        unknown.epochs[1].op_range.first_node = NodeId::new(4);
        unknown.epochs[1].op_range.last_node = NodeId::new(5);
        assert_has_code(
            &build_rom_window_plan(&unknown),
            RomWindowPlanDiagnosticCode::RomPolicyProjectionMismatch,
        );
    }

    #[test]
    fn pass_epoch_cover_is_backed_by_actual_infer_ir_hot_nodes() {
        let output = build_rom_window_plan(&fixture_inputs(
            PlacementProfile::StrictOnePerBank,
            vec![],
            vec![],
            vec![
                rom_const_with_epochs(7, 128, None, vec![ResidencyEpochId(0)]),
                rom_const_with_epochs(8, 128, None, vec![ResidencyEpochId(1)]),
            ],
        ));

        assert_eq!(
            output.outcome,
            RomWindowPlanOutcome::Succeeded,
            "{:?}",
            output.diagnostics
        );
        let plan = output.result.as_ref().expect("plan emitted");
        assert_eq!(plan.residency_epochs[0].op_range.first_node, NodeId::new(0));
        assert_eq!(plan.residency_epochs[1].op_range.first_node, NodeId::new(1));
        assert_eq!(
            plan.provenance
                .tensor_to_bank_assignment
                .iter()
                .map(|assignment| assignment.tensor)
                .collect::<Vec<_>>(),
            vec![ValueId::new(7), ValueId::new(8)]
        );
    }

    #[test]
    fn pass_epoch_cover_includes_sram_and_persist_hot_nodes() {
        let output = build_rom_window_plan(&fixture_inputs(
            PlacementProfile::Budgeted,
            vec![],
            vec![],
            vec![
                sram_paged_with_epochs(9, vec![ResidencyEpochId(0)]),
                persist_with_epochs(10, vec![ResidencyEpochId(1)]),
            ],
        ));

        assert_eq!(
            output.outcome,
            RomWindowPlanOutcome::Succeeded,
            "{:?}",
            output.diagnostics
        );
    }

    #[test]
    fn reject_epoch_cover_gap_against_actual_infer_ir_hot_node() {
        let mut inputs = fixture_inputs(
            PlacementProfile::Budgeted,
            vec![],
            vec![],
            vec![rom_const_with_epochs(
                7,
                128,
                None,
                vec![ResidencyEpochId(1)],
            )],
        );
        inputs.epochs = vec![epoch(0, None)];

        let output = build_rom_window_plan(&inputs);

        assert_has_code(
            &output,
            RomWindowPlanDiagnosticCode::RomPolicyProjectionMismatch,
        );
        assert_has_detail(&output, "hot GbInferIR node 1 is not covered");
    }

    #[test]
    fn reject_epoch_cover_gap_against_sram_hot_node() {
        let mut inputs = fixture_inputs(
            PlacementProfile::Budgeted,
            vec![],
            vec![],
            vec![sram_paged_with_epochs(9, vec![ResidencyEpochId(1)])],
        );
        inputs.epochs = vec![epoch(0, None)];

        let output = build_rom_window_plan(&inputs);

        assert_has_code(
            &output,
            RomWindowPlanDiagnosticCode::RomPolicyProjectionMismatch,
        );
        assert_has_detail(&output, "hot GbInferIR node 1 is not covered");
    }

    #[test]
    fn reject_epoch_cover_references_node_outside_cited_infer_ir() {
        let mut inputs = fixture_inputs(
            PlacementProfile::Budgeted,
            vec![],
            vec![],
            vec![rom_const(7, 128, None)],
        );
        inputs.epochs = vec![RomWindowEpochInput {
            id: ResidencyEpochId(0),
            op_range: NodeAnchorRange {
                first_node: NodeId::new(99),
                last_node: NodeId::new(100),
            },
            sram_page_binding: None,
            yield_kind: YieldKindHint::NoYieldsExpected,
        }];

        let output = build_rom_window_plan(&inputs);

        assert_has_code(
            &output,
            RomWindowPlanDiagnosticCode::RomPolicyProjectionMismatch,
        );
        assert_has_detail(&output, "not present in cited GbInferIR");
    }

    #[test]
    fn reject_no_hot_epoch_cover_references_node_outside_cited_infer_ir() {
        let mut inputs = fixture_inputs(PlacementProfile::Budgeted, vec![], vec![], vec![]);
        inputs.epochs = vec![RomWindowEpochInput {
            id: ResidencyEpochId(0),
            op_range: NodeAnchorRange {
                first_node: NodeId::new(99),
                last_node: NodeId::new(100),
            },
            sram_page_binding: None,
            yield_kind: YieldKindHint::NoYieldsExpected,
        }];

        let output = build_rom_window_plan(&inputs);

        assert_has_code(
            &output,
            RomWindowPlanDiagnosticCode::RomPolicyProjectionMismatch,
        );
        assert_has_detail(&output, "not present in cited GbInferIR");
    }

    #[test]
    fn reject_nonpoint_epoch_cover_when_cited_infer_ir_has_no_nodes() {
        let mut inputs = fixture_inputs(PlacementProfile::Budgeted, vec![], vec![], vec![]);
        inputs.infer_ir.nodes.clear();
        inputs.infer_ir.values.clear();
        let infer_ir_hash = infer_ir_self_hash(&inputs.infer_ir).expect("infer ir fixture hashes");
        inputs.input_identity.infer_ir_self_hash = infer_ir_hash;
        inputs.expected_input_hashes.infer_ir_self_hash = infer_ir_hash;
        inputs.epochs = vec![epoch(0, None)];

        let output = build_rom_window_plan(&inputs);

        assert_has_code(
            &output,
            RomWindowPlanDiagnosticCode::RomPolicyProjectionMismatch,
        );
        assert_has_detail(&output, "cited GbInferIR has no nodes");
    }

    #[test]
    fn reject_f_b9_commit_boundary_split_by_rom_epoch_refinement() {
        let mut inputs = fixture_inputs(PlacementProfile::Budgeted, vec![], vec![], vec![]);
        inputs.epochs.push(epoch(2, None));
        inputs
            .sram_page_plan
            .commit_boundaries
            .push(CommitBoundary {
                id: CommitBoundaryId(0),
                before_epoch: crate::sram_page_plan::SramEpochId(0),
                after_epoch: crate::sram_page_plan::SramEpochId(2),
                commit_group: crate::storage_plan::types::CommitGroupId(0),
                generation_delta: 1,
                member_bindings: Vec::new(),
                member_pages: Vec::new(),
                manifest_page: PageId(0),
                serialization_order: Vec::new(),
                yield_safe: YieldSafetyClass::NoYieldDuringCommit,
            });

        let output = build_rom_window_plan(&inputs);

        assert_has_code(
            &output,
            RomWindowPlanDiagnosticCode::RomPolicyProjectionMismatch,
        );
        assert_has_detail(&output, "not adjacent in RomWindowPlan epoch order");
    }

    #[test]
    fn failure_outputs_do_not_emit_window_cert() {
        let output = build_rom_window_plan(&fixture_inputs(
            PlacementProfile::Budgeted,
            vec![],
            vec![],
            vec![rom_const(7, 20_000, None)],
        ));

        assert_eq!(output.outcome, RomWindowPlanOutcome::Failed);
        let report = emit_rom_window_plan_report(&output).expect("failure report emits");
        assert!(
            emit_window_cert_report(&output, report.report_self_hash)
                .expect("failure cert skipped")
                .is_none()
        );
    }

    impl RomWindowPlan {
        fn summary_bank_count(&self) -> u32 {
            self.banks.len() as u32
        }
    }

    fn assert_has_code(output: &RomWindowPlanOutput, expected: RomWindowPlanDiagnosticCode) {
        assert_eq!(output.outcome, RomWindowPlanOutcome::Failed);
        assert!(
            output.diagnostics.iter().any(|diagnostic| matches!(
                diagnostic.code,
                ValidationCode::RomWindowPlan { code, .. } if code == expected
            )),
            "missing {expected:?} in {:?}",
            output.diagnostics
        );
    }

    fn assert_has_detail(output: &RomWindowPlanOutput, expected: &str) {
        assert!(
            output.diagnostics.iter().any(|diagnostic| matches!(
                &diagnostic.code,
                ValidationCode::RomWindowPlan {
                    provenance: RomWindowPlanDiagnosticProvenance::PolicyProjection {
                        detail,
                        ..
                    },
                    ..
                } if detail.contains(expected)
            )),
            "missing detail {expected:?} in {:?}",
            output.diagnostics
        );
    }

    fn assert_has_hash_mismatch_product(output: &RomWindowPlanOutput, expected: &str) {
        assert!(
            output.diagnostics.iter().any(|diagnostic| matches!(
                &diagnostic.code,
                ValidationCode::RomWindowPlan {
                    provenance: RomWindowPlanDiagnosticProvenance::HashMismatch {
                        product,
                        ..
                    },
                    ..
                } if product == expected
            )),
            "missing hash mismatch product {expected:?} in {:?}",
            output.diagnostics
        );
    }

    fn assert_has_phase_banks(output: &RomWindowPlanOutput, expected: &[u16]) {
        assert!(
            output.diagnostics.iter().any(|diagnostic| matches!(
                &diagnostic.code,
                ValidationCode::RomWindowPlan {
                    provenance: RomWindowPlanDiagnosticProvenance::Phase {
                        demanded_banks,
                        ..
                    },
                    ..
                } if demanded_banks == expected
            )),
            "missing phase banks {expected:?} in {:?}",
            output.diagnostics
        );
    }

    fn registry_inputs(
        kernels: Vec<ObservationKernelResidencyRecord>,
        luts: Vec<ObservationLutResidencyRecord>,
        reduction_subordinates: Vec<RangeReductionSubordinate>,
    ) -> RomWindowPlanRegistryInputs {
        let base_inputs = fixture_inputs(PlacementProfile::Budgeted, vec![], vec![], vec![]);
        let audit_parents = registry_audit_parents(&base_inputs);
        RomWindowPlanRegistryInputs {
            observation_registry: RomWindowObservationRegistry {
                observation_plan_self_hash: base_inputs.input_identity.observation_plan_self_hash,
                infer_ir_self_hash: audit_parents.infer_ir_self_hash,
                quant_graph_self_hash: audit_parents.quant_graph_self_hash,
                source: RomWindowRegistrySource::Available {
                    registry_key: "observation_plan.rom_window_facts".to_owned(),
                },
                kernels,
                luts,
            },
            range_registry: RomWindowRangeRegistry {
                range_plan_self_hash: base_inputs.input_identity.range_plan_self_hash,
                infer_ir_self_hash: audit_parents.infer_ir_self_hash,
                quant_graph_self_hash: audit_parents.quant_graph_self_hash,
                static_budget_self_hash: audit_parents.static_budget_self_hash,
                source: RomWindowRegistrySource::Available {
                    registry_key: "range_plan.rom_window_facts".to_owned(),
                },
                reduction_subordinates,
            },
            audit_parents,
            base_inputs,
        }
    }

    fn registry_audit_parents(inputs: &RomWindowPlanInputs) -> RomWindowPlanAuditParents {
        RomWindowPlanAuditParents {
            artifact_validation_self_hash: inputs.input_identity.artifact_validation_self_hash,
            policy_resolution_self_hash: inputs.input_identity.policy_resolution_self_hash,
            static_budget_self_hash: inputs.input_identity.static_budget_self_hash,
            quant_graph_self_hash: inputs.input_identity.quant_graph_self_hash,
            infer_ir_self_hash: inputs.input_identity.infer_ir_self_hash,
            observation_plan_self_hash: inputs.input_identity.observation_plan_self_hash,
            range_plan_self_hash: inputs.input_identity.range_plan_self_hash,
            storage_plan_self_hash: inputs.input_identity.storage_plan_self_hash,
            sram_page_plan_self_hash: inputs.input_identity.sram_page_plan_self_hash,
        }
    }

    fn fixture_inputs(
        profile: PlacementProfile,
        kernels: Vec<KernelResidencyInput>,
        luts: Vec<LutResidencyInput>,
        storage_bindings: Vec<StorageBindingInput>,
    ) -> RomWindowPlanInputs {
        let infer_ir = infer_ir_for_bindings(&storage_bindings);
        let infer_ir_hash = infer_ir_self_hash(&infer_ir).expect("infer ir fixture hashes");
        let hashes = RomWindowPlanInputHashes {
            artifact_validation_self_hash: hash(0x21),
            policy_resolution_self_hash: hash(0x22),
            static_budget_self_hash: hash(0x23),
            quant_graph_self_hash: hash(0x24),
            infer_ir_self_hash: infer_ir_hash,
            storage_plan_self_hash: hash(1),
            observation_plan_self_hash: hash(2),
            range_plan_self_hash: hash(3),
            sram_page_plan_self_hash: hash(4),
            runtime_chrome_budget_hash: hash(5),
            target_profile_hash: hash(6),
            rom_window_plan_policy_projection_hash: hash(7),
        };
        let identity = RomWindowPlanInputIdentity {
            artifact_validation_self_hash: hashes.artifact_validation_self_hash,
            policy_resolution_self_hash: hashes.policy_resolution_self_hash,
            static_budget_self_hash: hashes.static_budget_self_hash,
            quant_graph_self_hash: hashes.quant_graph_self_hash,
            infer_ir_self_hash: hashes.infer_ir_self_hash,
            storage_plan_self_hash: hashes.storage_plan_self_hash,
            observation_plan_self_hash: hashes.observation_plan_self_hash,
            range_plan_self_hash: hashes.range_plan_self_hash,
            sram_page_plan_self_hash: hashes.sram_page_plan_self_hash,
            runtime_chrome_budget_hash: hashes.runtime_chrome_budget_hash,
            target_profile_hash: hashes.target_profile_hash,
            rom_window_plan_policy_projection_hash: hashes.rom_window_plan_policy_projection_hash,
            runtime_mode: RuntimeMode::Steady,
            determinism: DeterminismClass::Deterministic,
            target_profile_id: TargetProfileId::from("dmg-mbc5"),
            schema_version: ROM_WINDOW_PLAN_SCHEMA_VERSION,
        };
        RomWindowPlanInputs {
            input_identity: identity.clone(),
            expected_input_hashes: hashes,
            infer_ir,
            runtime_chrome_budget: RuntimeChromeBudget {
                target: TargetProfileId::from("dmg-mbc5"),
                profile: CompileProfileId::from("Bringup"),
                runtime_nucleus_hash: hash(8),
                rom_slots: vec![
                    RomBudgetSlot {
                        id: BudgetSlotId::new(0),
                        class: BudgetSlotClass::Bank0Free,
                        usable_bytes: 16 * 1024,
                        reserved_slack: 0,
                        placement_caps: BTreeSet::from([profile]),
                    },
                    RomBudgetSlot {
                        id: BudgetSlotId::new(1),
                        class: BudgetSlotClass::CommonBank,
                        usable_bytes: 16 * 1024,
                        reserved_slack: 0,
                        placement_caps: BTreeSet::from([profile]),
                    },
                    RomBudgetSlot {
                        id: BudgetSlotId::new(2),
                        class: BudgetSlotClass::ExpertBank,
                        usable_bytes: 16 * 1024,
                        reserved_slack: 0,
                        placement_caps: BTreeSet::from([profile]),
                    },
                ],
                memory_caps: RuntimeMemoryCapSection {
                    wram_usable_bytes: 8 * 1024,
                    sram_usable_bytes: 32 * 1024,
                    hram_usable_bytes: 127,
                    source_target_profile_hash: hash(9),
                },
                wram_reserved: 128,
                sram_reserved: 512,
            },
            policy: RomWindowPlanPolicyProjection {
                placement_profile: profile,
                rom_window: RomWindowKnob {
                    kernel_residency_bias: RomKernelResidencyBias::PreferCommonBank,
                    kernel_duplication_bias: RomKernelDuplicationBias::Share,
                },
                max_bank_switches_per_token: Some(8),
            },
            sram_page_plan: sram_page_plan(identity.sram_page_plan_self_hash),
            epochs: vec![epoch(0, None), epoch(1, None)],
            kernels,
            luts,
            storage_bindings,
        }
    }

    fn kernel(
        id: &str,
        byte_size: u32,
        reachability: RomReachabilityClass,
        overlay_eligible: bool,
        requested_bank: Option<RomBankIndex>,
    ) -> KernelResidencyInput {
        KernelResidencyInput {
            kernel: KernelSpecId::from(id),
            byte_size,
            reachability,
            overlay_eligible,
            active_epochs: vec![ResidencyEpochId(0)],
            requested_bank,
        }
    }

    fn lut(
        id: &str,
        byte_size: u32,
        reachability: RomReachabilityClass,
        overlay_eligible: bool,
        requested_bank: Option<RomBankIndex>,
    ) -> LutResidencyInput {
        LutResidencyInput {
            lut: LutInstanceId(id.to_owned()),
            byte_size,
            reachability,
            overlay_eligible,
            active_epochs: vec![ResidencyEpochId(0)],
            requested_bank,
        }
    }

    fn observation_kernel(
        id: &str,
        byte_size: u32,
        reachability: RomReachabilityClass,
        overlay_eligible: bool,
        requested_bank: Option<RomBankIndex>,
        active_epochs: Vec<ResidencyEpochId>,
    ) -> ObservationKernelResidencyRecord {
        ObservationKernelResidencyRecord {
            kernel: KernelSpecId::from(id),
            byte_size,
            reachability: Some(reachability),
            overlay_eligible,
            active_epochs,
            requested_bank,
            source: RomWindowRegistrySource::Available {
                registry_key: id.to_owned(),
            },
        }
    }

    fn observation_lut(
        id: &str,
        byte_size: u32,
        reachability: RomReachabilityClass,
        overlay_eligible: bool,
        requested_bank: Option<RomBankIndex>,
        active_epochs: Vec<ResidencyEpochId>,
    ) -> ObservationLutResidencyRecord {
        ObservationLutResidencyRecord {
            lut: LutInstanceId(id.to_owned()),
            byte_size,
            reachability: Some(reachability),
            overlay_eligible,
            active_epochs,
            requested_bank,
            source: RomWindowRegistrySource::Available {
                registry_key: id.to_owned(),
            },
        }
    }

    fn rom_const(
        value: u32,
        payload_bytes: u32,
        requested_bank: Option<RomBankIndex>,
    ) -> StorageBindingInput {
        rom_const_with_epochs(
            value,
            payload_bytes,
            requested_bank,
            vec![ResidencyEpochId(0)],
        )
    }

    fn rom_const_with_epochs(
        value: u32,
        payload_bytes: u32,
        requested_bank: Option<RomBankIndex>,
        active_epochs: Vec<ResidencyEpochId>,
    ) -> StorageBindingInput {
        storage_binding_with_epochs(
            value,
            Materialization::Materialize {
                class: StorageClass::RomConst,
                lifetime: LifetimeClass::Token,
            },
            payload_bytes,
            requested_bank,
            active_epochs,
        )
    }

    fn sram_paged_with_epochs(
        value: u32,
        active_epochs: Vec<ResidencyEpochId>,
    ) -> StorageBindingInput {
        storage_binding_with_epochs(
            value,
            Materialization::Materialize {
                class: StorageClass::SramPaged,
                lifetime: LifetimeClass::Persistent,
            },
            128,
            None,
            active_epochs,
        )
    }

    fn persist_with_epochs(
        value: u32,
        active_epochs: Vec<ResidencyEpochId>,
    ) -> StorageBindingInput {
        storage_binding_with_epochs(
            value,
            Materialization::Persist {
                page: crate::storage_plan::types::PersistPageId(value),
                commit_group: crate::storage_plan::types::CommitGroupId(value),
            },
            128,
            None,
            active_epochs,
        )
    }

    fn storage_binding_with_epochs(
        value: u32,
        materialization: Materialization,
        payload_bytes: u32,
        requested_bank: Option<RomBankIndex>,
        active_epochs: Vec<ResidencyEpochId>,
    ) -> StorageBindingInput {
        let value = ValueId::new(value);
        let first_epoch = active_epochs.iter().map(|epoch| epoch.0).min().unwrap_or(0);
        let last_epoch = active_epochs
            .iter()
            .map(|epoch| epoch.0)
            .max()
            .unwrap_or(first_epoch)
            .saturating_add(1);
        StorageBindingInput {
            binding: StorageBinding {
                value,
                materialization,
                alias_class: AliasClassId(value.get()),
                live_range: AbstractLiveRange {
                    def_node: NodeId::new(first_epoch),
                    first_use_node: Some(NodeId::new(first_epoch)),
                    last_use_node: Some(NodeId::new(last_epoch)),
                    lifetime_class: LifetimeClass::Token,
                    checkpoint_stable: false,
                },
                justification: BindingJustification::DecisionRule(DecisionRuleId(1)),
            },
            payload_bytes,
            active_epochs,
            requested_bank,
        }
    }

    fn infer_ir_for_bindings(storage_bindings: &[StorageBindingInput]) -> GbInferIR {
        let mut node_outputs = BTreeMap::<NodeId, Vec<ValueId>>::from([
            (NodeId::new(0), Vec::new()),
            (NodeId::new(1), Vec::new()),
        ]);
        for binding in storage_bindings {
            node_outputs
                .entry(binding.binding.live_range.def_node)
                .or_default()
                .push(binding.binding.value);
        }
        let nodes = node_outputs
            .into_iter()
            .map(|(node_id, outputs)| GbNode {
                node_id,
                op: InferOp::Classify,
                inputs: Vec::new(),
                effects_in: Vec::new(),
                outputs,
                effects_out: Vec::new(),
                reduction_site: None,
            })
            .collect::<Vec<_>>();
        let values = storage_bindings
            .iter()
            .map(|binding| ValueDecl {
                value_id: binding.binding.value,
                kind: ValueKind::Activation,
                format: ValueFormat::Quant {
                    format: QuantFormat::Q8_8,
                },
                layout: ValueLayout::scalar(),
            })
            .collect::<Vec<_>>();
        GbInferIR {
            identity: InferIrIdentity {
                quant_graph_self_hash: hash(0x30),
                infer_ir_policy_projection_hash: hash(0x31),
                static_budget_self_hash: hash(0x32),
                requested_runtime_modes_hash: hash(0x33),
                determinism: DeterminismClass::Deterministic,
                topological_order_hash: hash(0x34),
            },
            token_inputs: Vec::new(),
            nodes,
            values,
            effects: Vec::new(),
            provenance: InferIrProvenance::default(),
            anchors: BTreeMap::new(),
        }
    }

    fn epoch(id: u32, sram_page_binding: Option<ValueId>) -> RomWindowEpochInput {
        RomWindowEpochInput {
            id: ResidencyEpochId(id),
            op_range: NodeAnchorRange {
                first_node: NodeId::new(id),
                last_node: NodeId::new(id + 1),
            },
            sram_page_binding,
            yield_kind: YieldKindHint::NoYieldsExpected,
        }
    }

    fn sram_page_plan(self_hash: Hash256) -> SramPagePlan {
        SramPagePlan {
            identity: SramPagePlanInputIdentity {
                storage_plan_self_hash: hash(10),
                observation_plan_self_hash: hash(11),
                range_plan_self_hash: hash(12),
                runtime_chrome_budget_hash: hash(13),
                target_profile_hash: hash(14),
                sram_page_plan_policy_projection_hash: hash(15),
                determinism: DeterminismClass::Deterministic,
                schema_version: crate::sram_page_plan::SRAM_PAGE_PLAN_SCHEMA_VERSION,
            },
            active_sets: Vec::new(),
            bindings: vec![SramPageBinding {
                binding_id: ValueId::new(99),
                page: PageId(0),
                commit_group: crate::storage_plan::types::CommitGroupId(0),
                op_range: NodeAnchorRange {
                    first_node: NodeId::new(99),
                    last_node: NodeId::new(100),
                },
                residency_role: SramResidencyRole::PersistentSequenceState,
                residency: PageResidency::FixedPage { page: PageId(0) },
                payload_bytes: 64,
                geometry: PersistentPageGeometry::dmg_mbc5_8k(),
                sequence_stream: crate::sram_page_plan::SequenceStreamId(0),
            }],
            pages: vec![PersistentPage {
                page: PageId(0),
                sequence_stream: crate::sram_page_plan::SequenceStreamId(0),
                commit_groups: vec![crate::storage_plan::types::CommitGroupId(0)],
                payload_bytes: 64,
                binding_count: 1,
            }],
            stream_index: vec![SramStreamIndexEntry {
                sequence_stream: crate::sram_page_plan::SequenceStreamId(0),
                pages: vec![PageId(0)],
            }],
            commit_boundaries: Vec::new(),
            page_rotations: Vec::new(),
            spill_policy: Default::default(),
            projections: crate::sram_page_plan::SramSwitchProjections::empty(),
            budgets: SramBudgetTally {
                total_bytes: 64,
                cap_bytes: 8 * 1024,
                page_count: 1,
                stream_count: 1,
                per_stream: Vec::new(),
            },
            geometry: PersistentPageGeometry::dmg_mbc5_8k(),
            sram_page_plan_self_hash: self_hash,
        }
    }

    trait FixtureInputExt {
        fn with_wram_overlay_bias(self) -> Self;
    }

    impl FixtureInputExt for RomWindowPlanInputs {
        fn with_wram_overlay_bias(mut self) -> Self {
            self.policy.rom_window.kernel_residency_bias =
                RomKernelResidencyBias::PreferWramOverlay;
            self
        }
    }

    fn hash(byte: u8) -> Hash256 {
        Hash256::from_bytes([byte; 32])
    }
}
