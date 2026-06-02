//! Stage 7 `SramPagePlan` construction, report, and cache-key surface.

use std::collections::{BTreeMap, BTreeSet};
use std::{error::Error, fmt};

use gbf_foundation::{
    CanonicalJsonError, DomainHash, EvidenceRef, Hash256, SemVer,
    canonical_json_bytes_omitting_fields, self_hash_omitting_fields,
};
use gbf_policy::{
    DiagnosticSeverity, RuntimeChromeBudget, SramKnob, SramPagePlanDiagnosticCode,
    SramPagePlanDiagnosticProvenance, SramSpillPolicy, ValidationCode, ValidationDetail,
    ValidationDiagnostic, ValidationOrigin,
};
use gbf_report::{
    CanonicalJsonError as ReportCanonicalJsonError, ReportBody, ReportEnvelope,
    ReportEnvelopeError, ReportOutcome, ReportSelfHashError, canonicalize, round_trip_self_hash,
};
use serde::{Deserialize, Serialize};

use crate::s1::quant_graph::DeterminismClass;
use crate::s3::infer_ir::{NodeId, ValueId};
use crate::stage_cache::{
    CodegenStageCacheError, StoreBackedStageCacheKeys, StoreBackedStageCellKind,
    StoreBackedStageExpectedHashes, StoreBackedStageRunOutput, StoreBackedStageRunResult,
    crate_feature_set_hash, run_store_backed_stage_with_cache, stage7_sram_page_plan_store_key,
};
use crate::storage_plan::types::{
    BindingJustification, CommitGroupId, Materialization, StorageBinding, StorageClass,
};
use crate::window::NodeAnchorRange;
use gbf_store::stage_cache::StageCache as StoreStageCache;

pub const SRAM_PAGE_PLAN_SCHEMA_ID: &str = "sram_page_plan.v1";
pub const SRAM_CERT_SCHEMA_ID: &str = "sram_cert.v1";
pub const SRAM_PAGE_PLAN_SCHEMA_VERSION: SemVer = SemVer::new(1, 0, 0);
pub const SRAM_CERT_SCHEMA_VERSION: SemVer = SemVer::new(1, 0, 0);
pub const SRAM_PAGE_PLAN_PASS_VERSION: &str = "stage7/v1";
const SYNTHETIC_SPILL_VALUE_BASE: u32 = 0xFFFF_0000;
const SYNTHETIC_SPILL_COMMIT_GROUP_BASE: u32 = 0xFFFF_0000;

pub type SramPagePlanReportEnvelope = ReportEnvelope<SramPagePlanReportBody>;

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PageId(pub u8);

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SequenceStreamId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersistentPageGeometry {
    pub header_bytes: u16,
    pub payload_bytes: u32,
    pub commit_word_bytes: u8,
    pub alignment: u16,
}

impl PersistentPageGeometry {
    #[must_use]
    pub const fn dmg_mbc5_8k() -> Self {
        Self {
            header_bytes: 16,
            payload_bytes: 8 * 1024 - 16 - 2,
            commit_word_bytes: 2,
            alignment: 16,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum PageResidency {
    FixedPage { page: PageId },
    SamePageAsLastMember,
    DistinctFromCommitGroup { commit_group: CommitGroupId },
    AnyPageInBudget,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SramPagePlanInputIdentity {
    pub storage_plan_self_hash: Hash256,
    pub observation_plan_self_hash: Hash256,
    pub range_plan_self_hash: Hash256,
    pub runtime_chrome_budget_hash: Hash256,
    pub target_profile_hash: Hash256,
    pub sram_page_plan_policy_projection_hash: Hash256,
    pub determinism: DeterminismClass,
    pub schema_version: SemVer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SramPagePlanInputHashes {
    pub storage_plan_self_hash: Hash256,
    pub observation_plan_self_hash: Hash256,
    pub range_plan_self_hash: Hash256,
    pub runtime_chrome_budget_hash: Hash256,
    pub target_profile_hash: Hash256,
    pub sram_page_plan_policy_projection_hash: Hash256,
}

impl SramPagePlanInputIdentity {
    #[must_use]
    pub const fn hashes(&self) -> SramPagePlanInputHashes {
        SramPagePlanInputHashes {
            storage_plan_self_hash: self.storage_plan_self_hash,
            observation_plan_self_hash: self.observation_plan_self_hash,
            range_plan_self_hash: self.range_plan_self_hash,
            runtime_chrome_budget_hash: self.runtime_chrome_budget_hash,
            target_profile_hash: self.target_profile_hash,
            sram_page_plan_policy_projection_hash: self.sram_page_plan_policy_projection_hash,
        }
    }

    #[must_use]
    pub fn hash_for_product(&self, product: SramPagePlanInputProduct) -> Hash256 {
        self.hashes().hash_for_product(product)
    }
}

impl SramPagePlanInputHashes {
    #[must_use]
    pub const fn hash_for_product(&self, product: SramPagePlanInputProduct) -> Hash256 {
        match product {
            SramPagePlanInputProduct::StoragePlan => self.storage_plan_self_hash,
            SramPagePlanInputProduct::ObservationPlan => self.observation_plan_self_hash,
            SramPagePlanInputProduct::RangePlan => self.range_plan_self_hash,
            SramPagePlanInputProduct::RuntimeChromeBudget => self.runtime_chrome_budget_hash,
            SramPagePlanInputProduct::TargetProfile => self.target_profile_hash,
            SramPagePlanInputProduct::PolicyProjection => {
                self.sram_page_plan_policy_projection_hash
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum SramPagePlanInputProduct {
    StoragePlan,
    ObservationPlan,
    RangePlan,
    RuntimeChromeBudget,
    TargetProfile,
    PolicyProjection,
}

impl SramPagePlanInputProduct {
    #[must_use]
    pub const fn field_name(self) -> &'static str {
        match self {
            Self::StoragePlan => "storage_plan_self_hash",
            Self::ObservationPlan => "observation_plan_self_hash",
            Self::RangePlan => "range_plan_self_hash",
            Self::RuntimeChromeBudget => "runtime_chrome_budget_hash",
            Self::TargetProfile => "target_profile_hash",
            Self::PolicyProjection => "sram_page_plan_policy_projection_hash",
        }
    }
}

const SRAM_PAGE_PLAN_INPUT_PRODUCTS: [SramPagePlanInputProduct; 6] = [
    SramPagePlanInputProduct::StoragePlan,
    SramPagePlanInputProduct::ObservationPlan,
    SramPagePlanInputProduct::RangePlan,
    SramPagePlanInputProduct::RuntimeChromeBudget,
    SramPagePlanInputProduct::TargetProfile,
    SramPagePlanInputProduct::PolicyProjection,
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SramPagePlanInputs {
    pub input_identity: SramPagePlanInputIdentity,
    pub expected_input_hashes: SramPagePlanInputHashes,
    pub runtime_chrome_budget: RuntimeChromeBudget,
    pub policy: SramKnob,
    pub switch_caps: SramSwitchCaps,
    pub geometry: PersistentPageGeometry,
    pub expected_geometry: PersistentPageGeometry,
    pub epochs: Vec<SramPagePlanEpochInput>,
    pub bindings: Vec<SramPagePlanBindingInput>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SramSwitchCaps {
    pub max_sram_page_switches_per_token: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SramPagePlanEpochInput {
    pub epoch: SramEpochId,
    pub op_range: NodeAnchorRange,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SramPagePlanBindingInput {
    pub binding: StorageBinding,
    pub payload_bytes: u32,
    pub sequence_stream: SequenceStreamId,
    pub residency: PageResidency,
    pub yield_resume: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SramPagePlan {
    pub identity: SramPagePlanInputIdentity,
    pub active_sets: Vec<SramWorkingSet>,
    pub bindings: Vec<SramPageBinding>,
    pub pages: Vec<PersistentPage>,
    pub stream_index: Vec<SramStreamIndexEntry>,
    pub commit_boundaries: Vec<CommitBoundary>,
    pub page_rotations: Vec<PageRotation>,
    pub spill_policy: SpillPolicy,
    pub projections: SramSwitchProjections,
    pub budgets: SramBudgetTally,
    pub geometry: PersistentPageGeometry,
    pub sram_page_plan_self_hash: Hash256,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SramPageBinding {
    pub binding_id: ValueId,
    pub page: PageId,
    pub commit_group: CommitGroupId,
    pub op_range: NodeAnchorRange,
    pub residency_role: SramResidencyRole,
    pub residency: PageResidency,
    pub payload_bytes: u32,
    pub geometry: PersistentPageGeometry,
    pub sequence_stream: SequenceStreamId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersistentPage {
    pub page: PageId,
    pub sequence_stream: SequenceStreamId,
    pub commit_groups: Vec<CommitGroupId>,
    pub payload_bytes: u32,
    pub binding_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SramStreamIndexEntry {
    pub sequence_stream: SequenceStreamId,
    pub pages: Vec<PageId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SramWorkingSet {
    pub epoch: SramEpochId,
    pub op_range: NodeAnchorRange,
    pub bindings: Vec<SramWorkingSetBinding>,
    pub bytes_in_use: u32,
    pub bytes_reserved: u32,
    pub commit_boundaries_in_range: Vec<CommitBoundaryId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SramWorkingSetBinding {
    pub binding: ValueId,
    pub residency_role: SramResidencyRole,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum SramResidencyRole {
    PersistentSequenceState,
    PersistentContinuation,
    PersistentTranscript,
    PersistentHarness,
    PersistentTrace,
    SramPagedScratch,
    SramPagedSpill,
}

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CommitBoundaryId(pub u32);

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SramEpochId(pub u32);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommitBoundary {
    pub id: CommitBoundaryId,
    pub before_epoch: SramEpochId,
    pub after_epoch: SramEpochId,
    pub commit_group: CommitGroupId,
    pub generation_delta: u32,
    pub member_bindings: Vec<ValueId>,
    pub member_pages: Vec<PageId>,
    pub manifest_page: PageId,
    pub serialization_order: Vec<ValueId>,
    pub yield_safe: YieldSafetyClass,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum YieldSafetyClass {
    NoYieldDuringCommit,
    YieldOnlyAfterManifest,
    YieldAcrossPageRotations,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageRotation {
    pub at_epoch_boundary: (SramEpochId, SramEpochId),
    pub from: SramVisiblePage,
    pub to: SramVisiblePage,
    pub triggered_by: PageRotationTrigger,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum SramVisiblePage {
    Unmapped,
    Mapped { page: PageId },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum PageRotationTrigger {
    EpochBoundary,
    CommitGroup { commit_boundary: CommitBoundaryId },
    PersistentRotation { binding: ValueId, generation: u32 },
    Spill { group: SpillGroupId },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SramSwitchProjections {
    pub projected_sram_page_switches_per_token: u16,
    pub upper_bound_per_token: u16,
    pub cap_per_token: u16,
    pub per_phase: Vec<PerPhaseSwitchCount>,
    pub source: SwitchProjectionSource,
}

impl SramSwitchProjections {
    #[must_use]
    pub fn empty() -> Self {
        Self {
            projected_sram_page_switches_per_token: 0,
            upper_bound_per_token: 0,
            cap_per_token: u16::MAX,
            per_phase: Vec::new(),
            source: SwitchProjectionSource::StaticEnumerationAtCommitBoundaries,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PerPhaseSwitchCount {
    pub epoch: SramEpochId,
    pub switches: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum SwitchProjectionSource {
    StaticEnumerationAtCommitBoundaries,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpillPolicy {
    pub default_residency: SpillResidency,
    pub persist_manifest_residency: PersistManifestResidency,
    pub cold_spill_residency: ColdSpillResidency,
    pub preference_order: SpillPreferenceOrder,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum SpillResidency {
    NeverSpill,
    SpillToSram { class: SramSpillClass },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum SramSpillClass {
    DedicatedSpillPage,
    SharedColdPage,
    OverflowGroup { group: SpillGroupId },
}

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SpillGroupId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum PersistManifestResidency {
    SamePageAsLastMember,
    DedicatedManifestPage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ColdSpillResidency {
    NoColdSpill,
    BoundedColdSpill { max_pages: u8 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum SpillPreferenceOrder {
    NoSpill,
    SpillOnPressure,
    SpillEager,
}

impl Default for SpillPolicy {
    fn default() -> Self {
        spill_policy_from_knob(SramSpillPolicy::NoSpill)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SramBudgetTally {
    pub total_bytes: u32,
    pub cap_bytes: u32,
    pub page_count: u32,
    pub stream_count: u32,
    pub per_stream: Vec<SramStreamBudget>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SramStreamBudget {
    pub sequence_stream: SequenceStreamId,
    pub payload_bytes: u32,
    pub page_count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SramPagePlanSummary {
    pub page_count: u32,
    pub stream_count: u32,
    pub total_bytes: u32,
    pub cap_bytes: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SramPagePlanOutput {
    pub input_identity: SramPagePlanInputIdentity,
    pub outcome: SramPagePlanOutcome,
    pub result: Option<SramPagePlan>,
    pub summary: Option<SramPagePlanSummary>,
    pub diagnostics: Vec<ValidationDiagnostic>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SramPagePlanOutcome {
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SramPagePlanReportBody {
    pub input_identity: SramPagePlanInputIdentity,
    pub result: Option<SramPagePlan>,
    pub summary: Option<SramPagePlanSummary>,
    pub diagnostics: Vec<ValidationDiagnostic>,
}

impl ReportBody for SramPagePlanReportBody {
    const REPORT_TYPE: &'static str = "SramPagePlanReport";
    const SCHEMA_ID: &'static str = SRAM_PAGE_PLAN_SCHEMA_ID;
    const SCHEMA_VERSION: &'static str = "1.0.0";

    fn validate_semantics(&self, outcome: ReportOutcome) -> Result<(), Vec<ValidationDiagnostic>> {
        validate_sram_page_plan_report_body(self, outcome)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SramCertBody {
    pub schema: String,
    pub schema_version: SemVer,
    pub cert_outcome: SramCertOutcome,
    pub report_self_hash: Hash256,
    pub claim: SramCertClaim,
    pub evidence: SramCertEvidence,
}

impl ReportBody for SramCertBody {
    const REPORT_TYPE: &'static str = "SramCert";
    const SCHEMA_ID: &'static str = SRAM_CERT_SCHEMA_ID;
    const SCHEMA_VERSION: &'static str = "1.0.0";

    fn validate_semantics(&self, outcome: ReportOutcome) -> Result<(), Vec<ValidationDiagnostic>> {
        validate_sram_cert_body(self, outcome)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum SramCertOutcome {
    Passed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SramCertClaim {
    pub sram_plan_self_hash: Hash256,
    pub single_page_invariant_holds: bool,
    pub all_persists_resolved: bool,
    pub all_sram_paged_resolved: bool,
    pub spill_policy_total: bool,
    pub commit_groups_contiguous: bool,
    pub page_switches_per_token: u16,
    pub page_switches_cap: u16,
    pub page_switches_per_token_within_cap: bool,
    pub isr_persists_yield_safe: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SramCertEvidence {
    pub active_set_count: u32,
    pub page_binding_count: u32,
    pub commit_boundary_count: u32,
    pub page_rotation_count: u32,
    pub persistent_kind_distribution: SramPersistentKindDistribution,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SramPersistentKindDistribution {
    pub sequence_state: u32,
    pub continuation: u32,
    pub transcript: u32,
    pub harness: u32,
    pub trace: u32,
}

pub fn build_sram_page_plan(input: &SramPagePlanInputs) -> SramPagePlanOutput {
    let hash_diagnostics = input_hash_mismatch_diagnostics(input);
    if !hash_diagnostics.is_empty() {
        return failed_output(input.input_identity.clone(), hash_diagnostics);
    }

    if input.geometry != input.expected_geometry {
        return failed_output(
            input.input_identity.clone(),
            vec![diagnostic(
                SramPagePlanDiagnosticCode::SramPageGeometryMismatch,
                SramPagePlanDiagnosticProvenance::Geometry {
                    observed_header_bytes: input.geometry.header_bytes,
                    observed_payload_bytes: input.geometry.payload_bytes,
                    observed_commit_word_bytes: input.geometry.commit_word_bytes,
                    observed_alignment: input.geometry.alignment,
                    expected_header_bytes: input.expected_geometry.header_bytes,
                    expected_payload_bytes: input.expected_geometry.payload_bytes,
                    expected_commit_word_bytes: input.expected_geometry.commit_word_bytes,
                    expected_alignment: input.expected_geometry.alignment,
                },
            )],
        );
    }

    if input.geometry.payload_bytes == 0 || input.geometry.alignment == 0 {
        return failed_output(
            input.input_identity.clone(),
            vec![diagnostic(
                SramPagePlanDiagnosticCode::SramTargetProfileLayoutUnsupported,
                SramPagePlanDiagnosticProvenance::TargetProfileLayout {
                    target_profile_hash: input.input_identity.target_profile_hash,
                    detail: "persistent page payload and alignment must be non-zero".to_owned(),
                },
            )],
        );
    }

    let mut candidates = Vec::new();
    for binding in &input.bindings {
        if !is_sram_relevant(&binding.binding.materialization) {
            continue;
        }
        if binding.payload_bytes == 0 {
            return failed_output(
                input.input_identity.clone(),
                vec![diagnostic(
                    SramPagePlanDiagnosticCode::SramPolicyProjectionMismatch,
                    SramPagePlanDiagnosticProvenance::PolicyProjection {
                        field: "payload_bytes".to_owned(),
                        detail: format!(
                            "binding {} has zero SRAM payload bytes",
                            binding.binding.value.get()
                        ),
                    },
                )],
            );
        }
        if binding.payload_bytes > input.geometry.payload_bytes {
            return failed_output(
                input.input_identity.clone(),
                vec![diagnostic(
                    SramPagePlanDiagnosticCode::SramPageOverflow,
                    SramPagePlanDiagnosticProvenance::Page {
                        invariant: "SPP-SC-3".to_owned(),
                        page: 0,
                        observed_bytes: binding.payload_bytes,
                        cap_bytes: input.geometry.payload_bytes,
                    },
                )],
            );
        }
        if binding.yield_resume && !matches!(binding.residency, PageResidency::SamePageAsLastMember)
        {
            return failed_output(
                input.input_identity.clone(),
                vec![diagnostic(
                    SramPagePlanDiagnosticCode::SramYieldResumeResidencyViolation,
                    SramPagePlanDiagnosticProvenance::Residency {
                        invariant: "SPP-SC-6".to_owned(),
                        binding_id: binding.binding.value.get(),
                        residency: format!("{:?}", binding.residency),
                    },
                )],
            );
        }
        let Some(commit_group) = commit_group_for_binding(binding) else {
            return failed_output(
                input.input_identity.clone(),
                vec![diagnostic(
                    SramPagePlanDiagnosticCode::SramPolicyProjectionMismatch,
                    SramPagePlanDiagnosticProvenance::PolicyProjection {
                        field: "materialization".to_owned(),
                        detail: format!(
                            "binding {} is marked SRAM-relevant without a Stage 7 commit group",
                            binding.binding.value.get()
                        ),
                    },
                )],
            );
        };
        let op_range = match op_range_for_binding(&binding.binding) {
            Ok(op_range) => op_range,
            Err(diagnostic) => {
                return failed_output(input.input_identity.clone(), vec![diagnostic]);
            }
        };
        candidates.push(SramCandidate {
            binding_id: binding.binding.value,
            commit_group,
            sequence_stream: binding.sequence_stream,
            payload_bytes: binding.payload_bytes,
            op_range,
            residency_role: residency_role_for_binding(&binding.binding),
            residency: binding.residency,
        });
    }

    candidates.sort_by_key(|candidate| {
        (
            candidate.sequence_stream,
            candidate.commit_group,
            candidate.binding_id,
        )
    });

    if let Some(diagnostic) = validate_commit_group_streams(&candidates) {
        return failed_output(input.input_identity.clone(), vec![diagnostic]);
    }
    let spill_policy = spill_policy_from_knob(input.policy.spill_policy);
    if let Some(diagnostic) =
        validate_cold_spill_policy(spill_policy, &input.runtime_chrome_budget, input.geometry)
    {
        return failed_output(input.input_identity.clone(), vec![diagnostic]);
    }

    let cap_bytes = input
        .runtime_chrome_budget
        .memory_caps
        .sram_usable_bytes
        .saturating_sub(input.runtime_chrome_budget.sram_reserved);

    let mut bindings = Vec::new();
    let mut page_stream: BTreeMap<PageId, SequenceStreamId> = BTreeMap::new();
    let mut page_bytes: BTreeMap<PageId, u32> = BTreeMap::new();
    let mut page_commit_groups: BTreeMap<PageId, BTreeSet<CommitGroupId>> = BTreeMap::new();
    let mut commit_pages: BTreeMap<CommitGroupId, Vec<PageId>> = BTreeMap::new();
    let mut stream_pages: BTreeMap<SequenceStreamId, BTreeSet<PageId>> = BTreeMap::new();
    let mut stream_bytes: BTreeMap<SequenceStreamId, u32> = BTreeMap::new();
    let mut next_page = 0u16;

    for candidate in candidates {
        let page_result = match candidate.residency {
            PageResidency::FixedPage { page } => Ok(page),
            PageResidency::SamePageAsLastMember => {
                match commit_pages
                    .get(&candidate.commit_group)
                    .and_then(|pages| pages.last().copied())
                {
                    Some(page) => Ok(page),
                    None => allocate_page(&mut next_page).ok_or_else(|| {
                        page_allocation_failure(input.input_identity.target_profile_hash)
                    }),
                }
            }
            PageResidency::DistinctFromCommitGroup { commit_group } => {
                allocate_distinct_page(&mut next_page, commit_pages.get(&commit_group)).ok_or_else(
                    || page_allocation_failure(input.input_identity.target_profile_hash),
                )
            }
            PageResidency::AnyPageInBudget => {
                match commit_pages
                    .get(&candidate.commit_group)
                    .and_then(|pages| pages.last().copied())
                {
                    Some(page) => Ok(page),
                    None => allocate_page(&mut next_page).ok_or_else(|| {
                        page_allocation_failure(input.input_identity.target_profile_hash)
                    }),
                }
            }
        };
        let page = match page_result {
            Ok(page) => page,
            Err(diagnostic) => {
                return failed_output(input.input_identity.clone(), vec![diagnostic]);
            }
        };

        if let Some(owner) = page_stream.get(&page).copied() {
            if owner != candidate.sequence_stream {
                return failed_output(
                    input.input_identity.clone(),
                    vec![diagnostic(
                        SramPagePlanDiagnosticCode::SramCrossStreamPageSharing,
                        SramPagePlanDiagnosticProvenance::CommitGroup {
                            invariant: "SPP-SC-7".to_owned(),
                            commit_group_id: candidate.commit_group.0,
                            sequence_streams: vec![owner.0, candidate.sequence_stream.0],
                        },
                    )],
                );
            }
        } else {
            page_stream.insert(page, candidate.sequence_stream);
        }

        let total_for_page = page_bytes
            .get(&page)
            .copied()
            .unwrap_or(0)
            .saturating_add(candidate.payload_bytes);
        if total_for_page > input.geometry.payload_bytes {
            return failed_output(
                input.input_identity.clone(),
                vec![diagnostic(
                    SramPagePlanDiagnosticCode::SramPageOverflow,
                    SramPagePlanDiagnosticProvenance::Page {
                        invariant: "SPP-SC-3".to_owned(),
                        page: page.0,
                        observed_bytes: total_for_page,
                        cap_bytes: input.geometry.payload_bytes,
                    },
                )],
            );
        }
        page_bytes.insert(page, total_for_page);
        page_commit_groups
            .entry(page)
            .or_default()
            .insert(candidate.commit_group);
        commit_pages
            .entry(candidate.commit_group)
            .or_default()
            .push(page);
        stream_pages
            .entry(candidate.sequence_stream)
            .or_default()
            .insert(page);
        *stream_bytes.entry(candidate.sequence_stream).or_default() += candidate.payload_bytes;

        let resolved_residency = PageResidency::FixedPage { page };
        bindings.push(SramPageBinding {
            binding_id: candidate.binding_id,
            page,
            commit_group: candidate.commit_group,
            op_range: candidate.op_range,
            residency_role: candidate.residency_role,
            residency: resolved_residency,
            payload_bytes: candidate.payload_bytes,
            geometry: input.geometry,
            sequence_stream: candidate.sequence_stream,
        });
    }

    if let Err(diagnostic) = materialize_spill_bindings(
        &mut bindings,
        spill_policy,
        &mut page_bytes,
        &mut page_stream,
        &mut page_commit_groups,
        &mut stream_pages,
        &mut stream_bytes,
        &mut next_page,
        input.geometry,
        input.input_identity.target_profile_hash,
    ) {
        return failed_output(input.input_identity.clone(), vec![diagnostic]);
    }

    let (commit_boundaries, page_rotations, projections) =
        match derive_commit_boundaries_and_rotations(
            &bindings,
            spill_policy,
            input.switch_caps,
            &mut page_bytes,
            &mut page_stream,
            &mut page_commit_groups,
            &mut stream_pages,
            &mut stream_bytes,
            &mut next_page,
            input.geometry,
        ) {
            Ok(derived) => derived,
            Err(diagnostic) => {
                return failed_output(input.input_identity.clone(), vec![diagnostic]);
            }
        };
    if projections.projected_sram_page_switches_per_token > projections.cap_per_token {
        return failed_output(
            input.input_identity.clone(),
            vec![diagnostic(
                SramPagePlanDiagnosticCode::SramPolicyProjectionMismatch,
                SramPagePlanDiagnosticProvenance::PolicyProjection {
                    field: "projections.projected_sram_page_switches_per_token".to_owned(),
                    detail: format!(
                        "projected SRAM page switches per token {} exceeds cap {}",
                        projections.projected_sram_page_switches_per_token,
                        projections.cap_per_token
                    ),
                },
            )],
        );
    }

    let total_bytes = page_bytes.values().copied().sum::<u32>();
    if total_bytes > cap_bytes {
        return failed_output(
            input.input_identity.clone(),
            vec![diagnostic(
                SramPagePlanDiagnosticCode::SramBudgetExceeded,
                SramPagePlanDiagnosticProvenance::Budget {
                    total_bytes,
                    cap_bytes,
                },
            )],
        );
    }

    let pages = page_bytes
        .iter()
        .map(|(page, payload_bytes)| PersistentPage {
            page: *page,
            sequence_stream: page_stream[page],
            commit_groups: page_commit_groups
                .get(page)
                .into_iter()
                .flat_map(|groups| groups.iter().copied())
                .collect(),
            payload_bytes: *payload_bytes,
            binding_count: bindings
                .iter()
                .filter(|binding| binding.page == *page)
                .count() as u32,
        })
        .collect::<Vec<_>>();
    let stream_index = stream_pages
        .iter()
        .map(|(sequence_stream, pages)| SramStreamIndexEntry {
            sequence_stream: *sequence_stream,
            pages: pages.iter().copied().collect(),
        })
        .collect::<Vec<_>>();
    let per_stream = stream_bytes
        .iter()
        .map(|(sequence_stream, payload_bytes)| SramStreamBudget {
            sequence_stream: *sequence_stream,
            payload_bytes: *payload_bytes,
            page_count: stream_pages
                .get(sequence_stream)
                .map(|pages| pages.len() as u32)
                .unwrap_or(0),
        })
        .collect::<Vec<_>>();
    let budgets = SramBudgetTally {
        total_bytes,
        cap_bytes,
        page_count: pages.len() as u32,
        stream_count: stream_index.len() as u32,
        per_stream,
    };
    let summary = SramPagePlanSummary {
        page_count: budgets.page_count,
        stream_count: budgets.stream_count,
        total_bytes,
        cap_bytes,
    };
    let active_sets = match derive_active_sets(
        &bindings,
        &commit_boundaries,
        &input.epochs,
        input.geometry,
        input.input_identity.target_profile_hash,
    ) {
        Ok(active_sets) => active_sets,
        Err(diagnostic) => {
            return failed_output(input.input_identity.clone(), vec![diagnostic]);
        }
    };
    let mut plan = SramPagePlan {
        identity: input.input_identity.clone(),
        active_sets,
        bindings,
        pages,
        stream_index,
        commit_boundaries,
        page_rotations,
        spill_policy,
        projections,
        budgets,
        geometry: input.geometry,
        sram_page_plan_self_hash: Hash256::ZERO,
    };

    if let Some(diagnostic) = validate_sram_page_plan_product_surface(&plan) {
        return failed_output(input.input_identity.clone(), vec![diagnostic]);
    }

    let self_hash = match sram_page_plan_self_hash(&plan) {
        Ok(hash) => hash,
        Err(error) => {
            return failed_output(
                input.input_identity.clone(),
                vec![diagnostic(
                    SramPagePlanDiagnosticCode::SramCanonicalSortDrift,
                    SramPagePlanDiagnosticProvenance::PolicyProjection {
                        field: "sram_page_plan_self_hash".to_owned(),
                        detail: error.to_string(),
                    },
                )],
            );
        }
    };
    plan.sram_page_plan_self_hash = self_hash;

    SramPagePlanOutput {
        input_identity: input.input_identity.clone(),
        outcome: SramPagePlanOutcome::Succeeded,
        result: Some(plan),
        summary: Some(summary),
        diagnostics: Vec::new(),
    }
}

pub fn emit_sram_page_plan_report(
    output: &SramPagePlanOutput,
) -> Result<SramPagePlanReportEnvelope, SramPagePlanEmitError> {
    let outcome = match output.outcome {
        SramPagePlanOutcome::Succeeded => ReportOutcome::Passed,
        SramPagePlanOutcome::Failed => ReportOutcome::Failed,
    };
    let body = SramPagePlanReportBody {
        input_identity: output.input_identity.clone(),
        result: output.result.clone(),
        summary: output.summary,
        diagnostics: output.diagnostics.clone(),
    };
    let envelope = ReportEnvelope::new(outcome, body)?.with_computed_self_hash()?;
    round_trip_self_hash(&envelope)?;
    Ok(envelope)
}

pub fn emit_sram_page_plan_json_bytes(
    output: &SramPagePlanOutput,
) -> Result<Vec<u8>, SramPagePlanEmitError> {
    Ok(canonicalize(&emit_sram_page_plan_report(output)?)?)
}

pub fn emit_sram_cert_report(
    output: &SramPagePlanOutput,
    report_self_hash: Hash256,
) -> Result<Option<SramCertBody>, SramPagePlanEmitError> {
    let Some(body) = build_sram_cert_body(output, report_self_hash) else {
        return Ok(None);
    };
    if let Err(diagnostics) = validate_sram_cert_body(&body, ReportOutcome::Passed) {
        return Err(SramPagePlanEmitError::CertificateInvariant(diagnostics));
    }
    Ok(Some(body))
}

pub fn emit_sram_cert_json_bytes(
    output: &SramPagePlanOutput,
    report_self_hash: Hash256,
) -> Result<Option<Vec<u8>>, SramPagePlanEmitError> {
    emit_sram_cert_report(output, report_self_hash)?
        .map(|body| {
            canonical_json_bytes_omitting_fields(&body, &[])
                .map_err(SramPagePlanEmitError::ProductCanonical)
        })
        .transpose()
}

pub fn parse_sram_page_plan_report_bytes(
    bytes: &[u8],
) -> Result<SramPagePlanReportEnvelope, ReportSelfHashError> {
    serde_json::from_slice(bytes).map_err(|error| ReportSelfHashError::Json {
        reason: error.to_string(),
    })
}

pub fn parse_sram_cert_report_bytes(bytes: &[u8]) -> Result<SramCertBody, ReportSelfHashError> {
    serde_json::from_slice(bytes).map_err(|error| ReportSelfHashError::Json {
        reason: error.to_string(),
    })
}

pub fn sram_page_plan_self_hash(plan: &SramPagePlan) -> Result<Hash256, CanonicalJsonError> {
    self_hash_omitting_fields(
        DomainHash::new(
            "gbf-codegen",
            "SramPagePlan",
            SRAM_PAGE_PLAN_SCHEMA_ID,
            "1.0.0",
        ),
        plan,
        "sram_page_plan_self_hash",
        &[],
    )
}

pub fn runtime_chrome_budget_hash(
    budget: &RuntimeChromeBudget,
) -> Result<Hash256, CanonicalJsonError> {
    DomainHash::new(
        "gbf-codegen",
        "RuntimeChromeBudget",
        SRAM_PAGE_PLAN_SCHEMA_ID,
        "1.0.0",
    )
    .hash(budget)
}

pub fn sram_page_plan_policy_projection_hash(
    knob: &SramKnob,
) -> Result<Hash256, CanonicalJsonError> {
    DomainHash::new(
        "gbf-codegen",
        "SramPagePlanPolicyProjection",
        SRAM_PAGE_PLAN_SCHEMA_ID,
        "1.0.0",
    )
    .hash(knob)
}

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SramPagePlanCacheKey(pub Hash256);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SramPagePlanCacheKeyInputs {
    pub storage_plan_self_hash: Hash256,
    pub observation_plan_self_hash: Hash256,
    pub range_plan_self_hash: Hash256,
    pub runtime_chrome_budget_hash: Hash256,
    pub target_profile_hash: Hash256,
    pub sram_page_plan_policy_projection_hash: Hash256,
    pub pass_version: String,
    pub crate_feature_set_hash: Hash256,
}

impl SramPagePlanCacheKeyInputs {
    #[must_use]
    pub fn from_input_identity(
        identity: &SramPagePlanInputIdentity,
        crate_feature_set_hash: Hash256,
    ) -> Self {
        Self {
            storage_plan_self_hash: identity.storage_plan_self_hash,
            observation_plan_self_hash: identity.observation_plan_self_hash,
            range_plan_self_hash: identity.range_plan_self_hash,
            runtime_chrome_budget_hash: identity.runtime_chrome_budget_hash,
            target_profile_hash: identity.target_profile_hash,
            sram_page_plan_policy_projection_hash: identity.sram_page_plan_policy_projection_hash,
            pass_version: SRAM_PAGE_PLAN_PASS_VERSION.to_owned(),
            crate_feature_set_hash,
        }
    }

    pub fn cache_key(&self) -> Result<SramPagePlanCacheKey, CanonicalJsonError> {
        sram_page_plan_cache_key(self)
    }
}

pub fn sram_page_plan_cache_key(
    inputs: &SramPagePlanCacheKeyInputs,
) -> Result<SramPagePlanCacheKey, CanonicalJsonError> {
    let canonical = canonical_json_bytes_omitting_fields(inputs, &[])?;
    DomainHash::new("gbf-codegen", "StageCacheKey", "sram_page_plan", "v1")
        .hash_canonical_bytes(&canonical)
        .map(SramPagePlanCacheKey)
}

pub fn run_sram_page_plan_with_cache(
    cache: &StoreStageCache<'_>,
    input: &SramPagePlanInputs,
    expected_hashes: StoreBackedStageExpectedHashes,
) -> Result<StoreBackedStageRunOutput<SramPagePlan>, CodegenStageCacheError> {
    let cache_key = SramPagePlanCacheKeyInputs::from_input_identity(
        &input.input_identity,
        crate_feature_set_hash(),
    )
    .cache_key()
    .map_err(|error| CodegenStageCacheError::StageCacheKey {
        stage_id: "7",
        message: error.to_string(),
    })?;
    let keys = StoreBackedStageCacheKeys::new(
        "7",
        stage7_sram_page_plan_store_key(cache_key, StoreBackedStageCellKind::Success),
        stage7_sram_page_plan_store_key(cache_key, StoreBackedStageCellKind::FailureMemo),
    );
    run_store_backed_stage_with_cache(cache, &keys, cache_key.0, expected_hashes, || {
        let output = build_sram_page_plan(input);
        let report = emit_sram_page_plan_report(&output).map_err(|error| {
            CodegenStageCacheError::StageEmit {
                stage_id: "7",
                message: error.to_string(),
            }
        })?;
        let report_self_hash = report.report_self_hash;
        match output.outcome {
            SramPagePlanOutcome::Succeeded => {
                let product =
                    output
                        .result
                        .ok_or_else(|| CodegenStageCacheError::StageOutputInvariant {
                            stage_id: "7",
                            message: "succeeded output is missing SramPagePlan product".to_owned(),
                        })?;
                Ok(StoreBackedStageRunResult::Success {
                    product_self_hash: product.sram_page_plan_self_hash,
                    product,
                    report_self_hash,
                })
            }
            SramPagePlanOutcome::Failed => Ok(StoreBackedStageRunResult::FailureMemo {
                diagnostics: output.diagnostics,
                report_self_hash,
            }),
        }
    })
}

#[derive(Debug)]
pub enum SramPagePlanEmitError {
    Envelope(ReportEnvelopeError),
    SelfHash(ReportSelfHashError),
    Canonical(ReportCanonicalJsonError),
    ProductCanonical(CanonicalJsonError),
    CertificateInvariant(Vec<ValidationDiagnostic>),
}

impl fmt::Display for SramPagePlanEmitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Envelope(error) => write!(f, "sram page report envelope failed: {error}"),
            Self::SelfHash(error) => write!(f, "sram page report self hash failed: {error}"),
            Self::Canonical(error) => {
                write!(f, "sram page report canonicalization failed: {error}")
            }
            Self::ProductCanonical(error) => {
                write!(f, "sram page product canonicalization failed: {error}")
            }
            Self::CertificateInvariant(diagnostics) => {
                write!(
                    f,
                    "sram certificate invariant failed with {} diagnostics",
                    diagnostics.len()
                )
            }
        }
    }
}

impl Error for SramPagePlanEmitError {
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

impl From<ReportEnvelopeError> for SramPagePlanEmitError {
    fn from(error: ReportEnvelopeError) -> Self {
        Self::Envelope(error)
    }
}

impl From<ReportSelfHashError> for SramPagePlanEmitError {
    fn from(error: ReportSelfHashError) -> Self {
        Self::SelfHash(error)
    }
}

impl From<ReportCanonicalJsonError> for SramPagePlanEmitError {
    fn from(error: ReportCanonicalJsonError) -> Self {
        Self::Canonical(error)
    }
}

fn input_hash_mismatch_diagnostics(input: &SramPagePlanInputs) -> Vec<ValidationDiagnostic> {
    SRAM_PAGE_PLAN_INPUT_PRODUCTS
        .iter()
        .copied()
        .filter_map(|product| {
            let recorded = input.input_identity.hash_for_product(product);
            let computed = input.expected_input_hashes.hash_for_product(product);
            (recorded != computed).then(|| {
                diagnostic(
                    SramPagePlanDiagnosticCode::SramInputHashMismatch,
                    SramPagePlanDiagnosticProvenance::HashMismatch {
                        product: product.field_name().to_owned(),
                        recorded,
                        computed,
                    },
                )
            })
        })
        .collect()
}

fn validate_commit_group_streams(candidates: &[SramCandidate]) -> Option<ValidationDiagnostic> {
    let mut streams_by_group: BTreeMap<CommitGroupId, BTreeSet<SequenceStreamId>> = BTreeMap::new();
    for candidate in candidates {
        streams_by_group
            .entry(candidate.commit_group)
            .or_default()
            .insert(candidate.sequence_stream);
    }
    streams_by_group
        .into_iter()
        .find(|(_, streams)| streams.len() > 1)
        .map(|(commit_group, streams)| {
            diagnostic(
                SramPagePlanDiagnosticCode::SramCommitGroupCrossStream,
                SramPagePlanDiagnosticProvenance::CommitGroup {
                    invariant: "SPP-SC-2".to_owned(),
                    commit_group_id: commit_group.0,
                    sequence_streams: streams.into_iter().map(|stream| stream.0).collect(),
                },
            )
        })
}

pub fn validate_sram_page_plan_product_surface(
    plan: &SramPagePlan,
) -> Option<ValidationDiagnostic> {
    let value = serde_json::to_value(plan).expect("sram page plan serializes");
    validate_sram_page_plan_json_surface(&value)
}

pub fn validate_sram_page_plan_json_surface(
    value: &serde_json::Value,
) -> Option<ValidationDiagnostic> {
    let text = value.to_string();
    for forbidden in ["AsmIR", "SectionRole"] {
        if text.contains(forbidden) {
            return Some(diagnostic(
                SramPagePlanDiagnosticCode::SramSectionRoleLeaked,
                SramPagePlanDiagnosticProvenance::JsonPath {
                    json_path: "$".to_owned(),
                    field_or_tag: forbidden.to_owned(),
                },
            ));
        }
    }
    for forbidden in ["SliceId", "LeaseId"] {
        if text.contains(forbidden) {
            return Some(diagnostic(
                SramPagePlanDiagnosticCode::SramSchedulingFieldLeaked,
                SramPagePlanDiagnosticProvenance::JsonPath {
                    json_path: "$".to_owned(),
                    field_or_tag: forbidden.to_owned(),
                },
            ));
        }
    }
    if text.contains("RepairProposal") || text.contains("repair_proposals") {
        return Some(diagnostic(
            SramPagePlanDiagnosticCode::SramRepairProvenanceForbidden,
            SramPagePlanDiagnosticProvenance::JsonPath {
                json_path: "$".to_owned(),
                field_or_tag: "repair".to_owned(),
            },
        ));
    }
    if text.contains("AnyPageInBudget") {
        return Some(diagnostic(
            SramPagePlanDiagnosticCode::SramResidencyUnresolved,
            SramPagePlanDiagnosticProvenance::Residency {
                invariant: "SPP-SC-5".to_owned(),
                binding_id: 0,
                residency: "AnyPageInBudget".to_owned(),
            },
        ));
    }
    None
}

fn validate_sram_page_plan_report_body(
    body: &SramPagePlanReportBody,
    outcome: ReportOutcome,
) -> Result<(), Vec<ValidationDiagnostic>> {
    let has_hard = body
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Hard);
    let mut diagnostics = Vec::new();
    match outcome {
        ReportOutcome::Passed => {
            if body.result.is_none() || body.summary.is_none() || has_hard {
                diagnostics.push(report_invariant("sram_page_plan.passed"));
            }
            if let Some(plan) = &body.result {
                diagnostics.extend(validate_sram_page_plan_semantic_invariants(plan));
            }
        }
        ReportOutcome::Failed => {
            if body.result.is_some() || body.summary.is_some() || !has_hard {
                diagnostics.push(report_invariant("sram_page_plan.failed"));
            }
        }
    }
    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(diagnostics)
    }
}

fn validate_sram_cert_body(
    body: &SramCertBody,
    outcome: ReportOutcome,
) -> Result<(), Vec<ValidationDiagnostic>> {
    let claim = body.claim;
    let valid = matches!(outcome, ReportOutcome::Passed)
        && body.schema == SRAM_CERT_SCHEMA_ID
        && body.schema_version == SRAM_CERT_SCHEMA_VERSION
        && matches!(body.cert_outcome, SramCertOutcome::Passed)
        && body.report_self_hash != Hash256::ZERO
        && claim.sram_plan_self_hash != Hash256::ZERO
        && claim.single_page_invariant_holds
        && claim.all_persists_resolved
        && claim.all_sram_paged_resolved
        && claim.spill_policy_total
        && claim.commit_groups_contiguous
        && claim.page_switches_per_token_within_cap
        && claim.page_switches_per_token <= claim.page_switches_cap
        && claim.isr_persists_yield_safe;
    if valid {
        Ok(())
    } else {
        Err(vec![report_invariant("sram_cert.claim")])
    }
}

#[allow(clippy::result_large_err, clippy::too_many_arguments)]
fn derive_commit_boundaries_and_rotations(
    bindings: &[SramPageBinding],
    spill_policy: SpillPolicy,
    switch_caps: SramSwitchCaps,
    page_bytes: &mut BTreeMap<PageId, u32>,
    page_stream: &mut BTreeMap<PageId, SequenceStreamId>,
    page_commit_groups: &mut BTreeMap<PageId, BTreeSet<CommitGroupId>>,
    stream_pages: &mut BTreeMap<SequenceStreamId, BTreeSet<PageId>>,
    stream_bytes: &mut BTreeMap<SequenceStreamId, u32>,
    next_page: &mut u16,
    _geometry: PersistentPageGeometry,
) -> Result<
    (
        Vec<CommitBoundary>,
        Vec<PageRotation>,
        SramSwitchProjections,
    ),
    ValidationDiagnostic,
> {
    let mut grouped: BTreeMap<CommitGroupId, Vec<&SramPageBinding>> = BTreeMap::new();
    for binding in bindings {
        if binding.residency_role != SramResidencyRole::SramPagedSpill {
            grouped
                .entry(binding.commit_group)
                .or_default()
                .push(binding);
        }
    }

    let mut commit_boundaries = Vec::with_capacity(grouped.len());
    for (index, (commit_group, mut members)) in grouped.into_iter().enumerate() {
        members.sort_by_key(|binding| binding.binding_id);
        let member_bindings = members
            .iter()
            .map(|binding| binding.binding_id)
            .collect::<Vec<_>>();
        let member_pages = members
            .iter()
            .map(|binding| binding.page)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let manifest_page = match spill_policy.persist_manifest_residency {
            PersistManifestResidency::SamePageAsLastMember => member_bindings
                .last()
                .and_then(|binding_id| {
                    members
                        .iter()
                        .find(|binding| binding.binding_id == *binding_id)
                        .map(|binding| binding.page)
                })
                .expect("grouped commit boundary has at least one serialization member"),
            PersistManifestResidency::DedicatedManifestPage => {
                let used_pages = page_bytes.keys().copied().collect::<BTreeSet<_>>();
                let manifest_page = allocate_unused_page(next_page, &used_pages)
                    .ok_or_else(|| manifest_residency_conflict(CommitBoundaryId(index as u32)))?;
                let stream = members
                    .first()
                    .map(|binding| binding.sequence_stream)
                    .expect("grouped commit boundary has at least one member");
                page_bytes.insert(manifest_page, 0);
                page_stream.insert(manifest_page, stream);
                page_commit_groups
                    .entry(manifest_page)
                    .or_default()
                    .insert(commit_group);
                stream_pages
                    .entry(stream)
                    .or_default()
                    .insert(manifest_page);
                stream_bytes.entry(stream).or_default();
                manifest_page
            }
        };
        let yield_safe = if members
            .iter()
            .any(|binding| matches!(binding.residency, PageResidency::FixedPage { .. }))
        {
            YieldSafetyClass::YieldOnlyAfterManifest
        } else {
            YieldSafetyClass::NoYieldDuringCommit
        };
        commit_boundaries.push(CommitBoundary {
            id: CommitBoundaryId(index as u32),
            before_epoch: SramEpochId(index as u32),
            after_epoch: SramEpochId(index.saturating_add(1) as u32),
            commit_group,
            generation_delta: 1,
            member_bindings: member_bindings.clone(),
            member_pages,
            manifest_page,
            serialization_order: member_bindings,
            yield_safe,
        });
    }
    validate_commit_boundary_manifest_placement(&commit_boundaries, spill_policy, bindings)?;

    let mut page_rotations = Vec::new();
    let mut per_phase = Vec::with_capacity(commit_boundaries.len());
    let mut visible_page = SramVisiblePage::Unmapped;
    for boundary in &commit_boundaries {
        let mut switches = 0u16;
        let mut visible_pages = boundary.member_pages.clone();
        if !visible_pages.contains(&boundary.manifest_page) {
            visible_pages.push(boundary.manifest_page);
        }
        for page in visible_pages {
            let next_page = SramVisiblePage::Mapped { page };
            if visible_page != next_page {
                page_rotations.push(PageRotation {
                    at_epoch_boundary: (boundary.before_epoch, boundary.after_epoch),
                    from: visible_page,
                    to: next_page,
                    triggered_by: PageRotationTrigger::CommitGroup {
                        commit_boundary: boundary.id,
                    },
                });
                visible_page = next_page;
                switches = switches.saturating_add(1);
            }
        }
        per_phase.push(PerPhaseSwitchCount {
            epoch: boundary.after_epoch,
            switches,
        });
    }
    let projected_sram_page_switches_per_token = saturating_u16(page_rotations.len());
    let projections = SramSwitchProjections {
        projected_sram_page_switches_per_token,
        upper_bound_per_token: projected_sram_page_switches_per_token,
        cap_per_token: switch_caps.max_sram_page_switches_per_token,
        per_phase,
        source: SwitchProjectionSource::StaticEnumerationAtCommitBoundaries,
    };
    Ok((commit_boundaries, page_rotations, projections))
}

fn validate_sram_page_plan_semantic_invariants(plan: &SramPagePlan) -> Vec<ValidationDiagnostic> {
    let mut diagnostics = Vec::new();
    if !single_page_invariant_holds(plan) {
        diagnostics.push(report_invariant("sram_plan.single_page_invariant"));
    }
    if !commit_groups_contiguous(plan) {
        diagnostics.push(report_invariant("sram_plan.commit_groups_contiguous"));
    }
    if !serialization_order_canonical(plan) {
        diagnostics.push(report_invariant("sram_plan.serialization_order"));
    }
    if !page_rotations_match_projection(plan) {
        diagnostics.push(report_invariant("sram_plan.page_rotations"));
    }
    if !page_switches_within_cap(plan) {
        diagnostics.push(report_invariant("sram_plan.page_switch_cap"));
    }
    if !working_sets_fit(plan) {
        diagnostics.push(report_invariant("sram_plan.working_sets_fit"));
    }
    if !manifest_residency_matches_policy(plan) {
        diagnostics.push(report_invariant("sram_plan.manifest_residency"));
    }
    if !cold_spill_spatial_discipline_holds(plan) {
        diagnostics.push(report_invariant("sram_plan.cold_spill_spatial_discipline"));
    }
    diagnostics
}

fn build_sram_cert_body(
    output: &SramPagePlanOutput,
    report_self_hash: Hash256,
) -> Option<SramCertBody> {
    if output.outcome != SramPagePlanOutcome::Succeeded || !output.diagnostics.is_empty() {
        return None;
    }
    let plan = output.result.as_ref()?;
    let claim = SramCertClaim {
        sram_plan_self_hash: plan.sram_page_plan_self_hash,
        single_page_invariant_holds: single_page_invariant_holds(plan),
        all_persists_resolved: all_persists_resolved(plan),
        all_sram_paged_resolved: all_sram_paged_resolved(plan),
        spill_policy_total: spill_policy_total(plan.spill_policy),
        commit_groups_contiguous: commit_groups_contiguous(plan),
        page_switches_per_token: plan.projections.projected_sram_page_switches_per_token,
        page_switches_cap: plan.projections.cap_per_token,
        page_switches_per_token_within_cap: page_switches_within_cap(plan),
        isr_persists_yield_safe: isr_persists_yield_safe(plan),
    };
    Some(SramCertBody {
        schema: SRAM_CERT_SCHEMA_ID.to_owned(),
        schema_version: SRAM_CERT_SCHEMA_VERSION,
        cert_outcome: SramCertOutcome::Passed,
        report_self_hash,
        claim,
        evidence: SramCertEvidence {
            active_set_count: plan.active_sets.len() as u32,
            page_binding_count: plan.bindings.len() as u32,
            commit_boundary_count: plan.commit_boundaries.len() as u32,
            page_rotation_count: plan.page_rotations.len() as u32,
            persistent_kind_distribution: persistent_kind_distribution(plan),
        },
    })
}

fn single_page_invariant_holds(plan: &SramPagePlan) -> bool {
    let page_by_binding = plan
        .bindings
        .iter()
        .map(|binding| (binding.binding_id, binding.page))
        .collect::<BTreeMap<_, _>>();
    plan.active_sets.iter().all(|active_set| {
        active_set
            .bindings
            .iter()
            .filter_map(|binding| page_by_binding.get(&binding.binding).copied())
            .collect::<BTreeSet<_>>()
            .len()
            <= 1
    })
}

fn all_persists_resolved(plan: &SramPagePlan) -> bool {
    plan.bindings
        .iter()
        .filter(|binding| is_persistent_residency_role(binding.residency_role))
        .all(|binding| matches!(binding.residency, PageResidency::FixedPage { .. }))
}

fn all_sram_paged_resolved(plan: &SramPagePlan) -> bool {
    plan.bindings
        .iter()
        .filter(|binding| is_sram_paged_residency_role(binding.residency_role))
        .all(|binding| matches!(binding.residency, PageResidency::FixedPage { .. }))
}

fn spill_policy_total(spill_policy: SpillPolicy) -> bool {
    matches!(
        (
            spill_policy.default_residency,
            spill_policy.cold_spill_residency,
            spill_policy.preference_order,
        ),
        (
            SpillResidency::NeverSpill,
            ColdSpillResidency::NoColdSpill,
            SpillPreferenceOrder::NoSpill
        ) | (
            SpillResidency::SpillToSram { .. },
            ColdSpillResidency::BoundedColdSpill { .. },
            SpillPreferenceOrder::SpillOnPressure | SpillPreferenceOrder::SpillEager
        )
    )
}

fn commit_groups_contiguous(plan: &SramPagePlan) -> bool {
    let mut seen = BTreeSet::new();
    for (index, boundary) in plan.commit_boundaries.iter().enumerate() {
        if boundary.id != CommitBoundaryId(index as u32)
            || boundary.before_epoch.0.saturating_add(1) != boundary.after_epoch.0
            || !seen.insert(boundary.commit_group)
        {
            return false;
        }
    }
    true
}

fn serialization_order_canonical(plan: &SramPagePlan) -> bool {
    plan.commit_boundaries.iter().all(|boundary| {
        let mut expected = boundary.member_bindings.clone();
        expected.sort();
        boundary.serialization_order == expected
    })
}

fn page_rotations_match_projection(plan: &SramPagePlan) -> bool {
    let per_phase_total = plan
        .projections
        .per_phase
        .iter()
        .map(|phase| u32::from(phase.switches))
        .sum::<u32>();
    u32::from(plan.projections.projected_sram_page_switches_per_token)
        == plan.page_rotations.len() as u32
        && per_phase_total == plan.page_rotations.len() as u32
        && plan
            .page_rotations
            .iter()
            .all(|rotation| rotation.from != rotation.to)
}

fn page_switches_within_cap(plan: &SramPagePlan) -> bool {
    plan.projections.projected_sram_page_switches_per_token <= plan.projections.cap_per_token
        && plan.projections.upper_bound_per_token <= plan.projections.cap_per_token
}

fn working_sets_fit(plan: &SramPagePlan) -> bool {
    let page_size_bytes = plan
        .geometry
        .payload_bytes
        .saturating_add(u32::from(plan.geometry.header_bytes))
        .saturating_add(u32::from(plan.geometry.commit_word_bytes));
    plan.active_sets
        .iter()
        .all(|active_set| active_set.bytes_reserved <= page_size_bytes)
}

fn manifest_residency_matches_policy(plan: &SramPagePlan) -> bool {
    plan.commit_boundaries.iter().all(|boundary| {
        match plan.spill_policy.persist_manifest_residency {
            PersistManifestResidency::SamePageAsLastMember => boundary
                .serialization_order
                .last()
                .and_then(|last| {
                    plan.bindings
                        .iter()
                        .find(|binding| binding.binding_id == *last)
                })
                .is_some_and(|binding| boundary.manifest_page == binding.page),
            PersistManifestResidency::DedicatedManifestPage => {
                !boundary.member_pages.contains(&boundary.manifest_page)
            }
        }
    })
}

fn isr_persists_yield_safe(plan: &SramPagePlan) -> bool {
    plan.commit_boundaries.iter().all(|boundary| {
        matches!(
            boundary.yield_safe,
            YieldSafetyClass::NoYieldDuringCommit
                | YieldSafetyClass::YieldOnlyAfterManifest
                | YieldSafetyClass::YieldAcrossPageRotations
        )
    })
}

fn persistent_kind_distribution(plan: &SramPagePlan) -> SramPersistentKindDistribution {
    SramPersistentKindDistribution {
        sequence_state: plan
            .bindings
            .iter()
            .filter(|binding| binding.residency_role == SramResidencyRole::PersistentSequenceState)
            .count() as u32,
        continuation: plan
            .bindings
            .iter()
            .filter(|binding| binding.residency_role == SramResidencyRole::PersistentContinuation)
            .count() as u32,
        transcript: plan
            .bindings
            .iter()
            .filter(|binding| binding.residency_role == SramResidencyRole::PersistentTranscript)
            .count() as u32,
        harness: plan
            .bindings
            .iter()
            .filter(|binding| binding.residency_role == SramResidencyRole::PersistentHarness)
            .count() as u32,
        trace: plan
            .bindings
            .iter()
            .filter(|binding| binding.residency_role == SramResidencyRole::PersistentTrace)
            .count() as u32,
    }
}

#[must_use]
pub fn is_yield_safe_at(boundary: &CommitBoundary, epoch: SramEpochId) -> bool {
    matches!(
        boundary.yield_safe,
        YieldSafetyClass::YieldAcrossPageRotations
    ) || (matches!(
        boundary.yield_safe,
        YieldSafetyClass::YieldOnlyAfterManifest
    ) && epoch == boundary.after_epoch)
}

fn spill_policy_from_knob(policy: SramSpillPolicy) -> SpillPolicy {
    match policy {
        SramSpillPolicy::NoSpill => SpillPolicy {
            default_residency: SpillResidency::NeverSpill,
            persist_manifest_residency: PersistManifestResidency::SamePageAsLastMember,
            cold_spill_residency: ColdSpillResidency::NoColdSpill,
            preference_order: SpillPreferenceOrder::NoSpill,
        },
        SramSpillPolicy::SpillOnPressure => SpillPolicy {
            default_residency: SpillResidency::SpillToSram {
                class: SramSpillClass::SharedColdPage,
            },
            persist_manifest_residency: PersistManifestResidency::SamePageAsLastMember,
            cold_spill_residency: ColdSpillResidency::BoundedColdSpill { max_pages: 1 },
            preference_order: SpillPreferenceOrder::SpillOnPressure,
        },
        SramSpillPolicy::SpillEager => SpillPolicy {
            default_residency: SpillResidency::SpillToSram {
                class: SramSpillClass::DedicatedSpillPage,
            },
            persist_manifest_residency: PersistManifestResidency::DedicatedManifestPage,
            cold_spill_residency: ColdSpillResidency::BoundedColdSpill { max_pages: 2 },
            preference_order: SpillPreferenceOrder::SpillEager,
        },
    }
}

fn validate_cold_spill_policy(
    spill_policy: SpillPolicy,
    runtime_chrome_budget: &RuntimeChromeBudget,
    geometry: PersistentPageGeometry,
) -> Option<ValidationDiagnostic> {
    let declared_pages = match spill_policy.cold_spill_residency {
        ColdSpillResidency::NoColdSpill => 0,
        ColdSpillResidency::BoundedColdSpill { max_pages } => max_pages,
    };
    let budget_max_pages = cold_spill_budget_max_pages(runtime_chrome_budget, geometry);
    (declared_pages > budget_max_pages).then(|| {
        diagnostic(
            SramPagePlanDiagnosticCode::SramPolicyProjectionMismatch,
            SramPagePlanDiagnosticProvenance::PolicyProjection {
                field: "spill_policy.cold_spill_residency".to_owned(),
                detail: format!(
                    "spill policy declares {declared_pages} cold-spill pages but budget permits {budget_max_pages}"
                ),
            },
        )
    })
}

fn cold_spill_budget_max_pages(
    runtime_chrome_budget: &RuntimeChromeBudget,
    geometry: PersistentPageGeometry,
) -> u8 {
    let page_size_bytes = geometry
        .payload_bytes
        .saturating_add(u32::from(geometry.header_bytes))
        .saturating_add(u32::from(geometry.commit_word_bytes));
    if page_size_bytes == 0 {
        return 0;
    }
    u8::try_from(runtime_chrome_budget.sram_reserved / page_size_bytes).unwrap_or(u8::MAX)
}

#[allow(clippy::result_large_err)]
fn validate_commit_boundary_manifest_placement(
    commit_boundaries: &[CommitBoundary],
    spill_policy: SpillPolicy,
    bindings: &[SramPageBinding],
) -> Result<(), ValidationDiagnostic> {
    let binding_pages = bindings
        .iter()
        .map(|binding| (binding.binding_id, binding.page))
        .collect::<BTreeMap<_, _>>();
    for boundary in commit_boundaries {
        let mut expected_serialization_order = boundary.member_bindings.clone();
        expected_serialization_order.sort();
        if boundary.serialization_order != expected_serialization_order {
            return Err(diagnostic(
                SramPagePlanDiagnosticCode::SramPolicyProjectionMismatch,
                SramPagePlanDiagnosticProvenance::PolicyProjection {
                    field: "commit_boundaries.serialization_order".to_owned(),
                    detail: format!(
                        "commit boundary {} serialization order is not canonical",
                        boundary.id.0
                    ),
                },
            ));
        }

        match spill_policy.persist_manifest_residency {
            PersistManifestResidency::SamePageAsLastMember => {
                let Some(last_binding) = boundary.serialization_order.last() else {
                    return Err(manifest_residency_conflict(boundary.id));
                };
                let Some(last_page) = binding_pages.get(last_binding).copied() else {
                    return Err(manifest_residency_conflict(boundary.id));
                };
                if boundary.manifest_page != last_page
                    || !boundary.member_pages.contains(&boundary.manifest_page)
                {
                    return Err(manifest_residency_conflict(boundary.id));
                }
            }
            PersistManifestResidency::DedicatedManifestPage => {
                if boundary.member_pages.contains(&boundary.manifest_page) {
                    return Err(manifest_residency_conflict(boundary.id));
                }
            }
        }
    }
    Ok(())
}

#[allow(clippy::result_large_err, clippy::too_many_arguments)]
fn materialize_spill_bindings(
    bindings: &mut Vec<SramPageBinding>,
    spill_policy: SpillPolicy,
    page_bytes: &mut BTreeMap<PageId, u32>,
    page_stream: &mut BTreeMap<PageId, SequenceStreamId>,
    page_commit_groups: &mut BTreeMap<PageId, BTreeSet<CommitGroupId>>,
    stream_pages: &mut BTreeMap<SequenceStreamId, BTreeSet<PageId>>,
    stream_bytes: &mut BTreeMap<SequenceStreamId, u32>,
    next_page: &mut u16,
    geometry: PersistentPageGeometry,
    target_profile_hash: Hash256,
) -> Result<(), ValidationDiagnostic> {
    let spill_page_count = spill_page_count_for_policy(spill_policy);
    if spill_page_count == 0 {
        return Ok(());
    }

    let used_pages = page_bytes.keys().copied().collect::<BTreeSet<_>>();
    let op_range = whole_plan_op_range(bindings);
    let spill_stream = bindings
        .first()
        .map(|binding| binding.sequence_stream)
        .unwrap_or(SequenceStreamId(0));
    for index in 0..spill_page_count {
        let binding_id = ValueId::new(SYNTHETIC_SPILL_VALUE_BASE.saturating_add(index));
        if bindings
            .iter()
            .any(|binding| binding.binding_id == binding_id)
        {
            return Err(diagnostic(
                SramPagePlanDiagnosticCode::SramPolicyProjectionMismatch,
                SramPagePlanDiagnosticProvenance::PolicyProjection {
                    field: "spill_policy.default_residency".to_owned(),
                    detail: format!(
                        "synthetic spill binding id {} collides with a storage binding",
                        binding_id.get()
                    ),
                },
            ));
        }

        let page = allocate_unused_page(next_page, &used_pages)
            .ok_or_else(|| page_allocation_failure(target_profile_hash))?;
        let commit_group = CommitGroupId(SYNTHETIC_SPILL_COMMIT_GROUP_BASE.saturating_add(index));
        page_bytes.insert(page, 0);
        page_stream.insert(page, spill_stream);
        page_commit_groups
            .entry(page)
            .or_default()
            .insert(commit_group);
        stream_pages.entry(spill_stream).or_default().insert(page);
        stream_bytes.entry(spill_stream).or_default();
        bindings.push(SramPageBinding {
            binding_id,
            page,
            commit_group,
            op_range,
            residency_role: SramResidencyRole::SramPagedSpill,
            residency: PageResidency::FixedPage { page },
            payload_bytes: 0,
            geometry,
            sequence_stream: spill_stream,
        });
    }
    Ok(())
}

fn spill_page_count_for_policy(spill_policy: SpillPolicy) -> u32 {
    let ColdSpillResidency::BoundedColdSpill { max_pages } = spill_policy.cold_spill_residency
    else {
        return 0;
    };
    match spill_policy.default_residency {
        SpillResidency::NeverSpill => 0,
        SpillResidency::SpillToSram {
            class: SramSpillClass::DedicatedSpillPage,
        } => u32::from(max_pages),
        SpillResidency::SpillToSram {
            class: SramSpillClass::SharedColdPage,
        }
        | SpillResidency::SpillToSram {
            class: SramSpillClass::OverflowGroup { .. },
        } => u32::from(max_pages.min(1)),
    }
}

fn whole_plan_op_range(bindings: &[SramPageBinding]) -> NodeAnchorRange {
    bindings
        .iter()
        .map(|binding| binding.op_range)
        .reduce(range_union)
        .unwrap_or(NodeAnchorRange {
            first_node: NodeId::new(0),
            last_node: NodeId::new(1),
        })
}

#[allow(clippy::result_large_err)]
fn derive_active_sets(
    bindings: &[SramPageBinding],
    commit_boundaries: &[CommitBoundary],
    epoch_inputs: &[SramPagePlanEpochInput],
    geometry: PersistentPageGeometry,
    target_profile_hash: Hash256,
) -> Result<Vec<SramWorkingSet>, ValidationDiagnostic> {
    if bindings
        .iter()
        .all(|binding| binding.residency_role == SramResidencyRole::SramPagedSpill)
    {
        return Ok(Vec::new());
    }

    let epochs = canonical_epochs(bindings, commit_boundaries, epoch_inputs)?;
    let mut active_sets = Vec::with_capacity(epochs.len());
    for epoch in epochs {
        let active_bindings = bindings
            .iter()
            .filter(|binding| binding.residency_role != SramResidencyRole::SramPagedSpill)
            .filter(|binding| ranges_intersect(binding.op_range, epoch.op_range))
            .collect::<Vec<_>>();
        if active_bindings.is_empty() {
            return Err(diagnostic(
                SramPagePlanDiagnosticCode::SramPolicyProjectionMismatch,
                SramPagePlanDiagnosticProvenance::PolicyProjection {
                    field: "active_sets.bindings".to_owned(),
                    detail: format!(
                        "epoch {} has no active SRAM page bindings in range {}..{}",
                        epoch.epoch.0,
                        epoch.op_range.first_node.get(),
                        epoch.op_range.last_node.get()
                    ),
                },
            ));
        }
        let pages = active_bindings
            .iter()
            .map(|binding| binding.page)
            .collect::<BTreeSet<_>>();
        if pages.len() > 1 {
            return Err(diagnostic(
                SramPagePlanDiagnosticCode::SramPolicyProjectionMismatch,
                SramPagePlanDiagnosticProvenance::PolicyProjection {
                    field: "active_sets".to_owned(),
                    detail: format!(
                        "epoch {} demands multiple SRAM pages: {:?}",
                        epoch.epoch.0, pages
                    ),
                },
            ));
        }

        let active_binding_ids = active_bindings
            .iter()
            .map(|binding| binding.binding_id)
            .collect::<BTreeSet<_>>();
        let commit_boundaries_in_range = commit_boundaries
            .iter()
            .filter(|boundary| {
                boundary
                    .member_bindings
                    .iter()
                    .any(|binding| active_binding_ids.contains(binding))
            })
            .map(|boundary| boundary.id)
            .collect::<Vec<_>>();
        let bytes_in_use = active_bindings
            .iter()
            .map(|binding| binding.payload_bytes)
            .sum::<u32>();
        let bytes_reserved = align_up_u32(bytes_in_use, u32::from(geometry.alignment))
            .saturating_add(u32::from(geometry.header_bytes))
            .saturating_add(u32::from(geometry.commit_word_bytes));
        let page_size_bytes = geometry
            .payload_bytes
            .saturating_add(u32::from(geometry.header_bytes))
            .saturating_add(u32::from(geometry.commit_word_bytes));
        if bytes_reserved > page_size_bytes {
            return Err(diagnostic(
                SramPagePlanDiagnosticCode::SramPageOverflow,
                SramPagePlanDiagnosticProvenance::Page {
                    invariant: "F-SPP-WorkingSetByteFit".to_owned(),
                    page: pages.iter().next().copied().unwrap_or(PageId(0)).0,
                    observed_bytes: bytes_reserved,
                    cap_bytes: page_size_bytes,
                },
            ));
        }

        active_sets.push(SramWorkingSet {
            epoch: epoch.epoch,
            op_range: epoch.op_range,
            bindings: active_bindings
                .into_iter()
                .map(|binding| SramWorkingSetBinding {
                    binding: binding.binding_id,
                    residency_role: binding.residency_role,
                })
                .collect(),
            bytes_in_use,
            bytes_reserved,
            commit_boundaries_in_range,
        });
    }

    if bindings
        .iter()
        .all(|binding| binding.residency_role == SramResidencyRole::SramPagedSpill)
        || !active_sets.is_empty()
    {
        Ok(active_sets)
    } else {
        Err(diagnostic(
            SramPagePlanDiagnosticCode::SramPolicyProjectionMismatch,
            SramPagePlanDiagnosticProvenance::PolicyProjection {
                field: "epochs".to_owned(),
                detail: format!(
                    "no SRAM epochs derived for target profile {:?}",
                    target_profile_hash
                ),
            },
        ))
    }
}

#[allow(clippy::result_large_err)]
fn canonical_epochs(
    bindings: &[SramPageBinding],
    commit_boundaries: &[CommitBoundary],
    epoch_inputs: &[SramPagePlanEpochInput],
) -> Result<Vec<SramPagePlanEpochInput>, ValidationDiagnostic> {
    if !epoch_inputs.is_empty() {
        let mut epochs = epoch_inputs.to_vec();
        epochs.sort_by_key(|epoch| (epoch.epoch, epoch.op_range.first_node));
        let mut seen = BTreeSet::new();
        for epoch in &epochs {
            if !seen.insert(epoch.epoch) {
                return Err(diagnostic(
                    SramPagePlanDiagnosticCode::SramPolicyProjectionMismatch,
                    SramPagePlanDiagnosticProvenance::PolicyProjection {
                        field: "epochs.epoch".to_owned(),
                        detail: format!("duplicate SRAM epoch {}", epoch.epoch.0),
                    },
                ));
            }
            validate_node_range(epoch.op_range, "epochs.op_range")?;
        }
        return Ok(epochs);
    }

    let bindings_by_id = bindings
        .iter()
        .map(|binding| (binding.binding_id, binding))
        .collect::<BTreeMap<_, _>>();
    let mut epochs = Vec::with_capacity(commit_boundaries.len());
    for boundary in commit_boundaries {
        let mut range = None;
        for binding_id in &boundary.member_bindings {
            let Some(binding) = bindings_by_id.get(binding_id) else {
                continue;
            };
            range = Some(match range {
                Some(existing) => range_union(existing, binding.op_range),
                None => binding.op_range,
            });
        }
        if let Some(op_range) = range {
            epochs.push(SramPagePlanEpochInput {
                epoch: boundary.before_epoch,
                op_range,
            });
        }
    }
    Ok(epochs)
}

#[allow(clippy::result_large_err)]
fn op_range_for_binding(binding: &StorageBinding) -> Result<NodeAnchorRange, ValidationDiagnostic> {
    let first_node = binding
        .live_range
        .first_use_node
        .map_or(binding.live_range.def_node, |first_use| {
            binding.live_range.def_node.min(first_use)
        });
    let last_node = binding
        .live_range
        .last_use_node
        .or(binding.live_range.first_use_node)
        .unwrap_or(binding.live_range.def_node);
    let minimum_last = NodeId::new(first_node.get().saturating_add(1));
    let op_range = NodeAnchorRange {
        first_node,
        last_node: last_node.max(minimum_last),
    };
    validate_node_range(op_range, "bindings.live_range")?;
    Ok(op_range)
}

#[allow(clippy::result_large_err)]
fn validate_node_range(
    op_range: NodeAnchorRange,
    field: &'static str,
) -> Result<(), ValidationDiagnostic> {
    if op_range.first_node <= op_range.last_node {
        Ok(())
    } else {
        Err(diagnostic(
            SramPagePlanDiagnosticCode::SramPolicyProjectionMismatch,
            SramPagePlanDiagnosticProvenance::PolicyProjection {
                field: field.to_owned(),
                detail: format!(
                    "inverted node range {}..{}",
                    op_range.first_node.get(),
                    op_range.last_node.get()
                ),
            },
        ))
    }
}

fn residency_role_for_binding(binding: &StorageBinding) -> SramResidencyRole {
    match &binding.materialization {
        Materialization::Persist { page, .. } => {
            residency_role_for_persist_binding(page.0, &binding.justification)
        }
        Materialization::Materialize {
            class: StorageClass::SramPaged,
            ..
        } => SramResidencyRole::SramPagedScratch,
        _ => SramResidencyRole::SramPagedScratch,
    }
}

fn residency_role_for_persist_binding(
    persist_page: u32,
    justification: &BindingJustification,
) -> SramResidencyRole {
    match justification {
        BindingJustification::DecisionRule(rule) => match rule.0 {
            3 => SramResidencyRole::PersistentSequenceState,
            5 => SramResidencyRole::PersistentContinuation,
            6 => SramResidencyRole::PersistentTranscript,
            7 => match persist_page & 0xF000_0000 {
                0x4000_0000 => SramResidencyRole::PersistentHarness,
                0x5000_0000 => SramResidencyRole::PersistentTrace,
                _ => SramResidencyRole::PersistentTrace,
            },
            _ => SramResidencyRole::PersistentSequenceState,
        },
        BindingJustification::ForcedRecompute => SramResidencyRole::PersistentSequenceState,
    }
}

fn is_persistent_residency_role(role: SramResidencyRole) -> bool {
    matches!(
        role,
        SramResidencyRole::PersistentSequenceState
            | SramResidencyRole::PersistentContinuation
            | SramResidencyRole::PersistentTranscript
            | SramResidencyRole::PersistentHarness
            | SramResidencyRole::PersistentTrace
    )
}

fn is_sram_paged_residency_role(role: SramResidencyRole) -> bool {
    matches!(
        role,
        SramResidencyRole::SramPagedScratch | SramResidencyRole::SramPagedSpill
    )
}

fn cold_spill_spatial_discipline_holds(plan: &SramPagePlan) -> bool {
    let spill_bindings = plan
        .bindings
        .iter()
        .filter(|binding| binding.residency_role == SramResidencyRole::SramPagedSpill)
        .collect::<Vec<_>>();
    match plan.spill_policy.cold_spill_residency {
        ColdSpillResidency::NoColdSpill => spill_bindings.is_empty(),
        ColdSpillResidency::BoundedColdSpill { max_pages } => {
            let pages = spill_bindings
                .iter()
                .map(|binding| binding.page)
                .collect::<BTreeSet<_>>();
            spill_bindings.len() <= usize::from(max_pages)
                && pages.len() == spill_bindings.len()
                && spill_bindings
                    .iter()
                    .all(|binding| matches!(binding.residency, PageResidency::FixedPage { .. }))
        }
    }
}

fn ranges_intersect(left: NodeAnchorRange, right: NodeAnchorRange) -> bool {
    if left.first_node == left.last_node {
        return point_in_range(left.first_node, right);
    }
    if right.first_node == right.last_node {
        return point_in_range(right.first_node, left);
    }
    left.first_node < right.last_node && right.first_node < left.last_node
}

fn point_in_range(point: NodeId, range: NodeAnchorRange) -> bool {
    if range.first_node == range.last_node {
        point == range.first_node
    } else {
        range.first_node <= point && point < range.last_node
    }
}

fn range_union(left: NodeAnchorRange, right: NodeAnchorRange) -> NodeAnchorRange {
    NodeAnchorRange {
        first_node: left.first_node.min(right.first_node),
        last_node: left.last_node.max(right.last_node),
    }
}

fn align_up_u32(value: u32, alignment: u32) -> u32 {
    if alignment == 0 {
        return value;
    }
    let remainder = value % alignment;
    if remainder == 0 {
        value
    } else {
        value.saturating_add(alignment - remainder)
    }
}

fn diagnostic(
    code: SramPagePlanDiagnosticCode,
    provenance: SramPagePlanDiagnosticProvenance,
) -> ValidationDiagnostic {
    ValidationDiagnostic::hard(
        ValidationOrigin::SramPagePlanConstruction,
        ValidationCode::SramPagePlan { code, provenance },
        ValidationDetail::Field {
            field: format!(
                "sram_page_plan.diagnostics.{}.{}.detail_template.v1",
                code.as_str(),
                code.name()
            )
            .into(),
        },
        vec![EvidenceRef {
            kind: "SramPagePlanConstruction".to_owned(),
            reference: code.as_str().to_owned(),
            hash: Some(Hash256::ZERO),
        }],
    )
}

fn report_invariant(field: &str) -> ValidationDiagnostic {
    ValidationDiagnostic::hard(
        ValidationOrigin::SramPagePlanConstruction,
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
    input_identity: SramPagePlanInputIdentity,
    diagnostics: Vec<ValidationDiagnostic>,
) -> SramPagePlanOutput {
    SramPagePlanOutput {
        input_identity,
        outcome: SramPagePlanOutcome::Failed,
        result: None,
        summary: None,
        diagnostics,
    }
}

fn is_sram_relevant(materialization: &Materialization) -> bool {
    matches!(
        materialization,
        Materialization::Persist { .. }
            | Materialization::Materialize {
                class: StorageClass::SramPaged,
                ..
            }
    )
}

fn commit_group_for_binding(binding: &SramPagePlanBindingInput) -> Option<CommitGroupId> {
    match binding.binding.materialization {
        Materialization::Persist { commit_group, .. } => Some(commit_group),
        Materialization::Materialize {
            class: StorageClass::SramPaged,
            ..
        } => Some(CommitGroupId(
            0x8000_0000u32.saturating_add(binding.binding.value.get()),
        )),
        _ => None,
    }
}

fn allocate_page(next_page: &mut u16) -> Option<PageId> {
    if *next_page > u16::from(u8::MAX) {
        return None;
    }
    let page = PageId(*next_page as u8);
    *next_page = next_page.saturating_add(1);
    Some(page)
}

fn allocate_distinct_page(next_page: &mut u16, forbidden: Option<&Vec<PageId>>) -> Option<PageId> {
    for _ in 0..=u8::MAX {
        let page = allocate_page(next_page)?;
        if forbidden.is_none_or(|pages| !pages.contains(&page)) {
            return Some(page);
        }
    }
    None
}

fn allocate_unused_page(next_page: &mut u16, used: &BTreeSet<PageId>) -> Option<PageId> {
    for _ in 0..=u8::MAX {
        let page = allocate_page(next_page)?;
        if !used.contains(&page) {
            return Some(page);
        }
    }
    None
}

fn page_allocation_failure(target_profile_hash: Hash256) -> ValidationDiagnostic {
    diagnostic(
        SramPagePlanDiagnosticCode::SramTargetProfileLayoutUnsupported,
        SramPagePlanDiagnosticProvenance::TargetProfileLayout {
            target_profile_hash,
            detail: "Stage 7 exhausted the u8 SRAM PageId address space".to_owned(),
        },
    )
}

fn manifest_residency_conflict(boundary: CommitBoundaryId) -> ValidationDiagnostic {
    diagnostic(
        SramPagePlanDiagnosticCode::SramPolicyProjectionMismatch,
        SramPagePlanDiagnosticProvenance::PolicyProjection {
            field: "spill_policy.persist_manifest_residency".to_owned(),
            detail: format!(
                "commit boundary {} cannot satisfy manifest residency placement",
                boundary.0
            ),
        },
    )
}

fn saturating_u16(value: usize) -> u16 {
    u16::try_from(value).unwrap_or(u16::MAX)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SramCandidate {
    binding_id: ValueId,
    commit_group: CommitGroupId,
    sequence_stream: SequenceStreamId,
    payload_bytes: u32,
    op_range: NodeAnchorRange,
    residency_role: SramResidencyRole,
    residency: PageResidency,
}

#[cfg(test)]
mod tests {
    use gbf_foundation::{CompileProfileId, TargetProfileId};
    use gbf_policy::{
        BudgetSlotClass, PlacementProfile, RomBudgetSlot, RuntimeMemoryCapSection,
        SramPageAggression,
    };

    use super::*;
    use crate::s3::infer_ir::NodeId;
    use crate::storage_plan::types::{
        AbstractLiveRange, AliasClassId, BindingJustification, DecisionRuleId, LifetimeClass,
        PersistPageId, StorageClass,
    };

    #[test]
    fn pass_single_stream_constructs_sorted_plan_and_report() {
        let output = build_sram_page_plan(&fixture_inputs(vec![
            binding(2, 0, 1, 512, PageResidency::AnyPageInBudget, false),
            binding(1, 0, 1, 256, PageResidency::SamePageAsLastMember, true),
        ]));
        assert_eq!(output.outcome, SramPagePlanOutcome::Succeeded);
        let plan = output.result.as_ref().expect("plan emitted");
        assert_eq!(
            plan.bindings
                .iter()
                .map(|binding| binding.binding_id.get())
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(plan.bindings[0].page, plan.bindings[1].page);
        assert_eq!(plan.active_sets.len(), 1);
        assert_eq!(plan.active_sets[0].bytes_in_use, 768);
        assert_eq!(
            plan.active_sets[0].bytes_reserved,
            768 + u32::from(PersistentPageGeometry::dmg_mbc5_8k().header_bytes)
                + u32::from(PersistentPageGeometry::dmg_mbc5_8k().commit_word_bytes)
        );
        assert_eq!(
            plan.active_sets[0].commit_boundaries_in_range,
            vec![CommitBoundaryId(0)]
        );
        assert_ne!(plan.sram_page_plan_self_hash, Hash256::ZERO);

        let bytes = emit_sram_page_plan_json_bytes(&output).expect("report emits");
        let parsed = parse_sram_page_plan_report_bytes(&bytes).expect("report parses");
        round_trip_self_hash(&parsed).expect("report self hash round trips");
        assert_eq!(canonicalize(&parsed).expect("canonical"), bytes);
    }

    #[test]
    fn pass_multi_stream_uses_distinct_pages_and_stream_index() {
        let output = build_sram_page_plan(&fixture_inputs(vec![
            binding(1, 0, 1, 256, PageResidency::AnyPageInBudget, false),
            binding(2, 1, 2, 128, PageResidency::AnyPageInBudget, false),
        ]));
        let plan = output.result.as_ref().expect("plan emitted");
        assert_eq!(plan.budgets.stream_count, 2);
        assert_eq!(plan.stream_index.len(), 2);
        assert_ne!(plan.bindings[0].page, plan.bindings[1].page);
    }

    #[test]
    fn pass_derives_commit_boundaries_rotations_and_projection() {
        let output = build_sram_page_plan(&fixture_inputs(vec![
            binding(
                1,
                7,
                1,
                128,
                PageResidency::FixedPage { page: PageId(1) },
                false,
            ),
            binding(
                2,
                8,
                1,
                128,
                PageResidency::FixedPage { page: PageId(2) },
                false,
            ),
            binding(
                3,
                9,
                1,
                128,
                PageResidency::FixedPage { page: PageId(1) },
                false,
            ),
        ]));
        let plan = output.result.as_ref().expect("plan emitted");
        assert_eq!(plan.commit_boundaries.len(), 3);
        assert_eq!(plan.commit_boundaries[0].commit_group, CommitGroupId(7));
        assert_eq!(
            plan.commit_boundaries[0].serialization_order,
            vec![ValueId::new(1)]
        );
        assert_eq!(plan.commit_boundaries[0].member_pages, vec![PageId(1)]);
        assert_eq!(plan.active_sets.len(), 3);
        assert_eq!(plan.page_rotations.len(), 3);
        assert_eq!(plan.page_rotations[0].from, SramVisiblePage::Unmapped);
        assert_eq!(
            plan.page_rotations[0].to,
            SramVisiblePage::Mapped { page: PageId(1) }
        );
        assert_eq!(
            plan.page_rotations[1].from,
            SramVisiblePage::Mapped { page: PageId(1) }
        );
        assert_eq!(
            plan.page_rotations[1].to,
            SramVisiblePage::Mapped { page: PageId(2) }
        );
        assert_eq!(
            plan.page_rotations[2].from,
            SramVisiblePage::Mapped { page: PageId(2) }
        );
        assert_eq!(
            plan.page_rotations[2].to,
            SramVisiblePage::Mapped { page: PageId(1) }
        );
        assert_eq!(plan.projections.projected_sram_page_switches_per_token, 3);
        assert_eq!(plan.projections.upper_bound_per_token, 3);
        assert_eq!(
            plan.projections
                .per_phase
                .iter()
                .map(|phase| (phase.epoch, phase.switches))
                .collect::<Vec<_>>(),
            vec![
                (SramEpochId(1), 1),
                (SramEpochId(2), 1),
                (SramEpochId(3), 1)
            ]
        );
    }

    #[test]
    fn pass_spill_eager_uses_dedicated_manifest_residency() {
        let mut inputs = fixture_inputs(vec![binding(
            1,
            7,
            1,
            128,
            PageResidency::AnyPageInBudget,
            false,
        )]);
        inputs.policy.spill_policy = SramSpillPolicy::SpillEager;
        inputs.runtime_chrome_budget.sram_reserved = 2 * page_size_bytes(inputs.geometry);

        let output = build_sram_page_plan(&inputs);
        assert_eq!(output.outcome, SramPagePlanOutcome::Succeeded);
        let plan = output.result.as_ref().expect("plan emitted");
        assert_eq!(
            plan.spill_policy.persist_manifest_residency,
            PersistManifestResidency::DedicatedManifestPage
        );
        assert_eq!(
            plan.spill_policy.cold_spill_residency,
            ColdSpillResidency::BoundedColdSpill { max_pages: 2 }
        );
        assert_eq!(plan.commit_boundaries.len(), 1);
        let boundary = &plan.commit_boundaries[0];
        assert!(!boundary.member_pages.contains(&boundary.manifest_page));
        assert!(
            plan.pages
                .iter()
                .any(|page| page.page == boundary.manifest_page && page.payload_bytes == 0)
        );
        assert_eq!(
            plan.bindings
                .iter()
                .filter(|binding| binding.residency_role == SramResidencyRole::SramPagedSpill)
                .count(),
            2
        );
        assert_eq!(plan.page_rotations.len(), 2);
        assert_eq!(plan.projections.projected_sram_page_switches_per_token, 2);
    }

    #[test]
    fn pass_maps_residency_roles_and_counts_persistent_kinds() {
        let mut continuation = binding(5, 5, 1, 32, PageResidency::AnyPageInBudget, false);
        set_persist_page_and_rule(&mut continuation, 0x2000_0005, 5);
        let mut transcript = binding(6, 6, 1, 32, PageResidency::AnyPageInBudget, false);
        set_persist_page_and_rule(&mut transcript, 0x3000_0006, 6);
        let mut harness = binding(7, 7, 1, 32, PageResidency::AnyPageInBudget, false);
        set_persist_page_and_rule(&mut harness, 0x4000_0007, 7);
        let mut trace = binding(8, 8, 1, 32, PageResidency::AnyPageInBudget, false);
        set_persist_page_and_rule(&mut trace, 0x5000_0008, 7);
        let scratch = sram_paged_binding(9, 1, 32);

        let mut inputs = fixture_inputs(vec![continuation, transcript, harness, trace, scratch]);
        inputs.switch_caps.max_sram_page_switches_per_token = 8;
        let output = build_sram_page_plan(&inputs);
        assert_eq!(output.outcome, SramPagePlanOutcome::Succeeded);
        let plan = output.result.as_ref().expect("plan emitted");
        assert!(plan.bindings.iter().any(|binding| {
            binding.residency_role == SramResidencyRole::PersistentContinuation
        }));
        assert!(
            plan.bindings.iter().any(|binding| {
                binding.residency_role == SramResidencyRole::PersistentTranscript
            })
        );
        assert!(
            plan.bindings
                .iter()
                .any(|binding| { binding.residency_role == SramResidencyRole::PersistentHarness })
        );
        assert!(
            plan.bindings
                .iter()
                .any(|binding| { binding.residency_role == SramResidencyRole::PersistentTrace })
        );
        assert!(
            plan.bindings
                .iter()
                .any(|binding| { binding.residency_role == SramResidencyRole::SramPagedScratch })
        );
        let report = emit_sram_page_plan_report(&output).expect("report emits");
        let cert = emit_sram_cert_report(&output, report.report_self_hash)
            .expect("cert emits")
            .expect("success emits cert");
        assert_eq!(cert.evidence.persistent_kind_distribution.continuation, 1);
        assert_eq!(cert.evidence.persistent_kind_distribution.transcript, 1);
        assert_eq!(cert.evidence.persistent_kind_distribution.harness, 1);
        assert_eq!(cert.evidence.persistent_kind_distribution.trace, 1);
    }

    #[test]
    fn pass_serialization_order_is_binding_id_not_page_order() {
        let mut inputs = fixture_inputs(vec![
            binding(
                2,
                7,
                1,
                32,
                PageResidency::FixedPage { page: PageId(1) },
                false,
            ),
            binding(
                1,
                7,
                1,
                32,
                PageResidency::FixedPage { page: PageId(2) },
                false,
            ),
        ]);
        inputs.epochs = vec![
            SramPagePlanEpochInput {
                epoch: SramEpochId(0),
                op_range: NodeAnchorRange {
                    first_node: NodeId::new(1),
                    last_node: NodeId::new(2),
                },
            },
            SramPagePlanEpochInput {
                epoch: SramEpochId(1),
                op_range: NodeAnchorRange {
                    first_node: NodeId::new(2),
                    last_node: NodeId::new(3),
                },
            },
        ];
        let output = build_sram_page_plan(&inputs);
        assert_eq!(output.outcome, SramPagePlanOutcome::Succeeded);
        let boundary = &output.result.as_ref().expect("plan").commit_boundaries[0];
        assert_eq!(
            boundary.serialization_order,
            vec![ValueId::new(1), ValueId::new(2)]
        );
        assert_eq!(boundary.manifest_page, PageId(1));
    }

    #[test]
    fn reject_empty_explicit_working_set_epoch() {
        let mut inputs = fixture_inputs(vec![binding(
            1,
            7,
            1,
            32,
            PageResidency::AnyPageInBudget,
            false,
        )]);
        inputs.epochs = vec![SramPagePlanEpochInput {
            epoch: SramEpochId(0),
            op_range: NodeAnchorRange {
                first_node: NodeId::new(10),
                last_node: NodeId::new(11),
            },
        }];
        assert_has_code(
            &build_sram_page_plan(&inputs),
            SramPagePlanDiagnosticCode::SramPolicyProjectionMismatch,
        );
    }

    #[test]
    fn yield_safety_predicate_matches_rfc_epoch_semantics() {
        let boundary = CommitBoundary {
            id: CommitBoundaryId(0),
            before_epoch: SramEpochId(2),
            after_epoch: SramEpochId(3),
            commit_group: CommitGroupId(7),
            generation_delta: 1,
            member_bindings: vec![ValueId::new(1)],
            member_pages: vec![PageId(1)],
            manifest_page: PageId(1),
            serialization_order: vec![ValueId::new(1)],
            yield_safe: YieldSafetyClass::YieldOnlyAfterManifest,
        };
        assert!(!is_yield_safe_at(&boundary, SramEpochId(2)));
        assert!(is_yield_safe_at(&boundary, SramEpochId(3)));

        let mut across_rotations = boundary.clone();
        across_rotations.yield_safe = YieldSafetyClass::YieldAcrossPageRotations;
        assert!(is_yield_safe_at(&across_rotations, SramEpochId(2)));
        assert!(is_yield_safe_at(&across_rotations, SramEpochId(99)));
    }

    #[test]
    fn reject_spill_policy_conflicting_with_cold_spill_budget() {
        let mut inputs = fixture_inputs(vec![binding(
            1,
            7,
            1,
            128,
            PageResidency::AnyPageInBudget,
            false,
        )]);
        inputs.policy.spill_policy = SramSpillPolicy::SpillEager;
        inputs.runtime_chrome_budget.sram_reserved = page_size_bytes(inputs.geometry);

        assert_has_code(
            &build_sram_page_plan(&inputs),
            SramPagePlanDiagnosticCode::SramPolicyProjectionMismatch,
        );
    }

    #[test]
    fn reject_multi_page_active_set_from_explicit_epoch() {
        let mut inputs = fixture_inputs(vec![
            binding(
                1,
                7,
                1,
                128,
                PageResidency::FixedPage { page: PageId(1) },
                false,
            ),
            binding(
                2,
                8,
                1,
                128,
                PageResidency::FixedPage { page: PageId(2) },
                false,
            ),
        ]);
        inputs.epochs = vec![SramPagePlanEpochInput {
            epoch: SramEpochId(0),
            op_range: NodeAnchorRange {
                first_node: NodeId::new(1),
                last_node: NodeId::new(3),
            },
        }];
        assert_has_code(
            &build_sram_page_plan(&inputs),
            SramPagePlanDiagnosticCode::SramPolicyProjectionMismatch,
        );
    }

    #[test]
    fn reject_commit_group_cross_stream() {
        let output = build_sram_page_plan(&fixture_inputs(vec![
            binding(1, 0, 1, 256, PageResidency::AnyPageInBudget, false),
            binding(2, 0, 2, 256, PageResidency::AnyPageInBudget, false),
        ]));
        assert_has_code(
            &output,
            SramPagePlanDiagnosticCode::SramCommitGroupCrossStream,
        );
    }

    #[test]
    fn reject_yield_resume_without_same_page_residency() {
        let output = build_sram_page_plan(&fixture_inputs(vec![binding(
            1,
            0,
            1,
            256,
            PageResidency::AnyPageInBudget,
            true,
        )]));
        assert_has_code(
            &output,
            SramPagePlanDiagnosticCode::SramYieldResumeResidencyViolation,
        );
    }

    #[test]
    fn reject_page_overflow_and_budget_overflow() {
        let mut page_overflow = fixture_inputs(vec![binding(
            1,
            0,
            1,
            PersistentPageGeometry::dmg_mbc5_8k().payload_bytes + 1,
            PageResidency::AnyPageInBudget,
            false,
        )]);
        assert_has_code(
            &build_sram_page_plan(&page_overflow),
            SramPagePlanDiagnosticCode::SramPageOverflow,
        );

        page_overflow.bindings = vec![binding(1, 0, 1, 512, PageResidency::AnyPageInBudget, false)];
        page_overflow
            .runtime_chrome_budget
            .memory_caps
            .sram_usable_bytes = 128;
        page_overflow.runtime_chrome_budget.sram_reserved = 0;
        assert_has_code(
            &build_sram_page_plan(&page_overflow),
            SramPagePlanDiagnosticCode::SramBudgetExceeded,
        );
    }

    #[test]
    fn reject_geometry_and_input_hash_mismatch() {
        let mut inputs = fixture_inputs(vec![binding(
            1,
            0,
            1,
            256,
            PageResidency::AnyPageInBudget,
            false,
        )]);
        inputs.expected_geometry.payload_bytes -= 1;
        assert_has_code(
            &build_sram_page_plan(&inputs),
            SramPagePlanDiagnosticCode::SramPageGeometryMismatch,
        );

        let mut inputs = fixture_inputs(vec![]);
        inputs.expected_input_hashes.storage_plan_self_hash = hash(99);
        assert_has_code(
            &build_sram_page_plan(&inputs),
            SramPagePlanDiagnosticCode::SramInputHashMismatch,
        );
    }

    #[test]
    fn determinism_and_cache_key_are_stable_and_sensitive_to_upstream() {
        let inputs = fixture_inputs(vec![
            binding(3, 1, 2, 64, PageResidency::AnyPageInBudget, false),
            binding(1, 0, 1, 64, PageResidency::AnyPageInBudget, false),
            binding(2, 0, 1, 64, PageResidency::SamePageAsLastMember, true),
        ]);
        let first = build_sram_page_plan(&inputs);
        let second = build_sram_page_plan(&inputs);
        assert_eq!(
            gbf_foundation::canonical_json_bytes_omitting_fields(
                first.result.as_ref().expect("first plan"),
                &["sram_page_plan_self_hash"],
            )
            .expect("first"),
            gbf_foundation::canonical_json_bytes_omitting_fields(
                second.result.as_ref().expect("second plan"),
                &["sram_page_plan_self_hash"],
            )
            .expect("second")
        );

        let key = SramPagePlanCacheKeyInputs::from_input_identity(&inputs.input_identity, hash(42))
            .cache_key()
            .expect("key");
        let mut changed = inputs.input_identity.clone();
        changed.storage_plan_self_hash = hash(43);
        let changed_key = SramPagePlanCacheKeyInputs::from_input_identity(&changed, hash(42))
            .cache_key()
            .expect("changed key");
        assert_ne!(key, changed_key);
    }

    fn assert_has_code(output: &SramPagePlanOutput, expected: SramPagePlanDiagnosticCode) {
        assert_eq!(output.outcome, SramPagePlanOutcome::Failed);
        assert!(
            output.diagnostics.iter().any(|diagnostic| matches!(
                diagnostic.code,
                ValidationCode::SramPagePlan { code, .. } if code == expected
            )),
            "missing {expected:?} in {:?}",
            output.diagnostics
        );
    }

    fn fixture_inputs(bindings: Vec<SramPagePlanBindingInput>) -> SramPagePlanInputs {
        let hashes = SramPagePlanInputHashes {
            storage_plan_self_hash: hash(1),
            observation_plan_self_hash: hash(2),
            range_plan_self_hash: hash(3),
            runtime_chrome_budget_hash: hash(4),
            target_profile_hash: hash(5),
            sram_page_plan_policy_projection_hash: hash(6),
        };
        SramPagePlanInputs {
            input_identity: SramPagePlanInputIdentity {
                storage_plan_self_hash: hashes.storage_plan_self_hash,
                observation_plan_self_hash: hashes.observation_plan_self_hash,
                range_plan_self_hash: hashes.range_plan_self_hash,
                runtime_chrome_budget_hash: hashes.runtime_chrome_budget_hash,
                target_profile_hash: hashes.target_profile_hash,
                sram_page_plan_policy_projection_hash: hashes.sram_page_plan_policy_projection_hash,
                determinism: DeterminismClass::Deterministic,
                schema_version: SRAM_PAGE_PLAN_SCHEMA_VERSION,
            },
            expected_input_hashes: hashes,
            runtime_chrome_budget: RuntimeChromeBudget {
                target: TargetProfileId::from("dmg-mbc5"),
                profile: CompileProfileId::from("Bringup"),
                runtime_nucleus_hash: hash(7),
                rom_slots: vec![RomBudgetSlot {
                    id: gbf_foundation::BudgetSlotId::new(0),
                    class: BudgetSlotClass::CommonBank,
                    usable_bytes: 16 * 1024,
                    reserved_slack: 0,
                    placement_caps: BTreeSet::from([PlacementProfile::Budgeted]),
                }],
                memory_caps: RuntimeMemoryCapSection {
                    wram_usable_bytes: 8 * 1024,
                    sram_usable_bytes: 32 * 1024,
                    hram_usable_bytes: 127,
                    source_target_profile_hash: hash(8),
                },
                wram_reserved: 0,
                sram_reserved: 512,
            },
            policy: SramKnob {
                page_aggression: SramPageAggression::Preserve,
                spill_policy: SramSpillPolicy::NoSpill,
            },
            switch_caps: SramSwitchCaps {
                max_sram_page_switches_per_token: 4,
            },
            geometry: PersistentPageGeometry::dmg_mbc5_8k(),
            expected_geometry: PersistentPageGeometry::dmg_mbc5_8k(),
            epochs: Vec::new(),
            bindings,
        }
    }

    fn binding(
        value_id: u32,
        commit_group: u32,
        stream: u32,
        payload_bytes: u32,
        residency: PageResidency,
        yield_resume: bool,
    ) -> SramPagePlanBindingInput {
        let value = ValueId::new(value_id);
        SramPagePlanBindingInput {
            binding: StorageBinding {
                value,
                materialization: Materialization::Persist {
                    page: PersistPageId(commit_group),
                    commit_group: CommitGroupId(commit_group),
                },
                alias_class: AliasClassId(value_id),
                live_range: AbstractLiveRange {
                    def_node: NodeId::new(value_id),
                    first_use_node: Some(NodeId::new(value_id)),
                    last_use_node: Some(NodeId::new(value_id + 1)),
                    lifetime_class: LifetimeClass::Persistent,
                    checkpoint_stable: true,
                },
                justification: BindingJustification::DecisionRule(DecisionRuleId(1)),
            },
            payload_bytes,
            sequence_stream: SequenceStreamId(stream),
            residency,
            yield_resume,
        }
    }

    fn set_persist_page_and_rule(binding: &mut SramPagePlanBindingInput, page: u32, rule: u32) {
        let Materialization::Persist {
            page: persist_page, ..
        } = &mut binding.binding.materialization
        else {
            panic!("expected persist binding");
        };
        *persist_page = PersistPageId(page);
        binding.binding.justification = BindingJustification::DecisionRule(DecisionRuleId(rule));
    }

    fn sram_paged_binding(
        value_id: u32,
        stream: u32,
        payload_bytes: u32,
    ) -> SramPagePlanBindingInput {
        let mut binding = binding(
            value_id,
            0,
            stream,
            payload_bytes,
            PageResidency::AnyPageInBudget,
            false,
        );
        binding.binding.materialization = Materialization::Materialize {
            class: StorageClass::SramPaged,
            lifetime: LifetimeClass::Token,
        };
        binding.binding.live_range.lifetime_class = LifetimeClass::Token;
        binding
    }

    fn hash(byte: u8) -> Hash256 {
        Hash256::from_bytes([byte; 32])
    }

    fn page_size_bytes(geometry: PersistentPageGeometry) -> u32 {
        geometry
            .payload_bytes
            .saturating_add(u32::from(geometry.header_bytes))
            .saturating_add(u32::from(geometry.commit_word_bytes))
    }
}
