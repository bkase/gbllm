//! Stage 10 `GbSchedIR` and Stage 10.5 `ResourceStateValidation` v1 surface.

use std::collections::{BTreeMap, BTreeSet};
use std::{error::Error, fmt};

use gbf_abi::SemanticCheckpointId;
use gbf_foundation::{
    CanonicalJsonError, DomainHash, EvidenceRef, ExpertId, Hash256, KernelSpecId, SemVer,
    TargetProfileId, canonical_json_bytes_omitting_fields, self_hash_omitting_fields,
};
use gbf_policy::{
    ResourceStateDiagnosticCode, ResourceStateDiagnosticProvenance, RuntimeMode, TraceProbeId,
    ValidationCode, ValidationDetail, ValidationDiagnostic, ValidationOrigin,
};
use gbf_report::{
    CanonicalJsonError as ReportCanonicalJsonError, ReportBody, ReportEnvelope,
    ReportEnvelopeError, ReportOutcome, ReportSelfHashError, canonicalize, round_trip_self_hash,
};
use serde::{Deserialize, Serialize};

use crate::arena::{ArenaBacking, ArenaPlan, ArenaSlotId};
use crate::overlay_plan::{OverlayId, OverlayInstallId, OverlayPlan};
use crate::s1::quant_graph::DeterminismClass;
use crate::s3::infer_ir::ValueId;
use crate::stage_cache::{
    CodegenStageCacheError, StoreBackedStageCacheKeys, StoreBackedStageCellKind,
    StoreBackedStageExpectedHashes, StoreBackedStageRunOutput, StoreBackedStageRunResult,
    run_store_backed_stage_with_cache, stage10_schedule_pack_store_key,
    stage105_resource_state_store_key,
};
use crate::window::{
    OverlayState, RomBankIndex, RomVisibility, RomWindowBindingId, RomWindowPlan, YieldKindHint,
};
use gbf_store::stage_cache::StageCache as StoreStageCache;

pub const SCHED_IR_SCHEMA_ID: &str = "sched_ir.v1";
pub const SCHED_IR_SCHEMA_VERSION: SemVer = SemVer::new(1, 0, 0);
pub const SCHED_IR_PASS_VERSION: &str = "stage10/v1";
pub const RESOURCE_STATE_CERT_SCHEMA_ID: &str = "resource_state.cert.v1";
pub const RESOURCE_STATE_CERT_SCHEMA_VERSION: SemVer = SemVer::new(1, 0, 0);
pub const RESOURCE_STATE_CERT_PASS_VERSION: &str = "resource_state/stage10_5/v1";

pub type SchedulePackReportEnvelope = ReportEnvelope<SchedulePackReportBody>;
pub type SliceReportEnvelope = ReportEnvelope<SliceReportBody>;
pub type ResourceStateCertEnvelope = ReportEnvelope<ResourceStateCertificate>;

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SliceId(pub u32);

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct LeaseId(pub u32);

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SchedEpochId(pub u32);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SchedulePackInputIdentity {
    pub infer_ir_self_hash: Hash256,
    pub observation_plan_self_hash: Hash256,
    pub range_plan_self_hash: Hash256,
    pub storage_plan_self_hash: Hash256,
    pub sram_page_plan_self_hash: Hash256,
    pub rom_window_plan_self_hash: Hash256,
    pub overlay_plan_self_hash: Hash256,
    pub arena_plan_self_hash: Hash256,
    pub policy_resolution_self_hash: Hash256,
    pub runtime_chrome_budget_self_hash: Hash256,
    pub feature_set_hash: Hash256,
    pub requested_runtime_modes: BTreeSet<RuntimeMode>,
    pub determinism: DeterminismClass,
    pub target_profile_id: TargetProfileId,
    pub schema_version: SemVer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SchedulePackInputHashes {
    pub infer_ir_self_hash: Hash256,
    pub observation_plan_self_hash: Hash256,
    pub range_plan_self_hash: Hash256,
    pub storage_plan_self_hash: Hash256,
    pub sram_page_plan_self_hash: Hash256,
    pub rom_window_plan_self_hash: Hash256,
    pub overlay_plan_self_hash: Hash256,
    pub arena_plan_self_hash: Hash256,
    pub policy_resolution_self_hash: Hash256,
    pub runtime_chrome_budget_self_hash: Hash256,
    pub feature_set_hash: Hash256,
}

impl SchedulePackInputIdentity {
    #[must_use]
    pub const fn hashes(&self) -> SchedulePackInputHashes {
        SchedulePackInputHashes {
            infer_ir_self_hash: self.infer_ir_self_hash,
            observation_plan_self_hash: self.observation_plan_self_hash,
            range_plan_self_hash: self.range_plan_self_hash,
            storage_plan_self_hash: self.storage_plan_self_hash,
            sram_page_plan_self_hash: self.sram_page_plan_self_hash,
            rom_window_plan_self_hash: self.rom_window_plan_self_hash,
            overlay_plan_self_hash: self.overlay_plan_self_hash,
            arena_plan_self_hash: self.arena_plan_self_hash,
            policy_resolution_self_hash: self.policy_resolution_self_hash,
            runtime_chrome_budget_self_hash: self.runtime_chrome_budget_self_hash,
            feature_set_hash: self.feature_set_hash,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SchedulePackInputs {
    pub input_identity: SchedulePackInputIdentity,
    pub expected_input_hashes: SchedulePackInputHashes,
    pub rom_window_plan: RomWindowPlan,
    pub overlay_plan: OverlayPlan,
    pub arena_plan: ArenaPlan,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SchedulePack {
    pub identity: SchedulePackInputIdentity,
    pub modes: Vec<ModeSchedule>,
    pub epochs: Vec<ModeResidencyEpochs>,
    pub leases: Vec<ResourceLease>,
    pub checkpoint_schema_hash: Hash256,
    pub continuation_abi_hash: Hash256,
    pub switch_policy: ModeSwitchPolicy,
    pub drift_monitor: RuntimeDriftMonitor,
    pub schedule_pack_self_hash: Hash256,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModeSchedule {
    pub mode: RuntimeMode,
    pub ir: GbSchedIR,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModeResidencyEpochs {
    pub mode: RuntimeMode,
    pub epochs: Vec<ResidencyEpoch>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GbSchedIR {
    pub mode: RuntimeMode,
    pub entry_slice: SliceId,
    pub slices: Vec<SchedSlice>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SchedSlice {
    pub id: SliceId,
    pub ops: Vec<SchedOp>,
    pub hard_cycles_to_safe_point: u32,
    pub soft_target_cycles: u32,
    pub max_interrupt_latency: u32,
    pub resources: ResourceVector,
    pub live_wram: Vec<ArenaSlotId>,
    pub live_sram: Vec<ArenaSlotId>,
    pub yield_kind: YieldKind,
    pub yield_check: YieldCheckClass,
    pub entry_residency: EntryResidency,
    pub interrupt_policy: InterruptPolicy,
    pub required_leases: Vec<LeaseId>,
    pub exit_kind: ExitKind,
    pub semantic_checkpoint_pins: Vec<SemanticCheckpointId>,
    pub trace_probe_pins: Vec<TraceProbeId>,
    pub successors: Vec<SliceId>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceVector {
    pub bank_switches: u16,
    pub sram_page_switches: u16,
    pub trace_bytes: u32,
    pub persist_bytes: u32,
    pub overlay_installs: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ResourceLeaseKind {
    RomWindow { binding: RomWindowBindingId },
    SramPage { binding: ValueId },
    Overlay { overlay: OverlayId },
    InterruptMask { policy: InterruptPolicy },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceLease {
    pub id: LeaseId,
    pub kind: ResourceLeaseKind,
    pub acquired_in: SliceId,
    pub released_in: SliceId,
    pub yield_safe: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResidencyEpoch {
    pub id: SchedEpochId,
    pub rom_window: RomWindowBindingId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overlay: Option<OverlayId>,
    pub residency: EntryResidency,
    pub slices: Vec<SliceId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", deny_unknown_fields)]
pub enum SchedOp {
    AcquireLease {
        lease: LeaseId,
    },
    ReleaseLease {
        lease: LeaseId,
    },
    OverlayInstall {
        install: OverlayInstallId,
    },
    BankSwitch {
        binding: RomWindowBindingId,
        bank: RomBankIndex,
    },
    SramPageSwitch {
        binding: ValueId,
    },
    KernelCall {
        spec: KernelSpecId,
        tile_index: u32,
    },
    Load {
        slot: ArenaSlotId,
    },
    Store {
        slot: ArenaSlotId,
    },
    Effect {
        effect_id: u32,
    },
    TraceProbe {
        probe: TraceProbeId,
    },
    SemanticCheckpoint {
        checkpoint: SemanticCheckpointId,
    },
    PersistCommit {
        binding: ValueId,
    },
    Yield {
        kind: YieldKind,
    },
    TailCall {
        target: SliceId,
    },
    EnterIsr,
    Halt,
    Fault,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum YieldKind {
    Micro,
    Frame,
    NeedInput,
    TokenReady,
    Finished,
    Fault,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum YieldCheckClass {
    OnceAtEnd,
    EveryNTiles { n: u16 },
    EveryLoadStore,
    NoPoll,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum EntryResidency {
    Bank0,
    Common {
        bank: RomBankIndex,
    },
    Expert {
        expert: ExpertId,
        bank: RomBankIndex,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum InterruptPolicy {
    Enabled,
    ShortCriticalSection,
    Disabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ExitKind {
    SaveContinuationAndYield,
    TailCall,
    EnterIsr,
    Halt,
    Fault,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModeSwitchPolicy {
    pub legal_switch_points: Vec<SemanticCheckpointId>,
    pub legal_epoch_boundaries: Vec<SchedEpochId>,
    pub ui_pressure_thresholds: Vec<UiPressureThreshold>,
    pub safe_mode_triggers: Vec<SafeModeTrigger>,
    pub drift_triggers: Vec<DriftTrigger>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiPressureThreshold {
    pub max_pending_frames: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum SafeModeTrigger {
    Fault,
    DriftViolation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeDriftMonitor {
    pub expected: DriftEnvelope,
    pub observed: ObservedDriftEnvelope,
    pub consecutive_violations: u16,
    pub window_frames: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DriftEnvelope {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slice_cycles_p95: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui_commit_cycles_p95: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_drop_rate_pct: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persist_overrun_rate_pct: Option<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservedDriftEnvelope {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slice_cycles_p95: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui_commit_cycles_p95: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_drop_rate_pct: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persist_overrun_rate_pct: Option<u16>,
}

impl ObservedDriftEnvelope {
    #[must_use]
    pub const fn all_none() -> Self {
        Self {
            slice_cycles_p95: None,
            ui_commit_cycles_p95: None,
            trace_drop_rate_pct: None,
            persist_overrun_rate_pct: None,
        }
    }

    #[must_use]
    pub const fn is_all_none(self) -> bool {
        self.slice_cycles_p95.is_none()
            && self.ui_commit_cycles_p95.is_none()
            && self.trace_drop_rate_pct.is_none()
            && self.persist_overrun_rate_pct.is_none()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DriftTrigger {
    pub metric: DriftMetric,
    pub threshold: u32,
    pub action: DriftAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum DriftMetric {
    SliceCyclesP95,
    UiCommitCyclesP95,
    TraceDropRatePct,
    PersistOverrunRatePct,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum DriftAction {
    ShrinkSlices,
    DropTrace,
    DemoteMode { mode: RuntimeMode },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SliceReportSummary {
    pub mode_count: u16,
    pub slice_count: u32,
    pub lease_count: u32,
    pub epoch_count: u32,
    pub max_interrupt_latency: u32,
    pub total_bank_switches: u32,
    pub total_sram_page_switches: u32,
    pub total_overlay_installs: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SchedulePackResult {
    pub product: SchedulePack,
    pub schedule_pack_self_hash: Hash256,
    pub summary: SliceReportSummary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchedulePackOutput {
    pub input_identity: SchedulePackInputIdentity,
    pub outcome: SchedulePackOutcome,
    pub result: Option<SchedulePackResult>,
    pub cert: Option<ResourceStateCertificate>,
    pub diagnostics: Vec<ValidationDiagnostic>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulePackOutcome {
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SchedulePackReportBody {
    pub pass_version: String,
    pub input_identity: SchedulePackInputIdentity,
    pub diagnostics: Vec<ValidationDiagnostic>,
    pub result: Option<SchedulePackResult>,
}

impl ReportBody for SchedulePackReportBody {
    const REPORT_TYPE: &'static str = "SchedulePackReport";
    const SCHEMA_ID: &'static str = SCHED_IR_SCHEMA_ID;
    const SCHEMA_VERSION: &'static str = "1.0.0";

    fn validate_semantics(&self, outcome: ReportOutcome) -> Result<(), Vec<ValidationDiagnostic>> {
        validate_schedule_report_body(self.result.is_some(), &self.diagnostics, outcome)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SliceReportBody {
    pub pass_version: String,
    pub schedule_pack_self_hash: Hash256,
    pub summary: SliceReportSummary,
    pub diagnostics: Vec<ValidationDiagnostic>,
}

impl ReportBody for SliceReportBody {
    const REPORT_TYPE: &'static str = "SliceReport";
    const SCHEMA_ID: &'static str = "slice_report.v1";
    const SCHEMA_VERSION: &'static str = "1.0.0";

    fn validate_semantics(&self, outcome: ReportOutcome) -> Result<(), Vec<ValidationDiagnostic>> {
        validate_schedule_report_body(true, &self.diagnostics, outcome)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceStateCertificate {
    pub pass_version: String,
    pub identity: SchedulePackInputIdentity,
    pub schedule_pack_self_hash: Hash256,
    pub lease_balance: LeaseBalanceSection,
    pub yield_safety: YieldSafetySection,
    pub isr_visible_residency: IsrVisibleResidencySection,
    pub overlay_bank_shadow: OverlayBankShadowSection,
    pub diagnostics: Vec<ValidationDiagnostic>,
    pub resource_state_cert_self_hash: Hash256,
}

impl ReportBody for ResourceStateCertificate {
    const REPORT_TYPE: &'static str = "ResourceStateCertificate";
    const SCHEMA_ID: &'static str = RESOURCE_STATE_CERT_SCHEMA_ID;
    const SCHEMA_VERSION: &'static str = "1.0.0";

    fn validate_semantics(&self, outcome: ReportOutcome) -> Result<(), Vec<ValidationDiagnostic>> {
        validate_schedule_report_body(true, &self.diagnostics, outcome)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LeaseBalanceSection {
    pub leases: Vec<LeaseBalanceFact>,
    pub all_balanced: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LeaseBalanceFact {
    pub lease: LeaseId,
    pub kind_discriminant: ResourceLeaseKindDiscriminant,
    pub acquired_in: SliceId,
    pub released_in: SliceId,
    pub yield_safe: bool,
    pub paths_checked: u32,
    pub all_paths_balanced: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ResourceLeaseKindDiscriminant {
    RomWindow,
    SramPage,
    Overlay,
    InterruptMask,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct YieldSafetySection {
    pub yield_events: Vec<YieldSafetyFact>,
    pub all_yields_safe: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct YieldSafetyFact {
    pub slice: SliceId,
    pub yield_kind: YieldKind,
    pub outstanding_leases: Vec<LeaseId>,
    pub all_yield_safe: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IsrVisibleResidencySection {
    pub enabled_slices: Vec<IsrVisibleResidencyFact>,
    pub all_isr_safe: bool,
    pub computed_reachability_confirmed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IsrVisibleResidencyFact {
    pub slice: SliceId,
    pub residency: EntryResidency,
    pub outstanding_leases: Vec<LeaseId>,
    pub all_safe: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverlayBankShadowSection {
    pub slices_checked: Vec<OverlayBankShadowConsistencyFact>,
    pub all_consistent: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverlayBankShadowConsistencyFact {
    pub slice: SliceId,
    pub epoch_id: SchedEpochId,
    pub entry_residency_matches: bool,
    pub overlay_installs_aligned: bool,
    pub overlay_lease_shape_satisfied: bool,
    pub bank_switches_bracketed: bool,
    pub all_consistent: bool,
}

pub fn build_schedule_pack(input: &SchedulePackInputs) -> SchedulePackOutput {
    let mut diagnostics = input_hash_mismatch_diagnostics(input);
    if input.input_identity.rom_window_plan_self_hash
        != input.rom_window_plan.rom_window_plan_self_hash
    {
        diagnostics.push(diagnostic(
            ResourceStateDiagnosticCode::SchedInputHashMismatch,
            ResourceStateDiagnosticProvenance::HashMismatch {
                product: "rom_window_plan".to_owned(),
                recorded: input.input_identity.rom_window_plan_self_hash,
                computed: input.rom_window_plan.rom_window_plan_self_hash,
            },
            ValidationOrigin::SchedIrConstruction,
        ));
    }
    if input.input_identity.overlay_plan_self_hash != input.overlay_plan.overlay_plan_self_hash {
        diagnostics.push(diagnostic(
            ResourceStateDiagnosticCode::SchedInputHashMismatch,
            ResourceStateDiagnosticProvenance::HashMismatch {
                product: "overlay_plan".to_owned(),
                recorded: input.input_identity.overlay_plan_self_hash,
                computed: input.overlay_plan.overlay_plan_self_hash,
            },
            ValidationOrigin::SchedIrConstruction,
        ));
    }
    if input.input_identity.arena_plan_self_hash != input.arena_plan.arena_plan_self_hash {
        diagnostics.push(diagnostic(
            ResourceStateDiagnosticCode::SchedInputHashMismatch,
            ResourceStateDiagnosticProvenance::HashMismatch {
                product: "arena_plan".to_owned(),
                recorded: input.input_identity.arena_plan_self_hash,
                computed: input.arena_plan.arena_plan_self_hash,
            },
            ValidationOrigin::SchedIrConstruction,
        ));
    }
    if !diagnostics.is_empty() {
        return failed_output(input.input_identity.clone(), diagnostics);
    }

    let runtime_modes = if input.input_identity.requested_runtime_modes.is_empty() {
        BTreeSet::from([RuntimeMode::Interactive])
    } else {
        input.input_identity.requested_runtime_modes.clone()
    };
    let live_wram = live_slots(&input.arena_plan, ArenaBacking::Wram);
    let live_sram = live_slots(&input.arena_plan, ArenaBacking::Sram);
    let overlay_for_active_epoch = input
        .overlay_plan
        .regions
        .first()
        .map(|region| region.id)
        .or_else(|| {
            input
                .arena_plan
                .overlay_reservation
                .per_region
                .first()
                .map(|entry| entry.overlay_id)
        });

    let mut all_leases = BTreeMap::new();
    let mut modes = Vec::new();
    let mut epochs_by_mode = Vec::new();
    for (mode_index, mode) in runtime_modes.into_iter().enumerate() {
        let id_base = (mode_index as u32).saturating_mul(10_000);
        let (ir, epochs, leases) = build_mode_schedule(
            mode,
            &input.rom_window_plan,
            &input.overlay_plan,
            overlay_for_active_epoch,
            &live_wram,
            &live_sram,
            id_base,
        );
        for lease in leases {
            all_leases.entry(lease.id).or_insert(lease);
        }
        modes.push(ModeSchedule { mode, ir });
        epochs_by_mode.push(ModeResidencyEpochs { mode, epochs });
    }

    let checkpoint_schema_hash = input.input_identity.observation_plan_self_hash;
    let continuation_abi_hash = match continuation_abi_hash(&input.input_identity) {
        Ok(hash) => hash,
        Err(error) => {
            return failed_output(
                input.input_identity.clone(),
                vec![diagnostic(
                    ResourceStateDiagnosticCode::ResourceStateReportRoundTripFailed,
                    ResourceStateDiagnosticProvenance::JsonPath {
                        json_path: "continuation_abi_hash".to_owned(),
                        field_or_tag: error.to_string(),
                    },
                    ValidationOrigin::SchedIrConstruction,
                )],
            );
        }
    };
    let legal_epoch_boundaries = epochs_by_mode
        .iter()
        .flat_map(|entry| entry.epochs.iter().map(|epoch| epoch.id))
        .collect();
    let mut pack = SchedulePack {
        identity: input.input_identity.clone(),
        modes,
        epochs: epochs_by_mode,
        leases: all_leases.into_values().collect(),
        checkpoint_schema_hash,
        continuation_abi_hash,
        switch_policy: ModeSwitchPolicy {
            legal_switch_points: Vec::new(),
            legal_epoch_boundaries,
            ui_pressure_thresholds: vec![UiPressureThreshold {
                max_pending_frames: 1,
            }],
            safe_mode_triggers: vec![SafeModeTrigger::Fault, SafeModeTrigger::DriftViolation],
            drift_triggers: vec![DriftTrigger {
                metric: DriftMetric::SliceCyclesP95,
                threshold: 17_556,
                action: DriftAction::ShrinkSlices,
            }],
        },
        drift_monitor: RuntimeDriftMonitor {
            expected: DriftEnvelope {
                slice_cycles_p95: Some(17_556),
                ui_commit_cycles_p95: None,
                trace_drop_rate_pct: Some(0),
                persist_overrun_rate_pct: Some(0),
            },
            observed: ObservedDriftEnvelope::all_none(),
            consecutive_violations: 0,
            window_frames: 60,
        },
        schedule_pack_self_hash: Hash256::ZERO,
    };

    if pack.modes.is_empty() {
        return failed_output(
            input.input_identity.clone(),
            vec![diagnostic(
                ResourceStateDiagnosticCode::SchedPackEmpty,
                ResourceStateDiagnosticProvenance::Mode {
                    mode: RuntimeMode::Interactive,
                },
                ValidationOrigin::SchedIrConstruction,
            )],
        );
    }

    let schedule_hash = match schedule_pack_self_hash(&pack) {
        Ok(hash) => hash,
        Err(error) => {
            return failed_output(
                input.input_identity.clone(),
                vec![diagnostic(
                    ResourceStateDiagnosticCode::ResourceStateReportRoundTripFailed,
                    ResourceStateDiagnosticProvenance::JsonPath {
                        json_path: "schedule_pack_self_hash".to_owned(),
                        field_or_tag: error.to_string(),
                    },
                    ValidationOrigin::SchedIrConstruction,
                )],
            );
        }
    };
    pack.schedule_pack_self_hash = schedule_hash;

    let validation = validate_resource_state(&pack);
    let mut cert = validation.certificate(input.input_identity.clone(), schedule_hash);
    let cert_hash = match resource_state_cert_self_hash(&cert) {
        Ok(hash) => hash,
        Err(error) => {
            return failed_output(
                input.input_identity.clone(),
                vec![diagnostic(
                    ResourceStateDiagnosticCode::ResourceStateReportRoundTripFailed,
                    ResourceStateDiagnosticProvenance::JsonPath {
                        json_path: "resource_state_cert_self_hash".to_owned(),
                        field_or_tag: error.to_string(),
                    },
                    ValidationOrigin::ResourceStateValidation,
                )],
            );
        }
    };
    cert.resource_state_cert_self_hash = cert_hash;

    if !validation.diagnostics.is_empty() {
        return SchedulePackOutput {
            input_identity: input.input_identity.clone(),
            outcome: SchedulePackOutcome::Failed,
            result: Some(SchedulePackResult {
                summary: slice_report_summary(&pack),
                schedule_pack_self_hash: schedule_hash,
                product: pack,
            }),
            cert: Some(cert),
            diagnostics: validation.diagnostics,
        };
    }

    SchedulePackOutput {
        input_identity: input.input_identity.clone(),
        outcome: SchedulePackOutcome::Succeeded,
        result: Some(SchedulePackResult {
            summary: slice_report_summary(&pack),
            schedule_pack_self_hash: schedule_hash,
            product: pack,
        }),
        cert: Some(cert),
        diagnostics: Vec::new(),
    }
}

pub fn validate_resource_state(pack: &SchedulePack) -> ResourceStateValidation {
    let mut validation = ResourceStateValidation::new();
    let lease_map: BTreeMap<LeaseId, &ResourceLease> =
        pack.leases.iter().map(|lease| (lease.id, lease)).collect();
    if lease_map.len() != pack.leases.len() {
        validation.diagnostics.push(diagnostic(
            ResourceStateDiagnosticCode::LeaseDoubleAcquire,
            ResourceStateDiagnosticProvenance::Lease {
                invariant: "lease ids are unique".to_owned(),
                lease_id: 0,
            },
            ValidationOrigin::ResourceStateValidation,
        ));
    }

    for requested in &pack.identity.requested_runtime_modes {
        if !pack.modes.iter().any(|entry| entry.mode == *requested) {
            validation.diagnostics.push(diagnostic(
                ResourceStateDiagnosticCode::ModeRequestedModeNotEmitted,
                ResourceStateDiagnosticProvenance::Mode { mode: *requested },
                ValidationOrigin::ResourceStateValidation,
            ));
        }
    }

    let mut computed_reachability_confirmed = !pack.modes.is_empty();
    for mode_schedule in &pack.modes {
        let mode = mode_schedule.mode;
        let ir = &mode_schedule.ir;
        let Some(epochs) = pack
            .epochs
            .iter()
            .find(|entry| entry.mode == mode)
            .map(|entry| entry.epochs.as_slice())
        else {
            validation.diagnostics.push(diagnostic(
                ResourceStateDiagnosticCode::SchedEpochCoverageGap,
                ResourceStateDiagnosticProvenance::Mode { mode },
                ValidationOrigin::ResourceStateValidation,
            ));
            computed_reachability_confirmed = false;
            continue;
        };
        computed_reachability_confirmed &=
            validate_mode_resource_state(ir, epochs, &lease_map, &mut validation);
    }
    validation
        .isr_visible_residency
        .computed_reachability_confirmed = computed_reachability_confirmed;

    if !pack.drift_monitor.observed.is_all_none() {
        validation.diagnostics.push(diagnostic(
            ResourceStateDiagnosticCode::DriftObservedNotAllNoneAtCompileTime,
            ResourceStateDiagnosticProvenance::Drift {
                invariant: "observed drift envelope must be empty at compile time".to_owned(),
                metric: "observed".to_owned(),
            },
            ValidationOrigin::ResourceStateValidation,
        ));
    }
    if pack.drift_monitor.consecutive_violations != 0 {
        validation.diagnostics.push(diagnostic(
            ResourceStateDiagnosticCode::DriftConsecutiveViolationsNonZeroAtCompileTime,
            ResourceStateDiagnosticProvenance::Drift {
                invariant: "consecutive violations must start at zero".to_owned(),
                metric: "consecutive_violations".to_owned(),
            },
            ValidationOrigin::ResourceStateValidation,
        ));
    }

    validation.finish();
    validation
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceStateValidation {
    pub diagnostics: Vec<ValidationDiagnostic>,
    pub lease_balance: LeaseBalanceSection,
    pub yield_safety: YieldSafetySection,
    pub isr_visible_residency: IsrVisibleResidencySection,
    pub overlay_bank_shadow: OverlayBankShadowSection,
}

impl ResourceStateValidation {
    #[must_use]
    pub fn new() -> Self {
        Self {
            diagnostics: Vec::new(),
            lease_balance: LeaseBalanceSection {
                leases: Vec::new(),
                all_balanced: true,
            },
            yield_safety: YieldSafetySection {
                yield_events: Vec::new(),
                all_yields_safe: true,
            },
            isr_visible_residency: IsrVisibleResidencySection {
                enabled_slices: Vec::new(),
                all_isr_safe: true,
                computed_reachability_confirmed: false,
            },
            overlay_bank_shadow: OverlayBankShadowSection {
                slices_checked: Vec::new(),
                all_consistent: true,
            },
        }
    }

    fn finish(&mut self) {
        self.lease_balance.leases.sort_by_key(|fact| fact.lease);
        self.lease_balance.leases.dedup_by_key(|fact| fact.lease);
        self.yield_safety
            .yield_events
            .sort_by_key(|fact| fact.slice);
        self.isr_visible_residency
            .enabled_slices
            .sort_by_key(|fact| fact.slice);
        self.overlay_bank_shadow
            .slices_checked
            .sort_by_key(|fact| fact.slice);
        self.lease_balance.all_balanced = self
            .lease_balance
            .leases
            .iter()
            .all(|fact| fact.all_paths_balanced);
        self.yield_safety.all_yields_safe = self
            .yield_safety
            .yield_events
            .iter()
            .all(|fact| fact.all_yield_safe);
        self.isr_visible_residency.all_isr_safe = self
            .isr_visible_residency
            .enabled_slices
            .iter()
            .all(|fact| fact.all_safe);
        self.overlay_bank_shadow.all_consistent = self
            .overlay_bank_shadow
            .slices_checked
            .iter()
            .all(|fact| fact.all_consistent);
    }

    #[must_use]
    pub fn certificate(
        &self,
        identity: SchedulePackInputIdentity,
        schedule_pack_self_hash: Hash256,
    ) -> ResourceStateCertificate {
        ResourceStateCertificate {
            pass_version: RESOURCE_STATE_CERT_PASS_VERSION.to_owned(),
            identity,
            schedule_pack_self_hash,
            lease_balance: self.lease_balance.clone(),
            yield_safety: self.yield_safety.clone(),
            isr_visible_residency: self.isr_visible_residency.clone(),
            overlay_bank_shadow: self.overlay_bank_shadow.clone(),
            diagnostics: self.diagnostics.clone(),
            resource_state_cert_self_hash: Hash256::ZERO,
        }
    }
}

impl Default for ResourceStateValidation {
    fn default() -> Self {
        Self::new()
    }
}

fn build_mode_schedule(
    mode: RuntimeMode,
    rom_window_plan: &RomWindowPlan,
    overlay_plan: &OverlayPlan,
    overlay_for_active_epoch: Option<OverlayId>,
    live_wram: &[ArenaSlotId],
    live_sram: &[ArenaSlotId],
    id_base: u32,
) -> (GbSchedIR, Vec<ResidencyEpoch>, Vec<ResourceLease>) {
    let source_epochs = if rom_window_plan.residency_epochs.is_empty() {
        Vec::new()
    } else {
        rom_window_plan.residency_epochs.clone()
    };
    let binding_map: BTreeMap<_, _> = rom_window_plan
        .rom_window_bindings
        .iter()
        .map(|binding| (binding.id, binding))
        .collect();
    let mut slices = Vec::new();
    let mut epochs = Vec::new();
    let mut leases = Vec::new();
    let mut next_lease = 0_u32;
    for (index, source_epoch) in source_epochs.iter().enumerate() {
        let slice_id = SliceId(id_base.saturating_add(index as u32));
        let successor =
            (index + 1 < source_epochs.len()).then(|| SliceId(id_base + index as u32 + 1));
        let binding = binding_map.get(&source_epoch.rom_window_binding);
        let visibility = binding
            .map(|binding| binding.visibility)
            .unwrap_or_else(RomVisibility::bank0_only);
        let entry_residency = entry_residency_from_visibility(visibility);
        let overlay = match source_epoch.overlay_state {
            OverlayState::NoOverlayActive => None,
            OverlayState::OverlayActive => overlay_for_active_epoch,
        };
        let mut ops = Vec::new();
        let mut required_leases = Vec::new();
        let mut resources = ResourceVector::default();

        if let Some(bank) = visibility.switchable {
            let lease_id = LeaseId(id_base.saturating_add(next_lease));
            next_lease += 1;
            leases.push(ResourceLease {
                id: lease_id,
                kind: ResourceLeaseKind::RomWindow {
                    binding: source_epoch.rom_window_binding,
                },
                acquired_in: slice_id,
                released_in: slice_id,
                yield_safe: false,
            });
            required_leases.push(lease_id);
            ops.push(SchedOp::AcquireLease { lease: lease_id });
            ops.push(SchedOp::BankSwitch {
                binding: source_epoch.rom_window_binding,
                bank,
            });
            resources.bank_switches = resources.bank_switches.saturating_add(1);
        }

        if let Some(binding) = source_epoch.sram_page_binding {
            let lease_id = LeaseId(id_base.saturating_add(next_lease));
            next_lease += 1;
            leases.push(ResourceLease {
                id: lease_id,
                kind: ResourceLeaseKind::SramPage { binding },
                acquired_in: slice_id,
                released_in: slice_id,
                yield_safe: false,
            });
            required_leases.push(lease_id);
            ops.push(SchedOp::AcquireLease { lease: lease_id });
            ops.push(SchedOp::SramPageSwitch { binding });
            resources.sram_page_switches = resources.sram_page_switches.saturating_add(1);
        }

        if let Some(overlay) = overlay {
            let lease_id = LeaseId(id_base.saturating_add(next_lease));
            next_lease += 1;
            leases.push(ResourceLease {
                id: lease_id,
                kind: ResourceLeaseKind::Overlay { overlay },
                acquired_in: slice_id,
                released_in: slice_id,
                yield_safe: true,
            });
            required_leases.push(lease_id);
            ops.push(SchedOp::AcquireLease { lease: lease_id });
            if let Some(install) = overlay_plan
                .installs
                .iter()
                .find(|install| install.region == overlay)
            {
                ops.push(SchedOp::OverlayInstall {
                    install: install.id,
                });
                resources.overlay_installs = resources.overlay_installs.saturating_add(1);
            }
        }

        for lease in required_leases.iter().rev() {
            ops.push(SchedOp::ReleaseLease { lease: *lease });
        }
        ops.push(if successor.is_some() {
            SchedOp::Yield {
                kind: yield_kind_from_hint(source_epoch.yield_kind),
            }
        } else {
            SchedOp::Halt
        });

        let interrupt_policy = if required_leases.iter().any(|lease| {
            leases
                .iter()
                .find(|candidate| candidate.id == *lease)
                .is_some_and(|lease| {
                    matches!(
                        lease.kind,
                        ResourceLeaseKind::RomWindow { .. }
                            | ResourceLeaseKind::SramPage { .. }
                            | ResourceLeaseKind::InterruptMask { .. }
                    )
                })
        }) {
            InterruptPolicy::ShortCriticalSection
        } else {
            InterruptPolicy::Enabled
        };
        let cadence = mode_cadence(mode);
        if mode == RuntimeMode::Trace {
            resources.trace_bytes = resources.trace_bytes.saturating_add(64);
        }
        slices.push(SchedSlice {
            id: slice_id,
            ops,
            hard_cycles_to_safe_point: cadence.hard_cycles_to_safe_point,
            soft_target_cycles: cadence.soft_target_cycles,
            max_interrupt_latency: cadence.max_interrupt_latency,
            resources,
            live_wram: live_wram.to_vec(),
            live_sram: live_sram.to_vec(),
            yield_kind: yield_kind_from_hint(source_epoch.yield_kind),
            yield_check: cadence.yield_check,
            entry_residency,
            interrupt_policy,
            required_leases,
            exit_kind: if successor.is_some() {
                ExitKind::SaveContinuationAndYield
            } else {
                ExitKind::Halt
            },
            semantic_checkpoint_pins: Vec::new(),
            trace_probe_pins: Vec::new(),
            successors: successor.into_iter().collect(),
        });
        epochs.push(ResidencyEpoch {
            id: SchedEpochId(id_base.saturating_add(index as u32)),
            rom_window: source_epoch.rom_window_binding,
            overlay,
            residency: entry_residency,
            slices: vec![slice_id],
        });
    }

    if slices.is_empty() {
        let cadence = mode_cadence(mode);
        slices.push(SchedSlice {
            id: SliceId(id_base),
            ops: vec![SchedOp::Halt],
            hard_cycles_to_safe_point: cadence.hard_cycles_to_safe_point,
            soft_target_cycles: cadence.soft_target_cycles,
            max_interrupt_latency: cadence.max_interrupt_latency,
            resources: ResourceVector::default(),
            live_wram: live_wram.to_vec(),
            live_sram: live_sram.to_vec(),
            yield_kind: YieldKind::Finished,
            yield_check: cadence.yield_check,
            entry_residency: EntryResidency::Bank0,
            interrupt_policy: InterruptPolicy::Enabled,
            required_leases: Vec::new(),
            exit_kind: ExitKind::Halt,
            semantic_checkpoint_pins: Vec::new(),
            trace_probe_pins: Vec::new(),
            successors: Vec::new(),
        });
        epochs.push(ResidencyEpoch {
            id: SchedEpochId(id_base),
            rom_window: RomWindowBindingId(0),
            overlay: None,
            residency: EntryResidency::Bank0,
            slices: vec![SliceId(id_base)],
        });
    }

    (
        GbSchedIR {
            mode,
            entry_slice: SliceId(id_base),
            slices,
        },
        epochs,
        leases,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ModeCadence {
    hard_cycles_to_safe_point: u32,
    soft_target_cycles: u32,
    max_interrupt_latency: u32,
    yield_check: YieldCheckClass,
}

fn mode_cadence(mode: RuntimeMode) -> ModeCadence {
    match mode {
        RuntimeMode::Interactive => ModeCadence {
            hard_cycles_to_safe_point: 17_556,
            soft_target_cycles: 8_778,
            max_interrupt_latency: 256,
            yield_check: YieldCheckClass::OnceAtEnd,
        },
        RuntimeMode::Trace => ModeCadence {
            hard_cycles_to_safe_point: 17_556,
            soft_target_cycles: 4_389,
            max_interrupt_latency: 128,
            yield_check: YieldCheckClass::EveryNTiles { n: 1 },
        },
        RuntimeMode::Steady => ModeCadence {
            hard_cycles_to_safe_point: 35_112,
            soft_target_cycles: 17_556,
            max_interrupt_latency: 512,
            yield_check: YieldCheckClass::OnceAtEnd,
        },
        RuntimeMode::Safe => ModeCadence {
            hard_cycles_to_safe_point: 17_556,
            soft_target_cycles: 4_389,
            max_interrupt_latency: 128,
            yield_check: YieldCheckClass::EveryLoadStore,
        },
    }
}

fn validate_mode_resource_state(
    ir: &GbSchedIR,
    epochs: &[ResidencyEpoch],
    lease_map: &BTreeMap<LeaseId, &ResourceLease>,
    validation: &mut ResourceStateValidation,
) -> bool {
    let slice_map: BTreeMap<SliceId, &SchedSlice> =
        ir.slices.iter().map(|slice| (slice.id, slice)).collect();
    let mut slice_to_epoch = BTreeMap::new();
    for epoch in epochs {
        for slice in &epoch.slices {
            if slice_to_epoch.insert(*slice, epoch).is_some() {
                validation.diagnostics.push(diagnostic(
                    ResourceStateDiagnosticCode::SchedEpochCoverageGap,
                    ResourceStateDiagnosticProvenance::Slice {
                        invariant: "slice is covered by more than one residency epoch".to_owned(),
                        slice_id: slice.0,
                    },
                    ValidationOrigin::ResourceStateValidation,
                ));
            }
        }
    }
    let flow = analyze_slice_graph(ir, &slice_map, lease_map, &mut validation.diagnostics);
    let mut all_slices_reachable = !ir.slices.is_empty() && slice_map.contains_key(&ir.entry_slice);

    for lease in lease_map.values().filter(|lease| {
        slice_map.contains_key(&lease.acquired_in) || slice_map.contains_key(&lease.released_in)
    }) {
        let acquired_seen = flow
            .values()
            .any(|fact| fact.reachable && fact.acquired.contains(&lease.id));
        let released_seen = flow
            .values()
            .any(|fact| fact.reachable && fact.released.contains(&lease.id));
        let terminal_balanced = flow
            .values()
            .filter(|fact| fact.reachable && fact.terminal)
            .all(|fact| !fact.exit_active.contains(&lease.id));
        validation.lease_balance.leases.push(LeaseBalanceFact {
            lease: lease.id,
            kind_discriminant: lease_kind_discriminant(&lease.kind),
            acquired_in: lease.acquired_in,
            released_in: lease.released_in,
            yield_safe: lease.yield_safe,
            paths_checked: flow.values().filter(|fact| fact.reachable).count() as u32,
            all_paths_balanced: acquired_seen && released_seen && terminal_balanced,
        });
    }

    for slice in &ir.slices {
        let Some(epoch) = slice_to_epoch.get(&slice.id) else {
            validation.diagnostics.push(diagnostic(
                ResourceStateDiagnosticCode::SchedEpochCoverageGap,
                ResourceStateDiagnosticProvenance::Slice {
                    invariant: "slice is not covered by any residency epoch".to_owned(),
                    slice_id: slice.id.0,
                },
                ValidationOrigin::ResourceStateValidation,
            ));
            continue;
        };
        let Some(slice_flow) = flow.get(&slice.id) else {
            validation.diagnostics.push(diagnostic(
                ResourceStateDiagnosticCode::SchedEpochCoverageGap,
                ResourceStateDiagnosticProvenance::Slice {
                    invariant: "slice is unreachable from mode entry".to_owned(),
                    slice_id: slice.id.0,
                },
                ValidationOrigin::ResourceStateValidation,
            ));
            continue;
        };
        if !slice_flow.reachable {
            all_slices_reachable = false;
            validation.diagnostics.push(diagnostic(
                ResourceStateDiagnosticCode::SchedEpochCoverageGap,
                ResourceStateDiagnosticProvenance::Slice {
                    invariant: "slice is unreachable from mode entry".to_owned(),
                    slice_id: slice.id.0,
                },
                ValidationOrigin::ResourceStateValidation,
            ));
            continue;
        }
        if slice.entry_residency != epoch.residency {
            validation.diagnostics.push(diagnostic(
                ResourceStateDiagnosticCode::SchedEntryResidencyEpochMismatch,
                ResourceStateDiagnosticProvenance::Epoch {
                    invariant: "slice entry residency must match epoch residency".to_owned(),
                    epoch_id: epoch.id.0,
                },
                ValidationOrigin::ResourceStateValidation,
            ));
        }
        for lease in &slice.required_leases {
            if !lease_map.contains_key(lease) {
                validation.diagnostics.push(diagnostic(
                    ResourceStateDiagnosticCode::LeaseRequiredLeaseNotAcquired,
                    ResourceStateDiagnosticProvenance::Lease {
                        invariant: "required lease is absent from pack lease table".to_owned(),
                        lease_id: lease.0,
                    },
                    ValidationOrigin::ResourceStateValidation,
                ));
            }
        }
        append_yield_safety_fact(slice, slice_flow, lease_map, validation);
        append_isr_visible_residency_fact(slice, slice_flow, lease_map, validation);
        append_overlay_bank_shadow_fact(slice, epoch, slice_flow, lease_map, validation);
    }
    for slice in &ir.slices {
        for successor in &slice.successors {
            if !slice_map.contains_key(successor) {
                validation.diagnostics.push(diagnostic(
                    ResourceStateDiagnosticCode::SchedEpochCoverageGap,
                    ResourceStateDiagnosticProvenance::Slice {
                        invariant: "successor slice is missing".to_owned(),
                        slice_id: successor.0,
                    },
                    ValidationOrigin::ResourceStateValidation,
                ));
            }
        }
    }
    all_slices_reachable && flow.values().all(|fact| fact.reachable)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SliceFlowFact {
    before_ops: Vec<BTreeSet<LeaseId>>,
    acquired: BTreeSet<LeaseId>,
    released: BTreeSet<LeaseId>,
    exit_active: BTreeSet<LeaseId>,
    terminal: bool,
    reachable: bool,
}

fn analyze_slice_graph(
    ir: &GbSchedIR,
    slice_map: &BTreeMap<SliceId, &SchedSlice>,
    lease_map: &BTreeMap<LeaseId, &ResourceLease>,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) -> BTreeMap<SliceId, SliceFlowFact> {
    let mut flow = BTreeMap::new();
    let mut entry_states = BTreeMap::from([(ir.entry_slice, BTreeSet::new())]);
    let mut pending = BTreeSet::from([ir.entry_slice]);

    while let Some(slice_id) = pending.pop_first() {
        if flow.contains_key(&slice_id) {
            continue;
        }
        let Some(slice) = slice_map.get(&slice_id) else {
            diagnostics.push(diagnostic(
                ResourceStateDiagnosticCode::SchedEpochCoverageGap,
                ResourceStateDiagnosticProvenance::Slice {
                    invariant: "reachable slice is missing".to_owned(),
                    slice_id: slice_id.0,
                },
                ValidationOrigin::ResourceStateValidation,
            ));
            continue;
        };
        let entry_active = entry_states
            .get(&slice_id)
            .cloned()
            .unwrap_or_else(BTreeSet::new);
        let slice_flow = simulate_slice_flow(slice, entry_active, lease_map, diagnostics);
        let mut successors = slice.successors.clone();
        successors.sort();
        successors.dedup();
        for successor in successors {
            if !slice_map.contains_key(&successor) {
                diagnostics.push(diagnostic(
                    ResourceStateDiagnosticCode::SchedEpochCoverageGap,
                    ResourceStateDiagnosticProvenance::Slice {
                        invariant: "successor slice is missing".to_owned(),
                        slice_id: successor.0,
                    },
                    ValidationOrigin::ResourceStateValidation,
                ));
                continue;
            }
            match entry_states.get(&successor) {
                Some(existing) if existing != &slice_flow.exit_active => {
                    diagnostics.push(diagnostic(
                        ResourceStateDiagnosticCode::LeaseUnbalanced,
                        ResourceStateDiagnosticProvenance::Slice {
                            invariant: "successor has divergent incoming lease state".to_owned(),
                            slice_id: successor.0,
                        },
                        ValidationOrigin::ResourceStateValidation,
                    ));
                }
                Some(_) => {}
                None => {
                    entry_states.insert(successor, slice_flow.exit_active.clone());
                    pending.insert(successor);
                }
            }
        }
        flow.insert(slice_id, slice_flow);
    }

    for slice in &ir.slices {
        flow.entry(slice.id).or_insert_with(|| SliceFlowFact {
            before_ops: vec![BTreeSet::new(); slice.ops.len()],
            acquired: BTreeSet::new(),
            released: BTreeSet::new(),
            exit_active: BTreeSet::new(),
            terminal: matches!(slice.exit_kind, ExitKind::Halt | ExitKind::Fault),
            reachable: false,
        });
    }

    flow
}

fn simulate_slice_flow(
    slice: &SchedSlice,
    entry_active: BTreeSet<LeaseId>,
    lease_map: &BTreeMap<LeaseId, &ResourceLease>,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) -> SliceFlowFact {
    let mut active = entry_active;
    let mut before_ops = Vec::with_capacity(slice.ops.len());
    let mut acquired = BTreeSet::new();
    let mut released = BTreeSet::new();
    for op in &slice.ops {
        before_ops.push(active.clone());
        match op {
            SchedOp::AcquireLease { lease } => {
                if !lease_map.contains_key(lease) {
                    diagnostics.push(diagnostic(
                        ResourceStateDiagnosticCode::LeaseRequiredLeaseNotAcquired,
                        ResourceStateDiagnosticProvenance::Lease {
                            invariant: "acquired lease is absent from pack lease table".to_owned(),
                            lease_id: lease.0,
                        },
                        ValidationOrigin::ResourceStateValidation,
                    ));
                }
                if !active.insert(*lease) || !acquired.insert(*lease) {
                    diagnostics.push(diagnostic(
                        ResourceStateDiagnosticCode::LeaseDoubleAcquire,
                        ResourceStateDiagnosticProvenance::Lease {
                            invariant: "lease acquired more than once in slice".to_owned(),
                            lease_id: lease.0,
                        },
                        ValidationOrigin::ResourceStateValidation,
                    ));
                }
                if let Some(resource_lease) = lease_map.get(lease)
                    && resource_lease.acquired_in != slice.id
                {
                    diagnostics.push(diagnostic(
                        ResourceStateDiagnosticCode::LeaseUnbalanced,
                        ResourceStateDiagnosticProvenance::Lease {
                            invariant: "lease acquired outside declared slice".to_owned(),
                            lease_id: lease.0,
                        },
                        ValidationOrigin::ResourceStateValidation,
                    ));
                }
            }
            SchedOp::ReleaseLease { lease } => {
                if !active.remove(lease) {
                    diagnostics.push(diagnostic(
                        ResourceStateDiagnosticCode::LeaseReleaseWithoutAcquire,
                        ResourceStateDiagnosticProvenance::Lease {
                            invariant: "lease released without active acquire".to_owned(),
                            lease_id: lease.0,
                        },
                        ValidationOrigin::ResourceStateValidation,
                    ));
                }
                released.insert(*lease);
                if let Some(resource_lease) = lease_map.get(lease)
                    && resource_lease.released_in != slice.id
                {
                    diagnostics.push(diagnostic(
                        ResourceStateDiagnosticCode::LeaseUnbalanced,
                        ResourceStateDiagnosticProvenance::Lease {
                            invariant: "lease released outside declared slice".to_owned(),
                            lease_id: lease.0,
                        },
                        ValidationOrigin::ResourceStateValidation,
                    ));
                }
            }
            SchedOp::Yield { .. } => {
                for active_lease in &active {
                    if lease_map
                        .get(active_lease)
                        .is_some_and(|lease| !lease.yield_safe)
                    {
                        diagnostics.push(diagnostic(
                            ResourceStateDiagnosticCode::LeaseYieldCrossesNonResumable,
                            ResourceStateDiagnosticProvenance::Lease {
                                invariant: "yield crosses non-resumable lease".to_owned(),
                                lease_id: active_lease.0,
                            },
                            ValidationOrigin::ResourceStateValidation,
                        ));
                    }
                }
            }
            _ => {}
        }
    }
    for lease in &slice.required_leases {
        if !lease_map.contains_key(lease) {
            diagnostics.push(diagnostic(
                ResourceStateDiagnosticCode::LeaseRequiredLeaseNotAcquired,
                ResourceStateDiagnosticProvenance::Lease {
                    invariant: "required lease is absent from pack lease table".to_owned(),
                    lease_id: lease.0,
                },
                ValidationOrigin::ResourceStateValidation,
            ));
        }
    }
    if matches!(slice.exit_kind, ExitKind::Halt | ExitKind::Fault) {
        for lease in &active {
            diagnostics.push(diagnostic(
                ResourceStateDiagnosticCode::LeaseUnbalanced,
                ResourceStateDiagnosticProvenance::Lease {
                    invariant: "terminal slice exits with active lease".to_owned(),
                    lease_id: lease.0,
                },
                ValidationOrigin::ResourceStateValidation,
            ));
        }
    }
    SliceFlowFact {
        before_ops,
        acquired,
        released,
        exit_active: active,
        terminal: matches!(slice.exit_kind, ExitKind::Halt | ExitKind::Fault),
        reachable: true,
    }
}

fn append_yield_safety_fact(
    slice: &SchedSlice,
    flow: &SliceFlowFact,
    lease_map: &BTreeMap<LeaseId, &ResourceLease>,
    validation: &mut ResourceStateValidation,
) {
    if slice.exit_kind != ExitKind::SaveContinuationAndYield {
        return;
    }
    let Some((index, yield_kind)) =
        slice
            .ops
            .iter()
            .enumerate()
            .rev()
            .find_map(|(index, op)| match op {
                SchedOp::Yield { kind } => Some((index, *kind)),
                _ => None,
            })
    else {
        validation.diagnostics.push(diagnostic(
            ResourceStateDiagnosticCode::LeaseYieldCrossesNonResumable,
            ResourceStateDiagnosticProvenance::Slice {
                invariant: "yielding slice has no terminal yield op".to_owned(),
                slice_id: slice.id.0,
            },
            ValidationOrigin::ResourceStateValidation,
        ));
        return;
    };
    let outstanding = flow.before_ops.get(index).cloned().unwrap_or_default();
    let all_yield_safe = outstanding
        .iter()
        .all(|lease| lease_map.get(lease).is_some_and(|lease| lease.yield_safe));
    for lease in &outstanding {
        if lease_map.get(lease).is_some_and(|lease| !lease.yield_safe) {
            validation.diagnostics.push(diagnostic(
                ResourceStateDiagnosticCode::LeaseYieldCrossesNonResumable,
                ResourceStateDiagnosticProvenance::Lease {
                    invariant: "yield crosses non-resumable lease".to_owned(),
                    lease_id: lease.0,
                },
                ValidationOrigin::ResourceStateValidation,
            ));
        }
    }
    validation.yield_safety.yield_events.push(YieldSafetyFact {
        slice: slice.id,
        yield_kind,
        outstanding_leases: outstanding.into_iter().collect(),
        all_yield_safe,
    });
}

fn append_isr_visible_residency_fact(
    slice: &SchedSlice,
    flow: &SliceFlowFact,
    lease_map: &BTreeMap<LeaseId, &ResourceLease>,
    validation: &mut ResourceStateValidation,
) {
    if slice.interrupt_policy != InterruptPolicy::Enabled {
        return;
    }
    let mut outstanding = BTreeSet::new();
    for active in &flow.before_ops {
        outstanding.extend(active.iter().copied());
    }
    let mut all_safe = !matches!(slice.entry_residency, EntryResidency::Expert { .. });
    if !all_safe {
        validation.diagnostics.push(diagnostic(
            ResourceStateDiagnosticCode::ResIsrEnabledInExpertBank,
            ResourceStateDiagnosticProvenance::Slice {
                invariant: "interrupt-enabled slices must not enter expert residency".to_owned(),
                slice_id: slice.id.0,
            },
            ValidationOrigin::ResourceStateValidation,
        ));
    }
    for lease in &outstanding {
        let code = match lease_map.get(lease).map(|lease| &lease.kind) {
            Some(ResourceLeaseKind::RomWindow { .. }) => {
                Some(ResourceStateDiagnosticCode::ResIsrEnabledHoldsRomWindowLease)
            }
            Some(ResourceLeaseKind::SramPage { .. }) => {
                Some(ResourceStateDiagnosticCode::ResIsrEnabledHoldsSramPageLease)
            }
            _ => None,
        };
        if let Some(code) = code {
            all_safe = false;
            validation.diagnostics.push(diagnostic(
                code,
                ResourceStateDiagnosticProvenance::Lease {
                    invariant: "interrupt-enabled slice holds switchable lease".to_owned(),
                    lease_id: lease.0,
                },
                ValidationOrigin::ResourceStateValidation,
            ));
        }
    }
    validation
        .isr_visible_residency
        .enabled_slices
        .push(IsrVisibleResidencyFact {
            slice: slice.id,
            residency: slice.entry_residency,
            outstanding_leases: outstanding.into_iter().collect(),
            all_safe,
        });
}

fn append_overlay_bank_shadow_fact(
    slice: &SchedSlice,
    epoch: &ResidencyEpoch,
    flow: &SliceFlowFact,
    lease_map: &BTreeMap<LeaseId, &ResourceLease>,
    validation: &mut ResourceStateValidation,
) {
    let entry_residency_matches = slice.entry_residency == epoch.residency;
    let mut overlay_installs_aligned = true;
    let mut overlay_lease_shape_satisfied = true;
    let mut bank_switches_bracketed = true;
    for (index, op) in slice.ops.iter().enumerate() {
        match op {
            SchedOp::OverlayInstall { install } => {
                let active = flow.before_ops.get(index).cloned().unwrap_or_default();
                let active_overlay_matches_epoch = epoch.overlay.is_some_and(|epoch_overlay| {
                    active.iter().any(|lease| {
                        lease_map.get(lease).is_some_and(|lease| {
                            matches!(
                                lease.kind,
                                ResourceLeaseKind::Overlay { overlay } if overlay == epoch_overlay
                            )
                        })
                    })
                });
                if epoch.overlay.is_none() {
                    overlay_installs_aligned = false;
                    validation.diagnostics.push(diagnostic(
                        ResourceStateDiagnosticCode::SchedOverlayInstallEpochMismatch,
                        ResourceStateDiagnosticProvenance::Epoch {
                            invariant: format!(
                                "overlay install {} is not aligned with active epoch lease",
                                install.0
                            ),
                            epoch_id: epoch.id.0,
                        },
                        ValidationOrigin::ResourceStateValidation,
                    ));
                }
                let active_overlay_lease_satisfies_shape = active.iter().any(|lease| {
                    lease_map.get(lease).is_some_and(|lease| {
                        matches!(
                            lease.kind,
                            ResourceLeaseKind::Overlay { overlay }
                                if Some(overlay) == epoch.overlay
                        )
                    })
                });
                if !active_overlay_matches_epoch || !active_overlay_lease_satisfies_shape {
                    overlay_lease_shape_satisfied = false;
                    validation.diagnostics.push(diagnostic(
                        ResourceStateDiagnosticCode::SchedOverlayInstallEpochMismatch,
                        ResourceStateDiagnosticProvenance::Epoch {
                            invariant: format!(
                                "overlay install {} lease shape is not satisfied by active leases",
                                install.0
                            ),
                            epoch_id: epoch.id.0,
                        },
                        ValidationOrigin::ResourceStateValidation,
                    ));
                }
            }
            SchedOp::BankSwitch { .. } => {
                let active = flow.before_ops.get(index).cloned().unwrap_or_default();
                let bracketed = active.iter().any(|lease| {
                    lease_map.get(lease).is_some_and(|lease| {
                        matches!(lease.kind, ResourceLeaseKind::RomWindow { .. })
                    })
                });
                if !bracketed {
                    bank_switches_bracketed = false;
                    validation.diagnostics.push(diagnostic(
                        ResourceStateDiagnosticCode::ResBankSwitchUnbracketed,
                        ResourceStateDiagnosticProvenance::Slice {
                            invariant: "bank switch must be bracketed by rom-window lease"
                                .to_owned(),
                            slice_id: slice.id.0,
                        },
                        ValidationOrigin::ResourceStateValidation,
                    ));
                }
            }
            _ => {}
        }
    }
    let all_consistent = entry_residency_matches
        && overlay_installs_aligned
        && overlay_lease_shape_satisfied
        && bank_switches_bracketed;
    validation
        .overlay_bank_shadow
        .slices_checked
        .push(OverlayBankShadowConsistencyFact {
            slice: slice.id,
            epoch_id: epoch.id,
            entry_residency_matches,
            overlay_installs_aligned,
            overlay_lease_shape_satisfied,
            bank_switches_bracketed,
            all_consistent,
        });
}

fn lease_kind_discriminant(kind: &ResourceLeaseKind) -> ResourceLeaseKindDiscriminant {
    match kind {
        ResourceLeaseKind::RomWindow { .. } => ResourceLeaseKindDiscriminant::RomWindow,
        ResourceLeaseKind::SramPage { .. } => ResourceLeaseKindDiscriminant::SramPage,
        ResourceLeaseKind::Overlay { .. } => ResourceLeaseKindDiscriminant::Overlay,
        ResourceLeaseKind::InterruptMask { .. } => ResourceLeaseKindDiscriminant::InterruptMask,
    }
}

fn input_hash_mismatch_diagnostics(input: &SchedulePackInputs) -> Vec<ValidationDiagnostic> {
    let recorded = input.input_identity.hashes();
    let computed = input.expected_input_hashes;
    let pairs = [
        (
            "infer_ir_self_hash",
            recorded.infer_ir_self_hash,
            computed.infer_ir_self_hash,
        ),
        (
            "observation_plan_self_hash",
            recorded.observation_plan_self_hash,
            computed.observation_plan_self_hash,
        ),
        (
            "range_plan_self_hash",
            recorded.range_plan_self_hash,
            computed.range_plan_self_hash,
        ),
        (
            "storage_plan_self_hash",
            recorded.storage_plan_self_hash,
            computed.storage_plan_self_hash,
        ),
        (
            "sram_page_plan_self_hash",
            recorded.sram_page_plan_self_hash,
            computed.sram_page_plan_self_hash,
        ),
        (
            "rom_window_plan_self_hash",
            recorded.rom_window_plan_self_hash,
            computed.rom_window_plan_self_hash,
        ),
        (
            "overlay_plan_self_hash",
            recorded.overlay_plan_self_hash,
            computed.overlay_plan_self_hash,
        ),
        (
            "arena_plan_self_hash",
            recorded.arena_plan_self_hash,
            computed.arena_plan_self_hash,
        ),
        (
            "policy_resolution_self_hash",
            recorded.policy_resolution_self_hash,
            computed.policy_resolution_self_hash,
        ),
        (
            "runtime_chrome_budget_self_hash",
            recorded.runtime_chrome_budget_self_hash,
            computed.runtime_chrome_budget_self_hash,
        ),
        (
            "feature_set_hash",
            recorded.feature_set_hash,
            computed.feature_set_hash,
        ),
    ];
    pairs
        .into_iter()
        .filter(|(_, recorded, computed)| recorded != computed)
        .map(|(product, recorded, computed)| {
            diagnostic(
                ResourceStateDiagnosticCode::SchedInputHashMismatch,
                ResourceStateDiagnosticProvenance::HashMismatch {
                    product: product.to_owned(),
                    recorded,
                    computed,
                },
                ValidationOrigin::SchedIrConstruction,
            )
        })
        .collect()
}

fn live_slots(plan: &ArenaPlan, backing: ArenaBacking) -> Vec<ArenaSlotId> {
    let arenas = match backing {
        ArenaBacking::Wram => &plan.wram_arenas,
        ArenaBacking::Sram => &plan.sram_arenas,
        ArenaBacking::Hram => &plan.hram_assignments,
    };
    let mut slots: Vec<_> = arenas
        .iter()
        .flat_map(|arena| arena.slots.iter().map(|slot| slot.id))
        .collect();
    slots.sort();
    slots.dedup();
    slots
}

fn entry_residency_from_visibility(visibility: RomVisibility) -> EntryResidency {
    match visibility.switchable {
        Some(bank) => EntryResidency::Common { bank },
        None => EntryResidency::Bank0,
    }
}

fn yield_kind_from_hint(hint: YieldKindHint) -> YieldKind {
    match hint {
        YieldKindHint::NoYieldsExpected => YieldKind::Micro,
        YieldKindHint::YieldsAtCommitBoundaries => YieldKind::Frame,
        YieldKindHint::YieldsAtTokenBoundary => YieldKind::TokenReady,
    }
}

fn continuation_abi_hash(
    identity: &SchedulePackInputIdentity,
) -> Result<Hash256, CanonicalJsonError> {
    #[derive(Serialize)]
    struct ContinuationAbiHashInput {
        schema_version: SemVer,
        target_profile_id: TargetProfileId,
        arena_plan_hash: Hash256,
        feature_set_hash: Hash256,
    }
    DomainHash::new(
        "gbf-codegen",
        "ContinuationAbi",
        SCHED_IR_SCHEMA_ID,
        "1.0.0",
    )
    .hash(&ContinuationAbiHashInput {
        schema_version: identity.schema_version,
        target_profile_id: identity.target_profile_id.clone(),
        arena_plan_hash: identity.arena_plan_self_hash,
        feature_set_hash: identity.feature_set_hash,
    })
}

fn slice_report_summary(pack: &SchedulePack) -> SliceReportSummary {
    let mut summary = SliceReportSummary {
        mode_count: pack.modes.len() as u16,
        slice_count: 0,
        lease_count: pack.leases.len() as u32,
        epoch_count: pack
            .epochs
            .iter()
            .map(|entry| entry.epochs.len())
            .sum::<usize>() as u32,
        max_interrupt_latency: 0,
        total_bank_switches: 0,
        total_sram_page_switches: 0,
        total_overlay_installs: 0,
    };
    for mode_schedule in &pack.modes {
        let ir = &mode_schedule.ir;
        summary.slice_count += ir.slices.len() as u32;
        for slice in &ir.slices {
            summary.max_interrupt_latency = summary
                .max_interrupt_latency
                .max(slice.max_interrupt_latency);
            summary.total_bank_switches += u32::from(slice.resources.bank_switches);
            summary.total_sram_page_switches += u32::from(slice.resources.sram_page_switches);
            summary.total_overlay_installs += u32::from(slice.resources.overlay_installs);
        }
    }
    summary
}

fn validate_schedule_report_body(
    has_result: bool,
    diagnostics: &[ValidationDiagnostic],
    outcome: ReportOutcome,
) -> Result<(), Vec<ValidationDiagnostic>> {
    let mut errors = Vec::new();
    match outcome {
        ReportOutcome::Passed => {
            if !has_result || !diagnostics.is_empty() {
                errors.push(report_semantic_diagnostic("outcome"));
            }
        }
        ReportOutcome::Failed => {
            if diagnostics.is_empty() {
                errors.push(report_semantic_diagnostic("diagnostics"));
            }
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn report_semantic_diagnostic(field: &str) -> ValidationDiagnostic {
    ValidationDiagnostic::hard(
        ValidationOrigin::Schema,
        ValidationCode::ReportSemanticInvariantViolated {
            field: gbf_foundation::FieldPath::from(field),
        },
        ValidationDetail::Field {
            field: gbf_foundation::FieldPath::from(field),
        },
        Vec::new(),
    )
}

fn diagnostic(
    code: ResourceStateDiagnosticCode,
    provenance: ResourceStateDiagnosticProvenance,
    origin: ValidationOrigin,
) -> ValidationDiagnostic {
    ValidationDiagnostic::hard(
        origin,
        ValidationCode::ResourceState { code, provenance },
        ValidationDetail::None,
        vec![EvidenceRef {
            kind: "ResourceStateValidation".to_owned(),
            reference: format!("diagnostic://resource_state/{}", code.as_str()),
            hash: None,
        }],
    )
}

fn failed_output(
    input_identity: SchedulePackInputIdentity,
    diagnostics: Vec<ValidationDiagnostic>,
) -> SchedulePackOutput {
    SchedulePackOutput {
        input_identity,
        outcome: SchedulePackOutcome::Failed,
        result: None,
        cert: None,
        diagnostics,
    }
}

pub fn emit_schedule_pack_report(
    output: &SchedulePackOutput,
) -> Result<SchedulePackReportEnvelope, SchedulePackEmitError> {
    let outcome = match output.outcome {
        SchedulePackOutcome::Succeeded => ReportOutcome::Passed,
        SchedulePackOutcome::Failed => ReportOutcome::Failed,
    };
    let body = SchedulePackReportBody {
        pass_version: SCHED_IR_PASS_VERSION.to_owned(),
        input_identity: output.input_identity.clone(),
        diagnostics: output.diagnostics.clone(),
        result: output.result.clone(),
    };
    let envelope = ReportEnvelope::new(outcome, body)?.with_computed_self_hash()?;
    round_trip_self_hash(&envelope)?;
    Ok(envelope)
}

pub fn emit_schedule_pack_json_bytes(
    output: &SchedulePackOutput,
) -> Result<Vec<u8>, SchedulePackEmitError> {
    Ok(canonicalize(&emit_schedule_pack_report(output)?)?)
}

pub fn emit_slice_report(
    output: &SchedulePackOutput,
) -> Result<SliceReportEnvelope, SchedulePackEmitError> {
    let result = output
        .result
        .as_ref()
        .ok_or(SchedulePackEmitError::MissingResult)?;
    let body = SliceReportBody {
        pass_version: SCHED_IR_PASS_VERSION.to_owned(),
        schedule_pack_self_hash: result.schedule_pack_self_hash,
        summary: result.summary,
        diagnostics: output.diagnostics.clone(),
    };
    let outcome = if output.diagnostics.is_empty() {
        ReportOutcome::Passed
    } else {
        ReportOutcome::Failed
    };
    let envelope = ReportEnvelope::new(outcome, body)?.with_computed_self_hash()?;
    round_trip_self_hash(&envelope)?;
    Ok(envelope)
}

pub fn emit_slice_report_json_bytes(
    output: &SchedulePackOutput,
) -> Result<Vec<u8>, SchedulePackEmitError> {
    Ok(canonicalize(&emit_slice_report(output)?)?)
}

pub fn emit_resource_state_cert(
    output: &SchedulePackOutput,
) -> Result<ResourceStateCertEnvelope, SchedulePackEmitError> {
    let cert = output
        .cert
        .clone()
        .ok_or(SchedulePackEmitError::MissingCert)?;
    let outcome = if cert.diagnostics.is_empty() {
        ReportOutcome::Passed
    } else {
        ReportOutcome::Failed
    };
    let envelope = ReportEnvelope::new(outcome, cert)?.with_computed_self_hash()?;
    round_trip_self_hash(&envelope)?;
    Ok(envelope)
}

pub fn emit_resource_state_cert_json_bytes(
    output: &SchedulePackOutput,
) -> Result<Vec<u8>, SchedulePackEmitError> {
    Ok(canonicalize(&emit_resource_state_cert(output)?)?)
}

pub fn schedule_pack_self_hash(pack: &SchedulePack) -> Result<Hash256, CanonicalJsonError> {
    self_hash_omitting_fields(
        DomainHash::new("gbf-codegen", "SchedulePack", SCHED_IR_SCHEMA_ID, "1.0.0"),
        pack,
        "schedule_pack_self_hash",
        &[],
    )
}

pub fn resource_state_cert_self_hash(
    cert: &ResourceStateCertificate,
) -> Result<Hash256, CanonicalJsonError> {
    self_hash_omitting_fields(
        DomainHash::new(
            "gbf-codegen",
            "ResourceStateCertificate",
            RESOURCE_STATE_CERT_SCHEMA_ID,
            "1.0.0",
        ),
        cert,
        "resource_state_cert_self_hash",
        &["schedule_pack_self_hash"],
    )
}

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SchedulePackCacheKey(pub Hash256);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SchedulePackCacheKeyInputs {
    pub input_hashes: SchedulePackInputHashes,
    pub requested_runtime_modes: BTreeSet<RuntimeMode>,
    pub pass_version: String,
}

impl SchedulePackCacheKeyInputs {
    #[must_use]
    pub fn from_input_identity(identity: &SchedulePackInputIdentity) -> Self {
        Self {
            input_hashes: identity.hashes(),
            requested_runtime_modes: identity.requested_runtime_modes.clone(),
            pass_version: SCHED_IR_PASS_VERSION.to_owned(),
        }
    }

    pub fn cache_key(&self) -> Result<SchedulePackCacheKey, CanonicalJsonError> {
        schedule_pack_cache_key(self)
    }
}

pub fn schedule_pack_cache_key(
    inputs: &SchedulePackCacheKeyInputs,
) -> Result<SchedulePackCacheKey, CanonicalJsonError> {
    let canonical = canonical_json_bytes_omitting_fields(inputs, &[])?;
    DomainHash::new("gbf-codegen", "StageCacheKey", "sched_ir", "v1")
        .hash_canonical_bytes(&canonical)
        .map(SchedulePackCacheKey)
}

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ResourceStateCacheKey(pub Hash256);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceStateCacheKeyInputs {
    pub schedule_pack_self_hash: Hash256,
    pub feature_set_hash: Hash256,
    pub pass_version: &'static str,
}

impl ResourceStateCacheKeyInputs {
    #[must_use]
    pub const fn from_schedule_pack(pack: &SchedulePack) -> Self {
        Self {
            schedule_pack_self_hash: pack.schedule_pack_self_hash,
            feature_set_hash: pack.identity.feature_set_hash,
            pass_version: RESOURCE_STATE_CERT_PASS_VERSION,
        }
    }

    pub fn cache_key(&self) -> Result<ResourceStateCacheKey, CanonicalJsonError> {
        resource_state_cache_key(self)
    }
}

pub fn resource_state_cache_key(
    inputs: &ResourceStateCacheKeyInputs,
) -> Result<ResourceStateCacheKey, CanonicalJsonError> {
    let canonical = canonical_json_bytes_omitting_fields(inputs, &[])?;
    DomainHash::new("gbf-codegen", "StageCacheKey", "resource_state.cert", "v1")
        .hash_canonical_bytes(&canonical)
        .map(ResourceStateCacheKey)
}

pub fn run_schedule_pack_with_cache(
    cache: &StoreStageCache<'_>,
    input: &SchedulePackInputs,
    expected_hashes: StoreBackedStageExpectedHashes,
) -> Result<StoreBackedStageRunOutput<SchedulePackResult>, CodegenStageCacheError> {
    let cache_key = SchedulePackCacheKeyInputs::from_input_identity(&input.input_identity)
        .cache_key()
        .map_err(|error| CodegenStageCacheError::StageCacheKey {
            stage_id: "10",
            message: error.to_string(),
        })?;
    let keys = StoreBackedStageCacheKeys::new(
        "10",
        stage10_schedule_pack_store_key(cache_key, StoreBackedStageCellKind::Success),
        stage10_schedule_pack_store_key(cache_key, StoreBackedStageCellKind::FailureMemo),
    );
    run_store_backed_stage_with_cache(cache, &keys, cache_key.0, expected_hashes, || {
        let output = build_schedule_pack(input);
        let report = emit_schedule_pack_report(&output).map_err(|error| {
            CodegenStageCacheError::StageEmit {
                stage_id: "10",
                message: error.to_string(),
            }
        })?;
        let report_self_hash = report.report_self_hash;
        match output.outcome {
            SchedulePackOutcome::Succeeded => {
                let product =
                    output
                        .result
                        .ok_or_else(|| CodegenStageCacheError::StageOutputInvariant {
                            stage_id: "10",
                            message: "succeeded output is missing SchedulePackResult".to_owned(),
                        })?;
                Ok(StoreBackedStageRunResult::Success {
                    product_self_hash: product.schedule_pack_self_hash,
                    product,
                    report_self_hash,
                })
            }
            SchedulePackOutcome::Failed => Ok(StoreBackedStageRunResult::FailureMemo {
                diagnostics: output.diagnostics,
                report_self_hash,
            }),
        }
    })
}

pub fn run_resource_state_validation_with_cache(
    cache: &StoreStageCache<'_>,
    pack: &SchedulePack,
    expected_hashes: StoreBackedStageExpectedHashes,
) -> Result<StoreBackedStageRunOutput<ResourceStateCertificate>, CodegenStageCacheError> {
    let cache_key = ResourceStateCacheKeyInputs::from_schedule_pack(pack)
        .cache_key()
        .map_err(|error| CodegenStageCacheError::StageCacheKey {
            stage_id: "10.5",
            message: error.to_string(),
        })?;
    let keys = StoreBackedStageCacheKeys::new(
        "10.5",
        stage105_resource_state_store_key(cache_key, StoreBackedStageCellKind::Success),
        stage105_resource_state_store_key(cache_key, StoreBackedStageCellKind::FailureMemo),
    );
    run_store_backed_stage_with_cache(cache, &keys, cache_key.0, expected_hashes, || {
        let validation = validate_resource_state(pack);
        let mut cert = validation.certificate(pack.identity.clone(), pack.schedule_pack_self_hash);
        cert.resource_state_cert_self_hash =
            resource_state_cert_self_hash(&cert).map_err(|error| {
                CodegenStageCacheError::StageEmit {
                    stage_id: "10.5",
                    message: error.to_string(),
                }
            })?;
        let output = SchedulePackOutput {
            input_identity: pack.identity.clone(),
            outcome: if validation.diagnostics.is_empty() {
                SchedulePackOutcome::Succeeded
            } else {
                SchedulePackOutcome::Failed
            },
            result: None,
            cert: Some(cert.clone()),
            diagnostics: validation.diagnostics.clone(),
        };
        let report = emit_resource_state_cert(&output).map_err(|error| {
            CodegenStageCacheError::StageEmit {
                stage_id: "10.5",
                message: error.to_string(),
            }
        })?;
        let report_self_hash = report.report_self_hash;
        if validation.diagnostics.is_empty() {
            Ok(StoreBackedStageRunResult::Success {
                product_self_hash: cert.resource_state_cert_self_hash,
                product: cert,
                report_self_hash,
            })
        } else {
            Ok(StoreBackedStageRunResult::FailureMemo {
                diagnostics: validation.diagnostics,
                report_self_hash,
            })
        }
    })
}

#[derive(Debug)]
pub enum SchedulePackEmitError {
    Envelope(ReportEnvelopeError),
    SelfHash(ReportSelfHashError),
    Canonical(ReportCanonicalJsonError),
    MissingResult,
    MissingCert,
}

impl fmt::Display for SchedulePackEmitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Envelope(error) => write!(f, "schedule report envelope failed: {error}"),
            Self::SelfHash(error) => write!(f, "schedule report self hash failed: {error}"),
            Self::Canonical(error) => write!(f, "schedule report canonicalization failed: {error}"),
            Self::MissingResult => write!(f, "schedule report requires a result"),
            Self::MissingCert => write!(f, "resource-state certificate requires a cert"),
        }
    }
}

impl Error for SchedulePackEmitError {}

impl From<ReportEnvelopeError> for SchedulePackEmitError {
    fn from(error: ReportEnvelopeError) -> Self {
        Self::Envelope(error)
    }
}

impl From<ReportSelfHashError> for SchedulePackEmitError {
    fn from(error: ReportSelfHashError) -> Self {
        Self::SelfHash(error)
    }
}

impl From<ReportCanonicalJsonError> for SchedulePackEmitError {
    fn from(error: ReportCanonicalJsonError) -> Self {
        Self::Canonical(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};

    use gbf_foundation::TargetProfileId;
    use gbf_policy::{PlacementProfile, SwitchProjectionSource};
    use gbf_store::blob::BlobStore;
    use gbf_store::stage_cache::StageCache;

    use crate::arena::{ArenaBindings, OverlayReservationHonor};
    use crate::overlay_plan::{
        OverlayInstall, OverlayInstallEvent, OverlayLeaseShape, OverlayRegion, OverlayReservation,
        OverlayReservationEntry, OverlayReservationKind, OverlayResident, OverlayResidentId,
        OverlaySource, OverlaySourceLease, OverlayWramRegionLease, WramRegionConstraint,
    };
    use crate::s3::infer_ir::NodeId;
    use crate::stage_cache::{
        CacheStatus, StoreBackedStageExpectedHashes, StoreBackedStageRunOutput,
    };
    use crate::window::{
        Bank0Demand, RomReachabilityClass, RomSwitchProjections, RomWindowBinding,
        RomWindowPlanInputIdentity, RomWindowPlanProvenance,
    };

    #[test]
    fn builds_balanced_schedule_and_canonical_reports() {
        let output = build_schedule_pack(&fixture_inputs());
        assert_eq!(
            output.outcome,
            SchedulePackOutcome::Succeeded,
            "{:?}",
            output.diagnostics
        );
        let result = output.result.as_ref().expect("schedule result");
        assert_eq!(result.summary.slice_count, 1);
        assert_eq!(result.summary.lease_count, 1);
        assert_eq!(result.summary.total_bank_switches, 1);
        assert_ne!(result.product.continuation_abi_hash, Hash256::ZERO);
        assert!(
            output
                .cert
                .as_ref()
                .expect("cert")
                .lease_balance
                .all_balanced,
            "lease-balance certificate facts should pass"
        );
        let cert = output.cert.as_ref().expect("cert");
        assert_eq!(
            cert.lease_balance.leases.len(),
            result.summary.lease_count as usize
        );
        assert!(cert.yield_safety.all_yields_safe);
        assert!(cert.isr_visible_residency.all_isr_safe);
        assert!(cert.isr_visible_residency.computed_reachability_confirmed);
        assert_eq!(
            cert.overlay_bank_shadow.slices_checked.len(),
            result.summary.slice_count as usize
        );
        assert!(
            cert.overlay_bank_shadow
                .slices_checked
                .iter()
                .all(|fact| fact.overlay_lease_shape_satisfied)
        );

        let sched_bytes = emit_schedule_pack_json_bytes(&output).expect("sched report");
        let sched_bytes_again = emit_schedule_pack_json_bytes(&output).expect("sched report again");
        let slice_bytes = emit_slice_report_json_bytes(&output).expect("slice report");
        let slice_bytes_again = emit_slice_report_json_bytes(&output).expect("slice report again");
        let cert_bytes = emit_resource_state_cert_json_bytes(&output).expect("resource cert");
        let cert_bytes_again =
            emit_resource_state_cert_json_bytes(&output).expect("resource cert again");
        assert_eq!(sched_bytes, sched_bytes_again);
        assert_eq!(slice_bytes, slice_bytes_again);
        assert_eq!(cert_bytes, cert_bytes_again);
        assert!(!sched_bytes.is_empty());
        assert!(!slice_bytes.is_empty());
        assert!(!cert_bytes.is_empty());
    }

    #[test]
    fn stage10_and_10_5_cache_aware_wrappers_replay_success() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = BlobStore::open(dir.path().to_path_buf()).expect("blob store");
        let cache = StageCache::new(&store);
        let inputs = fixture_inputs();

        let first_schedule = run_schedule_pack_with_cache(
            &cache,
            &inputs,
            StoreBackedStageExpectedHashes::default(),
        )
        .expect("first schedule run");
        let StoreBackedStageRunOutput::Success {
            product,
            product_self_hash,
            report_self_hash,
            status_entry,
            replayed,
        } = first_schedule
        else {
            panic!("schedule wrapper should succeed");
        };
        assert_eq!(status_entry.status, CacheStatus::Miss);
        assert!(!replayed);

        let second_schedule = run_schedule_pack_with_cache(
            &cache,
            &inputs,
            StoreBackedStageExpectedHashes {
                product_self_hash: Some(product_self_hash),
                success_report_self_hash: Some(report_self_hash),
                failure_report_self_hash: None,
            },
        )
        .expect("second schedule run");
        assert!(matches!(
            second_schedule,
            StoreBackedStageRunOutput::Success {
                status_entry,
                replayed: true,
                ..
            } if status_entry.status == CacheStatus::Hit
        ));

        let first_resource = run_resource_state_validation_with_cache(
            &cache,
            &product.product,
            StoreBackedStageExpectedHashes::default(),
        )
        .expect("first resource-state run");
        let StoreBackedStageRunOutput::Success {
            product_self_hash,
            report_self_hash,
            status_entry,
            replayed,
            ..
        } = first_resource
        else {
            panic!("resource-state wrapper should succeed");
        };
        assert_eq!(status_entry.status, CacheStatus::Miss);
        assert!(!replayed);

        let second_resource = run_resource_state_validation_with_cache(
            &cache,
            &product.product,
            StoreBackedStageExpectedHashes {
                product_self_hash: Some(product_self_hash),
                success_report_self_hash: Some(report_self_hash),
                failure_report_self_hash: None,
            },
        )
        .expect("second resource-state run");
        assert!(matches!(
            second_resource,
            StoreBackedStageRunOutput::Success {
                status_entry,
                replayed: true,
                ..
            } if status_entry.status == CacheStatus::Hit
        ));
    }

    #[test]
    fn resource_state_validation_rejects_yield_under_rom_window_lease() {
        let output = build_schedule_pack(&fixture_inputs());
        let mut pack = output.result.expect("schedule result").product;
        let slice = &mut pack
            .modes
            .iter_mut()
            .find(|entry| entry.mode == RuntimeMode::Interactive)
            .expect("interactive mode")
            .ir
            .slices[0];
        slice.ops.insert(
            2,
            SchedOp::Yield {
                kind: YieldKind::Frame,
            },
        );

        let validation = validate_resource_state(&pack);
        assert_has_resource_code(
            &validation,
            ResourceStateDiagnosticCode::LeaseYieldCrossesNonResumable,
        );
    }

    #[test]
    fn resource_state_validation_traverses_cross_slice_lease_flow() {
        let output = build_schedule_pack(&fixture_inputs());
        let mut pack = output.result.expect("schedule result").product;
        let lease = pack.leases[0].id;
        pack.leases[0].released_in = SliceId(1);

        let mode = &mut pack.modes[0].ir;
        let slice0 = &mut mode.slices[0];
        slice0
            .ops
            .retain(|op| !matches!(op, SchedOp::ReleaseLease { .. } | SchedOp::Halt));
        slice0.ops.push(SchedOp::TailCall { target: SliceId(1) });
        slice0.exit_kind = ExitKind::TailCall;
        slice0.successors = vec![SliceId(1)];

        let mut slice1 = slice0.clone();
        slice1.id = SliceId(1);
        slice1.ops = vec![SchedOp::ReleaseLease { lease }, SchedOp::Halt];
        slice1.exit_kind = ExitKind::Halt;
        slice1.successors.clear();
        mode.slices.push(slice1);
        pack.epochs[0].epochs[0].slices = vec![SliceId(0), SliceId(1)];

        let validation = validate_resource_state(&pack);
        assert_eq!(validation.diagnostics, Vec::new());
        assert!(validation.lease_balance.all_balanced);
        assert_eq!(validation.lease_balance.leases[0].paths_checked, 2);
        assert!(
            validation
                .isr_visible_residency
                .computed_reachability_confirmed
        );
    }

    #[test]
    fn resource_state_validation_rejects_divergent_incoming_lease_state() {
        let output = build_schedule_pack(&fixture_inputs());
        let mut pack = output.result.expect("schedule result").product;
        let lease = pack.leases[0].id;

        let mode = &mut pack.modes[0].ir;
        let slice0 = &mut mode.slices[0];
        slice0
            .ops
            .retain(|op| !matches!(op, SchedOp::ReleaseLease { .. } | SchedOp::Halt));
        slice0.ops.push(SchedOp::TailCall { target: SliceId(2) });
        slice0.exit_kind = ExitKind::TailCall;
        slice0.successors = vec![SliceId(1), SliceId(2)];

        let mut slice1 = slice0.clone();
        slice1.id = SliceId(1);
        slice1.ops = vec![
            SchedOp::ReleaseLease { lease },
            SchedOp::TailCall { target: SliceId(2) },
        ];
        slice1.exit_kind = ExitKind::TailCall;
        slice1.successors = vec![SliceId(2)];

        let mut slice2 = slice0.clone();
        slice2.id = SliceId(2);
        slice2.ops = vec![SchedOp::ReleaseLease { lease }, SchedOp::Halt];
        slice2.exit_kind = ExitKind::Halt;
        slice2.successors.clear();

        mode.slices.push(slice1);
        mode.slices.push(slice2);
        pack.epochs[0].epochs[0].slices = vec![SliceId(0), SliceId(1), SliceId(2)];

        let validation = validate_resource_state(&pack);
        assert_has_resource_code(&validation, ResourceStateDiagnosticCode::LeaseUnbalanced);
    }

    #[test]
    fn schedule_modes_carry_distinct_cadence_and_yield_checks() {
        let mut inputs = fixture_inputs();
        inputs.input_identity.requested_runtime_modes = BTreeSet::from([
            RuntimeMode::Interactive,
            RuntimeMode::Safe,
            RuntimeMode::Steady,
            RuntimeMode::Trace,
        ]);
        inputs.expected_input_hashes = inputs.input_identity.hashes();

        let output = build_schedule_pack(&inputs);
        assert_eq!(
            output.outcome,
            SchedulePackOutcome::Succeeded,
            "{:?}",
            output.diagnostics
        );
        let pack = output.result.expect("schedule result").product;
        let cadence_by_mode = pack
            .modes
            .iter()
            .map(|entry| {
                let slice = &entry.ir.slices[0];
                (
                    entry.mode,
                    (
                        slice.hard_cycles_to_safe_point,
                        slice.soft_target_cycles,
                        slice.max_interrupt_latency,
                        slice.yield_check,
                    ),
                )
            })
            .collect::<BTreeMap<_, _>>();

        assert_eq!(
            cadence_by_mode[&RuntimeMode::Interactive],
            (17_556, 8_778, 256, YieldCheckClass::OnceAtEnd)
        );
        assert_eq!(
            cadence_by_mode[&RuntimeMode::Trace],
            (17_556, 4_389, 128, YieldCheckClass::EveryNTiles { n: 1 })
        );
        assert_eq!(
            cadence_by_mode[&RuntimeMode::Steady],
            (35_112, 17_556, 512, YieldCheckClass::OnceAtEnd)
        );
        assert_eq!(
            cadence_by_mode[&RuntimeMode::Safe],
            (17_556, 4_389, 128, YieldCheckClass::EveryLoadStore)
        );
    }

    #[test]
    fn resource_state_validation_proves_overlay_install_lease_shape() {
        let mut inputs = fixture_inputs();
        inputs.rom_window_plan.residency_epochs[0].overlay_state = OverlayState::OverlayActive;
        inputs.overlay_plan = fixture_overlay_plan_with_install(&inputs.input_identity);

        let output = build_schedule_pack(&inputs);
        assert_eq!(
            output.outcome,
            SchedulePackOutcome::Succeeded,
            "{:?}",
            output.diagnostics
        );
        let result = output.result.as_ref().expect("schedule result");
        assert_eq!(result.summary.total_overlay_installs, 1);
        let cert = output.cert.as_ref().expect("cert");
        assert!(cert.isr_visible_residency.computed_reachability_confirmed);
        assert!(cert.overlay_bank_shadow.all_consistent);
        assert!(
            cert.overlay_bank_shadow
                .slices_checked
                .iter()
                .all(|fact| fact.overlay_installs_aligned && fact.overlay_lease_shape_satisfied)
        );

        let mut pack = result.product.clone();
        pack.modes[0].ir.slices[0]
            .ops
            .retain(|op| !matches!(op, SchedOp::AcquireLease { lease } if *lease == LeaseId(1)));
        let validation = validate_resource_state(&pack);
        assert_has_resource_code(
            &validation,
            ResourceStateDiagnosticCode::SchedOverlayInstallEpochMismatch,
        );
        assert!(!validation.overlay_bank_shadow.all_consistent);
    }

    #[test]
    fn resource_state_validation_rejects_representative_mutations() {
        assert_mutated_pack_code(
            |pack| {
                pack.modes[0].ir.slices[0]
                    .ops
                    .insert(1, SchedOp::AcquireLease { lease: LeaseId(0) });
            },
            ResourceStateDiagnosticCode::LeaseDoubleAcquire,
        );
        assert_mutated_pack_code(
            |pack| {
                pack.modes[0].ir.slices[0]
                    .ops
                    .retain(|op| !matches!(op, SchedOp::AcquireLease { .. }));
            },
            ResourceStateDiagnosticCode::LeaseReleaseWithoutAcquire,
        );
        assert_mutated_pack_code(
            |pack| {
                pack.modes[0].ir.slices[0].interrupt_policy = InterruptPolicy::Enabled;
            },
            ResourceStateDiagnosticCode::ResIsrEnabledHoldsRomWindowLease,
        );
        assert_mutated_pack_code(
            |pack| {
                pack.modes[0].ir.slices[0].entry_residency = EntryResidency::Bank0;
            },
            ResourceStateDiagnosticCode::SchedEntryResidencyEpochMismatch,
        );
        assert_mutated_pack_code(
            |pack| {
                pack.epochs[0].epochs[0].overlay = None;
                pack.modes[0].ir.slices[0].ops.insert(
                    0,
                    SchedOp::OverlayInstall {
                        install: OverlayInstallId(0),
                    },
                );
            },
            ResourceStateDiagnosticCode::SchedOverlayInstallEpochMismatch,
        );
        assert_mutated_pack_code(
            |pack| {
                pack.modes[0].ir.slices[0]
                    .ops
                    .retain(|op| !matches!(op, SchedOp::AcquireLease { .. }));
            },
            ResourceStateDiagnosticCode::ResBankSwitchUnbracketed,
        );
        assert_mutated_pack_code(
            |pack| {
                pack.epochs[0].epochs[0].slices.clear();
            },
            ResourceStateDiagnosticCode::SchedEpochCoverageGap,
        );
    }

    #[test]
    fn cache_keys_are_deterministic_and_independent_for_stage10_and_10_5() {
        let inputs = fixture_inputs();
        let key_a = SchedulePackCacheKeyInputs::from_input_identity(&inputs.input_identity)
            .cache_key()
            .expect("first K10");
        let key_b = SchedulePackCacheKeyInputs::from_input_identity(&inputs.input_identity)
            .cache_key()
            .expect("second K10");
        assert_eq!(key_a, key_b);

        let output = build_schedule_pack(&inputs);
        let pack = &output.result.as_ref().expect("schedule result").product;
        let cert_key_a = ResourceStateCacheKeyInputs::from_schedule_pack(pack)
            .cache_key()
            .expect("first K10.5");
        let cert_key_b = ResourceStateCacheKeyInputs::from_schedule_pack(pack)
            .cache_key()
            .expect("second K10.5");
        assert_eq!(cert_key_a, cert_key_b);

        let mut changed_identity = inputs.input_identity.clone();
        changed_identity.feature_set_hash = hash(0xfe);
        let changed_key = SchedulePackCacheKeyInputs::from_input_identity(&changed_identity)
            .cache_key()
            .expect("changed K10");
        assert_ne!(key_a, changed_key);

        let changed_cert_key = ResourceStateCacheKeyInputs {
            schedule_pack_self_hash: hash(0xfd),
            feature_set_hash: pack.identity.feature_set_hash,
            pass_version: RESOURCE_STATE_CERT_PASS_VERSION,
        }
        .cache_key()
        .expect("changed K10.5");
        assert_ne!(cert_key_a, changed_cert_key);
    }

    fn assert_mutated_pack_code(
        mutate: impl FnOnce(&mut SchedulePack),
        expected: ResourceStateDiagnosticCode,
    ) {
        let output = build_schedule_pack(&fixture_inputs());
        let mut pack = output.result.expect("schedule result").product;
        mutate(&mut pack);
        let validation = validate_resource_state(&pack);
        assert_has_resource_code(&validation, expected);
    }

    fn assert_has_resource_code(
        validation: &ResourceStateValidation,
        expected: ResourceStateDiagnosticCode,
    ) {
        assert!(
            validation.diagnostics.iter().any(|diagnostic| matches!(
                diagnostic.code,
                ValidationCode::ResourceState { code, .. } if code == expected
            )),
            "missing {expected:?} in {:?}",
            validation.diagnostics
        );
    }

    fn fixture_inputs() -> SchedulePackInputs {
        let identity = fixture_identity();
        SchedulePackInputs {
            expected_input_hashes: identity.hashes(),
            input_identity: identity.clone(),
            rom_window_plan: fixture_rom_window_plan(&identity),
            overlay_plan: fixture_overlay_plan(&identity),
            arena_plan: fixture_arena_plan(&identity),
        }
    }

    fn fixture_identity() -> SchedulePackInputIdentity {
        SchedulePackInputIdentity {
            infer_ir_self_hash: hash(0x01),
            observation_plan_self_hash: hash(0x02),
            range_plan_self_hash: hash(0x03),
            storage_plan_self_hash: hash(0x04),
            sram_page_plan_self_hash: hash(0x05),
            rom_window_plan_self_hash: hash(0x06),
            overlay_plan_self_hash: hash(0x07),
            arena_plan_self_hash: hash(0x08),
            policy_resolution_self_hash: hash(0x09),
            runtime_chrome_budget_self_hash: hash(0x0a),
            feature_set_hash: hash(0x0b),
            requested_runtime_modes: BTreeSet::from([RuntimeMode::Interactive]),
            determinism: DeterminismClass::BitExact,
            target_profile_id: TargetProfileId::from("dmg-mbc5"),
            schema_version: SCHED_IR_SCHEMA_VERSION,
        }
    }

    fn fixture_rom_window_plan(identity: &SchedulePackInputIdentity) -> RomWindowPlan {
        RomWindowPlan {
            identity: RomWindowPlanInputIdentity {
                artifact_validation_self_hash: hash(0x2d),
                policy_resolution_self_hash: identity.policy_resolution_self_hash,
                static_budget_self_hash: hash(0x2e),
                quant_graph_self_hash: hash(0x2f),
                infer_ir_self_hash: identity.infer_ir_self_hash,
                storage_plan_self_hash: identity.storage_plan_self_hash,
                observation_plan_self_hash: identity.observation_plan_self_hash,
                range_plan_self_hash: identity.range_plan_self_hash,
                sram_page_plan_self_hash: identity.sram_page_plan_self_hash,
                runtime_chrome_budget_hash: identity.runtime_chrome_budget_self_hash,
                target_profile_hash: hash(0x31),
                rom_window_plan_policy_projection_hash: hash(0x32),
                runtime_mode: RuntimeMode::Interactive,
                determinism: DeterminismClass::BitExact,
                target_profile_id: TargetProfileId::from("dmg-mbc5"),
                schema_version: crate::window::ROM_WINDOW_PLAN_SCHEMA_VERSION,
            },
            kernel_residency: BTreeMap::new(),
            lut_residency: BTreeMap::new(),
            rom_window_bindings: vec![RomWindowBinding {
                id: RomWindowBindingId(0),
                epoch: crate::window::ResidencyEpochId(0),
                visibility: RomVisibility {
                    bank0_visible: true,
                    switchable: Some(RomBankIndex(3)),
                },
                assigned_kernels: Vec::new(),
                assigned_luts: Vec::new(),
                assigned_tensors: Vec::new(),
                closure: None,
                provenance: Vec::new(),
            }],
            banks: Vec::new(),
            residency_epochs: vec![crate::window::ResidencyEpoch {
                id: crate::window::ResidencyEpochId(0),
                op_range: crate::window::NodeAnchorRange {
                    first_node: NodeId::new(0),
                    last_node: NodeId::new(0),
                },
                rom_window_binding: RomWindowBindingId(0),
                sram_page_binding: None,
                overlay_state: OverlayState::NoOverlayActive,
                yield_kind: YieldKindHint::YieldsAtCommitBoundaries,
            }],
            co_resident_closures: Vec::new(),
            overlay_demand: crate::window::WramOverlayDemand {
                kernels: Vec::new(),
                luts: Vec::new(),
                install_source_visibility: Vec::new(),
                total_overlay_bytes: 0,
                total_install_count_per_token_upper_bound: 0,
            },
            bank0_demand: Bank0Demand {
                kernels: Vec::new(),
                luts: Vec::new(),
                total_kernel_bytes: 0,
                total_lut_bytes: 0,
                remaining_slack_bytes: 16 * 1024,
            },
            projections: RomSwitchProjections {
                projected_bank_switches_per_token: 1,
                upper_bound_per_token: 4,
                per_phase: Vec::new(),
                source: SwitchProjectionSource::ConservativeStaticUpperBound,
            },
            profile: PlacementProfile::Budgeted,
            provenance: RomWindowPlanProvenance {
                kernel_to_reachability: BTreeMap::new(),
                lut_to_reachability: BTreeMap::new(),
                tensor_to_bank_assignment: Vec::new(),
                epoch_to_node_range: Vec::new(),
                closure_to_kernels: Vec::new(),
            },
            rom_window_plan_self_hash: identity.rom_window_plan_self_hash,
        }
    }

    fn fixture_overlay_plan(identity: &SchedulePackInputIdentity) -> OverlayPlan {
        OverlayPlan {
            identity: crate::overlay_plan::OverlayPlanInputIdentity {
                storage_plan_self_hash: identity.storage_plan_self_hash,
                sram_page_plan_self_hash: identity.sram_page_plan_self_hash,
                rom_window_plan_self_hash: identity.rom_window_plan_self_hash,
                runtime_chrome_budget_hash: identity.runtime_chrome_budget_self_hash,
                target_profile_hash: hash(0x41),
                overlay_plan_policy_projection_hash: hash(0x42),
                determinism: DeterminismClass::BitExact,
                target_profile_id: TargetProfileId::from("dmg-mbc5"),
                schema_version: crate::overlay_plan::OVERLAY_PLAN_SCHEMA_VERSION,
            },
            regions: Vec::new(),
            share_classes: Vec::new(),
            installs: Vec::new(),
            reservation: OverlayReservation {
                total_bytes: 0,
                per_region: Vec::new(),
                cap_bytes: 0,
                region_max_bytes: 0,
            },
            overlay_plan_self_hash: identity.overlay_plan_self_hash,
        }
    }

    fn fixture_overlay_plan_with_install(identity: &SchedulePackInputIdentity) -> OverlayPlan {
        let overlay = OverlayId(0);
        let resident = OverlayResidentId::Kernel {
            kernel: KernelSpecId::from("kernel.overlay"),
        };
        let source = OverlaySource::RomWindowOverlayDemand {
            resident: resident.clone(),
        };
        let install_event = OverlayInstallEvent::TokenBoundary;
        let mut plan = fixture_overlay_plan(identity);
        plan.regions = vec![OverlayRegion {
            id: overlay,
            bytes: 128,
            constraint: WramRegionConstraint::DmgWramC000Dfff,
            members: vec![OverlayResident {
                id: resident.clone(),
                payload_bytes: 128,
                reachability: RomReachabilityClass::HotPath,
                source: source.clone(),
            }],
            reservation_kind: OverlayReservationKind::WramOverlay,
            reservation_floor_bytes: 128,
            reservation_ceil_bytes: 128,
        }];
        plan.installs = vec![OverlayInstall {
            id: OverlayInstallId(0),
            region: overlay,
            member: resident,
            source: source.clone(),
            install_event,
            lease_shape: OverlayLeaseShape {
                source_lease: OverlaySourceLease {
                    source,
                    acquire_at: install_event,
                    release_at: install_event,
                },
                wram_region_lease: OverlayWramRegionLease {
                    region: overlay,
                    acquire_at: install_event,
                    release_at: install_event,
                },
            },
        }];
        plan.reservation = OverlayReservation {
            total_bytes: 128,
            per_region: vec![OverlayReservationEntry {
                region: overlay,
                bytes: 128,
                reservation_kind: OverlayReservationKind::WramOverlay,
            }],
            cap_bytes: 1024,
            region_max_bytes: 512,
        };
        plan
    }

    fn fixture_arena_plan(identity: &SchedulePackInputIdentity) -> ArenaPlan {
        ArenaPlan {
            identity: crate::arena::ArenaPlanInputIdentity {
                storage_plan_self_hash: identity.storage_plan_self_hash,
                sram_page_plan_self_hash: identity.sram_page_plan_self_hash,
                rom_window_plan_self_hash: identity.rom_window_plan_self_hash,
                overlay_plan_self_hash: identity.overlay_plan_self_hash,
                runtime_chrome_budget_hash: identity.runtime_chrome_budget_self_hash,
                target_profile_hash: hash(0x51),
                arena_plan_policy_projection_hash: hash(0x52),
                determinism: DeterminismClass::BitExact,
                target_profile_id: TargetProfileId::from("dmg-mbc5"),
                schema_version: crate::arena::ARENA_PLAN_SCHEMA_VERSION,
            },
            wram_arenas: Vec::new(),
            sram_arenas: Vec::new(),
            hram_assignments: Vec::new(),
            overlay_reservation: OverlayReservationHonor {
                total_bytes: 0,
                expected_total_bytes: 0,
                per_region: Vec::new(),
            },
            arena_bindings: ArenaBindings {
                materialize_to_slot: Vec::new(),
                persist_to_slot_pair: Vec::new(),
                overlay_to_arena: Vec::new(),
            },
            arena_plan_self_hash: identity.arena_plan_self_hash,
        }
    }

    fn hash(byte: u8) -> Hash256 {
        Hash256::from_bytes([byte; 32])
    }
}
