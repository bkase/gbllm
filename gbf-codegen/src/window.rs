//! Stage 8 `RomWindowPlan` construction, report, and cache-key surface.

use std::collections::{BTreeMap, BTreeSet};
use std::{error::Error, fmt};

use gbf_foundation::{
    CanonicalJsonError, DomainHash, EvidenceRef, Hash256, KernelSpecId, SemVer, TargetProfileId,
    canonical_json_bytes_omitting_fields, self_hash_omitting_fields,
};
use gbf_policy::{
    BudgetSlotClass, DiagnosticSeverity, PlacementProfile, RomWindowKnob,
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
use crate::s3::infer_ir::{NodeId, ValueId};
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

const ROM_WINDOW_PLAN_INPUT_PRODUCTS: [RomWindowPlanInputProduct; 7] = [
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
    pub runtime_chrome_budget: RuntimeChromeBudget,
    pub policy: RomWindowPlanPolicyProjection,
    pub sram_page_plan: SramPagePlan,
    pub epochs: Vec<RomWindowEpochInput>,
    pub kernels: Vec<KernelResidencyInput>,
    pub luts: Vec<LutResidencyInput>,
    pub storage_bindings: Vec<RomConstBindingInput>,
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
pub struct RomConstBindingInput {
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
    pub total_overlay_bytes: u32,
    pub total_install_count_per_token_upper_bound: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WramOverlayKernelDemand {
    pub kernel: KernelSpecId,
    pub byte_size: u32,
    pub reachability: RomReachabilityClass,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WramOverlayLutDemand {
    pub lut: LutInstanceId,
    pub byte_size: u32,
    pub reachability: RomReachabilityClass,
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

pub fn build_rom_window_plan(input: &RomWindowPlanInputs) -> RomWindowPlanOutput {
    let hash_diagnostics = input_hash_mismatch_diagnostics(input);
    if !hash_diagnostics.is_empty() {
        return failed_output(input.input_identity.clone(), hash_diagnostics);
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
            KernelResidency::WramOverlay => overlay_kernels.push(WramOverlayKernelDemand {
                kernel: kernel.kernel.clone(),
                byte_size: kernel.byte_size,
                reachability: kernel.reachability,
            }),
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
            LutResidency::WramStaged { .. } => overlay_luts.push(WramOverlayLutDemand {
                lut: lut.lut.clone(),
                byte_size: lut.byte_size,
                reachability: lut.reachability,
            }),
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
    let overlay_demand = WramOverlayDemand {
        kernels: overlay_kernels,
        luts: overlay_luts,
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
    );
    if let Some((epoch, demanded)) = bindings.iter().find_map(|binding| {
        demanded_banks_for_binding(binding, &kernel_residency, &lut_residency, &tensor_to_bank)
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

pub fn parse_rom_window_plan_report_bytes(
    bytes: &[u8],
) -> Result<RomWindowPlanReportEnvelope, ReportSelfHashError> {
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
}

impl fmt::Display for RomWindowPlanEmitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Envelope(error) => write!(f, "rom window report envelope failed: {error}"),
            Self::SelfHash(error) => write!(f, "rom window report self hash failed: {error}"),
            Self::Canonical(error) => {
                write!(f, "rom window report canonicalization failed: {error}")
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

fn build_window_bindings(
    input: &RomWindowPlanInputs,
    kernels: &[KernelResidencyInput],
    luts: &[LutResidencyInput],
    storage_bindings: &[RomConstBindingInput],
    kernel_residency: &BTreeMap<KernelSpecId, KernelResidency>,
    lut_residency: &BTreeMap<LutInstanceId, LutResidency>,
    tensor_to_bank: &BTreeMap<ValueId, RomBankIndex>,
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
    (banks.len() > 1).then_some(banks)
}

fn unique_demanded_bank(
    kernels: &[KernelSpecId],
    luts: &[LutInstanceId],
    tensors: &[ValueId],
    kernel_residency: &BTreeMap<KernelSpecId, KernelResidency>,
    lut_residency: &BTreeMap<LutInstanceId, LutResidency>,
    tensor_to_bank: &BTreeMap<ValueId, RomBankIndex>,
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
    (banks.len() == 1).then(|| *banks.iter().next().expect("one bank"))
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

    use gbf_foundation::{BudgetSlotId, CompileProfileId};
    use gbf_policy::{
        RomBudgetSlot, RomKernelDuplicationBias, RomKernelResidencyBias, RuntimeMemoryCapSection,
    };

    use super::*;
    use crate::sram_page_plan::{
        PageId, PageResidency, PersistentPage, PersistentPageGeometry, SramBudgetTally,
        SramPageBinding, SramPagePlanInputIdentity, SramStreamIndexEntry,
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
        assert!(plan.banks.is_empty());
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

    fn fixture_inputs(
        profile: PlacementProfile,
        kernels: Vec<KernelResidencyInput>,
        luts: Vec<LutResidencyInput>,
        storage_bindings: Vec<RomConstBindingInput>,
    ) -> RomWindowPlanInputs {
        let hashes = RomWindowPlanInputHashes {
            storage_plan_self_hash: hash(1),
            observation_plan_self_hash: hash(2),
            range_plan_self_hash: hash(3),
            sram_page_plan_self_hash: hash(4),
            runtime_chrome_budget_hash: hash(5),
            target_profile_hash: hash(6),
            rom_window_plan_policy_projection_hash: hash(7),
        };
        let identity = RomWindowPlanInputIdentity {
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

    fn rom_const(
        value: u32,
        payload_bytes: u32,
        requested_bank: Option<RomBankIndex>,
    ) -> RomConstBindingInput {
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
    ) -> RomConstBindingInput {
        let value = ValueId::new(value);
        RomConstBindingInput {
            binding: StorageBinding {
                value,
                materialization: Materialization::Materialize {
                    class: StorageClass::RomConst,
                    lifetime: LifetimeClass::Token,
                },
                alias_class: AliasClassId(value.get()),
                live_range: AbstractLiveRange {
                    def_node: NodeId::new(value.get()),
                    first_use_node: Some(NodeId::new(value.get())),
                    last_use_node: Some(NodeId::new(value.get() + 1)),
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
            bindings: vec![SramPageBinding {
                binding_id: ValueId::new(99),
                page: PageId(0),
                commit_group: crate::storage_plan::types::CommitGroupId(0),
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
