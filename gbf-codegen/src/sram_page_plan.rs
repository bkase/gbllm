//! Stage 7 `SramPagePlan` construction, report, and cache-key surface.

use std::collections::{BTreeMap, BTreeSet};
use std::{error::Error, fmt};

use gbf_foundation::{
    CanonicalJsonError, DomainHash, EvidenceRef, Hash256, SemVer,
    canonical_json_bytes_omitting_fields, self_hash_omitting_fields,
};
use gbf_policy::{
    DiagnosticSeverity, RuntimeChromeBudget, SramKnob, SramPagePlanDiagnosticCode,
    SramPagePlanDiagnosticProvenance, ValidationCode, ValidationDetail, ValidationDiagnostic,
    ValidationOrigin,
};
use gbf_report::{
    CanonicalJsonError as ReportCanonicalJsonError, ReportBody, ReportEnvelope,
    ReportEnvelopeError, ReportOutcome, ReportSelfHashError, canonicalize, round_trip_self_hash,
};
use serde::{Deserialize, Serialize};

use crate::s1::quant_graph::DeterminismClass;
use crate::s3::infer_ir::ValueId;
use crate::stage_cache::{
    CodegenStageCacheError, StoreBackedStageCacheKeys, StoreBackedStageCellKind,
    StoreBackedStageExpectedHashes, StoreBackedStageRunOutput, StoreBackedStageRunResult,
    crate_feature_set_hash, run_store_backed_stage_with_cache, stage7_sram_page_plan_store_key,
};
use crate::storage_plan::types::{CommitGroupId, Materialization, StorageBinding, StorageClass};
use gbf_store::stage_cache::StageCache as StoreStageCache;

pub const SRAM_PAGE_PLAN_SCHEMA_ID: &str = "sram_page_plan.v1";
pub const SRAM_PAGE_PLAN_SCHEMA_VERSION: SemVer = SemVer::new(1, 0, 0);
pub const SRAM_PAGE_PLAN_PASS_VERSION: &str = "stage7/v1";

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
    pub geometry: PersistentPageGeometry,
    pub expected_geometry: PersistentPageGeometry,
    pub bindings: Vec<SramPagePlanBindingInput>,
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
    pub bindings: Vec<SramPageBinding>,
    pub pages: Vec<PersistentPage>,
    pub stream_index: Vec<SramStreamIndexEntry>,
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
        candidates.push(SramCandidate {
            binding_id: binding.binding.value,
            commit_group,
            sequence_stream: binding.sequence_stream,
            payload_bytes: binding.payload_bytes,
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
            residency: resolved_residency,
            payload_bytes: candidate.payload_bytes,
            geometry: input.geometry,
            sequence_stream: candidate.sequence_stream,
        });
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
    let mut plan = SramPagePlan {
        identity: input.input_identity.clone(),
        bindings,
        pages,
        stream_index,
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

pub fn parse_sram_page_plan_report_bytes(
    bytes: &[u8],
) -> Result<SramPagePlanReportEnvelope, ReportSelfHashError> {
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
}

impl fmt::Display for SramPagePlanEmitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Envelope(error) => write!(f, "sram page report envelope failed: {error}"),
            Self::SelfHash(error) => write!(f, "sram page report self hash failed: {error}"),
            Self::Canonical(error) => {
                write!(f, "sram page report canonicalization failed: {error}")
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

fn page_allocation_failure(target_profile_hash: Hash256) -> ValidationDiagnostic {
    diagnostic(
        SramPagePlanDiagnosticCode::SramTargetProfileLayoutUnsupported,
        SramPagePlanDiagnosticProvenance::TargetProfileLayout {
            target_profile_hash,
            detail: "Stage 7 exhausted the u8 SRAM PageId address space".to_owned(),
        },
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SramCandidate {
    binding_id: ValueId,
    commit_group: CommitGroupId,
    sequence_stream: SequenceStreamId,
    payload_bytes: u32,
    residency: PageResidency,
}

#[cfg(test)]
mod tests {
    use gbf_foundation::{CompileProfileId, TargetProfileId};
    use gbf_policy::{BudgetSlotClass, PlacementProfile, RomBudgetSlot, RuntimeMemoryCapSection};

    use super::*;
    use crate::s3::infer_ir::NodeId;
    use crate::storage_plan::types::{
        AbstractLiveRange, AliasClassId, BindingJustification, DecisionRuleId, LifetimeClass,
        PersistPageId,
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
            geometry: PersistentPageGeometry::dmg_mbc5_8k(),
            expected_geometry: PersistentPageGeometry::dmg_mbc5_8k(),
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

    fn hash(byte: u8) -> Hash256 {
        Hash256::from_bytes([byte; 32])
    }
}
