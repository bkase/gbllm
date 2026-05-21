//! Stage 8.5 `OverlayPlan` construction, report, and cache-key surface.

use std::collections::{BTreeMap, BTreeSet};
use std::{error::Error, fmt};

use gbf_foundation::{
    CanonicalJsonError, DomainHash, EvidenceRef, Hash256, KernelSpecId, SemVer, TargetProfileId,
    canonical_json_bytes_omitting_fields, self_hash_omitting_fields,
};
use gbf_policy::{
    DiagnosticSeverity, OverlayPlanDiagnosticCode, OverlayPlanDiagnosticProvenance,
    RuntimeChromeBudget, RuntimeMode, ValidationCode, ValidationDetail, ValidationDiagnostic,
    ValidationOrigin,
};
use gbf_report::{
    CanonicalJsonError as ReportCanonicalJsonError, ReportBody, ReportEnvelope,
    ReportEnvelopeError, ReportOutcome, ReportSelfHashError, canonicalize, round_trip_self_hash,
};
use serde::{Deserialize, Serialize};

use crate::s1::quant_graph::DeterminismClass;
use crate::stage_cache::{
    CodegenStageCacheError, StoreBackedStageCacheKeys, StoreBackedStageCellKind,
    StoreBackedStageExpectedHashes, StoreBackedStageRunOutput, StoreBackedStageRunResult,
    crate_feature_set_hash, run_store_backed_stage_with_cache, stage85_overlay_plan_store_key,
};
use crate::window::{LutInstanceId, RomReachabilityClass, RomWindowPlan};
use gbf_store::stage_cache::StageCache as StoreStageCache;

pub const OVERLAY_PLAN_SCHEMA_ID: &str = "overlay_plan.v1";
pub const OVERLAY_PLAN_SCHEMA_VERSION: SemVer = SemVer::new(1, 0, 0);
pub const OVERLAY_PLAN_PASS_VERSION: &str = "stage8_5/v1";

pub type OverlayPlanReportEnvelope = ReportEnvelope<OverlayPlanReportBody>;

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct OverlayId(pub u32);

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct OverlayShareClassId(pub u32);

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct OverlayInstallId(pub u32);

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum OverlayResidentId {
    Kernel { kernel: KernelSpecId },
    Lut { lut: LutInstanceId },
}

impl fmt::Display for OverlayResidentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Kernel { kernel } => write!(f, "kernel:{kernel}"),
            Self::Lut { lut } => write!(f, "lut:{}", lut.0),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverlayResident {
    pub id: OverlayResidentId,
    pub payload_bytes: u32,
    pub reachability: RomReachabilityClass,
    pub source: OverlaySource,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum OverlaySource {
    RomWindowOverlayDemand { resident: OverlayResidentId },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum WramRegionConstraint {
    DmgWramC000Dfff,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum OverlayReservationKind {
    WramOverlay,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum OverlayEvictionPolicy {
    Undefined,
    ReloadOnUse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum OverlayInstallEvent {
    TokenBoundary,
    ExpertSwitch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverlayRegion {
    pub id: OverlayId,
    pub bytes: u16,
    pub constraint: WramRegionConstraint,
    pub members: Vec<OverlayResident>,
    pub reservation_kind: OverlayReservationKind,
    pub reservation_floor_bytes: u16,
    pub reservation_ceil_bytes: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverlayShareClass {
    pub id: OverlayShareClassId,
    pub region: OverlayId,
    pub members: Vec<OverlayResidentId>,
    pub eviction: OverlayEvictionPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverlayInstall {
    pub id: OverlayInstallId,
    pub region: OverlayId,
    pub member: OverlayResidentId,
    pub source: OverlaySource,
    pub install_event: OverlayInstallEvent,
    pub lease_shape: OverlayLeaseShape,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverlayLeaseShape {
    pub source_lease: OverlaySourceLease,
    pub wram_region_lease: OverlayWramRegionLease,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverlaySourceLease {
    pub source: OverlaySource,
    pub acquire_at: OverlayInstallEvent,
    pub release_at: OverlayInstallEvent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverlayWramRegionLease {
    pub region: OverlayId,
    pub acquire_at: OverlayInstallEvent,
    pub release_at: OverlayInstallEvent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverlayReservation {
    pub total_bytes: u16,
    pub per_region: Vec<OverlayReservationEntry>,
    pub cap_bytes: u16,
    pub region_max_bytes: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverlayReservationEntry {
    pub region: OverlayId,
    pub bytes: u16,
    pub reservation_kind: OverlayReservationKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverlayPlanInputIdentity {
    pub storage_plan_self_hash: Hash256,
    pub sram_page_plan_self_hash: Hash256,
    pub rom_window_plan_self_hash: Hash256,
    pub runtime_chrome_budget_hash: Hash256,
    pub target_profile_hash: Hash256,
    pub overlay_plan_policy_projection_hash: Hash256,
    pub determinism: DeterminismClass,
    pub target_profile_id: TargetProfileId,
    pub schema_version: SemVer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverlayPlanInputHashes {
    pub storage_plan_self_hash: Hash256,
    pub sram_page_plan_self_hash: Hash256,
    pub rom_window_plan_self_hash: Hash256,
    pub runtime_chrome_budget_hash: Hash256,
    pub target_profile_hash: Hash256,
    pub overlay_plan_policy_projection_hash: Hash256,
}

impl OverlayPlanInputIdentity {
    #[must_use]
    pub const fn hashes(&self) -> OverlayPlanInputHashes {
        OverlayPlanInputHashes {
            storage_plan_self_hash: self.storage_plan_self_hash,
            sram_page_plan_self_hash: self.sram_page_plan_self_hash,
            rom_window_plan_self_hash: self.rom_window_plan_self_hash,
            runtime_chrome_budget_hash: self.runtime_chrome_budget_hash,
            target_profile_hash: self.target_profile_hash,
            overlay_plan_policy_projection_hash: self.overlay_plan_policy_projection_hash,
        }
    }

    #[must_use]
    pub fn hash_for_product(&self, product: OverlayPlanInputProduct) -> Hash256 {
        self.hashes().hash_for_product(product)
    }
}

impl OverlayPlanInputHashes {
    #[must_use]
    pub const fn hash_for_product(&self, product: OverlayPlanInputProduct) -> Hash256 {
        match product {
            OverlayPlanInputProduct::StoragePlan => self.storage_plan_self_hash,
            OverlayPlanInputProduct::SramPagePlan => self.sram_page_plan_self_hash,
            OverlayPlanInputProduct::RomWindowPlan => self.rom_window_plan_self_hash,
            OverlayPlanInputProduct::RuntimeChromeBudget => self.runtime_chrome_budget_hash,
            OverlayPlanInputProduct::TargetProfile => self.target_profile_hash,
            OverlayPlanInputProduct::PolicyProjection => self.overlay_plan_policy_projection_hash,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum OverlayPlanInputProduct {
    StoragePlan,
    SramPagePlan,
    RomWindowPlan,
    RuntimeChromeBudget,
    TargetProfile,
    PolicyProjection,
}

impl OverlayPlanInputProduct {
    #[must_use]
    pub const fn field_name(self) -> &'static str {
        match self {
            Self::StoragePlan => "storage_plan_self_hash",
            Self::SramPagePlan => "sram_page_plan_self_hash",
            Self::RomWindowPlan => "rom_window_plan_self_hash",
            Self::RuntimeChromeBudget => "runtime_chrome_budget_hash",
            Self::TargetProfile => "target_profile_hash",
            Self::PolicyProjection => "overlay_plan_policy_projection_hash",
        }
    }
}

const OVERLAY_PLAN_INPUT_PRODUCTS: [OverlayPlanInputProduct; 6] = [
    OverlayPlanInputProduct::StoragePlan,
    OverlayPlanInputProduct::SramPagePlan,
    OverlayPlanInputProduct::RomWindowPlan,
    OverlayPlanInputProduct::RuntimeChromeBudget,
    OverlayPlanInputProduct::TargetProfile,
    OverlayPlanInputProduct::PolicyProjection,
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverlayPlanPolicyProjection {
    pub overlay_eviction_default: OverlayEvictionPolicy,
    pub overlay_install_event_default: Option<OverlayInstallEvent>,
    pub runtime_modes_requested: BTreeSet<RuntimeMode>,
    pub require_explicit_zero_reservation: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverlayTargetProfileSummary {
    pub allowed_overlay_constraints: BTreeSet<WramRegionConstraint>,
    pub default_overlay_constraint: WramRegionConstraint,
    pub wram_overlay_region_max_bytes: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverlayPlanInputs {
    pub input_identity: OverlayPlanInputIdentity,
    pub expected_input_hashes: OverlayPlanInputHashes,
    pub audit_parents: OverlayPlanAuditParents,
    pub runtime_chrome_budget: RuntimeChromeBudget,
    pub target_profile: OverlayTargetProfileSummary,
    pub policy: OverlayPlanPolicyProjection,
    pub rom_window_plan: RomWindowPlan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverlayPlanAuditParents {
    pub policy_resolution_self_hash: Hash256,
    pub artifact_validation_self_hash: Hash256,
    pub compile_request_hash: Hash256,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverlayPlan {
    pub identity: OverlayPlanInputIdentity,
    pub regions: Vec<OverlayRegion>,
    pub share_classes: Vec<OverlayShareClass>,
    pub installs: Vec<OverlayInstall>,
    pub reservation: OverlayReservation,
    pub overlay_plan_self_hash: Hash256,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverlayPlanSummary {
    pub region_count: u16,
    pub share_class_count: u16,
    pub install_count: u16,
    pub reserved_bytes: u16,
    pub cap_bytes: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverlayPlanResult {
    pub product: OverlayPlan,
    pub overlay_plan_self_hash: Hash256,
    pub summary: OverlayPlanSummary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlayPlanOutput {
    pub input_identity: OverlayPlanInputIdentity,
    pub audit_parents: OverlayPlanAuditParents,
    pub outcome: OverlayPlanOutcome,
    pub result: Option<OverlayPlanResult>,
    pub diagnostics: Vec<ValidationDiagnostic>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayPlanOutcome {
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverlayPlanReportBody {
    pub pass_version: String,
    pub input_identity: OverlayPlanInputIdentity,
    pub audit_parents: OverlayPlanAuditParents,
    pub diagnostics: Vec<ValidationDiagnostic>,
    pub result: Option<OverlayPlanResult>,
}

impl ReportBody for OverlayPlanReportBody {
    const REPORT_TYPE: &'static str = "OverlayPlanReport";
    const SCHEMA_ID: &'static str = OVERLAY_PLAN_SCHEMA_ID;
    const SCHEMA_VERSION: &'static str = "1.0.0";

    fn validate_semantics(&self, outcome: ReportOutcome) -> Result<(), Vec<ValidationDiagnostic>> {
        validate_overlay_plan_report_body(self, outcome)
    }
}

pub fn build_overlay_plan(input: &OverlayPlanInputs) -> OverlayPlanOutput {
    let hash_diagnostics = input_hash_mismatch_diagnostics(input);
    if !hash_diagnostics.is_empty() {
        return failed_output(
            input.input_identity.clone(),
            input.audit_parents,
            hash_diagnostics,
        );
    }

    if input.input_identity.rom_window_plan_self_hash
        != input.rom_window_plan.rom_window_plan_self_hash
    {
        return failed_output(
            input.input_identity.clone(),
            input.audit_parents,
            vec![diagnostic(
                OverlayPlanDiagnosticCode::OverlayInputHashMismatch,
                OverlayPlanDiagnosticProvenance::HashMismatch {
                    product: "rom_window_plan_self_hash".to_owned(),
                    recorded: input.input_identity.rom_window_plan_self_hash,
                    computed: input.rom_window_plan.rom_window_plan_self_hash,
                },
            )],
        );
    }

    if input.policy.runtime_modes_requested.is_empty() {
        return failed_output(
            input.input_identity.clone(),
            input.audit_parents,
            vec![diagnostic(
                OverlayPlanDiagnosticCode::OverlayResolvedPolicyProjectionMismatch,
                OverlayPlanDiagnosticProvenance::PolicyProjection {
                    field: "runtime_modes_requested".to_owned(),
                    detail: "OverlayPlanPolicyProjection requires at least one runtime mode"
                        .to_owned(),
                },
            )],
        );
    }

    if !input
        .target_profile
        .allowed_overlay_constraints
        .contains(&input.target_profile.default_overlay_constraint)
    {
        return failed_output(
            input.input_identity.clone(),
            input.audit_parents,
            vec![diagnostic(
                OverlayPlanDiagnosticCode::OverlayTargetProfileLayoutUnsupported,
                OverlayPlanDiagnosticProvenance::TargetProfileLayout {
                    target_profile_hash: input.input_identity.target_profile_hash,
                    detail: "default overlay constraint is not allowed by target profile"
                        .to_owned(),
                },
            )],
        );
    }

    let mut members = overlay_members(&input.rom_window_plan);
    members.sort_by(|left, right| left.id.cmp(&right.id));

    if members.is_empty() {
        if input.policy.require_explicit_zero_reservation
            && input.runtime_chrome_budget.wram_reserved > 0
        {
            return failed_output(
                input.input_identity.clone(),
                input.audit_parents,
                vec![diagnostic(
                    OverlayPlanDiagnosticCode::OverlayNoCandidatesButReservationDeclared,
                    OverlayPlanDiagnosticProvenance::Reservation {
                        total_bytes: 0,
                        cap_bytes: u32::from(input.runtime_chrome_budget.wram_reserved),
                    },
                )],
            );
        }
        return succeeded_output(input, Vec::new(), Vec::new(), Vec::new());
    }

    let Some(install_event) = input.policy.overlay_install_event_default else {
        return failed_output(
            input.input_identity.clone(),
            input.audit_parents,
            vec![diagnostic(
                OverlayPlanDiagnosticCode::OverlayInstallEventDefaultMissing,
                OverlayPlanDiagnosticProvenance::PolicyProjection {
                    field: "overlay_install_event_default".to_owned(),
                    detail: "OverlayPlan v1 requires a default install event".to_owned(),
                },
            )],
        );
    };

    let max_payload = members
        .iter()
        .map(|member| member.payload_bytes)
        .max()
        .unwrap_or(0);
    if max_payload > u32::from(input.target_profile.wram_overlay_region_max_bytes) {
        return failed_output(
            input.input_identity.clone(),
            input.audit_parents,
            vec![diagnostic(
                OverlayPlanDiagnosticCode::OverlayRegionPayloadExceedsRegionCap,
                OverlayPlanDiagnosticProvenance::Reservation {
                    total_bytes: max_payload,
                    cap_bytes: u32::from(input.target_profile.wram_overlay_region_max_bytes),
                },
            )],
        );
    }
    if max_payload > u32::from(input.runtime_chrome_budget.wram_reserved) {
        return failed_output(
            input.input_identity.clone(),
            input.audit_parents,
            vec![diagnostic(
                OverlayPlanDiagnosticCode::OverlayWramOverlayCapExceeded,
                OverlayPlanDiagnosticProvenance::Reservation {
                    total_bytes: max_payload,
                    cap_bytes: u32::from(input.runtime_chrome_budget.wram_reserved),
                },
            )],
        );
    }

    let region_bytes = u16::try_from(max_payload).unwrap_or(u16::MAX);
    let region = OverlayRegion {
        id: OverlayId(0),
        bytes: region_bytes,
        constraint: input.target_profile.default_overlay_constraint,
        members: members.clone(),
        reservation_kind: OverlayReservationKind::WramOverlay,
        reservation_floor_bytes: region_bytes,
        reservation_ceil_bytes: region_bytes,
    };

    let mut share_classes = Vec::new();
    if members.len() >= 2 {
        if matches!(
            input.policy.overlay_eviction_default,
            OverlayEvictionPolicy::Undefined
        ) {
            return failed_output(
                input.input_identity.clone(),
                input.audit_parents,
                vec![diagnostic(
                    OverlayPlanDiagnosticCode::OverlayShareClassEvictionUndefined,
                    OverlayPlanDiagnosticProvenance::Region {
                        invariant: "OP-SC-5".to_owned(),
                        region_id: region.id.0,
                    },
                )],
            );
        }
        share_classes.push(OverlayShareClass {
            id: OverlayShareClassId(0),
            region: region.id,
            members: members.iter().map(|member| member.id.clone()).collect(),
            eviction: input.policy.overlay_eviction_default,
        });
    }

    let installs = members
        .iter()
        .enumerate()
        .map(|(index, member)| OverlayInstall {
            id: OverlayInstallId(index as u32),
            region: region.id,
            member: member.id.clone(),
            source: member.source.clone(),
            install_event,
            lease_shape: OverlayLeaseShape {
                source_lease: OverlaySourceLease {
                    source: member.source.clone(),
                    acquire_at: install_event,
                    release_at: install_event,
                },
                wram_region_lease: OverlayWramRegionLease {
                    region: region.id,
                    acquire_at: install_event,
                    release_at: install_event,
                },
            },
        })
        .collect();

    succeeded_output(input, vec![region], share_classes, installs)
}

pub fn emit_overlay_plan_report(
    output: &OverlayPlanOutput,
) -> Result<OverlayPlanReportEnvelope, OverlayPlanEmitError> {
    let outcome = match output.outcome {
        OverlayPlanOutcome::Succeeded => ReportOutcome::Passed,
        OverlayPlanOutcome::Failed => ReportOutcome::Failed,
    };
    let body = OverlayPlanReportBody {
        pass_version: OVERLAY_PLAN_PASS_VERSION.to_owned(),
        input_identity: output.input_identity.clone(),
        audit_parents: output.audit_parents,
        diagnostics: output.diagnostics.clone(),
        result: output.result.clone(),
    };
    let envelope = ReportEnvelope::new(outcome, body)?.with_computed_self_hash()?;
    round_trip_self_hash(&envelope)?;
    Ok(envelope)
}

pub fn emit_overlay_plan_json_bytes(
    output: &OverlayPlanOutput,
) -> Result<Vec<u8>, OverlayPlanEmitError> {
    Ok(canonicalize(&emit_overlay_plan_report(output)?)?)
}

pub fn parse_overlay_plan_report_bytes(
    bytes: &[u8],
) -> Result<OverlayPlanReportEnvelope, ReportSelfHashError> {
    serde_json::from_slice(bytes).map_err(|error| ReportSelfHashError::Json {
        reason: error.to_string(),
    })
}

pub fn overlay_plan_self_hash(plan: &OverlayPlan) -> Result<Hash256, CanonicalJsonError> {
    self_hash_omitting_fields(
        DomainHash::new(
            "gbf-codegen",
            "OverlayPlan",
            OVERLAY_PLAN_SCHEMA_ID,
            "1.0.0",
        ),
        plan,
        "overlay_plan_self_hash",
        &[],
    )
}

pub fn overlay_plan_policy_projection_hash(
    policy: &OverlayPlanPolicyProjection,
) -> Result<Hash256, CanonicalJsonError> {
    DomainHash::new(
        "gbf-codegen",
        "OverlayPlanPolicyProjection",
        OVERLAY_PLAN_SCHEMA_ID,
        "1.0.0",
    )
    .hash(policy)
}

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct OverlayPlanCacheKey(pub Hash256);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverlayPlanCacheKeyInputs {
    pub storage_plan_self_hash: Hash256,
    pub sram_page_plan_self_hash: Hash256,
    pub rom_window_plan_self_hash: Hash256,
    pub runtime_chrome_budget_hash: Hash256,
    pub target_profile_hash: Hash256,
    pub overlay_plan_policy_projection_hash: Hash256,
    pub pass_version: String,
    pub crate_feature_set_hash: Hash256,
}

impl OverlayPlanCacheKeyInputs {
    #[must_use]
    pub fn from_input_identity(
        identity: &OverlayPlanInputIdentity,
        crate_feature_set_hash: Hash256,
    ) -> Self {
        Self {
            storage_plan_self_hash: identity.storage_plan_self_hash,
            sram_page_plan_self_hash: identity.sram_page_plan_self_hash,
            rom_window_plan_self_hash: identity.rom_window_plan_self_hash,
            runtime_chrome_budget_hash: identity.runtime_chrome_budget_hash,
            target_profile_hash: identity.target_profile_hash,
            overlay_plan_policy_projection_hash: identity.overlay_plan_policy_projection_hash,
            pass_version: OVERLAY_PLAN_PASS_VERSION.to_owned(),
            crate_feature_set_hash,
        }
    }

    pub fn cache_key(&self) -> Result<OverlayPlanCacheKey, CanonicalJsonError> {
        overlay_plan_cache_key(self)
    }
}

pub fn overlay_plan_cache_key(
    inputs: &OverlayPlanCacheKeyInputs,
) -> Result<OverlayPlanCacheKey, CanonicalJsonError> {
    let canonical = canonical_json_bytes_omitting_fields(inputs, &[])?;
    DomainHash::new("gbf-codegen", "StageCacheKey", "overlay_plan", "v1")
        .hash_canonical_bytes(&canonical)
        .map(OverlayPlanCacheKey)
}

pub fn run_overlay_plan_with_cache(
    cache: &StoreStageCache<'_>,
    input: &OverlayPlanInputs,
    expected_hashes: StoreBackedStageExpectedHashes,
) -> Result<StoreBackedStageRunOutput<OverlayPlanResult>, CodegenStageCacheError> {
    let cache_key = OverlayPlanCacheKeyInputs::from_input_identity(
        &input.input_identity,
        crate_feature_set_hash(),
    )
    .cache_key()
    .map_err(|error| CodegenStageCacheError::StageCacheKey {
        stage_id: "8.5",
        message: error.to_string(),
    })?;
    let keys = StoreBackedStageCacheKeys::new(
        "8.5",
        stage85_overlay_plan_store_key(cache_key, StoreBackedStageCellKind::Success),
        stage85_overlay_plan_store_key(cache_key, StoreBackedStageCellKind::FailureMemo),
    );
    run_store_backed_stage_with_cache(cache, &keys, cache_key.0, expected_hashes, || {
        let output = build_overlay_plan(input);
        let report = emit_overlay_plan_report(&output).map_err(|error| {
            CodegenStageCacheError::StageEmit {
                stage_id: "8.5",
                message: error.to_string(),
            }
        })?;
        let report_self_hash = report.report_self_hash;
        match output.outcome {
            OverlayPlanOutcome::Succeeded => {
                let product =
                    output
                        .result
                        .ok_or_else(|| CodegenStageCacheError::StageOutputInvariant {
                            stage_id: "8.5",
                            message: "succeeded output is missing OverlayPlanResult".to_owned(),
                        })?;
                Ok(StoreBackedStageRunResult::Success {
                    product_self_hash: product.overlay_plan_self_hash,
                    product,
                    report_self_hash,
                })
            }
            OverlayPlanOutcome::Failed => Ok(StoreBackedStageRunResult::FailureMemo {
                diagnostics: output.diagnostics,
                report_self_hash,
            }),
        }
    })
}

#[derive(Debug)]
pub enum OverlayPlanEmitError {
    Envelope(ReportEnvelopeError),
    SelfHash(ReportSelfHashError),
    Canonical(ReportCanonicalJsonError),
}

impl fmt::Display for OverlayPlanEmitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Envelope(error) => write!(f, "overlay plan report envelope failed: {error}"),
            Self::SelfHash(error) => write!(f, "overlay plan report self hash failed: {error}"),
            Self::Canonical(error) => {
                write!(f, "overlay plan report canonicalization failed: {error}")
            }
        }
    }
}

impl Error for OverlayPlanEmitError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Envelope(error) => Some(error),
            Self::SelfHash(error) => Some(error),
            Self::Canonical(error) => Some(error),
        }
    }
}

impl From<ReportEnvelopeError> for OverlayPlanEmitError {
    fn from(error: ReportEnvelopeError) -> Self {
        Self::Envelope(error)
    }
}

impl From<ReportSelfHashError> for OverlayPlanEmitError {
    fn from(error: ReportSelfHashError) -> Self {
        Self::SelfHash(error)
    }
}

impl From<ReportCanonicalJsonError> for OverlayPlanEmitError {
    fn from(error: ReportCanonicalJsonError) -> Self {
        Self::Canonical(error)
    }
}

fn input_hash_mismatch_diagnostics(input: &OverlayPlanInputs) -> Vec<ValidationDiagnostic> {
    OVERLAY_PLAN_INPUT_PRODUCTS
        .iter()
        .copied()
        .filter_map(|product| {
            let recorded = input.input_identity.hash_for_product(product);
            let computed = input.expected_input_hashes.hash_for_product(product);
            (recorded != computed).then(|| {
                diagnostic(
                    OverlayPlanDiagnosticCode::OverlayInputHashMismatch,
                    OverlayPlanDiagnosticProvenance::HashMismatch {
                        product: product.field_name().to_owned(),
                        recorded,
                        computed,
                    },
                )
            })
        })
        .collect()
}

pub fn validate_overlay_plan_product_surface(plan: &OverlayPlan) -> Option<ValidationDiagnostic> {
    let value = serde_json::to_value(plan).expect("overlay plan serializes");
    validate_overlay_plan_json_surface(&value)
}

pub fn validate_overlay_plan_json_surface(
    value: &serde_json::Value,
) -> Option<ValidationDiagnostic> {
    let text = value.to_string();
    for forbidden in ["AsmIR", "SectionRole", "BankPlacement"] {
        if text.contains(forbidden) {
            return Some(diagnostic(
                OverlayPlanDiagnosticCode::OverlaySectionRoleLeaked,
                OverlayPlanDiagnosticProvenance::JsonPath {
                    json_path: "$".to_owned(),
                    field_or_tag: forbidden.to_owned(),
                },
            ));
        }
    }
    for forbidden in [
        "ArenaPlan",
        "ArenaId",
        "ArenaSlot",
        "AddressSpace",
        "ByteRange",
        "SliceId",
        "LeaseId",
        "ResourceVector",
        "CycleBudget",
    ] {
        if text.contains(forbidden) {
            return Some(diagnostic(
                OverlayPlanDiagnosticCode::OverlaySchedulingFieldLeaked,
                OverlayPlanDiagnosticProvenance::JsonPath {
                    json_path: "$".to_owned(),
                    field_or_tag: forbidden.to_owned(),
                },
            ));
        }
    }
    if text.contains("RepairProposal") || text.contains("repair_proposals") {
        return Some(diagnostic(
            OverlayPlanDiagnosticCode::OverlayRepairProvenanceForbidden,
            OverlayPlanDiagnosticProvenance::JsonPath {
                json_path: "$".to_owned(),
                field_or_tag: "repair".to_owned(),
            },
        ));
    }
    None
}

fn validate_overlay_plan_report_body(
    body: &OverlayPlanReportBody,
    outcome: ReportOutcome,
) -> Result<(), Vec<ValidationDiagnostic>> {
    let has_hard = body
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Hard);
    let mut diagnostics = Vec::new();
    match outcome {
        ReportOutcome::Passed => {
            if body.result.is_none() || has_hard || body.pass_version != OVERLAY_PLAN_PASS_VERSION {
                diagnostics.push(diagnostic(
                    OverlayPlanDiagnosticCode::OverlayReportRoundTripFailed,
                    OverlayPlanDiagnosticProvenance::PolicyProjection {
                        field: "overlay_plan.passed".to_owned(),
                        detail: "passed overlay plan report body does not round-trip its semantic invariants"
                            .to_owned(),
                    },
                ));
            }
        }
        ReportOutcome::Failed => {
            if body.result.is_some() || !has_hard || body.pass_version != OVERLAY_PLAN_PASS_VERSION
            {
                diagnostics.push(diagnostic(
                    OverlayPlanDiagnosticCode::OverlayReportRoundTripFailed,
                    OverlayPlanDiagnosticProvenance::PolicyProjection {
                        field: "overlay_plan.failed".to_owned(),
                        detail: "failed overlay plan report body does not round-trip its semantic invariants"
                            .to_owned(),
                    },
                ));
            }
        }
    }
    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(diagnostics)
    }
}

fn diagnostic(
    code: OverlayPlanDiagnosticCode,
    provenance: OverlayPlanDiagnosticProvenance,
) -> ValidationDiagnostic {
    ValidationDiagnostic::hard(
        ValidationOrigin::OverlayPlanConstruction,
        ValidationCode::OverlayPlan { code, provenance },
        ValidationDetail::Field {
            field: format!(
                "overlay_plan.diagnostics.{}.{}.detail_template.v1",
                code.as_str(),
                code.name()
            )
            .into(),
        },
        vec![EvidenceRef {
            kind: "OverlayPlanConstruction".to_owned(),
            reference: code.as_str().to_owned(),
            hash: Some(Hash256::ZERO),
        }],
    )
}

fn failed_output(
    input_identity: OverlayPlanInputIdentity,
    audit_parents: OverlayPlanAuditParents,
    diagnostics: Vec<ValidationDiagnostic>,
) -> OverlayPlanOutput {
    OverlayPlanOutput {
        input_identity,
        audit_parents,
        outcome: OverlayPlanOutcome::Failed,
        result: None,
        diagnostics,
    }
}

fn succeeded_output(
    input: &OverlayPlanInputs,
    mut regions: Vec<OverlayRegion>,
    mut share_classes: Vec<OverlayShareClass>,
    mut installs: Vec<OverlayInstall>,
) -> OverlayPlanOutput {
    regions.sort_by_key(|region| {
        (
            region.constraint,
            std::cmp::Reverse(region.bytes),
            region.members.iter().map(|member| member.id.clone()).min(),
        )
    });
    share_classes.sort_by_key(|share_class| (share_class.region, share_class.id));
    installs.sort_by_key(|install| {
        (
            install.region,
            install.member.clone(),
            install.install_event,
        )
    });

    let reservation = compute_reservation(
        &regions,
        input.runtime_chrome_budget.wram_reserved,
        input.target_profile.wram_overlay_region_max_bytes,
    );
    let summary = OverlayPlanSummary {
        region_count: saturating_u16(regions.len()),
        share_class_count: saturating_u16(share_classes.len()),
        install_count: saturating_u16(installs.len()),
        reserved_bytes: reservation.total_bytes,
        cap_bytes: reservation.cap_bytes,
    };
    let mut plan = OverlayPlan {
        identity: input.input_identity.clone(),
        regions,
        share_classes,
        installs,
        reservation,
        overlay_plan_self_hash: Hash256::ZERO,
    };
    if let Some(diagnostic) = validate_overlay_plan_product_surface(&plan) {
        return failed_output(
            input.input_identity.clone(),
            input.audit_parents,
            vec![diagnostic],
        );
    }
    let invariant_diagnostics = validate_overlay_plan_product_invariants(input, &plan);
    if !invariant_diagnostics.is_empty() {
        return failed_output(
            input.input_identity.clone(),
            input.audit_parents,
            invariant_diagnostics,
        );
    }

    let self_hash = match overlay_plan_self_hash(&plan) {
        Ok(hash) => hash,
        Err(error) => {
            return failed_output(
                input.input_identity.clone(),
                input.audit_parents,
                vec![diagnostic(
                    OverlayPlanDiagnosticCode::OverlayCanonicalSortDrift,
                    OverlayPlanDiagnosticProvenance::PolicyProjection {
                        field: "overlay_plan_self_hash".to_owned(),
                        detail: error.to_string(),
                    },
                )],
            );
        }
    };
    plan.overlay_plan_self_hash = self_hash;

    OverlayPlanOutput {
        input_identity: input.input_identity.clone(),
        audit_parents: input.audit_parents,
        outcome: OverlayPlanOutcome::Succeeded,
        result: Some(OverlayPlanResult {
            product: plan,
            overlay_plan_self_hash: self_hash,
            summary,
        }),
        diagnostics: Vec::new(),
    }
}

fn compute_reservation(
    regions: &[OverlayRegion],
    cap_bytes: u16,
    region_max_bytes: u16,
) -> OverlayReservation {
    OverlayReservation {
        total_bytes: regions
            .iter()
            .fold(0u16, |total, region| total.saturating_add(region.bytes)),
        per_region: regions
            .iter()
            .map(|region| OverlayReservationEntry {
                region: region.id,
                bytes: region.bytes,
                reservation_kind: OverlayReservationKind::WramOverlay,
            })
            .collect(),
        cap_bytes,
        region_max_bytes,
    }
}

pub fn validate_overlay_plan_product_invariants(
    input: &OverlayPlanInputs,
    plan: &OverlayPlan,
) -> Vec<ValidationDiagnostic> {
    let mut diagnostics = Vec::new();
    let mut region_ids = BTreeSet::new();
    let mut region_members = BTreeMap::new();
    let candidates = overlay_members(&input.rom_window_plan)
        .into_iter()
        .map(|member| member.id)
        .collect::<BTreeSet<_>>();

    for region in &plan.regions {
        if !region_ids.insert(region.id) {
            diagnostics.push(diagnostic(
                OverlayPlanDiagnosticCode::OverlayRegionIdDuplicate,
                OverlayPlanDiagnosticProvenance::Region {
                    invariant: "OP-SC-duplicate-region-id".to_owned(),
                    region_id: region.id.0,
                },
            ));
        }
        if region.members.is_empty() {
            diagnostics.push(diagnostic(
                OverlayPlanDiagnosticCode::OverlayRegionEmptyButPopulated,
                OverlayPlanDiagnosticProvenance::Region {
                    invariant: "OP-SC-1".to_owned(),
                    region_id: region.id.0,
                },
            ));
        }
        if !input
            .target_profile
            .allowed_overlay_constraints
            .contains(&region.constraint)
        {
            diagnostics.push(diagnostic(
                OverlayPlanDiagnosticCode::OverlayTargetProfileLayoutUnsupported,
                OverlayPlanDiagnosticProvenance::TargetProfileLayout {
                    target_profile_hash: input.input_identity.target_profile_hash,
                    detail: format!(
                        "region {} uses an overlay constraint outside the target profile",
                        region.id.0
                    ),
                },
            ));
        }
        if region.bytes > input.target_profile.wram_overlay_region_max_bytes {
            diagnostics.push(diagnostic(
                OverlayPlanDiagnosticCode::OverlayRegionPayloadExceedsRegionCap,
                OverlayPlanDiagnosticProvenance::Reservation {
                    total_bytes: u32::from(region.bytes),
                    cap_bytes: u32::from(input.target_profile.wram_overlay_region_max_bytes),
                },
            ));
        }
        for member in &region.members {
            if member.payload_bytes > u32::from(region.bytes) {
                diagnostics.push(diagnostic(
                    OverlayPlanDiagnosticCode::OverlayMemberPayloadExceedsRegion,
                    OverlayPlanDiagnosticProvenance::Member {
                        invariant: "OP-SC-2".to_owned(),
                        member: member.id.to_string(),
                        payload_bytes: member.payload_bytes,
                        region_bytes: region.bytes,
                    },
                ));
            }
        }
        region_members.insert(
            region.id,
            region
                .members
                .iter()
                .map(|member| member.id.clone())
                .collect::<BTreeSet<_>>(),
        );
    }

    for share_class in &plan.share_classes {
        let Some(members) = region_members.get(&share_class.region) else {
            diagnostics.push(diagnostic(
                OverlayPlanDiagnosticCode::OverlayInstallReferencesUnknownRegion,
                OverlayPlanDiagnosticProvenance::Region {
                    invariant: "OP-SC-4".to_owned(),
                    region_id: share_class.region.0,
                },
            ));
            continue;
        };
        if matches!(share_class.eviction, OverlayEvictionPolicy::Undefined) {
            diagnostics.push(diagnostic(
                OverlayPlanDiagnosticCode::OverlayShareClassEvictionUndefined,
                OverlayPlanDiagnosticProvenance::Region {
                    invariant: "OP-SC-5".to_owned(),
                    region_id: share_class.region.0,
                },
            ));
        }
        for member in &share_class.members {
            if !members.contains(member) {
                diagnostics.push(diagnostic(
                    OverlayPlanDiagnosticCode::OverlayInstallReferencesUnknownMember,
                    OverlayPlanDiagnosticProvenance::Member {
                        invariant: "OP-SC-4".to_owned(),
                        member: member.to_string(),
                        payload_bytes: 0,
                        region_bytes: 0,
                    },
                ));
            }
        }
    }

    let mut installed_members = BTreeSet::new();
    for install in &plan.installs {
        let Some(members) = region_members.get(&install.region) else {
            diagnostics.push(diagnostic(
                OverlayPlanDiagnosticCode::OverlayInstallReferencesUnknownRegion,
                OverlayPlanDiagnosticProvenance::Install {
                    invariant: "OP-SC-6".to_owned(),
                    install_id: install.id.0,
                },
            ));
            continue;
        };
        if !members.contains(&install.member) {
            diagnostics.push(diagnostic(
                OverlayPlanDiagnosticCode::OverlayInstallReferencesUnknownMember,
                OverlayPlanDiagnosticProvenance::Install {
                    invariant: "OP-SC-7".to_owned(),
                    install_id: install.id.0,
                },
            ));
        }
        match &install.source {
            OverlaySource::RomWindowOverlayDemand { resident } => {
                if resident != &install.member || !candidates.contains(resident) {
                    diagnostics.push(diagnostic(
                        OverlayPlanDiagnosticCode::OverlayInstallSourceNotVisible,
                        OverlayPlanDiagnosticProvenance::Install {
                            invariant: "OP-SC-8".to_owned(),
                            install_id: install.id.0,
                        },
                    ));
                }
            }
        }
        if install.lease_shape.source_lease.source != install.source
            || install.lease_shape.source_lease.acquire_at != install.install_event
            || install.lease_shape.source_lease.release_at != install.install_event
            || install.lease_shape.wram_region_lease.region != install.region
            || install.lease_shape.wram_region_lease.acquire_at != install.install_event
            || install.lease_shape.wram_region_lease.release_at != install.install_event
        {
            diagnostics.push(diagnostic(
                OverlayPlanDiagnosticCode::OverlayLeaseShapeIncomplete,
                OverlayPlanDiagnosticProvenance::Install {
                    invariant: "OP-SC-14".to_owned(),
                    install_id: install.id.0,
                },
            ));
        }
        installed_members.insert(install.member.clone());
    }

    for candidate in candidates.difference(&installed_members) {
        diagnostics.push(diagnostic(
            OverlayPlanDiagnosticCode::OverlayCandidateNotInstalled,
            OverlayPlanDiagnosticProvenance::Member {
                invariant: "OP-SC-9".to_owned(),
                member: candidate.to_string(),
                payload_bytes: 0,
                region_bytes: 0,
            },
        ));
    }

    let reservation_total = plan
        .reservation
        .per_region
        .iter()
        .fold(0u32, |total, entry| {
            total.saturating_add(u32::from(entry.bytes))
        });
    if u32::from(plan.reservation.total_bytes) != reservation_total
        || reservation_total > u32::from(input.runtime_chrome_budget.wram_reserved)
        || plan.reservation.cap_bytes != input.runtime_chrome_budget.wram_reserved
    {
        diagnostics.push(diagnostic(
            OverlayPlanDiagnosticCode::OverlayWramOverlayCapExceeded,
            OverlayPlanDiagnosticProvenance::Reservation {
                total_bytes: reservation_total,
                cap_bytes: u32::from(input.runtime_chrome_budget.wram_reserved),
            },
        ));
    }

    let mut reservation_entries = BTreeMap::new();
    for entry in &plan.reservation.per_region {
        if reservation_entries.insert(entry.region, entry).is_some() {
            diagnostics.push(diagnostic(
                OverlayPlanDiagnosticCode::OverlayRegionIdDuplicate,
                OverlayPlanDiagnosticProvenance::Region {
                    invariant: "OP-SC-reservation-duplicate-region-id".to_owned(),
                    region_id: entry.region.0,
                },
            ));
        }
        if entry.bytes > plan.reservation.region_max_bytes {
            diagnostics.push(diagnostic(
                OverlayPlanDiagnosticCode::OverlayRegionPayloadExceedsRegionCap,
                OverlayPlanDiagnosticProvenance::Reservation {
                    total_bytes: u32::from(entry.bytes),
                    cap_bytes: u32::from(plan.reservation.region_max_bytes),
                },
            ));
        }
    }
    for region in &plan.regions {
        match reservation_entries.get(&region.id) {
            Some(entry) if entry.bytes == region.bytes => {}
            _ => diagnostics.push(diagnostic(
                OverlayPlanDiagnosticCode::OverlayMemberPayloadExceedsRegion,
                OverlayPlanDiagnosticProvenance::Region {
                    invariant: "OP-SC-13".to_owned(),
                    region_id: region.id.0,
                },
            )),
        }
    }

    if !is_canonical_overlay_product(plan) {
        diagnostics.push(diagnostic(
            OverlayPlanDiagnosticCode::OverlayCanonicalSortDrift,
            OverlayPlanDiagnosticProvenance::PolicyProjection {
                field: "overlay_plan_product_order".to_owned(),
                detail: "regions, share classes, installs, or reservations are not canonical"
                    .to_owned(),
            },
        ));
    }

    diagnostics
}

fn is_canonical_overlay_product(plan: &OverlayPlan) -> bool {
    plan.regions.windows(2).all(|pair| {
        let left = &pair[0];
        let right = &pair[1];
        (
            left.constraint,
            std::cmp::Reverse(left.bytes),
            left.members.iter().map(|member| member.id.clone()).min(),
        ) <= (
            right.constraint,
            std::cmp::Reverse(right.bytes),
            right.members.iter().map(|member| member.id.clone()).min(),
        )
    }) && plan
        .share_classes
        .windows(2)
        .all(|pair| (pair[0].region, pair[0].id) <= (pair[1].region, pair[1].id))
        && plan.installs.windows(2).all(|pair| {
            (
                pair[0].region,
                pair[0].member.clone(),
                pair[0].install_event,
            ) <= (
                pair[1].region,
                pair[1].member.clone(),
                pair[1].install_event,
            )
        })
        && plan
            .reservation
            .per_region
            .windows(2)
            .all(|pair| pair[0].region <= pair[1].region)
}

fn overlay_members(plan: &RomWindowPlan) -> Vec<OverlayResident> {
    let kernels = plan.overlay_demand.kernels.iter().map(|kernel| {
        let id = OverlayResidentId::Kernel {
            kernel: kernel.kernel.clone(),
        };
        OverlayResident {
            source: OverlaySource::RomWindowOverlayDemand {
                resident: id.clone(),
            },
            id,
            payload_bytes: kernel.byte_size,
            reachability: kernel.reachability,
        }
    });
    let luts = plan.overlay_demand.luts.iter().map(|lut| {
        let id = OverlayResidentId::Lut {
            lut: lut.lut.clone(),
        };
        OverlayResident {
            source: OverlaySource::RomWindowOverlayDemand {
                resident: id.clone(),
            },
            id,
            payload_bytes: lut.byte_size,
            reachability: lut.reachability,
        }
    });
    kernels.chain(luts).collect()
}

fn saturating_u16(value: usize) -> u16 {
    u16::try_from(value).unwrap_or(u16::MAX)
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::fs;
    use std::path::{Path, PathBuf};

    use gbf_foundation::{CompileProfileId, TargetProfileId};
    use gbf_policy::{
        PlacementProfile, RomBudgetSlot, RuntimeMemoryCapSection, SwitchProjectionSource,
    };
    use gbf_report::ReportOutcome;
    use serde::Deserialize;

    use super::*;
    use crate::window::{
        Bank0Demand, ROM_WINDOW_PLAN_SCHEMA_VERSION, RomSwitchProjections,
        RomWindowPlanInputIdentity, RomWindowPlanProvenance, WramOverlayDemand,
        WramOverlayKernelDemand, WramOverlayLutDemand,
    };

    #[test]
    fn pass_single_region_share_class_installs_and_report_round_trip() {
        let output = build_overlay_plan(&fixture_inputs(vec![
            resident_kernel("matvec", 384),
            resident_lut("gelu", 128),
        ]));
        assert_eq!(output.outcome, OverlayPlanOutcome::Succeeded);
        let result = output.result.as_ref().expect("overlay plan");
        assert_eq!(result.summary.region_count, 1);
        assert_eq!(result.summary.share_class_count, 1);
        assert_eq!(result.summary.install_count, 2);
        assert_eq!(result.summary.reserved_bytes, 384);
        assert_eq!(result.product.regions[0].bytes, 384);
        assert_eq!(result.product.reservation.per_region[0].bytes, 384);
        assert_eq!(
            result.overlay_plan_self_hash,
            result.product.overlay_plan_self_hash
        );

        let first = emit_overlay_plan_json_bytes(&output).expect("first report");
        let second = emit_overlay_plan_json_bytes(&output).expect("second report");
        assert_eq!(first, second);
        let parsed = parse_overlay_plan_report_bytes(&first).expect("parsed report");
        assert_eq!(parsed.outcome, ReportOutcome::Passed);
        assert_eq!(
            parsed
                .body
                .result
                .expect("parsed result")
                .summary
                .reserved_bytes,
            384
        );
    }

    #[test]
    fn reject_hash_region_eviction_and_missing_install_event() {
        let mut hash_mismatch = fixture_inputs(vec![resident_kernel("matvec", 128)]);
        hash_mismatch
            .expected_input_hashes
            .rom_window_plan_self_hash = hash(88);
        assert_has_code(
            &build_overlay_plan(&hash_mismatch),
            OverlayPlanDiagnosticCode::OverlayInputHashMismatch,
        );

        let mut region_overflow = fixture_inputs(vec![resident_kernel("matvec", 513)]);
        region_overflow.target_profile.wram_overlay_region_max_bytes = 512;
        assert_has_code(
            &build_overlay_plan(&region_overflow),
            OverlayPlanDiagnosticCode::OverlayRegionPayloadExceedsRegionCap,
        );

        let mut undefined_eviction = fixture_inputs(vec![
            resident_kernel("matvec", 128),
            resident_lut("gelu", 128),
        ]);
        undefined_eviction.policy.overlay_eviction_default = OverlayEvictionPolicy::Undefined;
        assert_has_code(
            &build_overlay_plan(&undefined_eviction),
            OverlayPlanDiagnosticCode::OverlayShareClassEvictionUndefined,
        );

        let mut missing_event = fixture_inputs(vec![resident_kernel("matvec", 128)]);
        missing_event.policy.overlay_install_event_default = None;
        assert_has_code(
            &build_overlay_plan(&missing_event),
            OverlayPlanDiagnosticCode::OverlayInstallEventDefaultMissing,
        );

        let failed_report = emit_overlay_plan_json_bytes(&build_overlay_plan(&missing_event))
            .expect("failed report emits");
        let parsed = parse_overlay_plan_report_bytes(&failed_report).expect("failed report parses");
        assert_eq!(parsed.outcome, ReportOutcome::Failed);
        assert!(parsed.body.result.is_none());

        let mut wram_cap = fixture_inputs(vec![resident_kernel("matvec", 1025)]);
        wram_cap.target_profile.wram_overlay_region_max_bytes = 2048;
        wram_cap.runtime_chrome_budget.wram_reserved = 1024;
        assert_has_code(
            &build_overlay_plan(&wram_cap),
            OverlayPlanDiagnosticCode::OverlayWramOverlayCapExceeded,
        );

        let mut empty_runtime_modes = fixture_inputs(vec![resident_kernel("matvec", 128)]);
        empty_runtime_modes.policy.runtime_modes_requested.clear();
        assert_has_code(
            &build_overlay_plan(&empty_runtime_modes),
            OverlayPlanDiagnosticCode::OverlayResolvedPolicyProjectionMismatch,
        );

        let mut unsupported_target = fixture_inputs(vec![resident_kernel("matvec", 128)]);
        unsupported_target
            .target_profile
            .allowed_overlay_constraints
            .clear();
        assert_has_code(
            &build_overlay_plan(&unsupported_target),
            OverlayPlanDiagnosticCode::OverlayTargetProfileLayoutUnsupported,
        );
    }

    #[test]
    fn reject_surface_leaks_are_hard_diagnostics() {
        assert_surface_code(
            serde_json::json!({"leak": "SectionRole"}),
            OverlayPlanDiagnosticCode::OverlaySectionRoleLeaked,
        );
        assert_surface_code(
            serde_json::json!({"leak": "ArenaPlan"}),
            OverlayPlanDiagnosticCode::OverlaySchedulingFieldLeaked,
        );
        assert_surface_code(
            serde_json::json!({"leak": "RepairProposal"}),
            OverlayPlanDiagnosticCode::OverlayRepairProvenanceForbidden,
        );
    }

    #[test]
    fn empty_overlay_demand_short_circuits_or_rejects_declared_zero_policy() {
        let empty = build_overlay_plan(&fixture_inputs(Vec::new()));
        assert_eq!(empty.outcome, OverlayPlanOutcome::Succeeded);
        let result = empty.result.as_ref().expect("empty overlay plan");
        assert!(result.product.regions.is_empty());
        assert_eq!(result.summary.reserved_bytes, 0);

        let mut explicit_zero = fixture_inputs(Vec::new());
        explicit_zero.policy.require_explicit_zero_reservation = true;
        assert_has_code(
            &build_overlay_plan(&explicit_zero),
            OverlayPlanDiagnosticCode::OverlayNoCandidatesButReservationDeclared,
        );
    }

    #[test]
    fn k11_cache_key_changes_with_rom_window_policy_and_features() {
        let identity = fixture_inputs(Vec::new()).input_identity;
        let base = OverlayPlanCacheKeyInputs::from_input_identity(&identity, hash(40))
            .cache_key()
            .expect("base key");
        let same = OverlayPlanCacheKeyInputs::from_input_identity(&identity, hash(40))
            .cache_key()
            .expect("same key");
        assert_eq!(base, same);

        let mut changed_rom = identity.clone();
        changed_rom.rom_window_plan_self_hash = hash(41);
        assert_ne!(
            base,
            OverlayPlanCacheKeyInputs::from_input_identity(&changed_rom, hash(40))
                .cache_key()
                .expect("rom key")
        );

        let mut changed_policy = identity.clone();
        changed_policy.overlay_plan_policy_projection_hash = hash(42);
        assert_ne!(
            base,
            OverlayPlanCacheKeyInputs::from_input_identity(&changed_policy, hash(40))
                .cache_key()
                .expect("policy key")
        );
        assert_ne!(
            base,
            OverlayPlanCacheKeyInputs::from_input_identity(&changed_policy, hash(43))
                .cache_key()
                .expect("feature key")
        );

        let mut changed_storage = identity.clone();
        changed_storage.storage_plan_self_hash = hash(44);
        assert_ne!(
            base,
            OverlayPlanCacheKeyInputs::from_input_identity(&changed_storage, hash(40))
                .cache_key()
                .expect("storage key")
        );

        let mut changed_sram = identity.clone();
        changed_sram.sram_page_plan_self_hash = hash(45);
        assert_ne!(
            base,
            OverlayPlanCacheKeyInputs::from_input_identity(&changed_sram, hash(40))
                .cache_key()
                .expect("sram key")
        );

        let mut changed_runtime = identity.clone();
        changed_runtime.runtime_chrome_budget_hash = hash(46);
        assert_ne!(
            base,
            OverlayPlanCacheKeyInputs::from_input_identity(&changed_runtime, hash(40))
                .cache_key()
                .expect("runtime key")
        );

        let mut changed_target = identity.clone();
        changed_target.target_profile_hash = hash(47);
        assert_ne!(
            base,
            OverlayPlanCacheKeyInputs::from_input_identity(&changed_target, hash(40))
                .cache_key()
                .expect("target key")
        );

        let mut changed_pass_version =
            OverlayPlanCacheKeyInputs::from_input_identity(&identity, hash(40));
        changed_pass_version.pass_version = "stage8_5/v2".to_owned();
        assert_ne!(
            base,
            changed_pass_version.cache_key().expect("pass version key")
        );
    }

    #[test]
    fn canonical_sort_is_stable_across_rom_overlay_demand_order() {
        let first = build_overlay_plan(&fixture_inputs(vec![
            resident_kernel("zeta", 128),
            resident_kernel("alpha", 256),
        ]));
        let second = build_overlay_plan(&fixture_inputs(vec![
            resident_kernel("alpha", 256),
            resident_kernel("zeta", 128),
        ]));
        assert_eq!(first.outcome, OverlayPlanOutcome::Succeeded);
        assert_eq!(second.outcome, OverlayPlanOutcome::Succeeded);
        assert_eq!(
            first.result.as_ref().expect("first").overlay_plan_self_hash,
            second
                .result
                .as_ref()
                .expect("second")
                .overlay_plan_self_hash
        );
    }

    #[test]
    fn file_backed_pass_fixtures_emit_report_without_overlay_cert_by_default() {
        for dir in fixture_dirs(&overlay_plan_fixture_root().join("pass")) {
            let fixture: OverlayPassFixture = read_fixture_json(&dir.join("fixture.json"));
            assert_eq!(fixture.fixture_schema, "overlay_plan.pass_fixture.v1");
            assert_eq!(fixture.expected_outcome, "passed");
            assert!(
                !fixture.expected_overlay_cert_emitted,
                "{} should document default-off overlay.cert.v1 emission",
                fixture.fixture_id
            );

            let output = build_overlay_plan(&pass_fixture_inputs(fixture.scenario));
            assert_eq!(output.outcome, OverlayPlanOutcome::Succeeded);
            assert_eq!(
                output.result.as_ref().expect("overlay result").summary,
                fixture.expected_summary,
                "{} summary drifted",
                fixture.fixture_id
            );
            let first = emit_overlay_plan_json_bytes(&output).expect("first report");
            let second = emit_overlay_plan_json_bytes(&output).expect("second report");
            assert_eq!(first, second, "{} report is canonical", fixture.fixture_id);
            let parsed = parse_overlay_plan_report_bytes(&first).expect("report parses");
            assert_eq!(parsed.outcome, ReportOutcome::Passed);
            assert!(
                !dir.join("overlay.cert.json").exists(),
                "{} should not materialize optional overlay.cert.v1 while default-off",
                fixture.fixture_id
            );
        }
    }

    #[test]
    fn file_backed_reject_fixtures_cover_every_overlay_diagnostic_code() {
        let mut codes = BTreeSet::new();
        for dir in fixture_dirs(&overlay_plan_fixture_root().join("reject")) {
            let fixture: OverlayRejectFixture = read_fixture_json(&dir.join("fixture.json"));
            assert_eq!(fixture.fixture_schema, "overlay_plan.reject_fixture.v1");
            let diagnostics = reject_fixture_diagnostics(fixture.scenario);
            let diagnostic_codes = diagnostic_codes(&diagnostics);
            assert!(
                diagnostic_codes.contains(&fixture.expected_code),
                "{} missing {:?} in {:?}",
                fixture.fixture_id,
                fixture.expected_code,
                diagnostics
            );
            let mut expected_codes = fixture.expected_extra_codes;
            expected_codes.insert(fixture.expected_code);
            assert_eq!(
                diagnostic_codes, expected_codes,
                "{} diagnostic set drifted",
                fixture.fixture_id
            );
            codes.insert(fixture.expected_code);
        }

        let expected = OverlayPlanDiagnosticCode::ALL
            .into_iter()
            .collect::<BTreeSet<_>>();
        assert_eq!(codes, expected);
    }

    #[test]
    fn full_reject_code_taxonomy_has_synthetic_fixtures() {
        let mut codes = BTreeSet::new();

        let mut hash_mismatch = fixture_inputs(vec![resident_kernel("matvec", 128)]);
        hash_mismatch
            .expected_input_hashes
            .rom_window_plan_self_hash = hash(88);
        collect_output_codes(&mut codes, &build_overlay_plan(&hash_mismatch));

        let mut wram_cap = fixture_inputs(vec![resident_kernel("matvec", 1025)]);
        wram_cap.target_profile.wram_overlay_region_max_bytes = 2048;
        wram_cap.runtime_chrome_budget.wram_reserved = 1024;
        collect_output_codes(&mut codes, &build_overlay_plan(&wram_cap));

        let mut region_overflow = fixture_inputs(vec![resident_kernel("matvec", 513)]);
        region_overflow.target_profile.wram_overlay_region_max_bytes = 512;
        collect_output_codes(&mut codes, &build_overlay_plan(&region_overflow));

        let mut undefined_eviction = fixture_inputs(vec![
            resident_kernel("matvec", 128),
            resident_lut("gelu", 128),
        ]);
        undefined_eviction.policy.overlay_eviction_default = OverlayEvictionPolicy::Undefined;
        collect_output_codes(&mut codes, &build_overlay_plan(&undefined_eviction));

        let mut missing_event = fixture_inputs(vec![resident_kernel("matvec", 128)]);
        missing_event.policy.overlay_install_event_default = None;
        collect_output_codes(&mut codes, &build_overlay_plan(&missing_event));

        let mut empty_runtime_modes = fixture_inputs(vec![resident_kernel("matvec", 128)]);
        empty_runtime_modes.policy.runtime_modes_requested.clear();
        collect_output_codes(&mut codes, &build_overlay_plan(&empty_runtime_modes));

        let mut unsupported_target = fixture_inputs(vec![resident_kernel("matvec", 128)]);
        unsupported_target
            .target_profile
            .allowed_overlay_constraints
            .clear();
        collect_output_codes(&mut codes, &build_overlay_plan(&unsupported_target));

        let mut explicit_zero = fixture_inputs(Vec::new());
        explicit_zero.policy.require_explicit_zero_reservation = true;
        collect_output_codes(&mut codes, &build_overlay_plan(&explicit_zero));

        collect_diagnostic_code(
            &mut codes,
            &validate_overlay_plan_json_surface(&serde_json::json!({"leak": "SectionRole"}))
                .expect("section leak"),
        );
        collect_diagnostic_code(
            &mut codes,
            &validate_overlay_plan_json_surface(&serde_json::json!({"leak": "ArenaPlan"}))
                .expect("schedule leak"),
        );
        collect_diagnostic_code(
            &mut codes,
            &validate_overlay_plan_json_surface(&serde_json::json!({"leak": "RepairProposal"}))
                .expect("repair leak"),
        );

        let inputs = fixture_inputs(vec![
            resident_kernel("matvec", 128),
            resident_lut("gelu", 64),
        ]);
        let valid = build_overlay_plan(&inputs)
            .result
            .expect("valid overlay result")
            .product;

        let mut empty_region = valid.clone();
        empty_region.regions[0].members.clear();
        collect_diagnostic_codes(
            &mut codes,
            &validate_overlay_plan_product_invariants(&inputs, &empty_region),
        );

        let mut duplicate_region = valid.clone();
        duplicate_region
            .regions
            .push(duplicate_region.regions[0].clone());
        collect_diagnostic_codes(
            &mut codes,
            &validate_overlay_plan_product_invariants(&inputs, &duplicate_region),
        );

        let ghost = OverlayResidentId::Kernel {
            kernel: KernelSpecId::from("ghost"),
        };

        let mut invisible_source = valid.clone();
        invisible_source.installs[0].source = OverlaySource::RomWindowOverlayDemand {
            resident: ghost.clone(),
        };
        collect_diagnostic_codes(
            &mut codes,
            &validate_overlay_plan_product_invariants(&inputs, &invisible_source),
        );

        let mut missing_install = valid.clone();
        missing_install.installs.pop();
        collect_diagnostic_codes(
            &mut codes,
            &validate_overlay_plan_product_invariants(&inputs, &missing_install),
        );

        let mut unknown_region = valid.clone();
        unknown_region.installs[0].region = OverlayId(99);
        collect_diagnostic_codes(
            &mut codes,
            &validate_overlay_plan_product_invariants(&inputs, &unknown_region),
        );

        let mut unknown_member = valid.clone();
        unknown_member.installs[0].member = ghost.clone();
        collect_diagnostic_codes(
            &mut codes,
            &validate_overlay_plan_product_invariants(&inputs, &unknown_member),
        );

        let mut incomplete_lease = valid.clone();
        incomplete_lease.installs[0]
            .lease_shape
            .wram_region_lease
            .region = OverlayId(99);
        collect_diagnostic_codes(
            &mut codes,
            &validate_overlay_plan_product_invariants(&inputs, &incomplete_lease),
        );

        let mut member_overflow = valid.clone();
        member_overflow.regions[0].bytes = 1;
        collect_diagnostic_codes(
            &mut codes,
            &validate_overlay_plan_product_invariants(&inputs, &member_overflow),
        );

        let mut sort_drift = valid.clone();
        let mut second_region = sort_drift.regions[0].clone();
        second_region.id = OverlayId(1);
        second_region.bytes = 512;
        second_region.reservation_floor_bytes = 512;
        second_region.reservation_ceil_bytes = 512;
        sort_drift.regions.push(second_region);
        collect_diagnostic_codes(
            &mut codes,
            &validate_overlay_plan_product_invariants(&inputs, &sort_drift),
        );

        let invalid_report_body = OverlayPlanReportBody {
            pass_version: OVERLAY_PLAN_PASS_VERSION.to_owned(),
            input_identity: inputs.input_identity,
            audit_parents: inputs.audit_parents,
            diagnostics: Vec::new(),
            result: None,
        };
        collect_diagnostic_codes(
            &mut codes,
            &validate_overlay_plan_report_body(&invalid_report_body, ReportOutcome::Passed)
                .expect_err("invalid report body"),
        );

        let expected = OverlayPlanDiagnosticCode::ALL
            .into_iter()
            .collect::<BTreeSet<_>>();
        assert_eq!(codes, expected);
    }

    fn assert_has_code(output: &OverlayPlanOutput, expected: OverlayPlanDiagnosticCode) {
        assert_eq!(output.outcome, OverlayPlanOutcome::Failed);
        assert!(
            output.diagnostics.iter().any(|diagnostic| matches!(
                diagnostic.code,
                ValidationCode::OverlayPlan { code, .. } if code == expected
            )),
            "missing {expected:?} in {:?}",
            output.diagnostics
        );
    }

    fn assert_surface_code(value: serde_json::Value, expected: OverlayPlanDiagnosticCode) {
        let diagnostic = validate_overlay_plan_json_surface(&value).expect("surface diagnostic");
        assert!(
            matches!(diagnostic.code, ValidationCode::OverlayPlan { code, .. } if code == expected),
            "missing {expected:?} in {:?}",
            diagnostic
        );
    }

    fn collect_output_codes(
        codes: &mut BTreeSet<OverlayPlanDiagnosticCode>,
        output: &OverlayPlanOutput,
    ) {
        collect_diagnostic_codes(codes, &output.diagnostics);
    }

    fn collect_diagnostic_codes(
        codes: &mut BTreeSet<OverlayPlanDiagnosticCode>,
        diagnostics: &[ValidationDiagnostic],
    ) {
        for diagnostic in diagnostics {
            collect_diagnostic_code(codes, diagnostic);
        }
    }

    fn collect_diagnostic_code(
        codes: &mut BTreeSet<OverlayPlanDiagnosticCode>,
        diagnostic: &ValidationDiagnostic,
    ) {
        if let ValidationCode::OverlayPlan { code, .. } = diagnostic.code {
            codes.insert(code);
        }
    }

    fn diagnostic_codes(
        diagnostics: &[ValidationDiagnostic],
    ) -> BTreeSet<OverlayPlanDiagnosticCode> {
        let mut codes = BTreeSet::new();
        collect_diagnostic_codes(&mut codes, diagnostics);
        codes
    }

    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct OverlayPassFixture {
        fixture_schema: String,
        fixture_id: String,
        scenario: OverlayPassScenario,
        expected_outcome: String,
        expected_summary: OverlayPlanSummary,
        expected_overlay_cert_emitted: bool,
    }

    #[derive(Debug, Clone, Copy, Deserialize)]
    #[serde(rename_all = "snake_case")]
    enum OverlayPassScenario {
        SingleRegionShareClass,
    }

    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct OverlayRejectFixture {
        fixture_schema: String,
        fixture_id: String,
        scenario: OverlayRejectScenario,
        expected_code: OverlayPlanDiagnosticCode,
        #[serde(default)]
        expected_extra_codes: BTreeSet<OverlayPlanDiagnosticCode>,
    }

    #[derive(Debug, Clone, Copy, Deserialize)]
    #[serde(rename_all = "snake_case")]
    enum OverlayRejectScenario {
        InputHashMismatch,
        WramOverlayCapExceeded,
        RegionPayloadExceedsRegionCap,
        RegionEmptyButPopulated,
        RegionIdDuplicate,
        ShareClassEvictionUndefined,
        InstallSourceNotVisible,
        InstallEventDefaultMissing,
        CandidateNotInstalled,
        InstallReferencesUnknownRegion,
        InstallReferencesUnknownMember,
        LeaseShapeIncomplete,
        MemberPayloadExceedsRegion,
        CanonicalSortDrift,
        ReportRoundTripFailed,
        SectionRoleLeaked,
        SchedulingFieldLeaked,
        RepairProvenanceForbidden,
        ResolvedPolicyProjectionMismatch,
        TargetProfileLayoutUnsupported,
        NoCandidatesButReservationDeclared,
    }

    fn pass_fixture_inputs(scenario: OverlayPassScenario) -> OverlayPlanInputs {
        match scenario {
            OverlayPassScenario::SingleRegionShareClass => fixture_inputs(vec![
                resident_kernel("matvec", 384),
                resident_lut("gelu", 128),
            ]),
        }
    }

    fn reject_fixture_diagnostics(scenario: OverlayRejectScenario) -> Vec<ValidationDiagnostic> {
        match scenario {
            OverlayRejectScenario::InputHashMismatch => {
                let mut input = fixture_inputs(vec![resident_kernel("matvec", 128)]);
                input.expected_input_hashes.rom_window_plan_self_hash = hash(88);
                build_overlay_plan(&input).diagnostics
            }
            OverlayRejectScenario::WramOverlayCapExceeded => {
                let mut input = fixture_inputs(vec![resident_kernel("matvec", 1025)]);
                input.target_profile.wram_overlay_region_max_bytes = 2048;
                input.runtime_chrome_budget.wram_reserved = 1024;
                build_overlay_plan(&input).diagnostics
            }
            OverlayRejectScenario::RegionPayloadExceedsRegionCap => {
                let mut input = fixture_inputs(vec![resident_kernel("matvec", 513)]);
                input.target_profile.wram_overlay_region_max_bytes = 512;
                build_overlay_plan(&input).diagnostics
            }
            OverlayRejectScenario::RegionEmptyButPopulated => {
                product_invariant_diagnostics(|product| product.regions[0].members.clear())
            }
            OverlayRejectScenario::RegionIdDuplicate => product_invariant_diagnostics(|product| {
                product.regions.push(product.regions[0].clone());
            }),
            OverlayRejectScenario::ShareClassEvictionUndefined => {
                let mut input = fixture_inputs(vec![
                    resident_kernel("matvec", 128),
                    resident_lut("gelu", 128),
                ]);
                input.policy.overlay_eviction_default = OverlayEvictionPolicy::Undefined;
                build_overlay_plan(&input).diagnostics
            }
            OverlayRejectScenario::InstallSourceNotVisible => {
                product_invariant_diagnostics(|product| {
                    let ghost = OverlayResidentId::Kernel {
                        kernel: KernelSpecId::from("ghost"),
                    };
                    product.installs[0].source =
                        OverlaySource::RomWindowOverlayDemand { resident: ghost };
                })
            }
            OverlayRejectScenario::InstallEventDefaultMissing => {
                let mut input = fixture_inputs(vec![resident_kernel("matvec", 128)]);
                input.policy.overlay_install_event_default = None;
                build_overlay_plan(&input).diagnostics
            }
            OverlayRejectScenario::CandidateNotInstalled => {
                product_invariant_diagnostics(|product| {
                    product.installs.pop();
                })
            }
            OverlayRejectScenario::InstallReferencesUnknownRegion => {
                product_invariant_diagnostics(|product| product.installs[0].region = OverlayId(99))
            }
            OverlayRejectScenario::InstallReferencesUnknownMember => {
                product_invariant_diagnostics(|product| {
                    product.installs[0].member = OverlayResidentId::Kernel {
                        kernel: KernelSpecId::from("ghost"),
                    };
                })
            }
            OverlayRejectScenario::LeaseShapeIncomplete => {
                product_invariant_diagnostics(|product| {
                    product.installs[0].lease_shape.wram_region_lease.region = OverlayId(99);
                })
            }
            OverlayRejectScenario::MemberPayloadExceedsRegion => {
                product_invariant_diagnostics(|product| product.regions[0].bytes = 1)
            }
            OverlayRejectScenario::CanonicalSortDrift => product_invariant_diagnostics(|product| {
                let mut second_region = product.regions[0].clone();
                second_region.id = OverlayId(1);
                second_region.bytes = 512;
                second_region.reservation_floor_bytes = 512;
                second_region.reservation_ceil_bytes = 512;
                product.regions.push(second_region);
            }),
            OverlayRejectScenario::ReportRoundTripFailed => {
                let input = fixture_inputs(vec![
                    resident_kernel("matvec", 128),
                    resident_lut("gelu", 64),
                ]);
                validate_overlay_plan_report_body(
                    &OverlayPlanReportBody {
                        pass_version: OVERLAY_PLAN_PASS_VERSION.to_owned(),
                        input_identity: input.input_identity,
                        audit_parents: input.audit_parents,
                        diagnostics: Vec::new(),
                        result: None,
                    },
                    ReportOutcome::Passed,
                )
                .expect_err("invalid report body")
            }
            OverlayRejectScenario::SectionRoleLeaked => {
                vec![
                    validate_overlay_plan_json_surface(&serde_json::json!({
                        "leak": "SectionRole"
                    }))
                    .expect("surface diagnostic"),
                ]
            }
            OverlayRejectScenario::SchedulingFieldLeaked => {
                vec![
                    validate_overlay_plan_json_surface(&serde_json::json!({
                        "leak": "ArenaPlan"
                    }))
                    .expect("surface diagnostic"),
                ]
            }
            OverlayRejectScenario::RepairProvenanceForbidden => {
                vec![
                    validate_overlay_plan_json_surface(&serde_json::json!({
                        "leak": "RepairProposal"
                    }))
                    .expect("surface diagnostic"),
                ]
            }
            OverlayRejectScenario::ResolvedPolicyProjectionMismatch => {
                let mut input = fixture_inputs(vec![resident_kernel("matvec", 128)]);
                input.policy.runtime_modes_requested.clear();
                build_overlay_plan(&input).diagnostics
            }
            OverlayRejectScenario::TargetProfileLayoutUnsupported => {
                let mut input = fixture_inputs(vec![resident_kernel("matvec", 128)]);
                input.target_profile.allowed_overlay_constraints.clear();
                build_overlay_plan(&input).diagnostics
            }
            OverlayRejectScenario::NoCandidatesButReservationDeclared => {
                let mut input = fixture_inputs(Vec::new());
                input.policy.require_explicit_zero_reservation = true;
                build_overlay_plan(&input).diagnostics
            }
        }
    }

    fn product_invariant_diagnostics(
        mutate: impl FnOnce(&mut OverlayPlan),
    ) -> Vec<ValidationDiagnostic> {
        let input = fixture_inputs(vec![
            resident_kernel("matvec", 128),
            resident_lut("gelu", 64),
        ]);
        let mut product = build_overlay_plan(&input)
            .result
            .expect("valid overlay result")
            .product;
        mutate(&mut product);
        validate_overlay_plan_product_invariants(&input, &product)
    }

    fn fixture_dirs(root: &Path) -> Vec<PathBuf> {
        let mut dirs = fs::read_dir(root)
            .unwrap_or_else(|error| panic!("fixture root {} reads: {error}", root.display()))
            .map(|entry| entry.expect("fixture dir entry reads").path())
            .filter(|path| path.is_dir())
            .collect::<Vec<_>>();
        dirs.sort();
        assert!(!dirs.is_empty(), "fixture root {} is empty", root.display());
        dirs
    }

    fn read_fixture_json<T>(path: &Path) -> T
    where
        T: for<'de> Deserialize<'de>,
    {
        let bytes = fs::read(path)
            .unwrap_or_else(|error| panic!("fixture file {} reads: {error}", path.display()));
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|error| panic!("fixture file {} parses: {error}", path.display()))
    }

    fn overlay_plan_fixture_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("overlay_plan")
    }

    fn fixture_inputs(residents: Vec<OverlayResident>) -> OverlayPlanInputs {
        let hashes = OverlayPlanInputHashes {
            storage_plan_self_hash: hash(1),
            sram_page_plan_self_hash: hash(2),
            rom_window_plan_self_hash: hash(3),
            runtime_chrome_budget_hash: hash(4),
            target_profile_hash: hash(5),
            overlay_plan_policy_projection_hash: hash(6),
        };
        OverlayPlanInputs {
            input_identity: OverlayPlanInputIdentity {
                storage_plan_self_hash: hashes.storage_plan_self_hash,
                sram_page_plan_self_hash: hashes.sram_page_plan_self_hash,
                rom_window_plan_self_hash: hashes.rom_window_plan_self_hash,
                runtime_chrome_budget_hash: hashes.runtime_chrome_budget_hash,
                target_profile_hash: hashes.target_profile_hash,
                overlay_plan_policy_projection_hash: hashes.overlay_plan_policy_projection_hash,
                determinism: DeterminismClass::Deterministic,
                target_profile_id: TargetProfileId::from("dmg-mbc5"),
                schema_version: OVERLAY_PLAN_SCHEMA_VERSION,
            },
            expected_input_hashes: hashes,
            audit_parents: OverlayPlanAuditParents {
                policy_resolution_self_hash: hash(7),
                artifact_validation_self_hash: hash(8),
                compile_request_hash: hash(9),
            },
            runtime_chrome_budget: RuntimeChromeBudget {
                target: TargetProfileId::from("dmg-mbc5"),
                profile: CompileProfileId::from("Bringup"),
                runtime_nucleus_hash: hash(10),
                rom_slots: Vec::<RomBudgetSlot>::new(),
                memory_caps: RuntimeMemoryCapSection {
                    wram_usable_bytes: 8 * 1024,
                    sram_usable_bytes: 32 * 1024,
                    hram_usable_bytes: 127,
                    source_target_profile_hash: hash(11),
                },
                wram_reserved: 1024,
                sram_reserved: 512,
            },
            target_profile: OverlayTargetProfileSummary {
                allowed_overlay_constraints: BTreeSet::from([
                    WramRegionConstraint::DmgWramC000Dfff,
                ]),
                default_overlay_constraint: WramRegionConstraint::DmgWramC000Dfff,
                wram_overlay_region_max_bytes: 1024,
            },
            policy: OverlayPlanPolicyProjection {
                overlay_eviction_default: OverlayEvictionPolicy::ReloadOnUse,
                overlay_install_event_default: Some(OverlayInstallEvent::ExpertSwitch),
                runtime_modes_requested: BTreeSet::from([RuntimeMode::Steady]),
                require_explicit_zero_reservation: false,
            },
            rom_window_plan: rom_window_plan(hashes.rom_window_plan_self_hash, residents),
        }
    }

    fn rom_window_plan(self_hash: Hash256, residents: Vec<OverlayResident>) -> RomWindowPlan {
        let kernels = residents
            .iter()
            .filter_map(|resident| match &resident.id {
                OverlayResidentId::Kernel { kernel } => Some(WramOverlayKernelDemand {
                    kernel: kernel.clone(),
                    byte_size: resident.payload_bytes,
                    reachability: resident.reachability,
                    source_bank: crate::window::RomBankIndex(1),
                    active_epochs: Vec::new(),
                }),
                OverlayResidentId::Lut { .. } => None,
            })
            .collect::<Vec<_>>();
        let luts = residents
            .iter()
            .filter_map(|resident| match &resident.id {
                OverlayResidentId::Lut { lut } => Some(WramOverlayLutDemand {
                    lut: lut.clone(),
                    byte_size: resident.payload_bytes,
                    reachability: resident.reachability,
                    source_bank: crate::window::RomBankIndex(1),
                    active_epochs: Vec::new(),
                }),
                OverlayResidentId::Kernel { .. } => None,
            })
            .collect::<Vec<_>>();
        RomWindowPlan {
            identity: RomWindowPlanInputIdentity {
                artifact_validation_self_hash: hash(0x10),
                policy_resolution_self_hash: hash(0x11),
                static_budget_self_hash: hash(0x12),
                quant_graph_self_hash: hash(0x13),
                infer_ir_self_hash: hash(0),
                storage_plan_self_hash: hash(1),
                observation_plan_self_hash: hash(12),
                range_plan_self_hash: hash(13),
                sram_page_plan_self_hash: hash(2),
                runtime_chrome_budget_hash: hash(4),
                target_profile_hash: hash(5),
                rom_window_plan_policy_projection_hash: hash(14),
                runtime_mode: RuntimeMode::Steady,
                determinism: DeterminismClass::Deterministic,
                target_profile_id: TargetProfileId::from("dmg-mbc5"),
                schema_version: ROM_WINDOW_PLAN_SCHEMA_VERSION,
            },
            kernel_residency: BTreeMap::new(),
            lut_residency: BTreeMap::new(),
            rom_window_bindings: Vec::new(),
            banks: Vec::new(),
            residency_epochs: Vec::new(),
            co_resident_closures: Vec::new(),
            overlay_demand: WramOverlayDemand {
                total_overlay_bytes: residents
                    .iter()
                    .map(|resident| resident.payload_bytes)
                    .sum::<u32>(),
                total_install_count_per_token_upper_bound: saturating_u16(residents.len()),
                install_source_visibility: Vec::new(),
                kernels,
                luts,
            },
            bank0_demand: Bank0Demand {
                kernels: Vec::new(),
                luts: Vec::new(),
                total_kernel_bytes: 0,
                total_lut_bytes: 0,
                remaining_slack_bytes: 0,
            },
            projections: RomSwitchProjections {
                projected_bank_switches_per_token: 0,
                upper_bound_per_token: 0,
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
            rom_window_plan_self_hash: self_hash,
        }
    }

    fn resident_kernel(id: &str, payload_bytes: u32) -> OverlayResident {
        let resident = OverlayResidentId::Kernel {
            kernel: KernelSpecId::from(id),
        };
        OverlayResident {
            id: resident.clone(),
            payload_bytes,
            reachability: RomReachabilityClass::HotPath,
            source: OverlaySource::RomWindowOverlayDemand { resident },
        }
    }

    fn resident_lut(id: &str, payload_bytes: u32) -> OverlayResident {
        let resident = OverlayResidentId::Lut {
            lut: LutInstanceId(id.to_owned()),
        };
        OverlayResident {
            id: resident.clone(),
            payload_bytes,
            reachability: RomReachabilityClass::HotPath,
            source: OverlaySource::RomWindowOverlayDemand { resident },
        }
    }

    fn hash(byte: u8) -> Hash256 {
        Hash256::from_bytes([byte; 32])
    }
}
