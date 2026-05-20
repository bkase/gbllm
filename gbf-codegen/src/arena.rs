//! Stage 9 `ArenaPlan` construction, report, certificate, and cache-key surface.

use std::collections::{BTreeMap, BTreeSet};
use std::{error::Error, fmt};

use gbf_foundation::{
    CanonicalJsonError, DomainHash, EvidenceRef, Hash256, SemVer, TargetProfileId,
    canonical_json_bytes_omitting_fields, self_hash_omitting_fields,
};
use gbf_policy::{
    ArenaPlanDiagnosticCode, ArenaPlanDiagnosticProvenance, DiagnosticSeverity,
    RuntimeChromeBudget, ValidationCode, ValidationDetail, ValidationDiagnostic, ValidationOrigin,
};
use gbf_report::{
    CanonicalJsonError as ReportCanonicalJsonError, ReportBody, ReportEnvelope,
    ReportEnvelopeError, ReportOutcome, ReportSelfHashError, canonicalize, round_trip_self_hash,
};
use serde::{Deserialize, Serialize};

use crate::overlay_plan::{OverlayId, OverlayPlan, OverlayResidentId};
use crate::s1::quant_graph::DeterminismClass;
use crate::s3::infer_ir::ValueId;
use crate::sram_page_plan::{
    PageId, PersistentPageGeometry, SequenceStreamId, SramPageBinding, SramPagePlan,
};
use crate::stage_cache::{
    CodegenStageCacheError, StoreBackedStageCacheKeys, StoreBackedStageCellKind,
    StoreBackedStageExpectedHashes, StoreBackedStageRunOutput, StoreBackedStageRunResult,
    crate_feature_set_hash, run_store_backed_stage_with_cache, stage9_arena_plan_store_key,
};
use crate::storage_plan::types::{
    AliasClassId, CommitGroupId, LifetimeClass, Materialization, PersistPageId, StorageBinding,
    StorageClass,
};
use gbf_store::stage_cache::StageCache as StoreStageCache;

pub const ARENA_PLAN_SCHEMA_ID: &str = "arena_plan.v1";
pub const ARENA_PLAN_SCHEMA_VERSION: SemVer = SemVer::new(1, 0, 0);
pub const ARENA_PLAN_PASS_VERSION: &str = "stage9/v1";
pub const ARENA_CERT_SCHEMA_ID: &str = "arena.cert.v1";
pub const ARENA_CERT_SCHEMA_VERSION: SemVer = SemVer::new(1, 0, 0);
pub const ARENA_CERT_PASS_VERSION: &str = "arena.cert/stage9/v1";

pub type ArenaPlanReportEnvelope = ReportEnvelope<ArenaPlanReportBody>;
pub type ArenaCertReportEnvelope = ReportEnvelope<ArenaCertBody>;

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ArenaId(pub u32);

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ArenaSlotId(pub u32);

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum NamedArena {
    WramActivationsPingA,
    WramActivationsPingB,
    WramAccumScratch,
    WramRouteScratch,
    WramDecodeScratch,
    WramContinuationRecord,
    WramOverlayRegion { overlay: OverlayId },
    SramSequenceStatePages { stream: SequenceStreamId },
    SramTracePages,
    SramHarnessCommandBlock,
    SramHarnessResultBlock,
    SramPersistedTranscript,
    SramColdSpill,
    HramFrameFlags,
    HramBankShadow,
    HramFaultCode,
    HramSchedulerScratch,
    HramYieldRequested,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ArenaBacking {
    Wram,
    Sram,
    Hram,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ByteRange {
    pub start: u16,
    pub len: u16,
}

impl ByteRange {
    #[must_use]
    pub const fn end(self) -> u32 {
        self.start as u32 + self.len as u32
    }

    #[must_use]
    pub const fn intersects(self, other: Self) -> bool {
        (self.start as u32) < other.end() && (other.start as u32) < self.end()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ArenaZerofillPolicy {
    ZeroOnBuild,
    RuntimeCleared,
    NotRequired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArenaInstance {
    pub id: ArenaId,
    pub named: NamedArena,
    pub byte_range: ByteRange,
    pub backing: ArenaBacking,
    pub alignment: u16,
    pub zerofill: ArenaZerofillPolicy,
    pub slots: Vec<ArenaSlot>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArenaSlot {
    pub id: ArenaSlotId,
    pub byte_offset: u16,
    pub size_bytes: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias_class_id: Option<AliasClassId>,
    pub lifetime_class: LifetimeClass,
    pub binding_kind: SlotBindingKind,
    pub binding_ref: SlotBindingRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum SlotBindingKind {
    MaterializedValue,
    PersistentPageA {
        page: PageId,
        commit_group: CommitGroupId,
    },
    PersistentPageB {
        page: PageId,
        commit_group: CommitGroupId,
    },
    OverlayMember {
        overlay: OverlayId,
        member: OverlayResidentId,
    },
    RuntimeFixed {
        runtime_kind: RuntimeFixedKind,
    },
    TraceRing {
        trace_ring: TraceRingId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum SlotBindingRef {
    Value {
        value: ValueId,
    },
    PersistPage {
        value: ValueId,
        page: PersistPageId,
    },
    OverlayMember {
        overlay: OverlayId,
        member: OverlayResidentId,
    },
    RuntimeFixed {
        runtime_kind: RuntimeFixedKind,
    },
    TraceRing {
        trace_ring: TraceRingId,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
#[allow(clippy::enum_variant_names)]
pub enum RuntimeFixedKind {
    HramFrameFlags,
    HramBankShadow,
    HramFaultCode,
    HramSchedulerScratch,
    HramYieldRequested,
}

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TraceRingId(pub u32);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArenaBindings {
    pub materialize_to_slot: Vec<MaterializeSlotBinding>,
    pub persist_to_slot_pair: Vec<PersistSlotPairBinding>,
    pub overlay_to_arena: Vec<OverlayArenaBinding>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaterializeSlotBinding {
    pub value: ValueId,
    pub slot: ArenaSlotId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersistSlotPairBinding {
    pub value: ValueId,
    pub persist_page: PersistPageId,
    pub sram_page: PageId,
    pub commit_group: CommitGroupId,
    pub slot_a: ArenaSlotId,
    pub slot_b: ArenaSlotId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverlayArenaBinding {
    pub overlay: OverlayId,
    pub arena: ArenaId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverlayReservationHonor {
    pub total_bytes: u16,
    pub expected_total_bytes: u16,
    pub per_region: Vec<OverlayReservationHonorEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverlayReservationHonorEntry {
    pub overlay_id: OverlayId,
    pub arena_id: ArenaId,
    pub bytes: u16,
    pub byte_range: ByteRange,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArenaPlanInputIdentity {
    pub storage_plan_self_hash: Hash256,
    pub sram_page_plan_self_hash: Hash256,
    pub rom_window_plan_self_hash: Hash256,
    pub overlay_plan_self_hash: Hash256,
    pub runtime_chrome_budget_hash: Hash256,
    pub target_profile_hash: Hash256,
    pub arena_plan_policy_projection_hash: Hash256,
    pub determinism: DeterminismClass,
    pub target_profile_id: TargetProfileId,
    pub schema_version: SemVer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArenaPlanInputHashes {
    pub storage_plan_self_hash: Hash256,
    pub sram_page_plan_self_hash: Hash256,
    pub rom_window_plan_self_hash: Hash256,
    pub overlay_plan_self_hash: Hash256,
    pub runtime_chrome_budget_hash: Hash256,
    pub target_profile_hash: Hash256,
    pub arena_plan_policy_projection_hash: Hash256,
}

impl ArenaPlanInputIdentity {
    #[must_use]
    pub const fn hashes(&self) -> ArenaPlanInputHashes {
        ArenaPlanInputHashes {
            storage_plan_self_hash: self.storage_plan_self_hash,
            sram_page_plan_self_hash: self.sram_page_plan_self_hash,
            rom_window_plan_self_hash: self.rom_window_plan_self_hash,
            overlay_plan_self_hash: self.overlay_plan_self_hash,
            runtime_chrome_budget_hash: self.runtime_chrome_budget_hash,
            target_profile_hash: self.target_profile_hash,
            arena_plan_policy_projection_hash: self.arena_plan_policy_projection_hash,
        }
    }

    #[must_use]
    pub fn hash_for_product(&self, product: ArenaPlanInputProduct) -> Hash256 {
        self.hashes().hash_for_product(product)
    }
}

impl ArenaPlanInputHashes {
    #[must_use]
    pub const fn hash_for_product(&self, product: ArenaPlanInputProduct) -> Hash256 {
        match product {
            ArenaPlanInputProduct::StoragePlan => self.storage_plan_self_hash,
            ArenaPlanInputProduct::SramPagePlan => self.sram_page_plan_self_hash,
            ArenaPlanInputProduct::RomWindowPlan => self.rom_window_plan_self_hash,
            ArenaPlanInputProduct::OverlayPlan => self.overlay_plan_self_hash,
            ArenaPlanInputProduct::RuntimeChromeBudget => self.runtime_chrome_budget_hash,
            ArenaPlanInputProduct::TargetProfile => self.target_profile_hash,
            ArenaPlanInputProduct::PolicyProjection => self.arena_plan_policy_projection_hash,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ArenaPlanInputProduct {
    StoragePlan,
    SramPagePlan,
    RomWindowPlan,
    OverlayPlan,
    RuntimeChromeBudget,
    TargetProfile,
    PolicyProjection,
}

impl ArenaPlanInputProduct {
    #[must_use]
    pub const fn field_name(self) -> &'static str {
        match self {
            Self::StoragePlan => "storage_plan_self_hash",
            Self::SramPagePlan => "sram_page_plan_self_hash",
            Self::RomWindowPlan => "rom_window_plan_self_hash",
            Self::OverlayPlan => "overlay_plan_self_hash",
            Self::RuntimeChromeBudget => "runtime_chrome_budget_hash",
            Self::TargetProfile => "target_profile_hash",
            Self::PolicyProjection => "arena_plan_policy_projection_hash",
        }
    }
}

const ARENA_PLAN_INPUT_PRODUCTS: [ArenaPlanInputProduct; 7] = [
    ArenaPlanInputProduct::StoragePlan,
    ArenaPlanInputProduct::SramPagePlan,
    ArenaPlanInputProduct::RomWindowPlan,
    ArenaPlanInputProduct::OverlayPlan,
    ArenaPlanInputProduct::RuntimeChromeBudget,
    ArenaPlanInputProduct::TargetProfile,
    ArenaPlanInputProduct::PolicyProjection,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArenaPlanPolicyProjection {
    pub arena_alignment_default: u16,
    pub arena_zerofill_policy: ArenaZerofillPolicy,
    pub persistent_page_geometry: PersistentPageGeometry,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArenaTargetProfileSummary {
    pub wram_usable_bytes: u32,
    pub sram_window_bytes: u32,
    pub hram_usable_bytes: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArenaPlanInputs {
    pub input_identity: ArenaPlanInputIdentity,
    pub expected_input_hashes: ArenaPlanInputHashes,
    pub runtime_chrome_budget: RuntimeChromeBudget,
    pub target_profile: ArenaTargetProfileSummary,
    pub policy: ArenaPlanPolicyProjection,
    pub storage_bindings: Vec<ArenaPlanBindingInput>,
    pub sram_page_plan: SramPagePlan,
    pub overlay_plan: OverlayPlan,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArenaPlanBindingInput {
    pub binding: StorageBinding,
    pub size_bytes: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArenaPlan {
    pub identity: ArenaPlanInputIdentity,
    pub wram_arenas: Vec<ArenaInstance>,
    pub sram_arenas: Vec<ArenaInstance>,
    pub hram_assignments: Vec<ArenaInstance>,
    pub overlay_reservation: OverlayReservationHonor,
    pub arena_bindings: ArenaBindings,
    pub arena_plan_self_hash: Hash256,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArenaPlanSummary {
    pub wram_arena_count: u16,
    pub sram_arena_count: u16,
    pub hram_assignment_count: u16,
    pub slot_count: u32,
    pub overlay_reserved_bytes: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArenaPlanResult {
    pub product: ArenaPlan,
    pub arena_plan_self_hash: Hash256,
    pub summary: ArenaPlanSummary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArenaPlanOutput {
    pub input_identity: ArenaPlanInputIdentity,
    pub outcome: ArenaPlanOutcome,
    pub result: Option<ArenaPlanResult>,
    pub cert: Option<ArenaCertBody>,
    pub diagnostics: Vec<ValidationDiagnostic>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArenaPlanOutcome {
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArenaPlanReportBody {
    pub pass_version: String,
    pub input_identity: ArenaPlanInputIdentity,
    pub diagnostics: Vec<ValidationDiagnostic>,
    pub result: Option<ArenaPlanResult>,
}

impl ReportBody for ArenaPlanReportBody {
    const REPORT_TYPE: &'static str = "ArenaPlanReport";
    const SCHEMA_ID: &'static str = ARENA_PLAN_SCHEMA_ID;
    const SCHEMA_VERSION: &'static str = "1.0.0";

    fn validate_semantics(&self, outcome: ReportOutcome) -> Result<(), Vec<ValidationDiagnostic>> {
        validate_arena_plan_report_body(self, outcome)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArenaCertBody {
    pub pass_version: String,
    pub arena_plan_self_hash: Hash256,
    pub address_invariants: ArenaAddressInvariants,
    pub reservation_honor: OverlayReservationHonor,
    pub persistent_page_geometry: ArenaPersistentGeometryWitness,
    pub harness_no_leak: bool,
    pub diagnostics: Vec<ValidationDiagnostic>,
    pub arena_cert_self_hash: Hash256,
}

impl ReportBody for ArenaCertBody {
    const REPORT_TYPE: &'static str = "ArenaCert";
    const SCHEMA_ID: &'static str = ARENA_CERT_SCHEMA_ID;
    const SCHEMA_VERSION: &'static str = "1.0.0";

    fn validate_semantics(&self, outcome: ReportOutcome) -> Result<(), Vec<ValidationDiagnostic>> {
        validate_arena_cert_body(self, outcome)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArenaAddressInvariants {
    pub f_addr_1_materialize_coverage: bool,
    pub f_addr_2_persist_coverage: bool,
    pub f_addr_3_alias_total_or_disjoint: bool,
    pub f_addr_4_lifetime_preservation: bool,
    pub f_addr_5_overlay_byte_equality: bool,
    pub f_addr_6_overlay_disjointness: bool,
    pub f_addr_7_overlay_member_start_of_region: bool,
    pub f_addr_8_sram_no_span: bool,
    pub f_addr_9_hram_page_bound: bool,
    pub f_addr_10_continuation_sized: bool,
    pub f_addr_11_harness_disjoint: bool,
    pub f_addr_12_persistent_page_header_aligned: bool,
    pub f_addr_13_slot_id_unique: bool,
    pub f_addr_14_binding_ref_resolves: bool,
}

impl ArenaAddressInvariants {
    #[must_use]
    pub const fn all_true() -> Self {
        Self {
            f_addr_1_materialize_coverage: true,
            f_addr_2_persist_coverage: true,
            f_addr_3_alias_total_or_disjoint: true,
            f_addr_4_lifetime_preservation: true,
            f_addr_5_overlay_byte_equality: true,
            f_addr_6_overlay_disjointness: true,
            f_addr_7_overlay_member_start_of_region: true,
            f_addr_8_sram_no_span: true,
            f_addr_9_hram_page_bound: true,
            f_addr_10_continuation_sized: true,
            f_addr_11_harness_disjoint: true,
            f_addr_12_persistent_page_header_aligned: true,
            f_addr_13_slot_id_unique: true,
            f_addr_14_binding_ref_resolves: true,
        }
    }

    #[must_use]
    pub const fn all_pass(self) -> bool {
        self.f_addr_1_materialize_coverage
            && self.f_addr_2_persist_coverage
            && self.f_addr_3_alias_total_or_disjoint
            && self.f_addr_4_lifetime_preservation
            && self.f_addr_5_overlay_byte_equality
            && self.f_addr_6_overlay_disjointness
            && self.f_addr_7_overlay_member_start_of_region
            && self.f_addr_8_sram_no_span
            && self.f_addr_9_hram_page_bound
            && self.f_addr_10_continuation_sized
            && self.f_addr_11_harness_disjoint
            && self.f_addr_12_persistent_page_header_aligned
            && self.f_addr_13_slot_id_unique
            && self.f_addr_14_binding_ref_resolves
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArenaPersistentGeometryWitness {
    pub geometry: PersistentPageGeometry,
    pub pages: Vec<ArenaPersistentPageWitness>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArenaPersistentPageWitness {
    pub value: ValueId,
    pub sram_page: PageId,
    pub commit_group: CommitGroupId,
    pub slot_a: ArenaSlotId,
    pub slot_b: ArenaSlotId,
    pub slot_size_bytes: u16,
}

pub fn build_arena_plan(input: &ArenaPlanInputs) -> ArenaPlanOutput {
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
                ArenaPlanDiagnosticCode::ArenaInputHashMismatch,
                ArenaPlanDiagnosticProvenance::HashMismatch {
                    product: "sram_page_plan_self_hash".to_owned(),
                    recorded: input.input_identity.sram_page_plan_self_hash,
                    computed: input.sram_page_plan.sram_page_plan_self_hash,
                },
            )],
        );
    }
    if input.input_identity.overlay_plan_self_hash != input.overlay_plan.overlay_plan_self_hash {
        return failed_output(
            input.input_identity.clone(),
            vec![diagnostic(
                ArenaPlanDiagnosticCode::ArenaInputHashMismatch,
                ArenaPlanDiagnosticProvenance::HashMismatch {
                    product: "overlay_plan_self_hash".to_owned(),
                    recorded: input.input_identity.overlay_plan_self_hash,
                    computed: input.overlay_plan.overlay_plan_self_hash,
                },
            )],
        );
    }
    if input.sram_page_plan.geometry != input.policy.persistent_page_geometry {
        return failed_output(
            input.input_identity.clone(),
            vec![diagnostic(
                ArenaPlanDiagnosticCode::ArenaPersistentPageGeometryMismatch,
                ArenaPlanDiagnosticProvenance::Geometry {
                    observed_header_bytes: input.sram_page_plan.geometry.header_bytes,
                    observed_payload_bytes: input.sram_page_plan.geometry.payload_bytes,
                    observed_commit_word_bytes: input.sram_page_plan.geometry.commit_word_bytes,
                    observed_alignment: input.sram_page_plan.geometry.alignment,
                    expected_header_bytes: input.policy.persistent_page_geometry.header_bytes,
                    expected_payload_bytes: input.policy.persistent_page_geometry.payload_bytes,
                    expected_commit_word_bytes: input
                        .policy
                        .persistent_page_geometry
                        .commit_word_bytes,
                    expected_alignment: input.policy.persistent_page_geometry.alignment,
                },
            )],
        );
    }
    if input.policy.arena_alignment_default == 0 {
        return failed_output(
            input.input_identity.clone(),
            vec![diagnostic(
                ArenaPlanDiagnosticCode::ArenaPolicyProjectionMismatch,
                ArenaPlanDiagnosticProvenance::PolicyProjection {
                    field: "arena_alignment_default".to_owned(),
                    detail: "ArenaPlan v1 requires non-zero arena alignment".to_owned(),
                },
            )],
        );
    }

    let mut slot_id = 0u32;
    let mut arena_id = 0u32;
    let mut wram_arenas = Vec::new();
    let mut sram_arenas = Vec::new();
    let mut hram_assignments = Vec::new();
    let mut builders = BTreeMap::<NamedArena, ArenaBuilder>::new();
    let mut materialize_to_slot = Vec::new();
    let mut persist_to_slot_pair = Vec::new();
    let mut overlay_to_arena = Vec::new();
    let mut persistent_witnesses = Vec::new();
    let mut overlay_honor_entries = Vec::new();

    if let Some(diagnostic) = validate_overlay_reservation_input(input) {
        return failed_output(input.input_identity.clone(), vec![diagnostic]);
    }

    let mut wram_cursor = 0u32;
    let mut reservation_total = 0u32;
    for region in &input.overlay_plan.regions {
        let range = byte_range(wram_cursor, u32::from(region.bytes)).ok_or_else(|| {
            diagnostic(
                ArenaPlanDiagnosticCode::ArenaOverlayReservationOverflow,
                ArenaPlanDiagnosticProvenance::Reservation {
                    invariant: "F-Reservation-NoOverflow".to_owned(),
                    total_bytes: wram_cursor.saturating_add(u32::from(region.bytes)),
                    expected_bytes: input.target_profile.wram_usable_bytes,
                },
            )
        });
        let range = match range {
            Ok(range) => range,
            Err(diagnostic) => {
                return failed_output(input.input_identity.clone(), vec![diagnostic]);
            }
        };
        let mut slots = Vec::new();
        for member in &region.members {
            if member.payload_bytes > u32::from(region.bytes) {
                return failed_output(
                    input.input_identity.clone(),
                    vec![diagnostic(
                        ArenaPlanDiagnosticCode::ArenaOverlayReservationOverflow,
                        ArenaPlanDiagnosticProvenance::Slot {
                            invariant: "F-Addr-7".to_owned(),
                            slot_id,
                            observed_bytes: member.payload_bytes,
                            cap_bytes: u32::from(region.bytes),
                        },
                    )],
                );
            }
            slots.push(ArenaSlot {
                id: next_slot_id(&mut slot_id),
                byte_offset: 0,
                size_bytes: member.payload_bytes as u16,
                alias_class_id: None,
                lifetime_class: LifetimeClass::Slice,
                binding_kind: SlotBindingKind::OverlayMember {
                    overlay: region.id,
                    member: member.id.clone(),
                },
                binding_ref: SlotBindingRef::OverlayMember {
                    overlay: region.id,
                    member: member.id.clone(),
                },
            });
        }
        let id = next_arena_id(&mut arena_id);
        wram_arenas.push(ArenaInstance {
            id,
            named: NamedArena::WramOverlayRegion { overlay: region.id },
            byte_range: range,
            backing: ArenaBacking::Wram,
            alignment: input.policy.arena_alignment_default,
            zerofill: input.policy.arena_zerofill_policy,
            slots,
        });
        overlay_to_arena.push(OverlayArenaBinding {
            overlay: region.id,
            arena: id,
        });
        overlay_honor_entries.push(OverlayReservationHonorEntry {
            overlay_id: region.id,
            arena_id: id,
            bytes: region.bytes,
            byte_range: range,
        });
        wram_cursor = wram_cursor.saturating_add(u32::from(region.bytes));
        reservation_total = reservation_total.saturating_add(u32::from(region.bytes));
    }

    if reservation_total != u32::from(input.overlay_plan.reservation.total_bytes) {
        return failed_output(
            input.input_identity.clone(),
            vec![diagnostic(
                ArenaPlanDiagnosticCode::ArenaOverlayReservationCountMismatch,
                ArenaPlanDiagnosticProvenance::Reservation {
                    invariant: "AP-SC-25".to_owned(),
                    total_bytes: reservation_total,
                    expected_bytes: u32::from(input.overlay_plan.reservation.total_bytes),
                },
            )],
        );
    }

    add_runtime_hram_slots(&mut builders, &mut slot_id);
    let storage_by_value = input
        .storage_bindings
        .iter()
        .map(|binding| (binding.binding.value, binding))
        .collect::<BTreeMap<_, _>>();
    if let Err(output) = add_persistent_slots(
        input,
        &storage_by_value,
        &mut builders,
        &mut slot_id,
        &mut persist_to_slot_pair,
        &mut persistent_witnesses,
    ) {
        return output;
    }
    if let Err(output) =
        add_materialized_slots(input, &mut builders, &mut slot_id, &mut materialize_to_slot)
    {
        return output;
    }

    let mut cursors = BackingCursors {
        wram: wram_cursor,
        sram: 0,
        hram: 0,
    };
    for (named, builder) in builders {
        let arena = match finalize_builder(
            input,
            named,
            builder,
            &mut cursors,
            next_arena_id(&mut arena_id),
        ) {
            Ok(arena) => arena,
            Err(output) => return output,
        };
        match arena.backing {
            ArenaBacking::Wram => wram_arenas.push(arena),
            ArenaBacking::Sram => sram_arenas.push(arena),
            ArenaBacking::Hram => hram_assignments.push(arena),
        }
    }

    wram_arenas.sort_by_key(|arena| (arena.byte_range.start, arena.named.clone()));
    sram_arenas.sort_by_key(|arena| (arena.byte_range.start, arena.named.clone()));
    hram_assignments.sort_by_key(|arena| (arena.byte_range.start, arena.named.clone()));
    materialize_to_slot.sort_by_key(|binding| binding.value);
    persist_to_slot_pair
        .sort_by_key(|binding| (binding.sram_page, binding.commit_group, binding.value));
    overlay_to_arena.sort_by_key(|binding| binding.overlay);
    persistent_witnesses
        .sort_by_key(|witness| (witness.sram_page, witness.commit_group, witness.value));

    let overlay_reservation = OverlayReservationHonor {
        total_bytes: u16::try_from(reservation_total).unwrap_or(u16::MAX),
        expected_total_bytes: input.overlay_plan.reservation.total_bytes,
        per_region: overlay_honor_entries,
    };
    let arena_bindings = ArenaBindings {
        materialize_to_slot,
        persist_to_slot_pair,
        overlay_to_arena,
    };
    let mut plan = ArenaPlan {
        identity: input.input_identity.clone(),
        wram_arenas,
        sram_arenas,
        hram_assignments,
        overlay_reservation,
        arena_bindings,
        arena_plan_self_hash: Hash256::ZERO,
    };

    if let Some(diagnostic) = validate_arena_plan_product_surface(&plan) {
        return failed_output(input.input_identity.clone(), vec![diagnostic]);
    }
    let invariants = validate_address_invariants(input, &plan);
    if !invariants.all_pass() {
        return failed_output(
            input.input_identity.clone(),
            vec![diagnostic(
                ArenaPlanDiagnosticCode::ArenaCertAddressInvariantFailed,
                ArenaPlanDiagnosticProvenance::PolicyProjection {
                    field: "address_invariants".to_owned(),
                    detail: "one or more F-Addr invariants are false".to_owned(),
                },
            )],
        );
    }

    let self_hash = match arena_plan_self_hash(&plan) {
        Ok(hash) => hash,
        Err(error) => {
            return failed_output(
                input.input_identity.clone(),
                vec![diagnostic(
                    ArenaPlanDiagnosticCode::ArenaCanonicalSortDrift,
                    ArenaPlanDiagnosticProvenance::PolicyProjection {
                        field: "arena_plan_self_hash".to_owned(),
                        detail: error.to_string(),
                    },
                )],
            );
        }
    };
    plan.arena_plan_self_hash = self_hash;
    let summary = ArenaPlanSummary {
        wram_arena_count: saturating_u16(plan.wram_arenas.len()),
        sram_arena_count: saturating_u16(plan.sram_arenas.len()),
        hram_assignment_count: saturating_u16(plan.hram_assignments.len()),
        slot_count: all_slots(&plan).len() as u32,
        overlay_reserved_bytes: plan.overlay_reservation.total_bytes,
    };
    let mut cert = ArenaCertBody {
        pass_version: ARENA_CERT_PASS_VERSION.to_owned(),
        arena_plan_self_hash: self_hash,
        address_invariants: invariants,
        reservation_honor: plan.overlay_reservation.clone(),
        persistent_page_geometry: ArenaPersistentGeometryWitness {
            geometry: input.policy.persistent_page_geometry,
            pages: persistent_witnesses,
        },
        harness_no_leak: true,
        diagnostics: Vec::new(),
        arena_cert_self_hash: Hash256::ZERO,
    };
    cert.arena_cert_self_hash = match arena_cert_self_hash(&cert) {
        Ok(hash) => hash,
        Err(error) => {
            return failed_output(
                input.input_identity.clone(),
                vec![diagnostic(
                    ArenaPlanDiagnosticCode::ArenaCanonicalSortDrift,
                    ArenaPlanDiagnosticProvenance::PolicyProjection {
                        field: "arena_cert_self_hash".to_owned(),
                        detail: error.to_string(),
                    },
                )],
            );
        }
    };

    ArenaPlanOutput {
        input_identity: input.input_identity.clone(),
        outcome: ArenaPlanOutcome::Succeeded,
        result: Some(ArenaPlanResult {
            product: plan,
            arena_plan_self_hash: self_hash,
            summary,
        }),
        cert: Some(cert),
        diagnostics: Vec::new(),
    }
}

pub fn emit_arena_plan_report(
    output: &ArenaPlanOutput,
) -> Result<ArenaPlanReportEnvelope, ArenaPlanEmitError> {
    let outcome = match output.outcome {
        ArenaPlanOutcome::Succeeded => ReportOutcome::Passed,
        ArenaPlanOutcome::Failed => ReportOutcome::Failed,
    };
    let body = ArenaPlanReportBody {
        pass_version: ARENA_PLAN_PASS_VERSION.to_owned(),
        input_identity: output.input_identity.clone(),
        diagnostics: output.diagnostics.clone(),
        result: output.result.clone(),
    };
    let envelope = ReportEnvelope::new(outcome, body)?.with_computed_self_hash()?;
    round_trip_self_hash(&envelope)?;
    Ok(envelope)
}

pub fn emit_arena_plan_json_bytes(output: &ArenaPlanOutput) -> Result<Vec<u8>, ArenaPlanEmitError> {
    Ok(canonicalize(&emit_arena_plan_report(output)?)?)
}

pub fn emit_arena_cert_report(
    output: &ArenaPlanOutput,
) -> Result<ArenaCertReportEnvelope, ArenaPlanEmitError> {
    let cert = output.cert.clone().ok_or(ArenaPlanEmitError::MissingCert)?;
    let envelope = ReportEnvelope::new(ReportOutcome::Passed, cert)?.with_computed_self_hash()?;
    round_trip_self_hash(&envelope)?;
    Ok(envelope)
}

pub fn emit_arena_cert_json_bytes(output: &ArenaPlanOutput) -> Result<Vec<u8>, ArenaPlanEmitError> {
    Ok(canonicalize(&emit_arena_cert_report(output)?)?)
}

pub fn parse_arena_plan_report_bytes(
    bytes: &[u8],
) -> Result<ArenaPlanReportEnvelope, ReportSelfHashError> {
    serde_json::from_slice(bytes).map_err(|error| ReportSelfHashError::Json {
        reason: error.to_string(),
    })
}

pub fn parse_arena_cert_report_bytes(
    bytes: &[u8],
) -> Result<ArenaCertReportEnvelope, ReportSelfHashError> {
    serde_json::from_slice(bytes).map_err(|error| ReportSelfHashError::Json {
        reason: error.to_string(),
    })
}

pub fn arena_plan_self_hash(plan: &ArenaPlan) -> Result<Hash256, CanonicalJsonError> {
    self_hash_omitting_fields(
        DomainHash::new("gbf-codegen", "ArenaPlan", ARENA_PLAN_SCHEMA_ID, "1.0.0"),
        plan,
        "arena_plan_self_hash",
        &[],
    )
}

pub fn arena_cert_self_hash(cert: &ArenaCertBody) -> Result<Hash256, CanonicalJsonError> {
    self_hash_omitting_fields(
        DomainHash::new("gbf-codegen", "ArenaCert", ARENA_CERT_SCHEMA_ID, "1.0.0"),
        cert,
        "arena_cert_self_hash",
        &["arena_plan_self_hash"],
    )
}

pub fn arena_plan_policy_projection_hash(
    policy: &ArenaPlanPolicyProjection,
) -> Result<Hash256, CanonicalJsonError> {
    DomainHash::new(
        "gbf-codegen",
        "ArenaPlanPolicyProjection",
        ARENA_PLAN_SCHEMA_ID,
        "1.0.0",
    )
    .hash(policy)
}

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ArenaPlanCacheKey(pub Hash256);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArenaPlanCacheKeyInputs {
    pub storage_plan_self_hash: Hash256,
    pub sram_page_plan_self_hash: Hash256,
    pub rom_window_plan_self_hash: Hash256,
    pub overlay_plan_self_hash: Hash256,
    pub runtime_chrome_budget_hash: Hash256,
    pub target_profile_hash: Hash256,
    pub arena_plan_policy_projection_hash: Hash256,
    pub pass_version: String,
    pub crate_feature_set_hash: Hash256,
}

impl ArenaPlanCacheKeyInputs {
    #[must_use]
    pub fn from_input_identity(
        identity: &ArenaPlanInputIdentity,
        crate_feature_set_hash: Hash256,
    ) -> Self {
        Self {
            storage_plan_self_hash: identity.storage_plan_self_hash,
            sram_page_plan_self_hash: identity.sram_page_plan_self_hash,
            rom_window_plan_self_hash: identity.rom_window_plan_self_hash,
            overlay_plan_self_hash: identity.overlay_plan_self_hash,
            runtime_chrome_budget_hash: identity.runtime_chrome_budget_hash,
            target_profile_hash: identity.target_profile_hash,
            arena_plan_policy_projection_hash: identity.arena_plan_policy_projection_hash,
            pass_version: ARENA_PLAN_PASS_VERSION.to_owned(),
            crate_feature_set_hash,
        }
    }

    pub fn cache_key(&self) -> Result<ArenaPlanCacheKey, CanonicalJsonError> {
        arena_plan_cache_key(self)
    }
}

pub fn arena_plan_cache_key(
    inputs: &ArenaPlanCacheKeyInputs,
) -> Result<ArenaPlanCacheKey, CanonicalJsonError> {
    let canonical = canonical_json_bytes_omitting_fields(inputs, &[])?;
    DomainHash::new("gbf-codegen", "StageCacheKey", "arena_plan", "v1")
        .hash_canonical_bytes(&canonical)
        .map(ArenaPlanCacheKey)
}

pub fn run_arena_plan_with_cache(
    cache: &StoreStageCache<'_>,
    input: &ArenaPlanInputs,
    expected_hashes: StoreBackedStageExpectedHashes,
) -> Result<StoreBackedStageRunOutput<ArenaPlanResult>, CodegenStageCacheError> {
    let cache_key = ArenaPlanCacheKeyInputs::from_input_identity(
        &input.input_identity,
        crate_feature_set_hash(),
    )
    .cache_key()
    .map_err(|error| CodegenStageCacheError::StageCacheKey {
        stage_id: "9",
        message: error.to_string(),
    })?;
    let keys = StoreBackedStageCacheKeys::new(
        "9",
        stage9_arena_plan_store_key(cache_key, StoreBackedStageCellKind::Success),
        stage9_arena_plan_store_key(cache_key, StoreBackedStageCellKind::FailureMemo),
    );
    run_store_backed_stage_with_cache(cache, &keys, cache_key.0, expected_hashes, || {
        let output = build_arena_plan(input);
        let report =
            emit_arena_plan_report(&output).map_err(|error| CodegenStageCacheError::StageEmit {
                stage_id: "9",
                message: error.to_string(),
            })?;
        let report_self_hash = report.report_self_hash;
        match output.outcome {
            ArenaPlanOutcome::Succeeded => {
                let product =
                    output
                        .result
                        .ok_or_else(|| CodegenStageCacheError::StageOutputInvariant {
                            stage_id: "9",
                            message: "succeeded output is missing ArenaPlanResult".to_owned(),
                        })?;
                Ok(StoreBackedStageRunResult::Success {
                    product_self_hash: product.arena_plan_self_hash,
                    product,
                    report_self_hash,
                })
            }
            ArenaPlanOutcome::Failed => Ok(StoreBackedStageRunResult::FailureMemo {
                diagnostics: output.diagnostics,
                report_self_hash,
            }),
        }
    })
}

#[derive(Debug)]
pub enum ArenaPlanEmitError {
    Envelope(ReportEnvelopeError),
    SelfHash(ReportSelfHashError),
    Canonical(ReportCanonicalJsonError),
    MissingCert,
}

impl fmt::Display for ArenaPlanEmitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Envelope(error) => write!(f, "arena report envelope failed: {error}"),
            Self::SelfHash(error) => write!(f, "arena report self hash failed: {error}"),
            Self::Canonical(error) => write!(f, "arena report canonicalization failed: {error}"),
            Self::MissingCert => {
                f.write_str("arena cert is mandatory on successful Stage 9 output")
            }
        }
    }
}

impl Error for ArenaPlanEmitError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Envelope(error) => Some(error),
            Self::SelfHash(error) => Some(error),
            Self::Canonical(error) => Some(error),
            Self::MissingCert => None,
        }
    }
}

impl From<ReportEnvelopeError> for ArenaPlanEmitError {
    fn from(error: ReportEnvelopeError) -> Self {
        Self::Envelope(error)
    }
}

impl From<ReportSelfHashError> for ArenaPlanEmitError {
    fn from(error: ReportSelfHashError) -> Self {
        Self::SelfHash(error)
    }
}

impl From<ReportCanonicalJsonError> for ArenaPlanEmitError {
    fn from(error: ReportCanonicalJsonError) -> Self {
        Self::Canonical(error)
    }
}

impl From<ValidationDiagnostic> for ArenaPlanOutput {
    fn from(diagnostic: ValidationDiagnostic) -> Self {
        failed_output(
            ArenaPlanInputIdentity {
                storage_plan_self_hash: Hash256::ZERO,
                sram_page_plan_self_hash: Hash256::ZERO,
                rom_window_plan_self_hash: Hash256::ZERO,
                overlay_plan_self_hash: Hash256::ZERO,
                runtime_chrome_budget_hash: Hash256::ZERO,
                target_profile_hash: Hash256::ZERO,
                arena_plan_policy_projection_hash: Hash256::ZERO,
                determinism: DeterminismClass::Deterministic,
                target_profile_id: TargetProfileId::from("unknown"),
                schema_version: ARENA_PLAN_SCHEMA_VERSION,
            },
            vec![diagnostic],
        )
    }
}

fn input_hash_mismatch_diagnostics(input: &ArenaPlanInputs) -> Vec<ValidationDiagnostic> {
    ARENA_PLAN_INPUT_PRODUCTS
        .iter()
        .copied()
        .filter_map(|product| {
            let recorded = input.input_identity.hash_for_product(product);
            let computed = input.expected_input_hashes.hash_for_product(product);
            (recorded != computed).then(|| {
                diagnostic(
                    ArenaPlanDiagnosticCode::ArenaInputHashMismatch,
                    ArenaPlanDiagnosticProvenance::HashMismatch {
                        product: product.field_name().to_owned(),
                        recorded,
                        computed,
                    },
                )
            })
        })
        .collect()
}

pub fn validate_arena_plan_product_surface(plan: &ArenaPlan) -> Option<ValidationDiagnostic> {
    let value = serde_json::to_value(plan).expect("arena plan serializes");
    validate_arena_plan_json_surface(&value)
}

pub fn validate_arena_plan_json_surface(value: &serde_json::Value) -> Option<ValidationDiagnostic> {
    let text = value.to_string();
    for forbidden in ["AsmIR", "SectionRole", "BankPlacement"] {
        if text.contains(forbidden) {
            return Some(diagnostic(
                ArenaPlanDiagnosticCode::ArenaSectionRoleLeaked,
                ArenaPlanDiagnosticProvenance::JsonPath {
                    json_path: "$".to_owned(),
                    field_or_tag: forbidden.to_owned(),
                },
            ));
        }
    }
    for forbidden in ["SliceId", "LeaseId", "ResourceVector", "CycleBudget"] {
        if text.contains(forbidden) {
            return Some(diagnostic(
                ArenaPlanDiagnosticCode::ArenaSchedulingFieldLeaked,
                ArenaPlanDiagnosticProvenance::JsonPath {
                    json_path: "$".to_owned(),
                    field_or_tag: forbidden.to_owned(),
                },
            ));
        }
    }
    if text.contains("RepairProposal") || text.contains("repair_proposals") {
        return Some(diagnostic(
            ArenaPlanDiagnosticCode::ArenaRepairProvenanceForbidden,
            ArenaPlanDiagnosticProvenance::JsonPath {
                json_path: "$".to_owned(),
                field_or_tag: "repair".to_owned(),
            },
        ));
    }
    None
}

fn validate_arena_plan_report_body(
    body: &ArenaPlanReportBody,
    outcome: ReportOutcome,
) -> Result<(), Vec<ValidationDiagnostic>> {
    let has_hard = body
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Hard);
    let mut diagnostics = Vec::new();
    match outcome {
        ReportOutcome::Passed => {
            if body.result.is_none() || has_hard || body.pass_version != ARENA_PLAN_PASS_VERSION {
                diagnostics.push(report_invariant("arena_plan.passed"));
            }
        }
        ReportOutcome::Failed => {
            if body.result.is_some() || !has_hard || body.pass_version != ARENA_PLAN_PASS_VERSION {
                diagnostics.push(report_invariant("arena_plan.failed"));
            }
        }
    }
    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(diagnostics)
    }
}

fn validate_arena_cert_body(
    body: &ArenaCertBody,
    outcome: ReportOutcome,
) -> Result<(), Vec<ValidationDiagnostic>> {
    let has_hard = body
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Hard);
    if outcome == ReportOutcome::Passed
        && !has_hard
        && body.pass_version == ARENA_CERT_PASS_VERSION
        && body.address_invariants.all_pass()
    {
        Ok(())
    } else {
        Err(vec![report_invariant("arena_cert.passed")])
    }
}

fn diagnostic(
    code: ArenaPlanDiagnosticCode,
    provenance: ArenaPlanDiagnosticProvenance,
) -> ValidationDiagnostic {
    ValidationDiagnostic::hard(
        ValidationOrigin::ArenaPlanConstruction,
        ValidationCode::ArenaPlan { code, provenance },
        ValidationDetail::Field {
            field: format!(
                "arena_plan.diagnostics.{}.{}.detail_template.v1",
                code.as_str(),
                code.name()
            )
            .into(),
        },
        vec![EvidenceRef {
            kind: "ArenaPlanConstruction".to_owned(),
            reference: code.as_str().to_owned(),
            hash: Some(Hash256::ZERO),
        }],
    )
}

fn report_invariant(field: &str) -> ValidationDiagnostic {
    ValidationDiagnostic::hard(
        ValidationOrigin::ArenaPlanConstruction,
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
    input_identity: ArenaPlanInputIdentity,
    diagnostics: Vec<ValidationDiagnostic>,
) -> ArenaPlanOutput {
    ArenaPlanOutput {
        input_identity,
        outcome: ArenaPlanOutcome::Failed,
        result: None,
        cert: None,
        diagnostics,
    }
}

fn add_runtime_hram_slots(builders: &mut BTreeMap<NamedArena, ArenaBuilder>, next_slot: &mut u32) {
    for (named, kind, size) in [
        (
            NamedArena::HramFrameFlags,
            RuntimeFixedKind::HramFrameFlags,
            1,
        ),
        (
            NamedArena::HramBankShadow,
            RuntimeFixedKind::HramBankShadow,
            1,
        ),
        (
            NamedArena::HramFaultCode,
            RuntimeFixedKind::HramFaultCode,
            1,
        ),
        (
            NamedArena::HramSchedulerScratch,
            RuntimeFixedKind::HramSchedulerScratch,
            8,
        ),
        (
            NamedArena::HramYieldRequested,
            RuntimeFixedKind::HramYieldRequested,
            1,
        ),
    ] {
        builder_for(builders, named, ArenaBacking::Hram)
            .slots
            .push(ArenaSlot {
                id: next_slot_id(next_slot),
                byte_offset: 0,
                size_bytes: size,
                alias_class_id: None,
                lifetime_class: LifetimeClass::Session,
                binding_kind: SlotBindingKind::RuntimeFixed { runtime_kind: kind },
                binding_ref: SlotBindingRef::RuntimeFixed { runtime_kind: kind },
            });
    }
}

#[allow(clippy::result_large_err)]
fn add_persistent_slots(
    input: &ArenaPlanInputs,
    storage_by_value: &BTreeMap<ValueId, &ArenaPlanBindingInput>,
    builders: &mut BTreeMap<NamedArena, ArenaBuilder>,
    next_slot: &mut u32,
    persist_to_slot_pair: &mut Vec<PersistSlotPairBinding>,
    persistent_witnesses: &mut Vec<ArenaPersistentPageWitness>,
) -> Result<(), ArenaPlanOutput> {
    let page_size = input
        .policy
        .persistent_page_geometry
        .payload_bytes
        .saturating_add(u32::from(
            input.policy.persistent_page_geometry.header_bytes,
        ))
        .saturating_add(u32::from(
            input.policy.persistent_page_geometry.commit_word_bytes,
        ));
    if page_size > input.target_profile.sram_window_bytes || page_size > u32::from(u16::MAX) {
        return Err(failed_output(
            input.input_identity.clone(),
            vec![diagnostic(
                ArenaPlanDiagnosticCode::ArenaSramSpanForbidden,
                ArenaPlanDiagnosticProvenance::Slot {
                    invariant: "F-Addr-8".to_owned(),
                    slot_id: 0,
                    observed_bytes: page_size,
                    cap_bytes: input.target_profile.sram_window_bytes,
                },
            )],
        ));
    }
    let mut page_stream = BTreeMap::<PageId, SequenceStreamId>::new();
    for binding in &input.sram_page_plan.bindings {
        if let Some(owner) = page_stream.insert(binding.page, binding.sequence_stream)
            && owner != binding.sequence_stream
        {
            return Err(failed_output(
                input.input_identity.clone(),
                vec![diagnostic(
                    ArenaPlanDiagnosticCode::ArenaCrossStreamPageSharing,
                    ArenaPlanDiagnosticProvenance::Binding {
                        invariant: "AP-SC-2".to_owned(),
                        binding_id: binding.binding_id.get(),
                    },
                )],
            ));
        }
        let Some(storage_input) = storage_by_value.get(&binding.binding_id) else {
            return Err(failed_output(
                input.input_identity.clone(),
                vec![diagnostic(
                    ArenaPlanDiagnosticCode::ArenaPersistentPageStreamMismatch,
                    ArenaPlanDiagnosticProvenance::Binding {
                        invariant: "F-Addr-14".to_owned(),
                        binding_id: binding.binding_id.get(),
                    },
                )],
            ));
        };
        let Materialization::Persist { page, commit_group } = storage_input.binding.materialization
        else {
            continue;
        };
        let slot_a = persistent_slot(next_slot, binding, page_size as u16, true);
        let slot_b = persistent_slot(next_slot, binding, page_size as u16, false);
        let slot_a_id = slot_a.id;
        let slot_b_id = slot_b.id;
        builder_for(
            builders,
            NamedArena::SramSequenceStatePages {
                stream: binding.sequence_stream,
            },
            ArenaBacking::Sram,
        )
        .slots
        .extend([slot_a, slot_b]);
        persist_to_slot_pair.push(PersistSlotPairBinding {
            value: binding.binding_id,
            persist_page: page,
            sram_page: binding.page,
            commit_group,
            slot_a: slot_a_id,
            slot_b: slot_b_id,
        });
        persistent_witnesses.push(ArenaPersistentPageWitness {
            value: binding.binding_id,
            sram_page: binding.page,
            commit_group,
            slot_a: slot_a_id,
            slot_b: slot_b_id,
            slot_size_bytes: page_size as u16,
        });
    }
    Ok(())
}

fn persistent_slot(
    next_slot: &mut u32,
    binding: &SramPageBinding,
    page_size: u16,
    is_a: bool,
) -> ArenaSlot {
    ArenaSlot {
        id: next_slot_id(next_slot),
        byte_offset: 0,
        size_bytes: page_size,
        alias_class_id: None,
        lifetime_class: LifetimeClass::Persistent,
        binding_kind: if is_a {
            SlotBindingKind::PersistentPageA {
                page: binding.page,
                commit_group: binding.commit_group,
            }
        } else {
            SlotBindingKind::PersistentPageB {
                page: binding.page,
                commit_group: binding.commit_group,
            }
        },
        binding_ref: SlotBindingRef::Value {
            value: binding.binding_id,
        },
    }
}

#[allow(clippy::result_large_err)]
fn add_materialized_slots(
    input: &ArenaPlanInputs,
    builders: &mut BTreeMap<NamedArena, ArenaBuilder>,
    next_slot: &mut u32,
    materialize_to_slot: &mut Vec<MaterializeSlotBinding>,
) -> Result<(), ArenaPlanOutput> {
    let mut groups = BTreeMap::<(NamedArena, AliasClassId), MaterializeGroup>::new();
    for binding in &input.storage_bindings {
        match &binding.binding.materialization {
            Materialization::Recompute => continue,
            Materialization::Persist { .. } => continue,
            Materialization::Materialize { class, lifetime } => {
                if binding.size_bytes == 0 || binding.size_bytes > u32::from(u16::MAX) {
                    return Err(failed_output(
                        input.input_identity.clone(),
                        vec![diagnostic(
                            ArenaPlanDiagnosticCode::ArenaAllocationFailed,
                            ArenaPlanDiagnosticProvenance::Binding {
                                invariant: "AP-SC-1".to_owned(),
                                binding_id: binding.binding.value.get(),
                            },
                        )],
                    ));
                }
                let Some(named) = arena_for(class.clone(), lifetime.clone()) else {
                    return Err(failed_output(
                        input.input_identity.clone(),
                        vec![diagnostic(
                            ArenaPlanDiagnosticCode::ArenaUnmappedStorageClass,
                            ArenaPlanDiagnosticProvenance::Binding {
                                invariant: "AP-SC-8".to_owned(),
                                binding_id: binding.binding.value.get(),
                            },
                        )],
                    ));
                };
                let key = (named, binding.binding.alias_class);
                let group = groups
                    .entry(key.clone())
                    .or_insert_with(|| MaterializeGroup {
                        backing: backing_for(&key.0),
                        lifetime: lifetime.clone(),
                        size_bytes: binding.size_bytes as u16,
                        values: Vec::new(),
                    });
                if group.lifetime != lifetime.clone() {
                    return Err(failed_output(
                        input.input_identity.clone(),
                        vec![diagnostic(
                            ArenaPlanDiagnosticCode::ArenaLifetimeClassMismatch,
                            ArenaPlanDiagnosticProvenance::AliasClass {
                                invariant: "AP-SC-7".to_owned(),
                                alias_class_id: binding.binding.alias_class.0,
                            },
                        )],
                    ));
                }
                group.size_bytes = group.size_bytes.max(binding.size_bytes as u16);
                group.values.push(binding.binding.value);
            }
        }
    }

    for ((named, alias_class), mut group) in groups {
        group.values.sort();
        let slot_id = next_slot_id(next_slot);
        for value in &group.values {
            materialize_to_slot.push(MaterializeSlotBinding {
                value: *value,
                slot: slot_id,
            });
        }
        builder_for(builders, named, group.backing)
            .slots
            .push(ArenaSlot {
                id: slot_id,
                byte_offset: 0,
                size_bytes: group.size_bytes,
                alias_class_id: Some(alias_class),
                lifetime_class: group.lifetime,
                binding_kind: SlotBindingKind::MaterializedValue,
                binding_ref: SlotBindingRef::Value {
                    value: group.values[0],
                },
            });
    }
    Ok(())
}

#[allow(clippy::result_large_err)]
fn finalize_builder(
    input: &ArenaPlanInputs,
    named: NamedArena,
    mut builder: ArenaBuilder,
    cursors: &mut BackingCursors,
    id: ArenaId,
) -> Result<ArenaInstance, ArenaPlanOutput> {
    builder.slots.sort_by_key(|slot| {
        (
            std::cmp::Reverse(slot.size_bytes),
            slot.alias_class_id.map(|id| id.0).unwrap_or(u32::MAX),
            lifetime_priority(slot.lifetime_class.clone()),
            binding_kind_order(&slot.binding_kind),
            binding_ref_order(&slot.binding_ref),
        )
    });
    let mut cursor = 0u32;
    for slot in &mut builder.slots {
        cursor = align_up(cursor, u32::from(input.policy.arena_alignment_default));
        slot.byte_offset = match u16::try_from(cursor) {
            Ok(offset) => offset,
            Err(_) => {
                return Err(failed_output(
                    input.input_identity.clone(),
                    vec![diagnostic(
                        ArenaPlanDiagnosticCode::ArenaAllocationFailed,
                        ArenaPlanDiagnosticProvenance::Slot {
                            invariant: "AP-SC-22".to_owned(),
                            slot_id: slot.id.0,
                            observed_bytes: cursor,
                            cap_bytes: u32::from(u16::MAX),
                        },
                    )],
                ));
            }
        };
        cursor = cursor.saturating_add(u32::from(slot.size_bytes));
    }
    let len = align_up(cursor, u32::from(input.policy.arena_alignment_default));
    let start = match builder.backing {
        ArenaBacking::Wram => take_range(&mut cursors.wram, len),
        ArenaBacking::Sram => take_range(&mut cursors.sram, len),
        ArenaBacking::Hram => take_range(&mut cursors.hram, len),
    };
    let byte_range = match start {
        Some(start) => ByteRange {
            start,
            len: len as u16,
        },
        None => {
            return Err(failed_output(
                input.input_identity.clone(),
                vec![diagnostic(
                    ArenaPlanDiagnosticCode::ArenaCapacityExceeded,
                    ArenaPlanDiagnosticProvenance::Arena {
                        invariant: "AP-SC-16".to_owned(),
                        arena_id: id.0,
                        named: format!("{named:?}"),
                    },
                )],
            ));
        }
    };
    let cap = match builder.backing {
        ArenaBacking::Wram => input.target_profile.wram_usable_bytes,
        ArenaBacking::Sram => input.runtime_chrome_budget.memory_caps.sram_usable_bytes,
        ArenaBacking::Hram => input.target_profile.hram_usable_bytes,
    };
    if byte_range.end() > cap {
        let code = match builder.backing {
            ArenaBacking::Hram => ArenaPlanDiagnosticCode::ArenaHramUsableCapExceeded,
            ArenaBacking::Wram => ArenaPlanDiagnosticCode::ArenaBank0WramOverflow,
            ArenaBacking::Sram => ArenaPlanDiagnosticCode::ArenaCapacityExceeded,
        };
        return Err(failed_output(
            input.input_identity.clone(),
            vec![diagnostic(
                code,
                ArenaPlanDiagnosticProvenance::Slot {
                    invariant: "AP-SC-16".to_owned(),
                    slot_id: 0,
                    observed_bytes: byte_range.end(),
                    cap_bytes: cap,
                },
            )],
        ));
    }
    Ok(ArenaInstance {
        id,
        named,
        byte_range,
        backing: builder.backing,
        alignment: input.policy.arena_alignment_default,
        zerofill: input.policy.arena_zerofill_policy,
        slots: builder.slots,
    })
}

fn validate_overlay_reservation_input(input: &ArenaPlanInputs) -> Option<ValidationDiagnostic> {
    let reservation = &input.overlay_plan.reservation;
    if reservation.per_region.len() != input.overlay_plan.regions.len() {
        return Some(diagnostic(
            ArenaPlanDiagnosticCode::ArenaOverlayReservationCountMismatch,
            ArenaPlanDiagnosticProvenance::Reservation {
                invariant: "AP-SC-25".to_owned(),
                total_bytes: reservation.per_region.len() as u32,
                expected_bytes: input.overlay_plan.regions.len() as u32,
            },
        ));
    }

    let mut seen = BTreeSet::new();
    for entry in &reservation.per_region {
        if !seen.insert(entry.region) {
            return Some(diagnostic(
                ArenaPlanDiagnosticCode::ArenaOverlayReservationCountMismatch,
                ArenaPlanDiagnosticProvenance::Reservation {
                    invariant: "AP-SC-25".to_owned(),
                    total_bytes: reservation.per_region.len() as u32,
                    expected_bytes: seen.len() as u32,
                },
            ));
        }
        let Some(region) = input
            .overlay_plan
            .regions
            .iter()
            .find(|region| region.id == entry.region)
        else {
            return Some(diagnostic(
                ArenaPlanDiagnosticCode::ArenaOverlayReservationCountMismatch,
                ArenaPlanDiagnosticProvenance::Reservation {
                    invariant: "AP-SC-25".to_owned(),
                    total_bytes: u32::from(entry.bytes),
                    expected_bytes: 0,
                },
            ));
        };
        if entry.bytes != region.bytes || entry.reservation_kind != region.reservation_kind {
            return Some(diagnostic(
                ArenaPlanDiagnosticCode::ArenaOverlayReservationCountMismatch,
                ArenaPlanDiagnosticProvenance::Reservation {
                    invariant: "AP-SC-24".to_owned(),
                    total_bytes: u32::from(region.bytes),
                    expected_bytes: u32::from(entry.bytes),
                },
            ));
        }
    }

    let per_region_total = reservation.per_region.iter().fold(0u32, |total, entry| {
        total.saturating_add(u32::from(entry.bytes))
    });
    if per_region_total != u32::from(reservation.total_bytes) {
        return Some(diagnostic(
            ArenaPlanDiagnosticCode::ArenaOverlayReservationCountMismatch,
            ArenaPlanDiagnosticProvenance::Reservation {
                invariant: "AP-SC-25".to_owned(),
                total_bytes: per_region_total,
                expected_bytes: u32::from(reservation.total_bytes),
            },
        ));
    }

    None
}

fn validate_address_invariants(
    input: &ArenaPlanInputs,
    plan: &ArenaPlan,
) -> ArenaAddressInvariants {
    let materialized_values = input
        .storage_bindings
        .iter()
        .filter(|binding| {
            matches!(
                binding.binding.materialization,
                Materialization::Materialize { .. }
            )
        })
        .map(|binding| binding.binding.value)
        .collect::<BTreeSet<_>>();
    let materialize_bound = plan
        .arena_bindings
        .materialize_to_slot
        .iter()
        .map(|binding| binding.value)
        .collect::<BTreeSet<_>>();
    let persist_values = input
        .storage_bindings
        .iter()
        .filter(|binding| {
            matches!(
                binding.binding.materialization,
                Materialization::Persist { .. }
            )
        })
        .map(|binding| binding.binding.value)
        .collect::<BTreeSet<_>>();
    let persist_bound = plan
        .arena_bindings
        .persist_to_slot_pair
        .iter()
        .map(|binding| binding.value)
        .collect::<BTreeSet<_>>();
    let slots = all_slots(plan);
    let slots_with_arena = all_slots_with_arena(plan);
    let slot_ids = slots.iter().map(|slot| slot.id).collect::<BTreeSet<_>>();
    let overlay_exact = overlay_reservation_honored(input, plan);
    let overlay_ranges = plan
        .wram_arenas
        .iter()
        .filter(|arena| matches!(arena.named, NamedArena::WramOverlayRegion { .. }))
        .map(|arena| arena.byte_range)
        .collect::<Vec<_>>();
    let non_overlay_wram = plan
        .wram_arenas
        .iter()
        .filter(|arena| !matches!(arena.named, NamedArena::WramOverlayRegion { .. }))
        .map(|arena| arena.byte_range)
        .collect::<Vec<_>>();
    let overlay_disjoint = overlay_ranges.iter().all(|left| {
        non_overlay_wram
            .iter()
            .all(|right| !left.intersects(*right))
    });
    let overlay_member_start = input.overlay_plan.regions.iter().all(|region| {
        let Some(arena) = plan.wram_arenas.iter().find(|arena| {
            matches!(
                arena.named,
                NamedArena::WramOverlayRegion { overlay } if overlay == region.id
            )
        }) else {
            return false;
        };
        region.members.iter().all(|member| {
            arena.slots.iter().any(|slot| {
                matches!(
                    &slot.binding_kind,
                    SlotBindingKind::OverlayMember { overlay, member: slot_member }
                        if *overlay == region.id && slot_member == &member.id
                ) && slot.byte_offset == 0
                    && u32::from(slot.size_bytes) <= u32::from(region.bytes)
            })
        })
    });
    let sram_no_span = plan.sram_arenas.iter().all(|arena| {
        arena.slots.iter().all(|slot| {
            let window = input.target_profile.sram_window_bytes;
            if window == 0 {
                return false;
            }
            let start_in_window = u32::from(slot.byte_offset) % window;
            start_in_window + u32::from(slot.size_bytes) <= window
        })
    });
    let hram_bound = plan
        .hram_assignments
        .iter()
        .all(|arena| arena.byte_range.end() <= input.target_profile.hram_usable_bytes);
    let alias_total_or_disjoint = alias_total_or_disjoint(&slots_with_arena);
    let lifetime_preserved = lifetime_preserved(input, plan, &slots);
    let continuation_sized = continuation_sized(input, plan);
    let harness_disjoint = harness_disjoint(plan);
    let persistent_page_header_aligned = persistent_page_header_aligned(input, plan);
    let binding_refs_resolve = slots.iter().all(|slot| match slot.binding_ref {
        SlotBindingRef::Value { value } => {
            materialized_values.contains(&value)
                || input
                    .sram_page_plan
                    .bindings
                    .iter()
                    .any(|binding| binding.binding_id == value)
        }
        SlotBindingRef::PersistPage { value, .. } => persist_values.contains(&value),
        SlotBindingRef::OverlayMember { overlay, .. } => input
            .overlay_plan
            .regions
            .iter()
            .any(|region| region.id == overlay),
        SlotBindingRef::RuntimeFixed { .. } | SlotBindingRef::TraceRing { .. } => true,
    });
    ArenaAddressInvariants {
        f_addr_1_materialize_coverage: materialized_values == materialize_bound,
        f_addr_2_persist_coverage: persist_values == persist_bound,
        f_addr_3_alias_total_or_disjoint: alias_total_or_disjoint,
        f_addr_4_lifetime_preservation: lifetime_preserved,
        f_addr_5_overlay_byte_equality: overlay_exact,
        f_addr_6_overlay_disjointness: overlay_disjoint,
        f_addr_7_overlay_member_start_of_region: overlay_member_start,
        f_addr_8_sram_no_span: sram_no_span,
        f_addr_9_hram_page_bound: hram_bound,
        f_addr_10_continuation_sized: continuation_sized,
        f_addr_11_harness_disjoint: harness_disjoint,
        f_addr_12_persistent_page_header_aligned: persistent_page_header_aligned,
        f_addr_13_slot_id_unique: slot_ids.len() == slots.len(),
        f_addr_14_binding_ref_resolves: binding_refs_resolve,
    }
}

fn overlay_reservation_honored(input: &ArenaPlanInputs, plan: &ArenaPlan) -> bool {
    if plan.overlay_reservation.total_bytes != input.overlay_plan.reservation.total_bytes
        || plan.overlay_reservation.expected_total_bytes
            != input.overlay_plan.reservation.total_bytes
        || plan.overlay_reservation.per_region.len()
            != input.overlay_plan.reservation.per_region.len()
    {
        return false;
    }

    input
        .overlay_plan
        .reservation
        .per_region
        .iter()
        .all(|reservation| {
            let matching_entries = plan
                .overlay_reservation
                .per_region
                .iter()
                .filter(|entry| entry.overlay_id == reservation.region)
                .collect::<Vec<_>>();
            let Some(entry) = matching_entries.first() else {
                return false;
            };
            if matching_entries.len() != 1
                || entry.bytes != reservation.bytes
                || entry.byte_range.len != reservation.bytes
            {
                return false;
            }
            plan.wram_arenas.iter().any(|arena| {
                arena.id == entry.arena_id
                    && matches!(
                        arena.named,
                        NamedArena::WramOverlayRegion { overlay } if overlay == reservation.region
                    )
                    && arena.byte_range == entry.byte_range
            })
        })
}

fn alias_total_or_disjoint(slots: &[(&ArenaInstance, &ArenaSlot)]) -> bool {
    for (index, (left_arena, left_slot)) in slots.iter().enumerate() {
        for (right_arena, right_slot) in slots.iter().skip(index + 1) {
            if left_arena.id != right_arena.id {
                continue;
            }
            let left_range = slot_range(left_slot);
            let right_range = slot_range(right_slot);
            if !left_range.intersects(right_range) {
                continue;
            }
            let same_alias = matches!(
                (left_slot.alias_class_id, right_slot.alias_class_id),
                (Some(left_alias), Some(right_alias)) if left_alias == right_alias
            );
            let same_range = left_range == right_range;
            let both_overlay_members = matches!(
                (&left_slot.binding_kind, &right_slot.binding_kind),
                (
                    SlotBindingKind::OverlayMember { overlay: left_overlay, .. },
                    SlotBindingKind::OverlayMember { overlay: right_overlay, .. }
                ) if left_overlay == right_overlay
            );
            if !(both_overlay_members || same_alias && same_range) {
                return false;
            }
        }
    }
    true
}

fn lifetime_preserved(input: &ArenaPlanInputs, plan: &ArenaPlan, slots: &[&ArenaSlot]) -> bool {
    input.storage_bindings.iter().all(|binding| {
        let Materialization::Materialize { lifetime, .. } = &binding.binding.materialization else {
            return true;
        };
        let Some(slot_id) = plan
            .arena_bindings
            .materialize_to_slot
            .iter()
            .find(|slot_binding| slot_binding.value == binding.binding.value)
            .map(|slot_binding| slot_binding.slot)
        else {
            return false;
        };
        let Some(slot) = slots.iter().find(|slot| slot.id == slot_id) else {
            return false;
        };
        &slot.lifetime_class == lifetime
    })
}

fn continuation_sized(input: &ArenaPlanInputs, plan: &ArenaPlan) -> bool {
    let session_materialization = input.storage_bindings.iter().any(|binding| {
        matches!(
            &binding.binding.materialization,
            Materialization::Materialize {
                class: StorageClass::WramHot,
                lifetime: LifetimeClass::Session
            }
        )
    });
    let continuation_arenas = plan
        .wram_arenas
        .iter()
        .filter(|arena| matches!(arena.named, NamedArena::WramContinuationRecord))
        .collect::<Vec<_>>();
    match (session_materialization, continuation_arenas.as_slice()) {
        (false, []) => true,
        (true, [arena]) => {
            arena.byte_range.len > 0
                && !arena.slots.is_empty()
                && arena.slots.iter().all(|slot| {
                    slot.size_bytes > 0
                        && matches!(slot.binding_kind, SlotBindingKind::MaterializedValue)
                })
        }
        _ => false,
    }
}

fn harness_disjoint(plan: &ArenaPlan) -> bool {
    let harness_ranges = plan
        .sram_arenas
        .iter()
        .filter(|arena| {
            matches!(
                arena.named,
                NamedArena::SramHarnessCommandBlock
                    | NamedArena::SramHarnessResultBlock
                    | NamedArena::SramPersistedTranscript
            )
        })
        .map(|arena| arena.byte_range)
        .collect::<Vec<_>>();
    let data_ranges = plan
        .sram_arenas
        .iter()
        .filter(|arena| {
            matches!(
                arena.named,
                NamedArena::SramSequenceStatePages { .. }
                    | NamedArena::SramColdSpill
                    | NamedArena::SramTracePages
            )
        })
        .map(|arena| arena.byte_range)
        .collect::<Vec<_>>();
    harness_ranges
        .iter()
        .all(|left| data_ranges.iter().all(|right| !left.intersects(*right)))
}

fn persistent_page_header_aligned(input: &ArenaPlanInputs, plan: &ArenaPlan) -> bool {
    let geometry = input.policy.persistent_page_geometry;
    if geometry.alignment == 0 {
        return false;
    }
    let page_size = geometry
        .payload_bytes
        .saturating_add(u32::from(geometry.header_bytes))
        .saturating_add(u32::from(geometry.commit_word_bytes));
    if page_size == 0 || page_size > u32::from(u16::MAX) {
        return false;
    }
    plan.sram_arenas.iter().all(|arena| {
        arena.slots.iter().all(|slot| {
            if !matches!(
                slot.binding_kind,
                SlotBindingKind::PersistentPageA { .. } | SlotBindingKind::PersistentPageB { .. }
            ) {
                return true;
            }
            u32::from(slot.byte_offset) % u32::from(geometry.alignment) == 0
                && u32::from(slot.size_bytes) == page_size
                && u32::from(slot.size_bytes)
                    >= u32::from(geometry.header_bytes) + u32::from(geometry.commit_word_bytes)
                && u32::from(slot.byte_offset) + u32::from(slot.size_bytes)
                    <= u32::from(arena.byte_range.len)
        })
    })
}

fn slot_range(slot: &ArenaSlot) -> ByteRange {
    ByteRange {
        start: slot.byte_offset,
        len: slot.size_bytes,
    }
}

fn all_slots(plan: &ArenaPlan) -> Vec<&ArenaSlot> {
    plan.wram_arenas
        .iter()
        .chain(plan.sram_arenas.iter())
        .chain(plan.hram_assignments.iter())
        .flat_map(|arena| arena.slots.iter())
        .collect()
}

fn all_slots_with_arena(plan: &ArenaPlan) -> Vec<(&ArenaInstance, &ArenaSlot)> {
    plan.wram_arenas
        .iter()
        .chain(plan.sram_arenas.iter())
        .chain(plan.hram_assignments.iter())
        .flat_map(|arena| arena.slots.iter().map(move |slot| (arena, slot)))
        .collect()
}

fn arena_for(class: StorageClass, lifetime: LifetimeClass) -> Option<NamedArena> {
    match (class, lifetime) {
        (StorageClass::WramHot, LifetimeClass::Slice) => Some(NamedArena::WramAccumScratch),
        (StorageClass::WramHot, LifetimeClass::ResumeWindow) => {
            Some(NamedArena::WramActivationsPingA)
        }
        (StorageClass::WramHot, LifetimeClass::Token) => Some(NamedArena::WramActivationsPingB),
        (StorageClass::WramHot, LifetimeClass::Session) => Some(NamedArena::WramContinuationRecord),
        (StorageClass::HramHot, LifetimeClass::Slice | LifetimeClass::Token) => {
            Some(NamedArena::HramSchedulerScratch)
        }
        (StorageClass::SramPaged, LifetimeClass::Session) => Some(NamedArena::SramColdSpill),
        (StorageClass::RomConst, _) | (_, LifetimeClass::Persistent) => None,
        (StorageClass::HramHot, _) | (StorageClass::SramPaged, _) => None,
    }
}

fn backing_for(named: &NamedArena) -> ArenaBacking {
    match named {
        NamedArena::WramActivationsPingA
        | NamedArena::WramActivationsPingB
        | NamedArena::WramAccumScratch
        | NamedArena::WramRouteScratch
        | NamedArena::WramDecodeScratch
        | NamedArena::WramContinuationRecord
        | NamedArena::WramOverlayRegion { .. } => ArenaBacking::Wram,
        NamedArena::SramSequenceStatePages { .. }
        | NamedArena::SramTracePages
        | NamedArena::SramHarnessCommandBlock
        | NamedArena::SramHarnessResultBlock
        | NamedArena::SramPersistedTranscript
        | NamedArena::SramColdSpill => ArenaBacking::Sram,
        NamedArena::HramFrameFlags
        | NamedArena::HramBankShadow
        | NamedArena::HramFaultCode
        | NamedArena::HramSchedulerScratch
        | NamedArena::HramYieldRequested => ArenaBacking::Hram,
    }
}

fn builder_for(
    builders: &mut BTreeMap<NamedArena, ArenaBuilder>,
    named: NamedArena,
    backing: ArenaBacking,
) -> &mut ArenaBuilder {
    builders.entry(named).or_insert_with(|| ArenaBuilder {
        backing,
        slots: Vec::new(),
    })
}

fn next_slot_id(next: &mut u32) -> ArenaSlotId {
    let id = ArenaSlotId(*next);
    *next = next.saturating_add(1);
    id
}

fn next_arena_id(next: &mut u32) -> ArenaId {
    let id = ArenaId(*next);
    *next = next.saturating_add(1);
    id
}

fn byte_range(start: u32, len: u32) -> Option<ByteRange> {
    Some(ByteRange {
        start: u16::try_from(start).ok()?,
        len: u16::try_from(len).ok()?,
    })
}

fn take_range(cursor: &mut u32, len: u32) -> Option<u16> {
    let start = u16::try_from(*cursor).ok()?;
    u16::try_from(len).ok()?;
    *cursor = cursor.saturating_add(len);
    Some(start)
}

fn align_up(value: u32, alignment: u32) -> u32 {
    if alignment <= 1 {
        value
    } else {
        value.div_ceil(alignment).saturating_mul(alignment)
    }
}

fn lifetime_priority(lifetime: LifetimeClass) -> u8 {
    match lifetime {
        LifetimeClass::Slice => 0,
        LifetimeClass::ResumeWindow => 1,
        LifetimeClass::Token => 2,
        LifetimeClass::Session => 3,
        LifetimeClass::Persistent => 4,
    }
}

fn binding_kind_order(kind: &SlotBindingKind) -> u8 {
    match kind {
        SlotBindingKind::MaterializedValue => 0,
        SlotBindingKind::PersistentPageA { .. } => 1,
        SlotBindingKind::PersistentPageB { .. } => 2,
        SlotBindingKind::OverlayMember { .. } => 3,
        SlotBindingKind::RuntimeFixed { .. } => 4,
        SlotBindingKind::TraceRing { .. } => 5,
    }
}

fn binding_ref_order(reference: &SlotBindingRef) -> u32 {
    match reference {
        SlotBindingRef::Value { value } | SlotBindingRef::PersistPage { value, .. } => value.get(),
        SlotBindingRef::OverlayMember { overlay, .. } => overlay.0,
        SlotBindingRef::RuntimeFixed { runtime_kind } => *runtime_kind as u32,
        SlotBindingRef::TraceRing { trace_ring } => trace_ring.0,
    }
}

fn saturating_u16(value: usize) -> u16 {
    u16::try_from(value).unwrap_or(u16::MAX)
}

#[derive(Debug, Clone)]
struct ArenaBuilder {
    backing: ArenaBacking,
    slots: Vec<ArenaSlot>,
}

#[derive(Debug, Clone)]
struct MaterializeGroup {
    backing: ArenaBacking,
    lifetime: LifetimeClass,
    size_bytes: u16,
    values: Vec<ValueId>,
}

#[derive(Debug, Clone, Copy)]
struct BackingCursors {
    wram: u32,
    sram: u32,
    hram: u32,
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use gbf_foundation::{CompileProfileId, KernelSpecId};
    use gbf_policy::{PlacementProfile, RomBudgetSlot, RuntimeMemoryCapSection};
    use gbf_report::ReportOutcome;

    use super::*;
    use crate::overlay_plan::{
        OverlayPlanInputIdentity, OverlayRegion, OverlayReservation, OverlayReservationEntry,
        OverlayReservationKind, WramRegionConstraint,
    };
    use crate::s3::infer_ir::NodeId;
    use crate::sram_page_plan::{PageResidency, SramBudgetTally, SramPagePlanInputIdentity};
    use crate::storage_plan::types::{
        AbstractLiveRange, BindingJustification, DecisionRuleId, PersistPageId,
    };

    #[test]
    fn pass_materialize_persist_overlay_cert_and_report_round_trip() {
        let output = build_arena_plan(&fixture_inputs());
        assert_eq!(
            output.outcome,
            ArenaPlanOutcome::Succeeded,
            "{:?}",
            output.diagnostics
        );
        let result = output.result.as_ref().expect("arena result");
        assert_eq!(result.summary.overlay_reserved_bytes, 128);
        assert_eq!(result.product.overlay_reservation.total_bytes, 128);
        let honor_entry = result
            .product
            .overlay_reservation
            .per_region
            .first()
            .expect("overlay honor entry");
        assert_eq!(honor_entry.overlay_id, OverlayId(0));
        assert_eq!(honor_entry.arena_id, ArenaId(0));
        assert_eq!(honor_entry.bytes, 128);
        assert_eq!(honor_entry.byte_range.len, 128);
        let honor_json = serde_json::to_value(honor_entry).expect("honor entry json");
        assert!(honor_json.get("arena_id").is_some());
        assert!(honor_json.get("bytes").is_some());
        assert!(honor_json.get("byte_range").is_some());
        assert!(honor_json.get("expected_bytes").is_none());
        assert_eq!(result.product.arena_bindings.materialize_to_slot.len(), 2);
        assert_eq!(result.product.arena_bindings.persist_to_slot_pair.len(), 1);

        let alias_slots = result
            .product
            .arena_bindings
            .materialize_to_slot
            .iter()
            .map(|binding| binding.slot)
            .collect::<BTreeSet<_>>();
        assert_eq!(alias_slots.len(), 1);

        let report = emit_arena_plan_json_bytes(&output).expect("report emits");
        let report_again = emit_arena_plan_json_bytes(&output).expect("report emits again");
        assert_eq!(report, report_again);
        let parsed = parse_arena_plan_report_bytes(&report).expect("report parses");
        assert_eq!(parsed.outcome, ReportOutcome::Passed);
        round_trip_self_hash(&parsed).expect("report self hash");

        let cert = emit_arena_cert_json_bytes(&output).expect("cert emits");
        let parsed_cert = parse_arena_cert_report_bytes(&cert).expect("cert parses");
        assert_eq!(parsed_cert.outcome, ReportOutcome::Passed);
        assert!(parsed_cert.body.address_invariants.all_pass());
        assert_eq!(parsed_cert.body.reservation_honor.total_bytes, 128);
        assert_ne!(parsed_cert.body.arena_cert_self_hash, Hash256::ZERO);
    }

    #[test]
    fn reject_hash_geometry_unmapped_and_sram_span() {
        let mut hash_mismatch = fixture_inputs();
        hash_mismatch.expected_input_hashes.overlay_plan_self_hash = hash(99);
        assert_has_code(
            &build_arena_plan(&hash_mismatch),
            ArenaPlanDiagnosticCode::ArenaInputHashMismatch,
        );

        let mut geometry = fixture_inputs();
        geometry.policy.persistent_page_geometry.alignment = 8;
        assert_has_code(
            &build_arena_plan(&geometry),
            ArenaPlanDiagnosticCode::ArenaPersistentPageGeometryMismatch,
        );

        let mut unmapped = fixture_inputs();
        unmapped.storage_bindings.push(ArenaPlanBindingInput {
            binding: storage_binding(
                10,
                Materialization::Materialize {
                    class: StorageClass::RomConst,
                    lifetime: LifetimeClass::Session,
                },
                4,
            ),
            size_bytes: 16,
        });
        assert_has_code(
            &build_arena_plan(&unmapped),
            ArenaPlanDiagnosticCode::ArenaUnmappedStorageClass,
        );

        let mut span = fixture_inputs();
        span.target_profile.sram_window_bytes = 128;
        assert_has_code(
            &build_arena_plan(&span),
            ArenaPlanDiagnosticCode::ArenaSramSpanForbidden,
        );

        let failed_report =
            emit_arena_plan_json_bytes(&build_arena_plan(&span)).expect("failed report emits");
        let parsed = parse_arena_plan_report_bytes(&failed_report).expect("failed report parses");
        assert_eq!(parsed.outcome, ReportOutcome::Failed);
        assert!(parsed.body.result.is_none());
    }

    #[test]
    fn reject_overlay_payload_and_reservation_region_drift() {
        let mut oversized = fixture_inputs();
        oversized.overlay_plan.regions[0].members[0].payload_bytes = 129;
        assert_has_code(
            &build_arena_plan(&oversized),
            ArenaPlanDiagnosticCode::ArenaOverlayReservationOverflow,
        );

        let mut byte_drift = fixture_inputs();
        byte_drift.overlay_plan.reservation.per_region[0].bytes = 64;
        assert_has_code(
            &build_arena_plan(&byte_drift),
            ArenaPlanDiagnosticCode::ArenaOverlayReservationCountMismatch,
        );

        let mut count_drift = fixture_inputs();
        count_drift.overlay_plan.reservation.per_region.clear();
        assert_has_code(
            &build_arena_plan(&count_drift),
            ArenaPlanDiagnosticCode::ArenaOverlayReservationCountMismatch,
        );
    }

    #[test]
    fn address_invariants_reject_alias_lifetime_continuation_harness_and_persist_drift() {
        let input = fixture_inputs();

        let mut alias_plan = successful_plan(&input);
        let arena = alias_plan
            .wram_arenas
            .iter_mut()
            .find(|arena| matches!(arena.named, NamedArena::WramAccumScratch))
            .expect("wram accum arena");
        let mut overlapping = arena.slots[0].clone();
        overlapping.id = ArenaSlotId(10_000);
        overlapping.alias_class_id = Some(AliasClassId(10_000));
        arena.slots.push(overlapping);
        assert!(!validate_address_invariants(&input, &alias_plan).f_addr_3_alias_total_or_disjoint);

        let mut lifetime_plan = successful_plan(&input);
        let materialized_slot = lifetime_plan
            .arena_bindings
            .materialize_to_slot
            .first()
            .expect("materialized binding")
            .slot;
        for arena in &mut lifetime_plan.wram_arenas {
            if let Some(slot) = arena
                .slots
                .iter_mut()
                .find(|slot| slot.id == materialized_slot)
            {
                slot.lifetime_class = LifetimeClass::Token;
            }
        }
        assert!(
            !validate_address_invariants(&input, &lifetime_plan).f_addr_4_lifetime_preservation
        );

        let mut continuation_plan = successful_plan(&input);
        continuation_plan.wram_arenas.push(ArenaInstance {
            id: ArenaId(10_001),
            named: NamedArena::WramContinuationRecord,
            byte_range: ByteRange {
                start: 4096,
                len: 0,
            },
            backing: ArenaBacking::Wram,
            alignment: input.policy.arena_alignment_default,
            zerofill: input.policy.arena_zerofill_policy,
            slots: Vec::new(),
        });
        assert!(
            !validate_address_invariants(&input, &continuation_plan).f_addr_10_continuation_sized
        );

        let mut harness_plan = successful_plan(&input);
        let sequence_range = harness_plan
            .sram_arenas
            .iter()
            .find(|arena| matches!(arena.named, NamedArena::SramSequenceStatePages { .. }))
            .expect("sequence-state arena")
            .byte_range;
        harness_plan.sram_arenas.push(ArenaInstance {
            id: ArenaId(10_002),
            named: NamedArena::SramHarnessCommandBlock,
            byte_range: sequence_range,
            backing: ArenaBacking::Sram,
            alignment: input.policy.arena_alignment_default,
            zerofill: input.policy.arena_zerofill_policy,
            slots: Vec::new(),
        });
        assert!(!validate_address_invariants(&input, &harness_plan).f_addr_11_harness_disjoint);

        let mut persistent_plan = successful_plan(&input);
        for arena in &mut persistent_plan.sram_arenas {
            if let Some(slot) = arena.slots.iter_mut().find(|slot| {
                matches!(
                    slot.binding_kind,
                    SlotBindingKind::PersistentPageA { .. }
                        | SlotBindingKind::PersistentPageB { .. }
                )
            }) {
                slot.byte_offset = 1;
                break;
            }
        }
        assert!(
            !validate_address_invariants(&input, &persistent_plan)
                .f_addr_12_persistent_page_header_aligned
        );
    }

    #[test]
    fn continuation_invariant_accepts_session_materialization_with_nonzero_arena() {
        let mut input = fixture_inputs();
        input.storage_bindings.push(ArenaPlanBindingInput {
            binding: storage_binding(
                11,
                Materialization::Materialize {
                    class: StorageClass::WramHot,
                    lifetime: LifetimeClass::Session,
                },
                11,
            ),
            size_bytes: 32,
        });
        let plan = successful_plan(&input);
        let invariants = validate_address_invariants(&input, &plan);
        assert!(invariants.f_addr_10_continuation_sized);
        assert!(plan.wram_arenas.iter().any(|arena| matches!(
            arena.named,
            NamedArena::WramContinuationRecord
        ) && arena.byte_range.len >= 32));
    }

    #[test]
    fn overlay_reservation_honor_cross_checks_per_region_entries() {
        let input = fixture_inputs();
        let mut plan = successful_plan(&input);
        plan.overlay_reservation.per_region[0].bytes = 64;
        assert!(!validate_address_invariants(&input, &plan).f_addr_5_overlay_byte_equality);

        let mut plan = successful_plan(&input);
        plan.overlay_reservation.per_region[0].arena_id = ArenaId(10_003);
        assert!(!validate_address_invariants(&input, &plan).f_addr_5_overlay_byte_equality);
    }

    #[test]
    fn k12_cache_key_changes_with_overlay_policy_and_features() {
        let identity = fixture_inputs().input_identity;
        let base = ArenaPlanCacheKeyInputs::from_input_identity(&identity, hash(40))
            .cache_key()
            .expect("base key");
        let same = ArenaPlanCacheKeyInputs::from_input_identity(&identity, hash(40))
            .cache_key()
            .expect("same key");
        assert_eq!(base, same);

        let mut changed_overlay = identity.clone();
        changed_overlay.overlay_plan_self_hash = hash(41);
        assert_ne!(
            base,
            ArenaPlanCacheKeyInputs::from_input_identity(&changed_overlay, hash(40))
                .cache_key()
                .expect("overlay key")
        );

        let mut changed_policy = identity;
        changed_policy.arena_plan_policy_projection_hash = hash(42);
        assert_ne!(
            base,
            ArenaPlanCacheKeyInputs::from_input_identity(&changed_policy, hash(40))
                .cache_key()
                .expect("policy key")
        );
        assert_ne!(
            base,
            ArenaPlanCacheKeyInputs::from_input_identity(&changed_policy, hash(43))
                .cache_key()
                .expect("feature key")
        );
    }

    fn assert_has_code(output: &ArenaPlanOutput, expected: ArenaPlanDiagnosticCode) {
        assert_eq!(output.outcome, ArenaPlanOutcome::Failed);
        assert!(
            output.diagnostics.iter().any(|diagnostic| matches!(
                diagnostic.code,
                ValidationCode::ArenaPlan { code, .. } if code == expected
            )),
            "missing {expected:?} in {:?}",
            output.diagnostics
        );
    }

    fn successful_plan(input: &ArenaPlanInputs) -> ArenaPlan {
        let output = build_arena_plan(input);
        assert_eq!(
            output.outcome,
            ArenaPlanOutcome::Succeeded,
            "{:?}",
            output.diagnostics
        );
        output.result.expect("arena result").product
    }

    fn fixture_inputs() -> ArenaPlanInputs {
        let hashes = ArenaPlanInputHashes {
            storage_plan_self_hash: hash(1),
            sram_page_plan_self_hash: hash(2),
            rom_window_plan_self_hash: hash(3),
            overlay_plan_self_hash: hash(4),
            runtime_chrome_budget_hash: hash(5),
            target_profile_hash: hash(6),
            arena_plan_policy_projection_hash: hash(7),
        };
        let geometry = PersistentPageGeometry::dmg_mbc5_8k();
        ArenaPlanInputs {
            input_identity: ArenaPlanInputIdentity {
                storage_plan_self_hash: hashes.storage_plan_self_hash,
                sram_page_plan_self_hash: hashes.sram_page_plan_self_hash,
                rom_window_plan_self_hash: hashes.rom_window_plan_self_hash,
                overlay_plan_self_hash: hashes.overlay_plan_self_hash,
                runtime_chrome_budget_hash: hashes.runtime_chrome_budget_hash,
                target_profile_hash: hashes.target_profile_hash,
                arena_plan_policy_projection_hash: hashes.arena_plan_policy_projection_hash,
                determinism: DeterminismClass::Deterministic,
                target_profile_id: TargetProfileId::from("dmg-mbc5"),
                schema_version: ARENA_PLAN_SCHEMA_VERSION,
            },
            expected_input_hashes: hashes,
            runtime_chrome_budget: runtime_budget(),
            target_profile: ArenaTargetProfileSummary {
                wram_usable_bytes: 8192,
                sram_window_bytes: 8192,
                hram_usable_bytes: 127,
            },
            policy: ArenaPlanPolicyProjection {
                arena_alignment_default: 16,
                arena_zerofill_policy: ArenaZerofillPolicy::ZeroOnBuild,
                persistent_page_geometry: geometry,
            },
            storage_bindings: vec![
                ArenaPlanBindingInput {
                    binding: storage_binding(
                        1,
                        Materialization::Materialize {
                            class: StorageClass::WramHot,
                            lifetime: LifetimeClass::Slice,
                        },
                        7,
                    ),
                    size_bytes: 64,
                },
                ArenaPlanBindingInput {
                    binding: storage_binding(
                        2,
                        Materialization::Materialize {
                            class: StorageClass::WramHot,
                            lifetime: LifetimeClass::Slice,
                        },
                        7,
                    ),
                    size_bytes: 64,
                },
                ArenaPlanBindingInput {
                    binding: storage_binding(
                        3,
                        Materialization::Persist {
                            page: PersistPageId(3),
                            commit_group: CommitGroupId(1),
                        },
                        8,
                    ),
                    size_bytes: 256,
                },
            ],
            sram_page_plan: sram_plan(geometry),
            overlay_plan: overlay_plan(),
        }
    }

    fn sram_plan(geometry: PersistentPageGeometry) -> SramPagePlan {
        SramPagePlan {
            identity: SramPagePlanInputIdentity {
                storage_plan_self_hash: hash(1),
                observation_plan_self_hash: hash(8),
                range_plan_self_hash: hash(9),
                runtime_chrome_budget_hash: hash(5),
                target_profile_hash: hash(6),
                sram_page_plan_policy_projection_hash: hash(10),
                determinism: DeterminismClass::Deterministic,
                schema_version: crate::sram_page_plan::SRAM_PAGE_PLAN_SCHEMA_VERSION,
            },
            bindings: vec![SramPageBinding {
                binding_id: ValueId::from(3),
                page: PageId(0),
                commit_group: CommitGroupId(1),
                residency: PageResidency::FixedPage { page: PageId(0) },
                payload_bytes: 256,
                geometry,
                sequence_stream: SequenceStreamId(1),
            }],
            pages: Vec::new(),
            stream_index: Vec::new(),
            budgets: SramBudgetTally {
                total_bytes: 256,
                cap_bytes: 32768,
                page_count: 1,
                stream_count: 1,
                per_stream: Vec::new(),
            },
            geometry,
            sram_page_plan_self_hash: hash(2),
        }
    }

    fn overlay_plan() -> OverlayPlan {
        OverlayPlan {
            identity: OverlayPlanInputIdentity {
                storage_plan_self_hash: hash(1),
                sram_page_plan_self_hash: hash(2),
                rom_window_plan_self_hash: hash(3),
                runtime_chrome_budget_hash: hash(5),
                target_profile_hash: hash(6),
                overlay_plan_policy_projection_hash: hash(11),
                determinism: DeterminismClass::Deterministic,
                target_profile_id: TargetProfileId::from("dmg-mbc5"),
                schema_version: crate::overlay_plan::OVERLAY_PLAN_SCHEMA_VERSION,
            },
            regions: vec![OverlayRegion {
                id: OverlayId(0),
                bytes: 128,
                constraint: WramRegionConstraint::DmgWramC000Dfff,
                members: vec![crate::overlay_plan::OverlayResident {
                    id: OverlayResidentId::Kernel {
                        kernel: KernelSpecId::from("matvec"),
                    },
                    payload_bytes: 128,
                    reachability: crate::window::RomReachabilityClass::HotPath,
                    source: crate::overlay_plan::OverlaySource::RomWindowOverlayDemand {
                        resident: OverlayResidentId::Kernel {
                            kernel: KernelSpecId::from("matvec"),
                        },
                    },
                }],
                reservation_kind: OverlayReservationKind::WramOverlay,
                reservation_floor_bytes: 128,
                reservation_ceil_bytes: 128,
            }],
            share_classes: Vec::new(),
            installs: Vec::new(),
            reservation: OverlayReservation {
                total_bytes: 128,
                per_region: vec![OverlayReservationEntry {
                    region: OverlayId(0),
                    bytes: 128,
                    reservation_kind: OverlayReservationKind::WramOverlay,
                }],
                cap_bytes: 1024,
                region_max_bytes: 1024,
            },
            overlay_plan_self_hash: hash(4),
        }
    }

    fn storage_binding(value: u32, materialization: Materialization, alias: u32) -> StorageBinding {
        StorageBinding {
            value: ValueId::from(value),
            materialization,
            alias_class: AliasClassId(alias),
            live_range: AbstractLiveRange {
                def_node: NodeId::from(value),
                first_use_node: Some(NodeId::from(value + 1)),
                last_use_node: Some(NodeId::from(value + 2)),
                lifetime_class: LifetimeClass::Slice,
                checkpoint_stable: false,
            },
            justification: BindingJustification::DecisionRule(DecisionRuleId(1)),
        }
    }

    fn runtime_budget() -> RuntimeChromeBudget {
        RuntimeChromeBudget {
            target: TargetProfileId::from("dmg-mbc5"),
            profile: CompileProfileId::from("Bringup"),
            runtime_nucleus_hash: hash(20),
            rom_slots: vec![RomBudgetSlot {
                id: gbf_foundation::BudgetSlotId::from(0u16),
                class: gbf_policy::BudgetSlotClass::Bank0Free,
                usable_bytes: 16 * 1024,
                reserved_slack: 0,
                placement_caps: BTreeSet::from([PlacementProfile::Budgeted]),
            }],
            memory_caps: RuntimeMemoryCapSection {
                wram_usable_bytes: 8192,
                sram_usable_bytes: 32768,
                hram_usable_bytes: 127,
                source_target_profile_hash: hash(6),
            },
            wram_reserved: 1024,
            sram_reserved: 0,
        }
    }

    fn hash(byte: u8) -> Hash256 {
        Hash256::from_bytes([byte; 32])
    }
}
