//! Stage 5 `RangePlan` core types and identity.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Instant;

use gbf_foundation::{CompileProfileId, EvidenceRef, ExpertId, FieldPath, Hash256, LayerId};
use gbf_policy::{
    DiagnosticSeverity, PolicySource, RangeCapsSpec, ReductionPlanCeiling, ReductionSiteId,
    RenormStrategyPolicy, ValidationCode, ValidationDetail, ValidationOrigin,
};
use gbf_report::{
    ReportBody, ReportEnvelope, ReportOutcome, ValidationDiagnostic, canonicalize_value,
};
use gbf_report::{canonicalize as canonicalize_report, compute_self_hash};
use gbf_store::stage_cache::StageCache as StoreStageCache;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::budget::{
    AccumulatorDomain, ReductionSiteProjection, StaticBudgetReductionSiteFacts, StaticBudgetReport,
};
use crate::s1::quant_graph::{DeterminismClass, ExpertWeightSlot, NormSite};
use crate::s3::infer_ir::{
    GbInferIR, InferOp, InferOpTag, NodeId, QuantGraphEntityRef, ResidualSite,
};
use crate::s3::infer_ir::{GbInferIRProduct, infer_ir_self_hash};
use crate::stage_cache::{
    CodegenStageCacheError, Stage5CacheKeyMaterial, Stage5ReportRewrapContext,
    get_stage5_failure_memo, get_stage5_success, put_stage5_failure_memo, put_stage5_success,
    rewrap_stage5_cached_failure, rewrap_stage5_cached_success,
};

pub const RANGE_PLAN_SCHEMA_VERSION: &str = "range_plan.v1";
pub const RANGE_PLAN_CORE_PRODUCT_SCHEMA_VERSION: &str = "range_plan_core_product.v1";
pub const RANGE_CERT_SCHEMA_VERSION: &str = "range.cert.v1";
pub const RANGE_REPORT_SCHEMA_SEMVER: &str = "1.0.0";
pub const RANGE_PLAN_SELF_HASH_COMPUTED_EVENT: &str = "gbf_codegen.range_plan.self_hash_computed";
pub const RANGE_POLICY_PROJECTION_HASH_COMPUTED_EVENT: &str =
    "gbf_codegen.range_policy_projection.hash_computed";
pub const RANGE_PLAN_CORE_PRODUCT_HASH_COMPUTED_EVENT: &str =
    "gbf_codegen.range_plan.core_product.hash_computed";
pub const RANGE_CERT_BODY_HASH_COMPUTED_EVENT: &str = "gbf_codegen.range_cert.body_hash_computed";
pub const RANGE_DETERMINISM_MISMATCH_CODE: &str = "RANGE-DETERMINISM-MISMATCH";
pub const RANGE_DUPLICATE_REDUCTION_SITE_ID_CODE: &str = "RANGE-DUPLICATE-REDUCTION-SITE-ID";
pub const RANGE_TERM_COUNT_ZERO_CODE: &str = "RANGE-TERM-COUNT-ZERO";
pub const RANGE_ACCUMULATOR_DOMAIN_UNSUPPORTED_V1_CODE: &str =
    "RANGE-ACCUMULATOR-DOMAIN-UNSUPPORTED-V1";
pub const RANGE_SITE_MISSING_FROM_STATIC_BUDGET_CODE: &str =
    "RANGE-SITE-MISSING-FROM-STATIC-BUDGET";
pub const RANGE_STATIC_BUDGET_SITE_ORPHANED_CODE: &str = "RANGE-STATIC-BUDGET-SITE-ORPHANED";
pub const RANGE_SITE_FACTS_INCONSISTENT_CODE: &str = "RANGE-SITE-FACTS-INCONSISTENT";
pub const RANGE_INTEGER_OVERFLOW_DURING_PROOF_CODE: &str = "RANGE-INTEGER-OVERFLOW-DURING-PROOF";
pub const RANGE_CEILING_OVERRIDE_INVALID_SELECTOR_CODE: &str =
    "RANGE-CEILING-OVERRIDE-INVALID-SELECTOR";
pub const RANGE_CEILING_OVERRIDE_AMBIGUOUS_CODE: &str = "RANGE-CEILING-OVERRIDE-AMBIGUOUS";
pub const RANGE_BITEXACT_REQUIRES_CHUNK_DIVIDES_CODE: &str =
    "RANGE-BITEXACT-REQUIRES-CHUNK-DIVIDES";
pub const RANGE_BITEXACT_RENORM_LOOP_RESERVED_V1_CODE: &str =
    "RANGE-BITEXACT-RENORM-LOOP-RESERVED-V1";
pub const RANGE_CHUNK_LEN_EXCEEDS_PROFILE_MAX_CODE: &str = "RANGE-CHUNK-LEN-EXCEEDS-PROFILE-MAX";
pub const RANGE_TILE_LEN_BELOW_PROFILE_MIN_CODE: &str = "RANGE-TILE-LEN-BELOW-PROFILE-MIN";
pub const RANGE_TILE_LEN_EXCEEDS_PROFILE_MAX_CODE: &str = "RANGE-TILE-LEN-EXCEEDS-PROFILE-MAX";
pub const RANGE_TILE_LEN_EXCEEDS_U16_CODE: &str = "RANGE-TILE-LEN-EXCEEDS-U16";
pub const RANGE_IDENTITY_BIND_EVENT: &str = "stage5.range_plan.identity_bind";
pub const RANGE_REDUCTION_SITE_ENUMERATION_EVENT: &str =
    "stage5.range_plan.reduction_site_enumeration";
pub const RANGE_SITE_FACTS_BINDING_EVENT: &str = "stage5.range_plan.site_facts_binding";
pub const RANGE_EFFECTIVE_CEILING_BINDING_EVENT: &str =
    "stage5.range_plan.effective_ceiling_binding";
pub const RANGE_PLAN_CANDIDATE_GENERATION_EVENT: &str =
    "stage5.range_plan.plan_candidate_generation";
pub const RANGE_PLAN_LENGTH_SELECTION_EVENT: &str = "stage5.range_plan.plan_length_selection";
pub const RANGE_CERTIFICATE_CONSTRUCTION_EVENT: &str = "stage5.range_plan.certificate_construction";
pub const RANGE_PLAN_CHOICE_EVENT: &str = "stage5.range_plan.plan_choice";
pub const RANGE_PROVENANCE_BIND_EVENT: &str = "stage5.range_plan.provenance_bind";
pub const RANGE_CANONICAL_SORT_EVENT: &str = "stage5.range_plan.canonical_sort";
pub const RANGE_SELF_CONSISTENCY_EVENT: &str = "stage5.range_plan.self_consistency";
pub const STAGE5_DRIVER_REPORT_EMIT_EVENT: &str = "stage5.driver.report_emit";
pub const STAGE5_DRIVER_FAILURE_MEMO_EVENT: &str = "stage5.driver.failure_memo";
pub const STAGE5_DRIVER_RUN_EVENT: &str = "stage5.driver.run";
pub const RANGE_CERT_VERIFIES_SINGLE_I16_EVENT: &str = "range_cert.verifies.single_i16";
pub const RANGE_CERT_VERIFIES_CHUNKED_I16_EVENT: &str = "range_cert.verifies.chunked_i16";
pub const RANGE_CERT_VERIFIES_RENORM_LOOP_EVENT: &str = "range_cert.verifies.renorm_loop";
pub const RANGE_CERT_VERIFIES_FAILED_EVENT: &str = "range_cert.verifies.failed";
pub const RANGE_CERT_REJECTS_BITEXACT_RENORM_LOOP_EVENT: &str =
    "range_cert.rejects.bitexact_renorm_loop";
pub const RANGE_CERT_RENORM_RECURRENCE_VERIFIES_EVENT: &str =
    "range_cert.renorm_recurrence_verifies";
pub const RANGE_CEILING_VIOLATED_SINGLE_I16_ONLY_CODE: &str =
    "RANGE-CEILING-VIOLATED-SINGLE-I16-ONLY";
pub const RANGE_CEILING_VIOLATED_NO_RENORM_LOOP_CODE: &str =
    "RANGE-CEILING-VIOLATED-NO-RENORM-LOOP";
pub const RANGE_NO_PROVEN_PLAN_WITHIN_CEILING_CODE: &str = "RANGE-NO-PROVEN-PLAN-WITHIN-CEILING";

const I16_ENVELOPE_U64: u64 = i16::MAX as u64;
const I32_ENVELOPE_U64: u64 = i32::MAX as u64;

#[cfg(test)]
thread_local! {
    static RANGE_CONSTRUCTION_EVENT_LOG: std::cell::RefCell<Vec<&'static str>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

#[cfg(test)]
fn record_range_construction_event(event: &'static str) {
    RANGE_CONSTRUCTION_EVENT_LOG.with(|log| log.borrow_mut().push(event));
}

#[cfg(not(test))]
fn record_range_construction_event(_event: &'static str) {}

#[cfg(test)]
fn take_recorded_range_construction_events() -> Vec<&'static str> {
    RANGE_CONSTRUCTION_EVENT_LOG.with(|log| std::mem::take(&mut *log.borrow_mut()))
}

pub type ValidationDiagnosticRecord = ValidationDiagnostic;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RangePlanInputs {
    pub infer_ir_product: GbInferIRProduct,
    pub infer_ir_self_hash: Hash256,
    pub quant_graph_self_hash: Hash256,
    pub static_budget_report: StaticBudgetReport,
    pub static_budget_self_hash: Hash256,
    pub range_policy_projection: RangePolicyProjection,
    pub audit_parents: RangePlanAuditParents,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RangePolicyProjection {
    pub profile_id: CompileProfileId,
    pub range_caps: RangeCapsSpec,
    pub reduction_ceiling: ReductionPlanCeiling,
    #[serde(with = "duplicate_rejecting_reduction_ceiling_overrides")]
    pub reduction_ceiling_overrides: BTreeMap<ReductionSelector, ReductionPlanCeiling>,
    pub determinism_class: DeterminismClass,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ReductionSelector(pub String);

impl ReductionSelector {
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

impl From<&str> for ReductionSelector {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for ReductionSelector {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

mod duplicate_rejecting_reduction_ceiling_overrides {
    use std::collections::BTreeMap;
    use std::fmt;

    use gbf_policy::ReductionPlanCeiling;
    use serde::de::{Error as _, MapAccess, Visitor};
    use serde::{Deserializer, Serialize, Serializer};

    use super::ReductionSelector;

    pub fn serialize<S>(
        overrides: &BTreeMap<ReductionSelector, ReductionPlanCeiling>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        overrides.serialize(serializer)
    }

    pub fn deserialize<'de, D>(
        deserializer: D,
    ) -> Result<BTreeMap<ReductionSelector, ReductionPlanCeiling>, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_map(DuplicateRejectingOverridesVisitor)
    }

    struct DuplicateRejectingOverridesVisitor;

    impl<'de> Visitor<'de> for DuplicateRejectingOverridesVisitor {
        type Value = BTreeMap<ReductionSelector, ReductionPlanCeiling>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a map of reduction ceiling overrides with unique keys")
        }

        fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
        where
            A: MapAccess<'de>,
        {
            let mut overrides = BTreeMap::new();
            while let Some((selector, ceiling)) =
                map.next_entry::<ReductionSelector, ReductionPlanCeiling>()?
            {
                if overrides.insert(selector.clone(), ceiling).is_some() {
                    return Err(A::Error::custom(format!(
                        "duplicate reduction ceiling override key {:?}",
                        selector.0
                    )));
                }
            }
            Ok(overrides)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockedRangeKnobs {
    pub reduction_ceiling_locked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RangePlanConstructionError {
    DeterminismMismatch {
        projection: DeterminismClass,
        infer_ir: DeterminismClass,
    },
    RangePolicyProjectionHash {
        message: String,
    },
    DuplicateReductionSiteId {
        site: ReductionSiteId,
        first_node: NodeId,
        duplicate_node: NodeId,
    },
    TermCountZero {
        site: ReductionSiteId,
    },
    AccumulatorDomainUnsupportedV1 {
        site: ReductionSiteId,
        accumulator_domain: AccumulatorDomain,
    },
    SiteMissingFromStaticBudget {
        site: ReductionSiteId,
        node: NodeId,
    },
    StaticBudgetSiteOrphaned {
        site: ReductionSiteId,
    },
    SiteFactsInconsistent {
        site: ReductionSiteId,
        computed_per_term_abs_max_q: Option<u64>,
        published_per_term_abs_max_q: u64,
    },
    CeilingOverrideInvalidSelector {
        selector: ReductionSelector,
    },
    CeilingOverrideAmbiguous {
        site: ReductionSiteId,
        selectors: Vec<ReductionSelector>,
    },
    IntegerOverflowDuringProof {
        site: Option<ReductionSiteId>,
        field: &'static str,
    },
}

impl RangePlanConstructionError {
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::DeterminismMismatch { .. } => RANGE_DETERMINISM_MISMATCH_CODE,
            Self::RangePolicyProjectionHash { .. } => "RANGE-POLICY-PROJECTION-HASH-FAILED",
            Self::DuplicateReductionSiteId { .. } => RANGE_DUPLICATE_REDUCTION_SITE_ID_CODE,
            Self::TermCountZero { .. } => RANGE_TERM_COUNT_ZERO_CODE,
            Self::AccumulatorDomainUnsupportedV1 { .. } => {
                RANGE_ACCUMULATOR_DOMAIN_UNSUPPORTED_V1_CODE
            }
            Self::SiteMissingFromStaticBudget { .. } => RANGE_SITE_MISSING_FROM_STATIC_BUDGET_CODE,
            Self::StaticBudgetSiteOrphaned { .. } => RANGE_STATIC_BUDGET_SITE_ORPHANED_CODE,
            Self::SiteFactsInconsistent { .. } => RANGE_SITE_FACTS_INCONSISTENT_CODE,
            Self::CeilingOverrideInvalidSelector { .. } => {
                RANGE_CEILING_OVERRIDE_INVALID_SELECTOR_CODE
            }
            Self::CeilingOverrideAmbiguous { .. } => RANGE_CEILING_OVERRIDE_AMBIGUOUS_CODE,
            Self::IntegerOverflowDuringProof { .. } => RANGE_INTEGER_OVERFLOW_DURING_PROOF_CODE,
        }
    }

    #[must_use]
    pub fn diagnostic(&self) -> ValidationDiagnostic {
        let field = match self {
            Self::DeterminismMismatch { .. } => "range_policy_projection.determinism_class",
            Self::RangePolicyProjectionHash { .. } => "range_policy_projection_hash",
            Self::DuplicateReductionSiteId { .. } => "infer_ir.nodes.reduction_site",
            Self::TermCountZero { .. } => "static_budget.reduction_site.term_count",
            Self::AccumulatorDomainUnsupportedV1 { .. } => {
                "static_budget.reduction_site.accumulator_domain"
            }
            Self::SiteMissingFromStaticBudget { .. } => "static_budget.reduction_site",
            Self::StaticBudgetSiteOrphaned { .. } => "static_budget.reduction_sites",
            Self::SiteFactsInconsistent { .. } => "static_budget.reduction_site.per_term_abs_max_q",
            Self::CeilingOverrideInvalidSelector { .. } | Self::CeilingOverrideAmbiguous { .. } => {
                "range_policy_projection.reduction_ceiling_overrides"
            }
            Self::IntegerOverflowDuringProof { field, .. } => field,
        };
        ValidationDiagnostic::hard(
            ValidationOrigin::RangePlanConstruction,
            ValidationCode::ReportSemanticInvariantViolated {
                field: FieldPath::from(field),
            },
            ValidationDetail::Field {
                field: FieldPath::from(field),
            },
            vec![EvidenceRef {
                kind: "stage5-range-plan-construction".to_owned(),
                reference: self.code().to_owned(),
                hash: Some(Hash256::ZERO),
            }],
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReductionSiteBinding {
    pub node_id: NodeId,
    pub site: ReductionSiteId,
    pub qg_ref: QuantGraphEntityRef,
    pub op_tag: InferOpTag,
    pub slot: Option<ExpertWeightSlot>,
    pub norm_site: Option<NormSite>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundReductionSiteFacts {
    pub binding: ReductionSiteBinding,
    pub facts: ReductionSiteFacts,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveCeilingBinding {
    pub site: ReductionSiteId,
    pub facts: ReductionSiteFacts,
    pub effective_ceiling: ReductionPlanCeiling,
    pub provenance: ReductionCeilingProvenance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ReductionPlanFamily {
    SingleI16,
    ChunkedI16,
    RenormLoop,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanCandidateGeneration {
    pub site: ReductionSiteId,
    pub candidate_families: Vec<ReductionPlanFamily>,
    pub candidates: Vec<ReductionPlan>,
    pub rejections: Vec<PlanCandidateRejection>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanCandidateRejection {
    pub family: ReductionPlanFamily,
    pub error: PlanLengthSelectionError,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertificateConstruction {
    pub plan: ReductionPlan,
    pub certificate: AccumulatorCertificate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanChoice {
    pub site: ReductionSiteId,
    pub chosen: Option<CertifiedReduction>,
    pub attempts: Vec<CertificateConstruction>,
    pub diagnostics: Vec<ValidationDiagnosticRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanLengthSelectionError {
    PerChunkExceedsI16Envelope {
        per_chunk_sum_bound: u64,
        envelope: u64,
    },
    ChunkLenExceedsProfileMax {
        chunk_len: u16,
        profile_chunk_max: u16,
    },
    BitExactRequiresChunkDivides {
        term_count: u32,
        chunk_len: u16,
    },
    BitExactRenormLoopReservedV1,
    PerTileExceedsI16Envelope {
        per_tile_sum_bound: u64,
        envelope: u64,
    },
    TileLenBelowProfileMin {
        tile_len: u16,
        profile_tile_min: u16,
    },
    TileLenExceedsProfileMax {
        tile_len: u16,
        profile_tile_max: u16,
    },
    TileLenExceedsU16 {
        term_count: u32,
    },
}

impl PlanLengthSelectionError {
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::PerChunkExceedsI16Envelope { .. } => "RANGE-PER-CHUNK-EXCEEDS-I16-ENVELOPE",
            Self::ChunkLenExceedsProfileMax { .. } => RANGE_CHUNK_LEN_EXCEEDS_PROFILE_MAX_CODE,
            Self::BitExactRequiresChunkDivides { .. } => RANGE_BITEXACT_REQUIRES_CHUNK_DIVIDES_CODE,
            Self::BitExactRenormLoopReservedV1 => RANGE_BITEXACT_RENORM_LOOP_RESERVED_V1_CODE,
            Self::PerTileExceedsI16Envelope { .. } => "RANGE-PER-TILE-EXCEEDS-I16-ENVELOPE",
            Self::TileLenBelowProfileMin { .. } => RANGE_TILE_LEN_BELOW_PROFILE_MIN_CODE,
            Self::TileLenExceedsProfileMax { .. } => RANGE_TILE_LEN_EXCEEDS_PROFILE_MAX_CODE,
            Self::TileLenExceedsU16 { .. } => RANGE_TILE_LEN_EXCEEDS_U16_CODE,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RangePlanAuditParents {
    pub policy_resolution_self_hash: Hash256,
    pub compile_request_hash: Hash256,
    pub artifact_aux_hash: Hash256,
    pub locked_range_knobs: LockedRangeKnobs,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RangePlanCoreProduct {
    pub range_plan: RangePlan,
    pub range_cert: RangeCertBody,
    pub range_plan_self_hash: Hash256,
    pub range_cert_body_hash: Hash256,
}

pub type RangePlanProduct = RangePlanCoreProduct;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RangePlanStageOutput {
    pub product: RangePlanCoreProduct,
    pub report: ReportEnvelope<RangePlanReportBody>,
    pub cert_report: ReportEnvelope<RangeCertBody>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RangePlanStageFailure {
    pub report: ReportEnvelope<RangePlanReportBody>,
    pub cert_report: Option<ReportEnvelope<RangeCertBody>>,
    pub diagnostics: Vec<ValidationDiagnosticRecord>,
}

#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum RunStage5Error {
    StageFailure(RangePlanStageFailure),
    StageCache(CodegenStageCacheError),
    ReportIo(io::Error),
}

#[derive(Clone, Copy)]
pub struct Stage5PassEnvironment<'a> {
    pub report_dir: Option<&'a Path>,
    pub stage_cache: Option<&'a StoreStageCache<'a>>,
}

impl<'a> Stage5PassEnvironment<'a> {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            report_dir: None,
            stage_cache: None,
        }
    }

    #[must_use]
    pub const fn with_report_dir(mut self, report_dir: &'a Path) -> Self {
        self.report_dir = Some(report_dir);
        self
    }

    #[must_use]
    pub const fn with_stage_cache(mut self, stage_cache: &'a StoreStageCache<'a>) -> Self {
        self.stage_cache = Some(stage_cache);
        self
    }
}

impl Default for Stage5PassEnvironment<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl RunStage5Error {
    #[must_use]
    pub const fn stage_failure(&self) -> Option<&RangePlanStageFailure> {
        match self {
            Self::StageFailure(failure) => Some(failure),
            Self::StageCache(_) | Self::ReportIo(_) => None,
        }
    }
}

impl fmt::Display for RunStage5Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StageFailure(failure) => write!(
                f,
                "Stage 5 range plan failed with {} diagnostic(s)",
                failure.diagnostics.len()
            ),
            Self::StageCache(err) => write!(f, "Stage 5 cache error: {err}"),
            Self::ReportIo(err) => write!(f, "Stage 5 report I/O error: {err}"),
        }
    }
}

impl std::error::Error for RunStage5Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::StageFailure(_) => None,
            Self::StageCache(err) => Some(err),
            Self::ReportIo(err) => Some(err),
        }
    }
}

impl From<CodegenStageCacheError> for RunStage5Error {
    fn from(value: CodegenStageCacheError) -> Self {
        Self::StageCache(value)
    }
}

impl From<io::Error> for RunStage5Error {
    fn from(value: io::Error) -> Self {
        Self::ReportIo(value)
    }
}

impl From<serde_json::Error> for RunStage5Error {
    fn from(value: serde_json::Error) -> Self {
        Self::ReportIo(io::Error::other(value.to_string()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RangePlanCoreSuccess {
    pub product: RangePlanCoreProduct,
    pub range_plan_body: RangePlanReportBody,
    pub range_cert_body: RangeCertBody,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RangePlanCoreFailure {
    pub range_plan_body: RangePlanReportBody,
    pub range_cert_body: Option<RangeCertBody>,
    pub diagnostics: Vec<ValidationDiagnosticRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RangePlanReportBody {
    pub input_identity: RangePlanReportInputIdentity,
    pub result: Option<RangePlanReportResult>,
    pub diagnostics: Vec<ValidationDiagnosticRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RangePlanReportInputIdentity {
    pub infer_ir_self_hash: Hash256,
    pub quant_graph_self_hash: Hash256,
    pub static_budget_self_hash: Hash256,
    pub range_policy_projection_hash: Hash256,
    pub policy_resolution_self_hash: Hash256,
    pub compile_request_hash: Hash256,
    pub artifact_aux_hash: Hash256,
    pub determinism: DeterminismClass,
}

impl RangePlanReportInputIdentity {
    #[must_use]
    pub fn from_inputs(inputs: &RangePlanInputs, identity: &RangePlanIdentity) -> Self {
        Self {
            infer_ir_self_hash: identity.infer_ir_self_hash,
            quant_graph_self_hash: identity.quant_graph_self_hash,
            static_budget_self_hash: identity.static_budget_self_hash,
            range_policy_projection_hash: identity.range_policy_projection_hash,
            policy_resolution_self_hash: inputs.audit_parents.policy_resolution_self_hash,
            compile_request_hash: inputs.audit_parents.compile_request_hash,
            artifact_aux_hash: inputs.audit_parents.artifact_aux_hash,
            determinism: identity.determinism,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RangePlanReportResult {
    pub product: RangePlan,
    pub entry_count: u32,
    pub single_i16_count: u32,
    pub chunked_i16_count: u32,
    pub renorm_loop_count: u32,
    #[serde(with = "effective_ceiling_histogram")]
    pub effective_ceiling_histogram: BTreeMap<ReductionPlanCeiling, u32>,
    #[serde(with = "ceiling_provenance_histogram")]
    pub ceiling_provenance_histogram: BTreeMap<ReductionCeilingProvenanceTag, u32>,
    pub range_cert_report_self_hash: Hash256,
    pub range_plan_self_hash: Hash256,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ReductionCeilingProvenanceTag {
    Global,
    LayerOverride,
    SiteOverride,
}

impl From<&ReductionCeilingProvenance> for ReductionCeilingProvenanceTag {
    fn from(value: &ReductionCeilingProvenance) -> Self {
        match value {
            ReductionCeilingProvenance::Global { .. } => Self::Global,
            ReductionCeilingProvenance::LayerOverride { .. } => Self::LayerOverride,
            ReductionCeilingProvenance::SiteOverride { .. } => Self::SiteOverride,
        }
    }
}

mod effective_ceiling_histogram {
    use std::collections::BTreeMap;

    use gbf_policy::ReductionPlanCeiling;
    use serde::de::Error as _;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S>(
        histogram: &BTreeMap<ReductionPlanCeiling, u32>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        histogram
            .iter()
            .map(|(key, count)| (ceiling_key(*key), *count))
            .collect::<BTreeMap<_, _>>()
            .serialize(serializer)
    }

    pub fn deserialize<'de, D>(
        deserializer: D,
    ) -> Result<BTreeMap<ReductionPlanCeiling, u32>, D::Error>
    where
        D: Deserializer<'de>,
    {
        BTreeMap::<String, u32>::deserialize(deserializer)?
            .into_iter()
            .map(|(key, count)| {
                parse_ceiling_key(&key)
                    .map(|ceiling| (ceiling, count))
                    .ok_or_else(|| D::Error::custom(format!("unknown reduction ceiling {key:?}")))
            })
            .collect()
    }

    fn ceiling_key(key: ReductionPlanCeiling) -> &'static str {
        match key {
            ReductionPlanCeiling::ExactOnly => "ExactOnly",
            ReductionPlanCeiling::Conservative => "Conservative",
            ReductionPlanCeiling::Adaptive => "Adaptive",
        }
    }

    fn parse_ceiling_key(key: &str) -> Option<ReductionPlanCeiling> {
        match key {
            "ExactOnly" => Some(ReductionPlanCeiling::ExactOnly),
            "Conservative" => Some(ReductionPlanCeiling::Conservative),
            "Adaptive" => Some(ReductionPlanCeiling::Adaptive),
            _ => None,
        }
    }
}

mod ceiling_provenance_histogram {
    use std::collections::BTreeMap;

    use serde::de::Error as _;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use super::ReductionCeilingProvenanceTag;

    pub fn serialize<S>(
        histogram: &BTreeMap<ReductionCeilingProvenanceTag, u32>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        histogram
            .iter()
            .map(|(key, count)| (provenance_key(*key), *count))
            .collect::<BTreeMap<_, _>>()
            .serialize(serializer)
    }

    pub fn deserialize<'de, D>(
        deserializer: D,
    ) -> Result<BTreeMap<ReductionCeilingProvenanceTag, u32>, D::Error>
    where
        D: Deserializer<'de>,
    {
        BTreeMap::<String, u32>::deserialize(deserializer)?
            .into_iter()
            .map(|(key, count)| {
                parse_provenance_key(&key)
                    .map(|provenance| (provenance, count))
                    .ok_or_else(|| D::Error::custom(format!("unknown ceiling provenance {key:?}")))
            })
            .collect()
    }

    fn provenance_key(key: ReductionCeilingProvenanceTag) -> &'static str {
        match key {
            ReductionCeilingProvenanceTag::Global => "Global",
            ReductionCeilingProvenanceTag::LayerOverride => "LayerOverride",
            ReductionCeilingProvenanceTag::SiteOverride => "SiteOverride",
        }
    }

    fn parse_provenance_key(key: &str) -> Option<ReductionCeilingProvenanceTag> {
        match key {
            "Global" => Some(ReductionCeilingProvenanceTag::Global),
            "LayerOverride" => Some(ReductionCeilingProvenanceTag::LayerOverride),
            "SiteOverride" => Some(ReductionCeilingProvenanceTag::SiteOverride),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RangePlan {
    pub identity: RangePlanIdentity,
    pub entries: Vec<RangePlanEntry>,
    pub provenance: RangePlanProvenance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RangePlanIdentity {
    pub infer_ir_self_hash: Hash256,
    pub quant_graph_self_hash: Hash256,
    pub static_budget_self_hash: Hash256,
    pub range_policy_projection_hash: Hash256,
    pub determinism: DeterminismClass,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RangePlanEntry {
    pub site: ReductionSiteId,
    pub plan: ReductionPlan,
    pub site_facts: ReductionSiteFacts,
    pub effective_ceiling: ReductionPlanCeiling,
    pub ceiling_provenance: ReductionCeilingProvenance,
}

/// The per-reduction fact bundle copied into the Stage 5 product so later
/// consumers can re-check plan choices without re-reading Stage 2 internals.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReductionSiteFacts {
    pub site: ReductionSiteId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layer: Option<LayerId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expert: Option<ExpertId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slot: Option<ExpertWeightSlot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub norm_site: Option<NormSite>,
    pub term_count: u32,
    pub input_max_abs_q: u32,
    pub weight_max_abs_q: u32,
    pub per_term_abs_max_q: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bias_max_abs_q: Option<u32>,
    pub accumulator_domain: AccumulatorDomain,
    pub op_tag: InferOpTag,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ReductionPlan {
    SingleI16,
    ChunkedI16 { chunk_len: u16 },
    RenormLoop { tile_len: u16, renorm: RenormSpec },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenormSpec {
    pub strategy: RenormStrategy,
    pub recurrence: RenormRecurrence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum RenormStrategy {
    ExactPostBoundary,
    DynamicMargin { margin_q16_16: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenormRecurrence {
    pub input_scale_q16_16: u32,
    pub output_scale_q16_16: u32,
    pub rounding: RenormRounding,
    pub saturation: RenormSaturationPolicy,
    pub max_rounding_error_q16_16: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum RenormRounding {
    TowardZero,
    NearestEven,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum RenormSaturationPolicy {
    Forbidden,
    AtNamedNumericBoundary { boundary: NamedNumericBoundary },
}

/// Named saturation boundaries recognized by the Stage 5 range plan schema.
///
/// The v1 variant set is `ResidualCombine`, `ClassifyLogit`,
/// `FfnActivationOutput`, and `FinalClamp`. Adding, removing, or renaming a
/// variant changes the public `range_plan` JSON contract and must be evaluated
/// with the range plan schema/version before adoption.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum NamedNumericBoundary {
    ResidualCombine {
        #[serde(skip_serializing_if = "Option::is_none")]
        layer: Option<LayerId>,
        site: ResidualSite,
    },
    ClassifyLogit,
    FfnActivationOutput {
        layer: LayerId,
        expert: ExpertId,
    },
    FinalClamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ReductionCeilingProvenance {
    Global {
        source: PolicySource,
    },
    LayerOverride {
        layer: LayerId,
        source: PolicySource,
    },
    SiteOverride {
        site: ReductionSiteId,
        source: PolicySource,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RangePlanProvenance {
    pub site_to_node: BTreeMap<ReductionSiteId, NodeId>,
    pub site_to_qg: BTreeMap<ReductionSiteId, QuantGraphEntityRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CertifiedReduction {
    pub site: ReductionSiteId,
    pub plan: ReductionPlan,
    pub facts: ReductionSiteFacts,
    pub proof: AccumulatorCertificate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum AccumulatorCertificate {
    SingleI16Proof {
        site: ReductionSiteId,
        term_count: u64,
        per_term_abs_max: u64,
        sum_bound: u64,
        bias_abs_max: u64,
        total_abs_max: u64,
        i16_envelope: u64,
        slack: u64,
    },
    ChunkedI16Proof {
        site: ReductionSiteId,
        chunk_len: u16,
        chunk_count: u64,
        per_term_abs_max: u64,
        per_chunk_sum_bound: u64,
        per_chunk_i16_slack: u64,
        cross_chunk_sum_bound: u64,
        bias_abs_max: u64,
        total_abs_max: u64,
        i32_envelope: u64,
        slack: u64,
    },
    RenormLoopProof {
        site: ReductionSiteId,
        tile_len: u16,
        tile_count: u64,
        per_term_abs_max: u64,
        per_tile_sum_bound: u64,
        per_tile_i16_slack: u64,
        renorm: RenormSpec,
        bias_abs_max: u64,
        total_abs_max: u64,
        slack: u64,
    },
    Failed {
        site: ReductionSiteId,
        attempted_plan: ReductionPlan,
        proof_state: AccumulatorProofState,
        witness: AccumulatorFailureWitness,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RangeCertBody {
    pub identity: RangeCertIdentity,
    pub cert_outcome: CertOutcome,
    pub certificates: Vec<CertifiedReduction>,
    pub site_to_certificate_index: BTreeMap<ReductionSiteId, u32>,
    pub diagnostics: Vec<ValidationDiagnosticRecord>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RangeCertIdentity {
    pub range_plan_self_hash: Option<Hash256>,
    pub infer_ir_self_hash: Hash256,
    pub quant_graph_self_hash: Hash256,
    pub static_budget_self_hash: Hash256,
    pub determinism: DeterminismClass,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum CertOutcome {
    Verified,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum AccumulatorProofState {
    SumExceedsI16Envelope {
        sum_bound: u64,
        envelope: u64,
    },
    PerChunkExceedsI16Envelope {
        per_chunk_sum_bound: u64,
        envelope: u64,
    },
    CrossChunkExceedsI32Envelope {
        cross_chunk_sum_bound: u64,
        envelope: u64,
    },
    PerTileExceedsI16Envelope {
        per_tile_sum_bound: u64,
        envelope: u64,
    },
    LengthZero {
        length_field: LengthField,
    },
    ChunkLenExceedsProfileMax {
        chunk_len: u16,
        profile_chunk_max: u16,
    },
    TileLenBelowProfileMin {
        tile_len: u16,
        profile_tile_min: u16,
    },
    TileLenExceedsProfileMax {
        tile_len: u16,
        profile_tile_max: u16,
    },
    BitExactRequiresChunkDivides {
        term_count: u32,
        chunk_len: u16,
    },
    TileLenExceedsU16 {
        term_count: u32,
    },
    DeterminismRequiresEnforcedRenorm,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum AccumulatorFailureWitness {
    BoundCalculation {
        input_max_abs_q: u32,
        weight_max_abs_q: u32,
        term_count: u32,
        bias: u32,
    },
    BitExactSaturationForbidden,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum LengthField {
    ChunkLen,
    TileLen,
}

pub fn range_plan_self_hash(plan: &RangePlan) -> Result<Hash256, serde_json::Error> {
    let hash = domain_hash("RangePlan", RANGE_PLAN_SCHEMA_VERSION, plan)?;
    tracing::info!(
        event = RANGE_PLAN_SELF_HASH_COMPUTED_EVENT,
        hash = %hash,
        "gbf_codegen.range_plan.self_hash_computed"
    );
    Ok(hash)
}

pub fn range_policy_projection_hash(
    projection: &RangePolicyProjection,
) -> Result<Hash256, serde_json::Error> {
    let hash = domain_hash(
        "RangePolicyProjection",
        RANGE_PLAN_SCHEMA_VERSION,
        projection,
    )?;
    tracing::info!(
        event = RANGE_POLICY_PROJECTION_HASH_COMPUTED_EVENT,
        hash = %hash,
        "gbf_codegen.range_policy_projection.hash_computed"
    );
    Ok(hash)
}

pub fn range_cert_body_hash(body: &RangeCertBody) -> Result<Hash256, serde_json::Error> {
    let hash = domain_hash("RangeCertBody", RANGE_CERT_SCHEMA_VERSION, body)?;
    tracing::info!(
        event = RANGE_CERT_BODY_HASH_COMPUTED_EVENT,
        hash = %hash,
        "gbf_codegen.range_cert.body_hash_computed"
    );
    Ok(hash)
}

pub fn range_plan_core_product_hash(
    product: &RangePlanCoreProduct,
) -> Result<Hash256, serde_json::Error> {
    let hash = domain_hash(
        "RangePlanCoreProduct",
        RANGE_PLAN_CORE_PRODUCT_SCHEMA_VERSION,
        product,
    )?;
    tracing::info!(
        event = RANGE_PLAN_CORE_PRODUCT_HASH_COMPUTED_EVENT,
        hash = %hash,
        "gbf_codegen.range_plan.core_product.hash_computed"
    );
    Ok(hash)
}

#[allow(clippy::result_large_err)]
pub fn build_range_plan_core(
    inputs: &RangePlanInputs,
) -> Result<RangePlanCoreSuccess, RangePlanCoreFailure> {
    let identity = match bind_range_plan_identity(inputs) {
        Ok(identity) => identity,
        Err(error) => return Err(range_plan_core_failure_from_error(inputs, &error)),
    };
    let sites = match enumerate_reduction_sites(&inputs.infer_ir_product.infer_ir) {
        Ok(sites) => sites,
        Err(error) => return Err(range_plan_core_failure_from_error(inputs, &error)),
    };
    let bound_facts =
        match bind_reduction_site_facts_for_report(&sites, &inputs.static_budget_report) {
            Ok(bound_facts) => bound_facts,
            Err(error) => {
                return Err(range_plan_core_failure_from_error_with_identity(
                    inputs, &identity, &error,
                ));
            }
        };
    let ceilings = match bind_effective_ceilings(&bound_facts, &inputs.range_policy_projection) {
        Ok(ceilings) => ceilings,
        Err(error) => {
            return Err(range_plan_core_failure_from_error_with_identity(
                inputs, &identity, &error,
            ));
        }
    };

    let mut entries = Vec::new();
    let mut certificates = Vec::new();
    let mut attempted_certificates = Vec::new();
    let mut diagnostics = Vec::new();
    for binding in &ceilings {
        let generation = generate_plan_candidates(
            &binding.facts,
            binding.effective_ceiling,
            &inputs.range_policy_projection.range_caps,
            identity.determinism,
        );
        let choice = choose_plan(
            binding,
            &generation,
            &inputs.range_policy_projection.range_caps,
            identity.determinism,
        );
        diagnostics.extend(choice.diagnostics.clone());
        if let Some(chosen) = choice.chosen {
            entries.push(RangePlanEntry {
                site: binding.site.clone(),
                plan: chosen.plan.clone(),
                site_facts: binding.facts.clone(),
                effective_ceiling: binding.effective_ceiling,
                ceiling_provenance: binding.provenance.clone(),
            });
            certificates.push(chosen);
        } else {
            attempted_certificates.extend(failed_certified_attempts(binding, &choice));
        }
    }

    if !diagnostics.is_empty() {
        let cert_body = (!attempted_certificates.is_empty()).then(|| {
            failed_range_cert_body(&identity, None, attempted_certificates, diagnostics.clone())
        });
        return Err(range_plan_core_failure(
            inputs,
            &identity,
            diagnostics,
            cert_body,
        ));
    }

    let provenance = bind_range_plan_provenance(&sites);
    let mut range_plan = RangePlan {
        identity,
        entries,
        provenance,
    };
    canonical_sort_range_plan(&mut range_plan.entries, &mut certificates);
    let range_plan_self_hash =
        range_plan_self_hash(&range_plan).expect("range plan product hashes");
    let site_to_certificate_index = site_to_certificate_index(&certificates);
    let mut range_cert = RangeCertBody {
        identity: RangeCertIdentity {
            range_plan_self_hash: Some(range_plan_self_hash),
            infer_ir_self_hash: identity.infer_ir_self_hash,
            quant_graph_self_hash: identity.quant_graph_self_hash,
            static_budget_self_hash: identity.static_budget_self_hash,
            determinism: identity.determinism,
        },
        cert_outcome: range_cert_outcome(&certificates, &[]),
        certificates,
        site_to_certificate_index,
        diagnostics: Vec::new(),
    };
    let self_consistency = self_consistency_diagnostics(&range_plan, &range_cert);
    if !self_consistency.is_empty() {
        range_cert.diagnostics = self_consistency.clone();
        range_cert.cert_outcome =
            range_cert_outcome(&range_cert.certificates, &range_cert.diagnostics);
        return Err(range_plan_core_failure(
            inputs,
            &identity,
            self_consistency,
            Some(range_cert),
        ));
    }

    let range_cert_body_hash = range_cert_body_hash(&range_cert).expect("range cert body hashes");
    let product = RangePlanCoreProduct {
        range_plan: range_plan.clone(),
        range_cert: range_cert.clone(),
        range_plan_self_hash,
        range_cert_body_hash,
    };
    let cert_report_self_hash = report_self_hash_for(ReportOutcome::Passed, range_cert.clone());
    let range_plan_body =
        range_plan_report_body(inputs, &product, cert_report_self_hash, Vec::new());

    Ok(RangePlanCoreSuccess {
        product,
        range_plan_body,
        range_cert_body: range_cert,
    })
}

#[allow(clippy::result_large_err)]
pub fn run_stage5(
    inputs: RangePlanInputs,
    env: Stage5PassEnvironment<'_>,
) -> Result<RangePlanStageOutput, RunStage5Error> {
    let started = Instant::now();
    let material = Stage5CacheKeyMaterial::from_inputs(&inputs)?;
    let context = Stage5ReportRewrapContext::from_inputs(&inputs);

    if let Err(diagnostics) = validate_stage5_driver_preconditions(&inputs) {
        let identity = failure_identity(&inputs);
        let failure = range_plan_core_failure(&inputs, &identity, diagnostics, None);
        let stage_failure = wrap_stage5_failure(failure.clone())?;
        emit_stage5_failure_reports(env.report_dir, &stage_failure)?;
        if let Some(cache) = env.stage_cache {
            put_stage5_failure_memo(cache, &material, &cache_failure_from_core(&failure))?;
            emit_stage5_failure_memo_event(&stage_failure);
        }
        tracing::info!(
            target: "gbf_codegen::s5",
            event = %STAGE5_DRIVER_RUN_EVENT,
            cache_state = "precondition_failure",
            audit_rewrap = false,
            total_ms = started.elapsed().as_millis() as u64,
        );
        return Err(RunStage5Error::StageFailure(stage_failure));
    }

    if let Some(cache) = env.stage_cache {
        if let Some(product) = get_stage5_success(cache, &material)? {
            let output = rewrap_stage5_cached_success(&product, &context)?;
            emit_stage5_success_reports(env.report_dir, &output)?;
            tracing::info!(
                target: "gbf_codegen::s5",
                event = %STAGE5_DRIVER_RUN_EVENT,
                range_plan_self_hash = %output.product.range_plan_self_hash,
                cache_state = "hit_success",
                audit_rewrap = true,
                total_ms = started.elapsed().as_millis() as u64,
            );
            return Ok(output);
        }

        if let Some(failure) = get_stage5_failure_memo(cache, &material)? {
            let replay = rewrap_stage5_cached_failure(&failure, &context)?;
            let stage_failure = RangePlanStageFailure {
                report: replay.report,
                cert_report: replay.cert_report,
                diagnostics: replay.diagnostics,
            };
            emit_stage5_failure_reports(env.report_dir, &stage_failure)?;
            tracing::info!(
                target: "gbf_codegen::s5",
                event = %STAGE5_DRIVER_RUN_EVENT,
                cache_state = "hit_failure_memo",
                audit_rewrap = true,
                total_ms = started.elapsed().as_millis() as u64,
            );
            return Err(RunStage5Error::StageFailure(stage_failure));
        }
    }

    match build_range_plan_core(&inputs) {
        Ok(success) => {
            let output = wrap_stage5_success(success)?;
            emit_stage5_success_reports(env.report_dir, &output)?;
            if let Some(cache) = env.stage_cache {
                put_stage5_success(cache, &material, &output.product)?;
            }
            tracing::info!(
                target: "gbf_codegen::s5",
                event = %STAGE5_DRIVER_RUN_EVENT,
                range_plan_self_hash = %output.product.range_plan_self_hash,
                cache_state = "miss_success",
                audit_rewrap = false,
                total_ms = started.elapsed().as_millis() as u64,
            );
            Ok(output)
        }
        Err(failure) => {
            let stage_failure = wrap_stage5_failure(failure.clone())?;
            emit_stage5_failure_reports(env.report_dir, &stage_failure)?;
            if let Some(cache) = env.stage_cache {
                put_stage5_failure_memo(cache, &material, &cache_failure_from_core(&failure))?;
                emit_stage5_failure_memo_event(&stage_failure);
            }
            tracing::info!(
                target: "gbf_codegen::s5",
                event = %STAGE5_DRIVER_RUN_EVENT,
                cache_state = "miss_failure",
                audit_rewrap = false,
                total_ms = started.elapsed().as_millis() as u64,
            );
            Err(RunStage5Error::StageFailure(stage_failure))
        }
    }
}

fn validate_stage5_driver_preconditions(
    inputs: &RangePlanInputs,
) -> Result<(), Vec<ValidationDiagnosticRecord>> {
    let mut diagnostics = Vec::new();

    match infer_ir_self_hash(&inputs.infer_ir_product.infer_ir) {
        Ok(computed) => {
            if inputs.infer_ir_self_hash != computed
                || inputs.infer_ir_product.infer_ir_self_hash != computed
            {
                diagnostics.push(stage5_hash_mismatch_diagnostic(
                    "infer_ir_self_hash",
                    computed,
                    inputs.infer_ir_self_hash,
                ));
            }
        }
        Err(error) => diagnostics.push(stage5_precondition_diagnostic_with_reference(
            "infer_ir_self_hash",
            error.to_string(),
        )),
    }

    let ir_quant_hash = inputs
        .infer_ir_product
        .infer_ir
        .identity
        .quant_graph_self_hash;
    if inputs.quant_graph_self_hash != ir_quant_hash {
        diagnostics.push(stage5_hash_mismatch_diagnostic(
            "quant_graph_self_hash",
            ir_quant_hash,
            inputs.quant_graph_self_hash,
        ));
    }

    if inputs.static_budget_self_hash != inputs.static_budget_report.static_budget_self_hash {
        diagnostics.push(stage5_hash_mismatch_diagnostic(
            "static_budget_self_hash",
            inputs.static_budget_report.static_budget_self_hash,
            inputs.static_budget_self_hash,
        ));
    }
    if inputs.static_budget_report.report.outcome != ReportOutcome::Passed {
        diagnostics.push(stage5_precondition_diagnostic(
            "static_budget_report.report.outcome",
            "RP-Pre-3",
        ));
    }

    if inputs.range_policy_projection.determinism_class
        != inputs.infer_ir_product.infer_ir.identity.determinism
    {
        diagnostics.push(
            RangePlanConstructionError::DeterminismMismatch {
                projection: inputs.range_policy_projection.determinism_class,
                infer_ir: inputs.infer_ir_product.infer_ir.identity.determinism,
            }
            .diagnostic(),
        );
    }

    if !reduction_ceiling_is_valid(inputs.range_policy_projection.reduction_ceiling) {
        diagnostics.push(stage5_precondition_diagnostic(
            "range_policy_projection.reduction_ceiling",
            "RP-Pre-5",
        ));
    }

    if diagnostics.is_empty() {
        match enumerate_reduction_sites(&inputs.infer_ir_product.infer_ir)
            .and_then(|sites| {
                bind_reduction_site_facts_for_report(&sites, &inputs.static_budget_report)
            })
            .and_then(|bound| bind_effective_ceilings(&bound, &inputs.range_policy_projection))
        {
            Ok(_) => {}
            Err(error) => diagnostics.push(error.diagnostic()),
        }
    }

    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(diagnostics)
    }
}

const fn reduction_ceiling_is_valid(ceiling: ReductionPlanCeiling) -> bool {
    matches!(
        ceiling,
        ReductionPlanCeiling::ExactOnly
            | ReductionPlanCeiling::Conservative
            | ReductionPlanCeiling::Adaptive
    )
}

#[allow(clippy::result_large_err)]
fn wrap_stage5_success(
    success: RangePlanCoreSuccess,
) -> Result<RangePlanStageOutput, RunStage5Error> {
    let cert_report = stage5_report_envelope(ReportOutcome::Passed, success.range_cert_body)?;
    let mut range_plan_body = success.range_plan_body;
    if let Some(result) = &mut range_plan_body.result {
        result.range_cert_report_self_hash = cert_report.report_self_hash;
    }
    let report = stage5_report_envelope(ReportOutcome::Passed, range_plan_body)?;
    Ok(RangePlanStageOutput {
        product: success.product,
        report,
        cert_report,
    })
}

#[allow(clippy::result_large_err)]
fn wrap_stage5_failure(
    failure: RangePlanCoreFailure,
) -> Result<RangePlanStageFailure, RunStage5Error> {
    let report = stage5_report_envelope(ReportOutcome::Failed, failure.range_plan_body)?;
    let cert_report = failure
        .range_cert_body
        .map(|body| stage5_report_envelope(ReportOutcome::Failed, body))
        .transpose()?;
    Ok(RangePlanStageFailure {
        report,
        cert_report,
        diagnostics: failure.diagnostics,
    })
}

#[allow(clippy::result_large_err)]
fn stage5_report_envelope<B>(
    outcome: ReportOutcome,
    body: B,
) -> Result<ReportEnvelope<B>, RunStage5Error>
where
    B: ReportBody + Serialize,
{
    ReportEnvelope::new(outcome, body)
        .map_err(|err| RunStage5Error::ReportIo(io::Error::other(err.to_string())))?
        .with_computed_self_hash()
        .map_err(|err| RunStage5Error::ReportIo(io::Error::other(err.to_string())))
}

#[allow(clippy::result_large_err)]
fn emit_stage5_success_reports(
    report_dir: Option<&Path>,
    output: &RangePlanStageOutput,
) -> Result<(), RunStage5Error> {
    emit_stage5_report_files(
        report_dir,
        vec![
            (
                PathBuf::from("range_plan.json"),
                canonicalize_stage5_report(&output.report, "range_plan.json")?,
            ),
            (
                PathBuf::from("certs").join("range.cert.json"),
                canonicalize_stage5_report(&output.cert_report, "certs/range.cert.json")?,
            ),
        ],
    )
}

#[allow(clippy::result_large_err)]
fn emit_stage5_failure_reports(
    report_dir: Option<&Path>,
    failure: &RangePlanStageFailure,
) -> Result<(), RunStage5Error> {
    let mut reports = vec![(
        PathBuf::from("range_plan.json"),
        canonicalize_stage5_report(&failure.report, "range_plan.json")?,
    )];
    if let Some(cert_report) = &failure.cert_report {
        reports.push((
            PathBuf::from("certs").join("range.cert.json"),
            canonicalize_stage5_report(cert_report, "certs/range.cert.json")?,
        ));
    }
    emit_stage5_report_files(report_dir, reports)
}

#[allow(clippy::result_large_err)]
fn canonicalize_stage5_report<B>(
    report: &ReportEnvelope<B>,
    file_name: &'static str,
) -> Result<Vec<u8>, RunStage5Error>
where
    B: ReportBody + Serialize,
{
    canonicalize_report(report).map_err(|err| {
        RunStage5Error::ReportIo(io::Error::other(format!(
            "Stage 5 report {file_name} did not canonicalize: {err}"
        )))
    })
}

#[allow(clippy::result_large_err)]
fn emit_stage5_report_files(
    report_dir: Option<&Path>,
    reports: Vec<(PathBuf, Vec<u8>)>,
) -> Result<(), RunStage5Error> {
    let Some(report_dir) = report_dir else {
        return Ok(());
    };
    fs::create_dir_all(report_dir).map_err(|err| {
        io::Error::new(
            err.kind(),
            format!(
                "failed to create Stage 5 report directory {}: {err}",
                report_dir.display()
            ),
        )
    })?;
    let mut staged = Vec::with_capacity(reports.len());
    for (relative, bytes) in reports {
        let path = report_dir.join(&relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                io::Error::new(
                    err.kind(),
                    format!(
                        "failed to create Stage 5 report directory {}: {err}",
                        parent.display()
                    ),
                )
            })?;
        }
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, &bytes).map_err(|err| {
            io::Error::new(
                err.kind(),
                format!(
                    "failed to write Stage 5 report temp {}: {err}",
                    tmp.display()
                ),
            )
        })?;
        staged.push((tmp, path, bytes.len()));
    }
    for (tmp, path, len) in staged {
        fs::rename(&tmp, &path).map_err(|err| {
            io::Error::new(
                err.kind(),
                format!(
                    "failed to publish Stage 5 report {} from {}: {err}",
                    path.display(),
                    tmp.display()
                ),
            )
        })?;
        tracing::info!(
            target: "gbf_codegen::s5",
            event = %STAGE5_DRIVER_REPORT_EMIT_EVENT,
            canonical_bytes_len = len as u64,
            report_path = %path.display(),
            "stage5.driver.report_emit"
        );
    }
    Ok(())
}

fn emit_stage5_failure_memo_event(failure: &RangePlanStageFailure) {
    #[cfg(test)]
    tracing_core::callsite::rebuild_interest_cache();

    tracing::info!(
        target: "gbf_codegen::s5",
        event = %STAGE5_DRIVER_FAILURE_MEMO_EVENT,
        diagnostic_count = failure.diagnostics.len() as u64,
        has_cert_body = failure.cert_report.is_some(),
        "stage5.driver.failure_memo"
    );
}

fn cache_failure_from_core(
    failure: &RangePlanCoreFailure,
) -> crate::stage_cache::RangePlanCoreFailure {
    crate::stage_cache::RangePlanCoreFailure {
        range_plan_body: failure.range_plan_body.clone(),
        range_cert_body: failure.range_cert_body.clone(),
        diagnostics: failure.diagnostics.clone(),
    }
}

fn range_plan_core_failure_from_error(
    inputs: &RangePlanInputs,
    error: &RangePlanConstructionError,
) -> RangePlanCoreFailure {
    let identity = failure_identity(inputs);
    range_plan_core_failure_from_error_with_identity(inputs, &identity, error)
}

fn range_plan_core_failure_from_error_with_identity(
    inputs: &RangePlanInputs,
    identity: &RangePlanIdentity,
    error: &RangePlanConstructionError,
) -> RangePlanCoreFailure {
    range_plan_core_failure(inputs, identity, vec![error.diagnostic()], None)
}

fn range_plan_core_failure(
    inputs: &RangePlanInputs,
    identity: &RangePlanIdentity,
    diagnostics: Vec<ValidationDiagnosticRecord>,
    range_cert_body: Option<RangeCertBody>,
) -> RangePlanCoreFailure {
    RangePlanCoreFailure {
        range_plan_body: RangePlanReportBody {
            input_identity: RangePlanReportInputIdentity::from_inputs(inputs, identity),
            result: None,
            diagnostics: diagnostics.clone(),
        },
        range_cert_body,
        diagnostics,
    }
}

fn failed_range_cert_body(
    identity: &RangePlanIdentity,
    range_plan_self_hash: Option<Hash256>,
    mut certificates: Vec<CertifiedReduction>,
    diagnostics: Vec<ValidationDiagnosticRecord>,
) -> RangeCertBody {
    certificates.sort_by(|left, right| left.site.cmp(&right.site));
    let site_to_certificate_index = site_to_certificate_index(&certificates);
    RangeCertBody {
        identity: RangeCertIdentity {
            range_plan_self_hash,
            infer_ir_self_hash: identity.infer_ir_self_hash,
            quant_graph_self_hash: identity.quant_graph_self_hash,
            static_budget_self_hash: identity.static_budget_self_hash,
            determinism: identity.determinism,
        },
        cert_outcome: range_cert_outcome(&certificates, &diagnostics),
        certificates,
        site_to_certificate_index,
        diagnostics,
    }
}

fn failed_certified_attempts(
    binding: &EffectiveCeilingBinding,
    choice: &PlanChoice,
) -> Vec<CertifiedReduction> {
    choice
        .attempts
        .iter()
        .filter(|attempt| matches!(attempt.certificate, AccumulatorCertificate::Failed { .. }))
        .map(|attempt| CertifiedReduction {
            site: binding.site.clone(),
            plan: attempt.plan.clone(),
            facts: binding.facts.clone(),
            proof: attempt.certificate.clone(),
        })
        .collect()
}

fn range_plan_report_body(
    inputs: &RangePlanInputs,
    product: &RangePlanCoreProduct,
    range_cert_report_self_hash: Hash256,
    diagnostics: Vec<ValidationDiagnosticRecord>,
) -> RangePlanReportBody {
    let mut single_i16_count = 0;
    let mut chunked_i16_count = 0;
    let mut renorm_loop_count = 0;
    let mut effective_ceiling_histogram = BTreeMap::from([
        (ReductionPlanCeiling::ExactOnly, 0),
        (ReductionPlanCeiling::Conservative, 0),
        (ReductionPlanCeiling::Adaptive, 0),
    ]);
    let mut ceiling_provenance_histogram = BTreeMap::from([
        (ReductionCeilingProvenanceTag::Global, 0),
        (ReductionCeilingProvenanceTag::LayerOverride, 0),
        (ReductionCeilingProvenanceTag::SiteOverride, 0),
    ]);

    for entry in &product.range_plan.entries {
        match entry.plan {
            ReductionPlan::SingleI16 => single_i16_count += 1,
            ReductionPlan::ChunkedI16 { .. } => chunked_i16_count += 1,
            ReductionPlan::RenormLoop { .. } => renorm_loop_count += 1,
        }
        *effective_ceiling_histogram
            .entry(entry.effective_ceiling)
            .or_insert(0) += 1;
        *ceiling_provenance_histogram
            .entry(ReductionCeilingProvenanceTag::from(
                &entry.ceiling_provenance,
            ))
            .or_insert(0) += 1;
    }

    RangePlanReportBody {
        input_identity: RangePlanReportInputIdentity::from_inputs(
            inputs,
            &product.range_plan.identity,
        ),
        result: Some(RangePlanReportResult {
            product: product.range_plan.clone(),
            entry_count: u32::try_from(product.range_plan.entries.len())
                .expect("Stage 5 range-plan entry count fits u32"),
            single_i16_count,
            chunked_i16_count,
            renorm_loop_count,
            effective_ceiling_histogram,
            ceiling_provenance_histogram,
            range_cert_report_self_hash,
            range_plan_self_hash: product.range_plan_self_hash,
        }),
        diagnostics,
    }
}

fn report_self_hash_for<B>(outcome: ReportOutcome, body: B) -> Hash256
where
    B: ReportBody + Serialize,
{
    let env = ReportEnvelope::new(outcome, body).expect("Stage 5 report body validates");
    compute_self_hash(&env).expect("Stage 5 report self-hash computes")
}

fn failure_identity(inputs: &RangePlanInputs) -> RangePlanIdentity {
    let range_policy_projection_hash =
        range_policy_projection_hash(&inputs.range_policy_projection)
            .expect("range policy projection hashes for failure report");
    RangePlanIdentity {
        infer_ir_self_hash: inputs.infer_ir_self_hash,
        quant_graph_self_hash: inputs.quant_graph_self_hash,
        static_budget_self_hash: inputs.static_budget_self_hash,
        range_policy_projection_hash,
        determinism: inputs.infer_ir_product.infer_ir.identity.determinism,
    }
}

fn stage5_hash_mismatch_diagnostic(
    field: &'static str,
    expected: Hash256,
    observed: Hash256,
) -> ValidationDiagnostic {
    ValidationDiagnostic::hard(
        ValidationOrigin::SemanticCore,
        ValidationCode::SemanticCoreHashMismatch,
        ValidationDetail::HashMismatch { expected, observed },
        vec![EvidenceRef {
            kind: "stage5-precondition".to_owned(),
            reference: field.to_owned(),
            hash: Some(observed),
        }],
    )
}

fn stage5_precondition_diagnostic(
    field: &'static str,
    reference: &'static str,
) -> ValidationDiagnostic {
    stage5_precondition_diagnostic_with_reference(field, reference.to_owned())
}

fn stage5_precondition_diagnostic_with_reference(
    field: &'static str,
    reference: String,
) -> ValidationDiagnostic {
    ValidationDiagnostic::hard(
        ValidationOrigin::RangePlanConstruction,
        ValidationCode::ReportSemanticInvariantViolated {
            field: FieldPath::from(field),
        },
        ValidationDetail::Field {
            field: FieldPath::from(field),
        },
        vec![EvidenceRef {
            kind: "stage5-precondition".to_owned(),
            reference,
            hash: Some(Hash256::ZERO),
        }],
    )
}

pub fn bind_range_plan_identity(
    inputs: &RangePlanInputs,
) -> Result<RangePlanIdentity, RangePlanConstructionError> {
    let infer_ir_determinism = inputs.infer_ir_product.infer_ir.identity.determinism;
    let projection_determinism = inputs.range_policy_projection.determinism_class;
    if projection_determinism != infer_ir_determinism {
        return Err(RangePlanConstructionError::DeterminismMismatch {
            projection: projection_determinism,
            infer_ir: infer_ir_determinism,
        });
    }

    let range_policy_projection_hash =
        range_policy_projection_hash(&inputs.range_policy_projection).map_err(|error| {
            RangePlanConstructionError::RangePolicyProjectionHash {
                message: error.to_string(),
            }
        })?;
    let identity = RangePlanIdentity {
        infer_ir_self_hash: inputs.infer_ir_self_hash,
        quant_graph_self_hash: inputs.quant_graph_self_hash,
        static_budget_self_hash: inputs.static_budget_self_hash,
        range_policy_projection_hash,
        determinism: infer_ir_determinism,
    };

    record_range_construction_event(RANGE_IDENTITY_BIND_EVENT);
    tracing::info!(
        target: "gbf_codegen::s5",
        event = %RANGE_IDENTITY_BIND_EVENT,
        infer_ir_self_hash = %identity.infer_ir_self_hash,
        quant_graph_self_hash = %identity.quant_graph_self_hash,
        static_budget_self_hash = %identity.static_budget_self_hash,
        range_policy_projection_hash = %identity.range_policy_projection_hash,
        determinism = ?identity.determinism,
    );

    Ok(identity)
}

pub fn enumerate_reduction_sites(
    g: &GbInferIR,
) -> Result<Vec<ReductionSiteBinding>, RangePlanConstructionError> {
    let mut by_site = BTreeMap::<ReductionSiteId, ReductionSiteBinding>::new();
    let mut site_count = 0_u64;

    for node in &g.nodes {
        let Some(site) = node.reduction_site.clone() else {
            continue;
        };
        site_count += 1;
        let binding = ReductionSiteBinding {
            node_id: node.node_id,
            site: site.clone(),
            qg_ref: g
                .provenance
                .nodes
                .get(&node.node_id)
                .cloned()
                .expect("GbInferIR provenance is total for nodes"),
            op_tag: node.op.tag(),
            slot: expert_weight_slot_for_op(node.op),
            norm_site: norm_site_for_qg_ref(
                g.provenance
                    .nodes
                    .get(&node.node_id)
                    .expect("GbInferIR provenance is total for nodes"),
            ),
        };

        if let Some(previous) = by_site.insert(site.clone(), binding.clone()) {
            return Err(RangePlanConstructionError::DuplicateReductionSiteId {
                site,
                first_node: previous.node_id,
                duplicate_node: node.node_id,
            });
        }
    }

    record_range_construction_event(RANGE_REDUCTION_SITE_ENUMERATION_EVENT);
    tracing::info!(
        target: "gbf_codegen::s5",
        event = %RANGE_REDUCTION_SITE_ENUMERATION_EVENT,
        site_count,
        unique_count = by_site.len() as u64,
    );

    Ok(by_site.into_values().collect())
}

pub fn bind_reduction_site_facts_for_report(
    sites: &[ReductionSiteBinding],
    static_budget: &StaticBudgetReport,
) -> Result<Vec<BoundReductionSiteFacts>, RangePlanConstructionError> {
    let static_budget_site_ids = static_budget
        .reduction_site_facts
        .iter()
        .map(|projection| projection.site.clone());
    bind_reduction_site_facts(sites, static_budget, static_budget_site_ids)
}

pub fn bind_reduction_site_facts<B, I>(
    sites: &[ReductionSiteBinding],
    static_budget: &B,
    static_budget_site_ids: I,
) -> Result<Vec<BoundReductionSiteFacts>, RangePlanConstructionError>
where
    B: StaticBudgetReductionSiteFacts + ?Sized,
    I: IntoIterator<Item = ReductionSiteId>,
{
    let node_sites = sites
        .iter()
        .map(|binding| binding.site.clone())
        .collect::<BTreeSet<_>>();
    for site in static_budget_site_ids {
        if !node_sites.contains(&site) {
            return Err(RangePlanConstructionError::StaticBudgetSiteOrphaned { site });
        }
    }

    let mut bound = Vec::with_capacity(sites.len());
    for binding in sites {
        let projection = static_budget
            .reduction_site_projection(&binding.site)
            .ok_or_else(|| RangePlanConstructionError::SiteMissingFromStaticBudget {
                site: binding.site.clone(),
                node: binding.node_id,
            })?;
        let facts = reduction_site_facts_from_projection(binding, projection)?;

        record_range_construction_event(RANGE_SITE_FACTS_BINDING_EVENT);
        tracing::info!(
            target: "gbf_codegen::s5",
            event = %RANGE_SITE_FACTS_BINDING_EVENT,
            site = binding.site.0.as_str(),
            term_count = facts.term_count as u64,
            per_term_abs_max_q = facts.per_term_abs_max_q,
            accumulator_domain = ?facts.accumulator_domain,
        );

        bound.push(BoundReductionSiteFacts {
            binding: binding.clone(),
            facts,
        });
    }

    Ok(bound)
}

pub fn bind_effective_ceilings(
    bound_facts: &[BoundReductionSiteFacts],
    projection: &RangePolicyProjection,
) -> Result<Vec<EffectiveCeilingBinding>, RangePlanConstructionError> {
    let selectors = parse_and_resolve_ceiling_overrides(bound_facts, projection)?;
    let mut result = Vec::with_capacity(bound_facts.len());

    for bound in bound_facts {
        let mut site_matches = Vec::new();
        let mut layer_matches = Vec::new();
        for resolved in &selectors {
            if resolved.matches(&bound.facts) {
                match resolved.specificity {
                    ReductionSelectorSpecificity::Site => site_matches.push(resolved),
                    ReductionSelectorSpecificity::Layer => layer_matches.push(resolved),
                }
            }
        }

        let (effective_ceiling, provenance) = if site_matches.len() > 1 {
            return Err(RangePlanConstructionError::CeilingOverrideAmbiguous {
                site: bound.facts.site.clone(),
                selectors: site_matches
                    .iter()
                    .map(|resolved| resolved.selector.clone())
                    .collect(),
            });
        } else if let Some(resolved) = site_matches.first() {
            (
                resolved.ceiling,
                ReductionCeilingProvenance::SiteOverride {
                    site: bound.facts.site.clone(),
                    source: PolicySource::CompileRequestOverride,
                },
            )
        } else if layer_matches.len() > 1 {
            return Err(RangePlanConstructionError::CeilingOverrideAmbiguous {
                site: bound.facts.site.clone(),
                selectors: layer_matches
                    .iter()
                    .map(|resolved| resolved.selector.clone())
                    .collect(),
            });
        } else if let Some(resolved) = layer_matches.first() {
            (
                resolved.ceiling,
                ReductionCeilingProvenance::LayerOverride {
                    layer: resolved.layer.expect("layer selector has layer"),
                    source: PolicySource::CompileRequestOverride,
                },
            )
        } else {
            (
                projection.reduction_ceiling,
                ReductionCeilingProvenance::Global {
                    source: PolicySource::ProfileDefault,
                },
            )
        };

        record_range_construction_event(RANGE_EFFECTIVE_CEILING_BINDING_EVENT);
        tracing::info!(
            target: "gbf_codegen::s5",
            event = %RANGE_EFFECTIVE_CEILING_BINDING_EVENT,
            site = bound.facts.site.0.as_str(),
            effective_ceiling = ?effective_ceiling,
            provenance_kind = ?ReductionCeilingProvenanceTag::from(&provenance),
        );

        result.push(EffectiveCeilingBinding {
            site: bound.facts.site.clone(),
            facts: bound.facts.clone(),
            effective_ceiling,
            provenance,
        });
    }

    Ok(result)
}

pub fn candidate_families_under_ceiling(ceiling: ReductionPlanCeiling) -> Vec<ReductionPlanFamily> {
    match ceiling {
        ReductionPlanCeiling::ExactOnly => vec![ReductionPlanFamily::SingleI16],
        ReductionPlanCeiling::Conservative => vec![
            ReductionPlanFamily::SingleI16,
            ReductionPlanFamily::ChunkedI16,
        ],
        ReductionPlanCeiling::Adaptive => vec![
            ReductionPlanFamily::SingleI16,
            ReductionPlanFamily::ChunkedI16,
            ReductionPlanFamily::RenormLoop,
        ],
    }
}

pub fn generate_plan_candidates(
    facts: &ReductionSiteFacts,
    ceiling: ReductionPlanCeiling,
    caps: &RangeCapsSpec,
    determinism: DeterminismClass,
) -> PlanCandidateGeneration {
    let candidate_families = candidate_families_under_ceiling(ceiling);
    let mut candidates = Vec::new();
    let mut rejections = Vec::new();

    for family in &candidate_families {
        match canonical_candidate_for_family(*family, facts, caps, determinism) {
            Ok(candidate) => candidates.push(candidate),
            Err(error) => rejections.push(PlanCandidateRejection {
                family: *family,
                error,
            }),
        }
    }

    record_range_construction_event(RANGE_PLAN_CANDIDATE_GENERATION_EVENT);
    tracing::info!(
        target: "gbf_codegen::s5",
        event = %RANGE_PLAN_CANDIDATE_GENERATION_EVENT,
        site = facts.site.0.as_str(),
        candidate_families = ?candidate_families,
    );

    PlanCandidateGeneration {
        site: facts.site.clone(),
        candidate_families,
        candidates,
        rejections,
    }
}

pub fn canonical_candidate_for_family(
    family: ReductionPlanFamily,
    facts: &ReductionSiteFacts,
    caps: &RangeCapsSpec,
    determinism: DeterminismClass,
) -> Result<ReductionPlan, PlanLengthSelectionError> {
    match family {
        ReductionPlanFamily::SingleI16 => Ok(ReductionPlan::SingleI16),
        ReductionPlanFamily::ChunkedI16 => Ok(ReductionPlan::ChunkedI16 {
            chunk_len: choose_chunk_len(facts, caps, determinism)?,
        }),
        ReductionPlanFamily::RenormLoop => Ok(ReductionPlan::RenormLoop {
            tile_len: choose_tile_len(facts, caps, determinism)?,
            renorm: renorm_spec_for_policy(facts, caps.renorm_strategy),
        }),
    }
}

pub fn choose_chunk_len(
    facts: &ReductionSiteFacts,
    caps: &RangeCapsSpec,
    determinism: DeterminismClass,
) -> Result<u16, PlanLengthSelectionError> {
    let selection = choose_chunk_len_with_bounds(facts, caps, determinism)?;
    record_range_construction_event(RANGE_PLAN_LENGTH_SELECTION_EVENT);
    tracing::info!(
        target: "gbf_codegen::s5",
        event = %RANGE_PLAN_LENGTH_SELECTION_EVENT,
        site = facts.site.0.as_str(),
        family = ?ReductionPlanFamily::ChunkedI16,
        chosen_len = selection.len as u64,
        max_safe = selection.max_safe,
        profile_max = selection.profile_max as u64,
    );
    Ok(selection.len)
}

pub fn choose_tile_len(
    facts: &ReductionSiteFacts,
    caps: &RangeCapsSpec,
    determinism: DeterminismClass,
) -> Result<u16, PlanLengthSelectionError> {
    if determinism == DeterminismClass::BitExact {
        return Err(PlanLengthSelectionError::BitExactRenormLoopReservedV1);
    }

    let selection = choose_tile_len_with_bounds(facts, caps)?;
    record_range_construction_event(RANGE_PLAN_LENGTH_SELECTION_EVENT);
    tracing::info!(
        target: "gbf_codegen::s5",
        event = %RANGE_PLAN_LENGTH_SELECTION_EVENT,
        site = facts.site.0.as_str(),
        family = ?ReductionPlanFamily::RenormLoop,
        chosen_len = selection.len as u64,
        max_safe = selection.max_safe,
        profile_max = selection.profile_max as u64,
    );
    Ok(selection.len)
}

#[derive(Debug, Clone)]
struct ResolvedCeilingOverride {
    selector: ReductionSelector,
    specificity: ReductionSelectorSpecificity,
    site: Option<ReductionSiteId>,
    layer: Option<LayerId>,
    ceiling: ReductionPlanCeiling,
}

impl ResolvedCeilingOverride {
    fn matches(&self, facts: &ReductionSiteFacts) -> bool {
        match self.specificity {
            ReductionSelectorSpecificity::Site => self.site.as_ref() == Some(&facts.site),
            ReductionSelectorSpecificity::Layer => {
                self.layer.is_some() && self.layer == facts.layer
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReductionSelectorSpecificity {
    Layer,
    Site,
}

fn parse_and_resolve_ceiling_overrides(
    bound_facts: &[BoundReductionSiteFacts],
    projection: &RangePolicyProjection,
) -> Result<Vec<ResolvedCeilingOverride>, RangePlanConstructionError> {
    let sites = bound_facts
        .iter()
        .map(|bound| bound.facts.site.clone())
        .collect::<BTreeSet<_>>();
    let layers = bound_facts
        .iter()
        .filter_map(|bound| bound.facts.layer)
        .collect::<BTreeSet<_>>();
    let mut resolved = Vec::with_capacity(projection.reduction_ceiling_overrides.len());

    for (selector, ceiling) in &projection.reduction_ceiling_overrides {
        match parse_reduction_selector(selector) {
            Some(ParsedReductionSelector::Site(site)) if sites.contains(&site) => {
                resolved.push(ResolvedCeilingOverride {
                    selector: selector.clone(),
                    specificity: ReductionSelectorSpecificity::Site,
                    site: Some(site),
                    layer: None,
                    ceiling: *ceiling,
                });
            }
            Some(ParsedReductionSelector::Layer(layer)) if layers.contains(&layer) => {
                resolved.push(ResolvedCeilingOverride {
                    selector: selector.clone(),
                    specificity: ReductionSelectorSpecificity::Layer,
                    site: None,
                    layer: Some(layer),
                    ceiling: *ceiling,
                });
            }
            _ => {
                return Err(RangePlanConstructionError::CeilingOverrideInvalidSelector {
                    selector: selector.clone(),
                });
            }
        }
    }

    Ok(resolved)
}

enum ParsedReductionSelector {
    Site(ReductionSiteId),
    Layer(LayerId),
}

fn parse_reduction_selector(selector: &ReductionSelector) -> Option<ParsedReductionSelector> {
    let raw = selector.0.trim();
    if raw.is_empty() {
        return None;
    }
    if let Some(site) = raw.strip_prefix("site:") {
        return (!site.is_empty())
            .then(|| ParsedReductionSelector::Site(ReductionSiteId(site.into())));
    }
    if let Some(layer) = raw.strip_prefix("layer:") {
        let layer = layer.parse::<u16>().ok()?;
        return Some(ParsedReductionSelector::Layer(LayerId::new(layer)));
    }
    // Back-compat grammar: an unprefixed selector is a site selector.
    Some(ParsedReductionSelector::Site(ReductionSiteId(raw.into())))
}

#[derive(Debug, Clone, Copy)]
struct SelectedLength {
    len: u16,
    max_safe: u64,
    profile_max: u16,
}

fn choose_chunk_len_with_bounds(
    facts: &ReductionSiteFacts,
    caps: &RangeCapsSpec,
    determinism: DeterminismClass,
) -> Result<SelectedLength, PlanLengthSelectionError> {
    if caps.profile_chunk_max == 0 {
        return Err(PlanLengthSelectionError::ChunkLenExceedsProfileMax {
            chunk_len: 1,
            profile_chunk_max: caps.profile_chunk_max,
        });
    }
    if determinism == DeterminismClass::BitExact && facts.term_count == 0 {
        return Err(PlanLengthSelectionError::BitExactRequiresChunkDivides {
            term_count: facts.term_count,
            chunk_len: 1,
        });
    }

    let max_safe = match (i16::MAX as u64).checked_div(facts.per_term_abs_max_q) {
        None => u64::from(caps.profile_chunk_max),
        Some(0) => {
            return Err(PlanLengthSelectionError::PerChunkExceedsI16Envelope {
                per_chunk_sum_bound: facts.per_term_abs_max_q,
                envelope: i16::MAX as u64,
            });
        }
        Some(raw_max_safe) => raw_max_safe.min(u64::from(caps.profile_chunk_max)),
    };

    let Some(len) = (match determinism {
        DeterminismClass::BitExact => max_pow2_divisor_le(facts.term_count, max_safe),
        DeterminismClass::Deterministic | DeterminismClass::Nondeterministic => {
            max_pow2_le(max_safe)
        }
    }) else {
        return Err(PlanLengthSelectionError::BitExactRequiresChunkDivides {
            term_count: facts.term_count,
            chunk_len: max_safe.min(u64::from(u16::MAX)) as u16,
        });
    };

    debug_assert!(len <= caps.profile_chunk_max);

    Ok(SelectedLength {
        len,
        max_safe,
        profile_max: caps.profile_chunk_max,
    })
}

fn choose_tile_len_with_bounds(
    facts: &ReductionSiteFacts,
    caps: &RangeCapsSpec,
) -> Result<SelectedLength, PlanLengthSelectionError> {
    if caps.profile_tile_min > caps.profile_tile_max {
        return Err(PlanLengthSelectionError::TileLenExceedsProfileMax {
            tile_len: caps.profile_tile_min,
            profile_tile_max: caps.profile_tile_max,
        });
    }

    let margin_abs = match caps.renorm_strategy {
        RenormStrategyPolicy::ExactPostBoundaryOnly => 0_u64,
        RenormStrategyPolicy::DynamicMargin { margin_q16_16 } => {
            ((i16::MAX as u64) * u64::from(margin_q16_16)) / 65_536
        }
    };
    let safe_envelope = (i16::MAX as u64).saturating_sub(margin_abs);
    if facts.per_term_abs_max_q == 0 {
        return Ok(SelectedLength {
            len: caps.profile_tile_min,
            max_safe: u64::from(caps.profile_tile_max),
            profile_max: caps.profile_tile_max,
        });
    }
    let raw_max_safe = match safe_envelope.checked_div(facts.per_term_abs_max_q) {
        None => u64::from(caps.profile_tile_max),
        Some(raw_max_safe) => raw_max_safe,
    };

    if raw_max_safe == 0 {
        return Err(PlanLengthSelectionError::PerTileExceedsI16Envelope {
            per_tile_sum_bound: facts.per_term_abs_max_q,
            envelope: safe_envelope,
        });
    }
    if raw_max_safe < u64::from(caps.profile_tile_min) {
        return Err(PlanLengthSelectionError::PerTileExceedsI16Envelope {
            per_tile_sum_bound: checked_mul_u64(
                u64::from(caps.profile_tile_min),
                facts.per_term_abs_max_q,
            )
            .unwrap_or(u64::MAX),
            envelope: safe_envelope,
        });
    }

    let max_safe = raw_max_safe.min(u64::from(caps.profile_tile_max));
    let Some(len) = max_pow2_between(caps.profile_tile_min, max_safe) else {
        return Err(PlanLengthSelectionError::TileLenBelowProfileMin {
            tile_len: max_safe.min(u64::from(u16::MAX)) as u16,
            profile_tile_min: caps.profile_tile_min,
        });
    };

    Ok(SelectedLength {
        len,
        max_safe,
        profile_max: caps.profile_tile_max,
    })
}

fn max_pow2_le(max: u64) -> Option<u16> {
    if max == 0 {
        return None;
    }
    let capped = max.min(u64::from(u16::MAX));
    let pow = 1_u64 << (u64::BITS - 1 - capped.leading_zeros());
    u16::try_from(pow).ok()
}

fn max_pow2_divisor_le(term_count: u32, max: u64) -> Option<u16> {
    let mut candidate = max_pow2_le(max)?;
    loop {
        if term_count.is_multiple_of(u32::from(candidate)) {
            return Some(candidate);
        }
        if candidate == 1 {
            return None;
        }
        candidate /= 2;
    }
}

fn max_pow2_between(min: u16, max: u64) -> Option<u16> {
    let mut candidate = max_pow2_le(max)?;
    while candidate < min {
        let next = candidate.checked_mul(2)?;
        if u64::from(next) > max {
            return None;
        }
        candidate = next;
    }
    Some(candidate)
}

fn renorm_spec_for_policy(facts: &ReductionSiteFacts, policy: RenormStrategyPolicy) -> RenormSpec {
    RenormSpec {
        strategy: match policy {
            RenormStrategyPolicy::ExactPostBoundaryOnly => RenormStrategy::ExactPostBoundary,
            RenormStrategyPolicy::DynamicMargin { margin_q16_16 } => {
                RenormStrategy::DynamicMargin { margin_q16_16 }
            }
        },
        recurrence: RenormRecurrence {
            input_scale_q16_16: 0x0001_0000,
            output_scale_q16_16: match policy {
                RenormStrategyPolicy::ExactPostBoundaryOnly => 0x0001_0000,
                RenormStrategyPolicy::DynamicMargin { .. } => 0x0010_0000,
            },
            rounding: RenormRounding::NearestEven,
            saturation: RenormSaturationPolicy::AtNamedNumericBoundary {
                boundary: named_numeric_boundary_for_facts(facts),
            },
            max_rounding_error_q16_16: 1,
        },
    }
}

fn named_numeric_boundary_for_facts(facts: &ReductionSiteFacts) -> NamedNumericBoundary {
    match (facts.op_tag, facts.layer, facts.expert) {
        (InferOpTag::Classify, _, _) => NamedNumericBoundary::ClassifyLogit,
        (InferOpTag::ExpertMatVec, Some(layer), Some(expert)) => {
            NamedNumericBoundary::FfnActivationOutput { layer, expert }
        }
        _ => NamedNumericBoundary::FinalClamp,
    }
}

fn reduction_site_facts_from_projection(
    binding: &ReductionSiteBinding,
    projection: &ReductionSiteProjection,
) -> Result<ReductionSiteFacts, RangePlanConstructionError> {
    reduction_site_facts_from_projection_with_published(binding, projection, None)
}

pub fn reduction_site_facts_from_projection_with_published(
    binding: &ReductionSiteBinding,
    projection: &ReductionSiteProjection,
    published_per_term_abs_max_q: Option<u64>,
) -> Result<ReductionSiteFacts, RangePlanConstructionError> {
    if projection.term_count == 0 {
        return Err(RangePlanConstructionError::TermCountZero {
            site: binding.site.clone(),
        });
    }
    if projection.accumulator_domain != AccumulatorDomain::RawIntegerProducts {
        return Err(RangePlanConstructionError::AccumulatorDomainUnsupportedV1 {
            site: binding.site.clone(),
            accumulator_domain: projection.accumulator_domain,
        });
    }

    let per_term_abs_max_q = checked_per_term_abs_max_q(
        u128::from(projection.input_max_abs_q),
        u128::from(projection.weight_max_abs_q),
        published_per_term_abs_max_q,
    )
    .map_err(|error| match error {
        PerTermAbsMaxError::Inconsistent {
            computed,
            published,
        } => RangePlanConstructionError::SiteFactsInconsistent {
            site: binding.site.clone(),
            computed_per_term_abs_max_q: computed,
            published_per_term_abs_max_q: published,
        },
        PerTermAbsMaxError::Overflow => RangePlanConstructionError::IntegerOverflowDuringProof {
            site: Some(binding.site.clone()),
            field: "static_budget.reduction_site.per_term_abs_max_q",
        },
    })?;

    Ok(ReductionSiteFacts {
        site: projection.site.clone(),
        layer: projection.layer,
        expert: projection.expert,
        slot: binding.slot,
        norm_site: binding.norm_site,
        term_count: projection.term_count,
        input_max_abs_q: projection.input_max_abs_q,
        weight_max_abs_q: projection.weight_max_abs_q,
        per_term_abs_max_q,
        bias_max_abs_q: projection.bias_max_abs_q,
        accumulator_domain: projection.accumulator_domain,
        op_tag: binding.op_tag,
    })
}

pub fn checked_per_term_abs_max_q(
    input_max_abs_q: u128,
    weight_max_abs_q: u128,
    published_per_term_abs_max_q: Option<u64>,
) -> Result<u64, PerTermAbsMaxError> {
    let computed = input_max_abs_q
        .checked_mul(weight_max_abs_q)
        .and_then(|value| u64::try_from(value).ok());
    match (published_per_term_abs_max_q, computed) {
        (Some(published), Some(computed)) if published != computed => {
            Err(PerTermAbsMaxError::Inconsistent {
                computed: Some(computed),
                published,
            })
        }
        (Some(published), _) => Ok(published),
        (None, Some(computed)) => Ok(computed),
        (None, None) => Err(PerTermAbsMaxError::Overflow),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PerTermAbsMaxError {
    Inconsistent {
        computed: Option<u64>,
        published: u64,
    },
    Overflow,
}

#[must_use]
pub fn construct_accumulator_certificate(
    plan: &ReductionPlan,
    facts: &ReductionSiteFacts,
    determinism: DeterminismClass,
) -> AccumulatorCertificate {
    let certificate = match plan {
        ReductionPlan::SingleI16 => construct_single_i16_certificate(plan, facts),
        ReductionPlan::ChunkedI16 { chunk_len } => {
            construct_chunked_i16_certificate(*chunk_len, plan, facts, determinism)
        }
        ReductionPlan::RenormLoop { tile_len, renorm } => {
            construct_renorm_loop_certificate(*tile_len, renorm, plan, facts, determinism)
        }
    };

    let outcome = if matches!(certificate, AccumulatorCertificate::Failed { .. }) {
        "failed"
    } else {
        "constructed"
    };
    record_range_construction_event(RANGE_CERTIFICATE_CONSTRUCTION_EVENT);
    tracing::info!(
        target: "gbf_codegen::s5",
        event = %RANGE_CERTIFICATE_CONSTRUCTION_EVENT,
        site = facts.site.0.as_str(),
        plan_family = ?plan_family(plan),
        outcome,
    );

    certificate
}

#[must_use]
pub fn verifies(
    cert: &AccumulatorCertificate,
    plan: &ReductionPlan,
    facts: &ReductionSiteFacts,
    determinism: DeterminismClass,
) -> bool {
    verifies_impl(cert, plan, facts, determinism)
}

#[must_use]
pub fn verifies_with_determinism(
    cert: &AccumulatorCertificate,
    plan: &ReductionPlan,
    facts: &ReductionSiteFacts,
    determinism: DeterminismClass,
) -> bool {
    verifies_impl(cert, plan, facts, determinism)
}

fn verifies_impl(
    cert: &AccumulatorCertificate,
    plan: &ReductionPlan,
    facts: &ReductionSiteFacts,
    determinism: DeterminismClass,
) -> bool {
    if facts.accumulator_domain != AccumulatorDomain::RawIntegerProducts {
        return false;
    }

    match cert {
        AccumulatorCertificate::SingleI16Proof {
            site,
            term_count,
            per_term_abs_max,
            sum_bound,
            bias_abs_max,
            total_abs_max,
            i16_envelope,
            slack,
        } => {
            let ok = site == &facts.site
                && *term_count == u64::from(facts.term_count)
                && *per_term_abs_max == facts.per_term_abs_max_q
                && checked_mul_u64(*term_count, *per_term_abs_max) == Some(*sum_bound)
                && *bias_abs_max == facts_bias_abs_max(facts)
                && checked_add_u64(*sum_bound, *bias_abs_max) == Some(*total_abs_max)
                && *i16_envelope == I16_ENVELOPE_U64
                && *total_abs_max <= *i16_envelope
                && i16_envelope.checked_sub(*total_abs_max) == Some(*slack)
                && matches!(plan, ReductionPlan::SingleI16);
            if ok {
                record_range_construction_event(RANGE_CERT_VERIFIES_SINGLE_I16_EVENT);
                tracing::info!(
                    target: "gbf_verify::range_cert",
                    event = %RANGE_CERT_VERIFIES_SINGLE_I16_EVENT,
                    site = facts.site.0.as_str(),
                    slack = *slack,
                );
            }
            ok
        }
        AccumulatorCertificate::ChunkedI16Proof {
            site,
            chunk_len,
            chunk_count,
            per_term_abs_max,
            per_chunk_sum_bound,
            per_chunk_i16_slack,
            cross_chunk_sum_bound,
            bias_abs_max,
            total_abs_max,
            i32_envelope,
            slack,
        } => {
            let plan_chunk_len = match plan {
                ReductionPlan::ChunkedI16 { chunk_len } => Some(*chunk_len),
                _ => None,
            };
            let ok = *chunk_len > 0
                && site == &facts.site
                && plan_chunk_len == Some(*chunk_len)
                && ceil_div_u64(u64::from(facts.term_count), u64::from(*chunk_len))
                    == Some(*chunk_count)
                && *per_term_abs_max == facts.per_term_abs_max_q
                && checked_mul_u64(u64::from(*chunk_len), *per_term_abs_max)
                    == Some(*per_chunk_sum_bound)
                && *per_chunk_sum_bound <= I16_ENVELOPE_U64
                && I16_ENVELOPE_U64.checked_sub(*per_chunk_sum_bound) == Some(*per_chunk_i16_slack)
                && (determinism != DeterminismClass::BitExact
                    || facts.term_count.is_multiple_of(u32::from(*chunk_len)))
                && checked_mul_u64(u64::from(facts.term_count), *per_term_abs_max)
                    == Some(*cross_chunk_sum_bound)
                && *bias_abs_max == facts_bias_abs_max(facts)
                && checked_add_u64(*cross_chunk_sum_bound, *bias_abs_max) == Some(*total_abs_max)
                && *i32_envelope == I32_ENVELOPE_U64
                && *total_abs_max <= *i32_envelope
                && i32_envelope.checked_sub(*total_abs_max) == Some(*slack);
            if ok {
                record_range_construction_event(RANGE_CERT_VERIFIES_CHUNKED_I16_EVENT);
                tracing::info!(
                    target: "gbf_verify::range_cert",
                    event = %RANGE_CERT_VERIFIES_CHUNKED_I16_EVENT,
                    site = facts.site.0.as_str(),
                    slack = *slack,
                );
            }
            ok
        }
        AccumulatorCertificate::RenormLoopProof {
            site,
            tile_len,
            tile_count,
            per_term_abs_max,
            per_tile_sum_bound,
            per_tile_i16_slack,
            renorm,
            bias_abs_max,
            total_abs_max,
            slack,
        } => {
            if determinism == DeterminismClass::BitExact {
                record_range_construction_event(RANGE_CERT_REJECTS_BITEXACT_RENORM_LOOP_EVENT);
                tracing::info!(
                    target: "gbf_verify::range_cert",
                    event = %RANGE_CERT_REJECTS_BITEXACT_RENORM_LOOP_EVENT,
                    site = facts.site.0.as_str(),
                    tile_len = *tile_len as u64,
                    tile_count = *tile_count,
                    "BitExact RenormLoop is reserved in range_plan.v1",
                );
                return false;
            }
            let (plan_tile_len, plan_renorm) = match plan {
                ReductionPlan::RenormLoop { tile_len, renorm } => (Some(*tile_len), Some(renorm)),
                _ => (None, None),
            };
            let ok = *tile_len > 0
                && site == &facts.site
                && plan_tile_len == Some(*tile_len)
                && plan_renorm == Some(renorm)
                && ceil_div_u64(u64::from(facts.term_count), u64::from(*tile_len))
                    == Some(*tile_count)
                && *per_term_abs_max == facts.per_term_abs_max_q
                && checked_mul_u64(u64::from(*tile_len), *per_term_abs_max)
                    == Some(*per_tile_sum_bound)
                && *per_tile_sum_bound <= I16_ENVELOPE_U64
                && I16_ENVELOPE_U64.checked_sub(*per_tile_sum_bound) == Some(*per_tile_i16_slack)
                && *bias_abs_max == facts_bias_abs_max(facts)
                && renorm_recurrence_verifies(
                    facts,
                    *tile_len,
                    *tile_count,
                    renorm.strategy.clone(),
                    renorm.recurrence,
                    *total_abs_max,
                )
                && *total_abs_max <= I16_ENVELOPE_U64
                && I16_ENVELOPE_U64.checked_sub(*total_abs_max) == Some(*slack);
            if ok {
                record_range_construction_event(RANGE_CERT_VERIFIES_RENORM_LOOP_EVENT);
                tracing::info!(
                    target: "gbf_verify::range_cert",
                    event = %RANGE_CERT_VERIFIES_RENORM_LOOP_EVENT,
                    site = facts.site.0.as_str(),
                    slack = *slack,
                );
            }
            ok
        }
        AccumulatorCertificate::Failed {
            site,
            proof_state,
            witness,
            ..
        } => {
            record_range_construction_event(RANGE_CERT_VERIFIES_FAILED_EVENT);
            tracing::info!(
                target: "gbf_verify::range_cert",
                event = %RANGE_CERT_VERIFIES_FAILED_EVENT,
                site = site.0.as_str(),
                proof_state = ?proof_state,
                witness_kind = ?witness,
            );
            false
        }
    }
}

#[must_use]
pub fn verifies_for(
    plan: &ReductionPlan,
    facts: &ReductionSiteFacts,
    determinism: DeterminismClass,
) -> bool {
    verifies_for_impl(plan, facts, determinism)
}

#[must_use]
pub fn verifies_for_with_determinism(
    plan: &ReductionPlan,
    facts: &ReductionSiteFacts,
    determinism: DeterminismClass,
) -> bool {
    verifies_for_impl(plan, facts, determinism)
}

fn verifies_for_impl(
    plan: &ReductionPlan,
    facts: &ReductionSiteFacts,
    determinism: DeterminismClass,
) -> bool {
    let cert = construct_accumulator_certificate(plan, facts, determinism);
    !matches!(cert, AccumulatorCertificate::Failed { .. })
        && verifies(&cert, plan, facts, determinism)
}

#[must_use]
pub fn renorm_recurrence_verifies(
    facts: &ReductionSiteFacts,
    tile_len: u16,
    tile_count: u64,
    strategy: RenormStrategy,
    recurrence: RenormRecurrence,
    claimed_total_abs_max: u64,
) -> bool {
    let ok = facts.accumulator_domain == AccumulatorDomain::RawIntegerProducts
        && recurrence.output_scale_q16_16 > 0
        && recurrence.max_rounding_error_q16_16 == rounding_error_q16_16(recurrence.rounding)
        && renorm_closed_form_bound(facts, tile_len, tile_count, &strategy, recurrence)
            == Some(claimed_total_abs_max);

    record_range_construction_event(RANGE_CERT_RENORM_RECURRENCE_VERIFIES_EVENT);
    tracing::info!(
        target: "gbf_verify::range_cert",
        event = %RANGE_CERT_RENORM_RECURRENCE_VERIFIES_EVENT,
        site = facts.site.0.as_str(),
        scale = recurrence.output_scale_q16_16,
        rounding = ?recurrence.rounding,
        saturation = ?recurrence.saturation,
        ok,
    );

    ok
}

#[must_use]
pub fn choose_plan(
    binding: &EffectiveCeilingBinding,
    generation: &PlanCandidateGeneration,
    caps: &RangeCapsSpec,
    determinism: DeterminismClass,
) -> PlanChoice {
    let mut attempts = generation
        .candidates
        .iter()
        .map(|plan| CertificateConstruction {
            plan: plan.clone(),
            certificate: construct_accumulator_certificate(plan, &binding.facts, determinism),
        })
        .collect::<Vec<_>>();
    attempts.sort_by_key(|attempt| plan_family(&attempt.plan));

    let chosen = attempts
        .iter()
        .find(|attempt| {
            verifies_with_determinism(
                &attempt.certificate,
                &attempt.plan,
                &binding.facts,
                determinism,
            )
        })
        .map(|attempt| CertifiedReduction {
            site: binding.site.clone(),
            plan: attempt.plan.clone(),
            facts: binding.facts.clone(),
            proof: attempt.certificate.clone(),
        });

    let diagnostics = if chosen.is_some() {
        Vec::new()
    } else {
        vec![plan_choice_diagnostic(
            &binding.site,
            plan_choice_failure_code(binding, caps, determinism),
        )]
    };

    record_range_construction_event(RANGE_PLAN_CHOICE_EVENT);
    tracing::info!(
        target: "gbf_codegen::s5",
        event = %RANGE_PLAN_CHOICE_EVENT,
        site = binding.site.0.as_str(),
        chosen_family = ?chosen.as_ref().map(|certified| plan_family(&certified.plan)),
        attempts = attempts.len() as u64,
    );

    PlanChoice {
        site: binding.site.clone(),
        chosen,
        attempts,
        diagnostics,
    }
}

#[must_use]
pub fn bind_range_plan_provenance(sites: &[ReductionSiteBinding]) -> RangePlanProvenance {
    let provenance = RangePlanProvenance {
        site_to_node: sites
            .iter()
            .map(|binding| (binding.site.clone(), binding.node_id))
            .collect(),
        site_to_qg: sites
            .iter()
            .map(|binding| (binding.site.clone(), binding.qg_ref.clone()))
            .collect(),
    };

    record_range_construction_event(RANGE_PROVENANCE_BIND_EVENT);
    tracing::info!(
        target: "gbf_codegen::s5",
        event = %RANGE_PROVENANCE_BIND_EVENT,
        site_count = sites.len() as u64,
    );

    provenance
}

pub fn canonical_sort_range_plan(
    entries: &mut [RangePlanEntry],
    certificates: &mut [CertifiedReduction],
) -> BTreeMap<ReductionSiteId, u32> {
    entries.sort_by(|left, right| left.site.cmp(&right.site));
    certificates.sort_by(|left, right| left.site.cmp(&right.site));
    let index = site_to_certificate_index(certificates);

    record_range_construction_event(RANGE_CANONICAL_SORT_EVENT);
    tracing::info!(
        target: "gbf_codegen::s5",
        event = %RANGE_CANONICAL_SORT_EVENT,
        entry_count = entries.len() as u64,
        certificate_count = certificates.len() as u64,
    );

    index
}

#[must_use]
pub fn site_to_certificate_index(
    certificates: &[CertifiedReduction],
) -> BTreeMap<ReductionSiteId, u32> {
    certificates
        .iter()
        .enumerate()
        .map(|(index, certificate)| {
            (
                certificate.site.clone(),
                u32::try_from(index).expect("certificate index fits u32"),
            )
        })
        .collect()
}

#[must_use]
pub fn range_cert_outcome(
    certificates: &[CertifiedReduction],
    diagnostics: &[ValidationDiagnosticRecord],
) -> CertOutcome {
    if certificates
        .iter()
        .any(|certified| matches!(certified.proof, AccumulatorCertificate::Failed { .. }))
        || has_hard_diagnostic(diagnostics)
    {
        CertOutcome::Failed
    } else {
        CertOutcome::Verified
    }
}

#[must_use]
pub fn self_consistency_diagnostics(
    range_plan: &RangePlan,
    range_cert: &RangeCertBody,
) -> Vec<ValidationDiagnosticRecord> {
    let mut diagnostics = Vec::new();
    let mut entry_sites = BTreeSet::new();
    for entry in &range_plan.entries {
        if !entry_sites.insert(entry.site.clone()) {
            diagnostics.push(self_consistency_diagnostic("range_plan.entries.site"));
        }
        if entry.site != entry.site_facts.site {
            diagnostics.push(self_consistency_diagnostic("range_plan.entries.site_facts"));
        }
        if !candidate_families_under_ceiling(entry.effective_ceiling)
            .contains(&plan_family(&entry.plan))
        {
            diagnostics.push(self_consistency_diagnostic(
                "range_plan.entries.effective_ceiling",
            ));
        }
        if entry.site_facts.accumulator_domain != AccumulatorDomain::RawIntegerProducts {
            diagnostics.push(self_consistency_diagnostic(
                "range_plan.entries.site_facts.accumulator_domain",
            ));
        }
        if !smallest_canonical_family_holds(entry, range_plan.identity.determinism) {
            diagnostics.push(self_consistency_diagnostic("range_plan.entries.plan"));
        }
        if !bitexact_entry_constraints_hold(entry, range_plan.identity.determinism) {
            diagnostics.push(self_consistency_diagnostic(
                "range_plan.entries.plan.determinism",
            ));
        }
    }

    let expected_index = site_to_certificate_index(&range_cert.certificates);
    if expected_index != range_cert.site_to_certificate_index
        || expected_index.len() != range_cert.certificates.len()
    {
        diagnostics.push(self_consistency_diagnostic(
            "range_cert.site_to_certificate_index",
        ));
    }

    for entry in &range_plan.entries {
        let Some(index) = range_cert.site_to_certificate_index.get(&entry.site) else {
            diagnostics.push(self_consistency_diagnostic("range_cert.certificates"));
            continue;
        };
        let Some(certified) = range_cert.certificates.get(*index as usize) else {
            diagnostics.push(self_consistency_diagnostic(
                "range_cert.site_to_certificate_index",
            ));
            continue;
        };
        if certified.site != entry.site
            || certified.plan != entry.plan
            || certified.facts != entry.site_facts
            || !verifies(
                &certified.proof,
                &entry.plan,
                &entry.site_facts,
                range_plan.identity.determinism,
            )
        {
            diagnostics.push(self_consistency_diagnostic("range_cert.certificates.proof"));
        }
    }

    // RP-SC-12 is currently owned by report/policy schema validators: local
    // ValidationDiagnostic records expose neither RepairProposal source nor
    // AuthorizedRelaxation operation, so this pure RangePlan/RangeCert check has
    // no additional repair/relaxation predicate to evaluate.
    record_range_construction_event(RANGE_SELF_CONSISTENCY_EVENT);
    tracing::info!(
        target: "gbf_codegen::s5",
        event = %RANGE_SELF_CONSISTENCY_EVENT,
        diagnostic_count = diagnostics.len() as u64,
    );

    diagnostics
}

fn smallest_canonical_family_holds(entry: &RangePlanEntry, determinism: DeterminismClass) -> bool {
    match &entry.plan {
        ReductionPlan::SingleI16 => true,
        // RangeCaps are not embedded in range_plan.v1, so the pure RP-SC-6
        // check proves the cap-independent smaller-family case: SingleI16 must
        // not verify once Stage 5 chose a wider family.
        ReductionPlan::ChunkedI16 { .. } | ReductionPlan::RenormLoop { .. } => {
            !verifies_for(&ReductionPlan::SingleI16, &entry.site_facts, determinism)
        }
    }
}

fn bitexact_entry_constraints_hold(entry: &RangePlanEntry, determinism: DeterminismClass) -> bool {
    if determinism != DeterminismClass::BitExact {
        return true;
    }

    match &entry.plan {
        ReductionPlan::SingleI16 => true,
        ReductionPlan::ChunkedI16 { chunk_len } => {
            *chunk_len != 0
                && entry
                    .site_facts
                    .term_count
                    .is_multiple_of(u32::from(*chunk_len))
        }
        ReductionPlan::RenormLoop { .. } => false,
    }
}

fn construct_single_i16_certificate(
    plan: &ReductionPlan,
    facts: &ReductionSiteFacts,
) -> AccumulatorCertificate {
    let term_count = u64::from(facts.term_count);
    let per_term_abs_max = facts.per_term_abs_max_q;
    let Some(sum_bound) = checked_mul_u64(term_count, per_term_abs_max) else {
        return failed_certificate(
            plan,
            facts,
            AccumulatorProofState::SumExceedsI16Envelope {
                sum_bound: u64::MAX,
                envelope: I16_ENVELOPE_U64,
            },
        );
    };
    let bias_abs_max = facts_bias_abs_max(facts);
    let Some(total_abs_max) = checked_add_u64(sum_bound, bias_abs_max) else {
        return failed_certificate(
            plan,
            facts,
            AccumulatorProofState::SumExceedsI16Envelope {
                sum_bound: u64::MAX,
                envelope: I16_ENVELOPE_U64,
            },
        );
    };
    if total_abs_max > I16_ENVELOPE_U64 {
        return failed_certificate(
            plan,
            facts,
            AccumulatorProofState::SumExceedsI16Envelope {
                sum_bound: total_abs_max,
                envelope: I16_ENVELOPE_U64,
            },
        );
    }
    AccumulatorCertificate::SingleI16Proof {
        site: facts.site.clone(),
        term_count,
        per_term_abs_max,
        sum_bound,
        bias_abs_max,
        total_abs_max,
        i16_envelope: I16_ENVELOPE_U64,
        slack: I16_ENVELOPE_U64 - total_abs_max,
    }
}

fn construct_chunked_i16_certificate(
    chunk_len: u16,
    plan: &ReductionPlan,
    facts: &ReductionSiteFacts,
    determinism: DeterminismClass,
) -> AccumulatorCertificate {
    if chunk_len == 0 {
        return failed_certificate(
            plan,
            facts,
            AccumulatorProofState::LengthZero {
                length_field: LengthField::ChunkLen,
            },
        );
    }
    if determinism == DeterminismClass::BitExact
        && !facts.term_count.is_multiple_of(u32::from(chunk_len))
    {
        return failed_certificate(
            plan,
            facts,
            AccumulatorProofState::BitExactRequiresChunkDivides {
                term_count: facts.term_count,
                chunk_len,
            },
        );
    }

    let per_term_abs_max = facts.per_term_abs_max_q;
    let Some(per_chunk_sum_bound) = checked_mul_u64(u64::from(chunk_len), per_term_abs_max) else {
        return failed_certificate(
            plan,
            facts,
            AccumulatorProofState::PerChunkExceedsI16Envelope {
                per_chunk_sum_bound: u64::MAX,
                envelope: I16_ENVELOPE_U64,
            },
        );
    };
    if per_chunk_sum_bound > I16_ENVELOPE_U64 {
        return failed_certificate(
            plan,
            facts,
            AccumulatorProofState::PerChunkExceedsI16Envelope {
                per_chunk_sum_bound,
                envelope: I16_ENVELOPE_U64,
            },
        );
    }

    let Some(cross_chunk_sum_bound) =
        checked_mul_u64(u64::from(facts.term_count), per_term_abs_max)
    else {
        return failed_certificate(
            plan,
            facts,
            AccumulatorProofState::CrossChunkExceedsI32Envelope {
                cross_chunk_sum_bound: u64::MAX,
                envelope: I32_ENVELOPE_U64,
            },
        );
    };
    let bias_abs_max = facts_bias_abs_max(facts);
    let Some(total_abs_max) = checked_add_u64(cross_chunk_sum_bound, bias_abs_max) else {
        return failed_certificate(
            plan,
            facts,
            AccumulatorProofState::CrossChunkExceedsI32Envelope {
                cross_chunk_sum_bound: u64::MAX,
                envelope: I32_ENVELOPE_U64,
            },
        );
    };
    if total_abs_max > I32_ENVELOPE_U64 {
        return failed_certificate(
            plan,
            facts,
            AccumulatorProofState::CrossChunkExceedsI32Envelope {
                cross_chunk_sum_bound: total_abs_max,
                envelope: I32_ENVELOPE_U64,
            },
        );
    }

    AccumulatorCertificate::ChunkedI16Proof {
        site: facts.site.clone(),
        chunk_len,
        chunk_count: ceil_div_u64(u64::from(facts.term_count), u64::from(chunk_len))
            .expect("non-zero chunk len"),
        per_term_abs_max,
        per_chunk_sum_bound,
        per_chunk_i16_slack: I16_ENVELOPE_U64 - per_chunk_sum_bound,
        cross_chunk_sum_bound,
        bias_abs_max,
        total_abs_max,
        i32_envelope: I32_ENVELOPE_U64,
        slack: I32_ENVELOPE_U64 - total_abs_max,
    }
}

fn construct_renorm_loop_certificate(
    tile_len: u16,
    renorm: &RenormSpec,
    plan: &ReductionPlan,
    facts: &ReductionSiteFacts,
    determinism: DeterminismClass,
) -> AccumulatorCertificate {
    if determinism == DeterminismClass::BitExact {
        // range_plan.v1 reserves every BitExact RenormLoop form, including the
        // degenerate full-tile shape. The schema keeps the saturation witness
        // explicit so downstream validators can distinguish this deterministic
        // rejection from ordinary bound arithmetic failures.
        return failed_certificate_with_witness(
            plan,
            facts,
            AccumulatorProofState::DeterminismRequiresEnforcedRenorm,
            AccumulatorFailureWitness::BitExactSaturationForbidden,
        );
    }
    if tile_len == 0 {
        return failed_certificate(
            plan,
            facts,
            AccumulatorProofState::LengthZero {
                length_field: LengthField::TileLen,
            },
        );
    }

    let per_term_abs_max = facts.per_term_abs_max_q;
    let Some(per_tile_sum_bound) = checked_mul_u64(u64::from(tile_len), per_term_abs_max) else {
        return failed_certificate(
            plan,
            facts,
            AccumulatorProofState::PerTileExceedsI16Envelope {
                per_tile_sum_bound: u64::MAX,
                envelope: I16_ENVELOPE_U64,
            },
        );
    };
    if per_tile_sum_bound > I16_ENVELOPE_U64 {
        return failed_certificate(
            plan,
            facts,
            AccumulatorProofState::PerTileExceedsI16Envelope {
                per_tile_sum_bound,
                envelope: I16_ENVELOPE_U64,
            },
        );
    }

    let tile_count =
        ceil_div_u64(u64::from(facts.term_count), u64::from(tile_len)).expect("non-zero tile len");
    let Some(total_abs_max) = renorm_closed_form_bound(
        facts,
        tile_len,
        tile_count,
        &renorm.strategy,
        renorm.recurrence,
    ) else {
        return failed_certificate(
            plan,
            facts,
            AccumulatorProofState::SumExceedsI16Envelope {
                sum_bound: u64::MAX,
                envelope: I16_ENVELOPE_U64,
            },
        );
    };
    if total_abs_max > I16_ENVELOPE_U64 {
        return failed_certificate(
            plan,
            facts,
            AccumulatorProofState::SumExceedsI16Envelope {
                sum_bound: total_abs_max,
                envelope: I16_ENVELOPE_U64,
            },
        );
    }

    AccumulatorCertificate::RenormLoopProof {
        site: facts.site.clone(),
        tile_len,
        tile_count,
        per_term_abs_max,
        per_tile_sum_bound,
        per_tile_i16_slack: I16_ENVELOPE_U64 - per_tile_sum_bound,
        renorm: renorm.clone(),
        bias_abs_max: facts_bias_abs_max(facts),
        total_abs_max,
        slack: I16_ENVELOPE_U64 - total_abs_max,
    }
}

fn failed_certificate(
    plan: &ReductionPlan,
    facts: &ReductionSiteFacts,
    proof_state: AccumulatorProofState,
) -> AccumulatorCertificate {
    failed_certificate_with_witness(
        plan,
        facts,
        proof_state,
        AccumulatorFailureWitness::BoundCalculation {
            input_max_abs_q: facts.input_max_abs_q,
            weight_max_abs_q: facts.weight_max_abs_q,
            term_count: facts.term_count,
            bias: facts.bias_max_abs_q.unwrap_or(0),
        },
    )
}

fn failed_certificate_with_witness(
    plan: &ReductionPlan,
    facts: &ReductionSiteFacts,
    proof_state: AccumulatorProofState,
    witness: AccumulatorFailureWitness,
) -> AccumulatorCertificate {
    AccumulatorCertificate::Failed {
        site: facts.site.clone(),
        attempted_plan: plan.clone(),
        proof_state,
        witness,
    }
}

fn renorm_closed_form_bound(
    facts: &ReductionSiteFacts,
    tile_len: u16,
    tile_count: u64,
    _strategy: &RenormStrategy,
    recurrence: RenormRecurrence,
) -> Option<u64> {
    if tile_len == 0 || recurrence.output_scale_q16_16 == 0 {
        return None;
    }
    if ceil_div_u64(u64::from(facts.term_count), u64::from(tile_len))? != tile_count {
        return None;
    }
    let per_tile_sum_bound = checked_mul_u64(u64::from(tile_len), facts.per_term_abs_max_q)?;
    let rounding_error_units =
        ceil_div_u64(u64::from(recurrence.max_rounding_error_q16_16), 65_536)?;
    let mut state = 0_u64;
    for _ in 0..tile_count {
        let pre_scale = checked_add_u64(state, per_tile_sum_bound)?;
        let scaled = ceil_div_u128(
            u128::from(pre_scale).checked_mul(u128::from(recurrence.input_scale_q16_16))?,
            u128::from(recurrence.output_scale_q16_16),
        )?;
        state = checked_add_u64(u64::try_from(scaled).ok()?, rounding_error_units)?;
    }
    checked_add_u64(state, facts_bias_abs_max(facts))
}

// Conservative q16.16 rounding-error budget. `NearestEven` uses one
// fractional unit so the integer-unit recurrence charges at most one extra
// accumulator unit after ceiling, without host-inspecting differentiable state.
const fn rounding_error_q16_16(rounding: RenormRounding) -> u32 {
    match rounding {
        RenormRounding::TowardZero => 0,
        RenormRounding::NearestEven => 1,
    }
}

fn plan_choice_failure_code(
    binding: &EffectiveCeilingBinding,
    caps: &RangeCapsSpec,
    determinism: DeterminismClass,
) -> &'static str {
    match binding.effective_ceiling {
        ReductionPlanCeiling::ExactOnly => RANGE_CEILING_VIOLATED_SINGLE_I16_ONLY_CODE,
        ReductionPlanCeiling::Conservative
            if canonical_candidate_for_family(
                ReductionPlanFamily::RenormLoop,
                &binding.facts,
                caps,
                determinism,
            )
            .is_ok_and(|renorm| {
                verifies_for_with_determinism(&renorm, &binding.facts, determinism)
            }) =>
        {
            RANGE_CEILING_VIOLATED_NO_RENORM_LOOP_CODE
        }
        _ => RANGE_NO_PROVEN_PLAN_WITHIN_CEILING_CODE,
    }
}

fn plan_choice_diagnostic(site: &ReductionSiteId, code: &'static str) -> ValidationDiagnostic {
    ValidationDiagnostic::hard(
        ValidationOrigin::RangePlanConstruction,
        ValidationCode::ReportSemanticInvariantViolated {
            field: FieldPath::from("range_plan.entries"),
        },
        ValidationDetail::Field {
            field: FieldPath::from("range_plan.entries"),
        },
        vec![EvidenceRef {
            kind: "stage5-range-plan-choice".to_owned(),
            reference: format!("{code}:{}", site.0),
            hash: Some(Hash256::ZERO),
        }],
    )
}

fn self_consistency_diagnostic(field: &'static str) -> ValidationDiagnostic {
    ValidationDiagnostic::hard(
        ValidationOrigin::SemanticCore,
        ValidationCode::ReportSemanticInvariantViolated {
            field: FieldPath::from(field),
        },
        ValidationDetail::Field {
            field: FieldPath::from(field),
        },
        vec![EvidenceRef {
            kind: "stage5-range-plan-self-consistency".to_owned(),
            reference: "RP-SC".to_owned(),
            hash: Some(Hash256::ZERO),
        }],
    )
}

fn plan_family(plan: &ReductionPlan) -> ReductionPlanFamily {
    match plan {
        ReductionPlan::SingleI16 => ReductionPlanFamily::SingleI16,
        ReductionPlan::ChunkedI16 { .. } => ReductionPlanFamily::ChunkedI16,
        ReductionPlan::RenormLoop { .. } => ReductionPlanFamily::RenormLoop,
    }
}

fn facts_bias_abs_max(facts: &ReductionSiteFacts) -> u64 {
    u64::from(facts.bias_max_abs_q.unwrap_or(0))
}

const fn checked_mul_u64(left: u64, right: u64) -> Option<u64> {
    left.checked_mul(right)
}

const fn checked_add_u64(left: u64, right: u64) -> Option<u64> {
    left.checked_add(right)
}

fn ceil_div_u64(numerator: u64, denominator: u64) -> Option<u64> {
    if denominator == 0 {
        return None;
    }
    let extra = if numerator.is_multiple_of(denominator) {
        0
    } else {
        1
    };
    Some(numerator / denominator + extra)
}

fn ceil_div_u128(numerator: u128, denominator: u128) -> Option<u128> {
    if denominator == 0 {
        return None;
    }
    Some(numerator / denominator + u128::from(!numerator.is_multiple_of(denominator)))
}

fn expert_weight_slot_for_op(op: InferOp) -> Option<ExpertWeightSlot> {
    match op {
        InferOp::ExpertMatVec { slot, .. } => Some(slot),
        _ => None,
    }
}

fn norm_site_for_qg_ref(qg_ref: &QuantGraphEntityRef) -> Option<NormSite> {
    match qg_ref {
        QuantGraphEntityRef::NormSite { site } => Some(*site),
        _ => None,
    }
}

impl ReportBody for RangePlanReportBody {
    const REPORT_TYPE: &'static str = "RangePlanReport";
    const SCHEMA_ID: &'static str = RANGE_PLAN_SCHEMA_VERSION;
    const SCHEMA_VERSION: &'static str = RANGE_REPORT_SCHEMA_SEMVER;

    fn validate_semantics(&self, outcome: ReportOutcome) -> Result<(), Vec<ValidationDiagnostic>> {
        validate_product_report_body(outcome, self.result.is_some(), &self.diagnostics)
    }
}

impl ReportBody for RangeCertBody {
    const REPORT_TYPE: &'static str = "RangeCertBody";
    const SCHEMA_ID: &'static str = RANGE_CERT_SCHEMA_VERSION;
    const SCHEMA_VERSION: &'static str = RANGE_REPORT_SCHEMA_SEMVER;

    fn validate_semantics(&self, outcome: ReportOutcome) -> Result<(), Vec<ValidationDiagnostic>> {
        let has_hard = has_hard_diagnostic(&self.diagnostics);
        let has_failed_certificate = self
            .certificates
            .iter()
            .any(|cert| matches!(cert.proof, AccumulatorCertificate::Failed { .. }));

        match outcome {
            ReportOutcome::Passed
                if self.identity.range_plan_self_hash.is_some()
                    && self.cert_outcome == CertOutcome::Verified
                    && !has_failed_certificate
                    && !has_hard =>
            {
                Ok(())
            }
            ReportOutcome::Failed
                if self.cert_outcome == CertOutcome::Failed
                    && (has_failed_certificate || has_hard) =>
            {
                Ok(())
            }
            _ => Err(vec![product_report_invariant_diagnostic("cert_outcome")]),
        }
    }
}

fn validate_product_report_body(
    outcome: ReportOutcome,
    has_result: bool,
    diagnostics: &[ValidationDiagnostic],
) -> Result<(), Vec<ValidationDiagnostic>> {
    let has_hard = has_hard_diagnostic(diagnostics);

    match outcome {
        ReportOutcome::Passed if has_result && !has_hard => Ok(()),
        ReportOutcome::Failed if !has_result && has_hard => Ok(()),
        ReportOutcome::Passed => {
            let mut diagnostics = Vec::new();
            if !has_result {
                diagnostics.push(product_report_invariant_diagnostic("result"));
            }
            if has_hard {
                diagnostics.push(product_report_invariant_diagnostic("diagnostics"));
            }
            Err(diagnostics)
        }
        ReportOutcome::Failed => {
            let mut diagnostics = Vec::new();
            if has_result {
                diagnostics.push(product_report_invariant_diagnostic("result"));
            }
            if !has_hard {
                diagnostics.push(product_report_invariant_diagnostic("diagnostics"));
            }
            Err(diagnostics)
        }
    }
}

fn has_hard_diagnostic(diagnostics: &[ValidationDiagnostic]) -> bool {
    diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Hard)
}

fn product_report_invariant_diagnostic(field: &'static str) -> ValidationDiagnostic {
    ValidationDiagnostic::hard(
        ValidationOrigin::SemanticCore,
        ValidationCode::ReportSemanticInvariantViolated {
            field: FieldPath::from(field),
        },
        ValidationDetail::Field {
            field: FieldPath::from(field),
        },
        Vec::new(),
    )
}

fn domain_hash<T: Serialize>(
    type_name: &str,
    schema_version: &str,
    value: &T,
) -> Result<Hash256, serde_json::Error> {
    let canonical = canonical_json_bytes(value)?;
    Ok(domain_hash_from_canonical(
        type_name,
        schema_version,
        &canonical,
    ))
}

fn domain_hash_from_canonical(type_name: &str, schema_version: &str, canonical: &[u8]) -> Hash256 {
    let mut hasher = Sha256::new();
    hasher.update(format!("gbf:gbf-codegen:{type_name}:{schema_version}\0"));
    hasher.update(canonical);
    Hash256::from_bytes(hasher.finalize().into())
}

fn canonical_json_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, serde_json::Error> {
    let value = serde_json::to_value(value)?;
    Ok(canonicalize_value(&value).expect("Stage 5 material canonicalizes"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::collections::BTreeSet;
    use std::io::Write;
    use std::sync::{Arc, Mutex};
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::budget::{RuntimeChromeBudgetSection, runtime_chrome_budget_section_hash};
    use crate::s3::infer_ir::{
        GbInferIR, InferIrAuditParents, InferIrIdentity, InferIrProvenance, TokenIngressMode,
        TokenInput, TokenInputId, ValueDecl, ValueFormat, ValueId, ValueKind, ValueLayout,
        ValueProducerRef,
    };
    use gbf_foundation::{BudgetSlotId, TargetProfileId};
    use gbf_policy::{
        BRINGUP_COMPILE_PROFILE_TOML, BudgetFailure, BudgetSlotClass, CompileProfileSpecLoadError,
        PlacementProfile, RenormStrategyPolicy, RomBudgetSlot, RuntimeChromeBudget,
        RuntimeMemoryCapSection, RuntimeMode, budget_failure_diagnostic, load_compile_profile_spec,
    };
    use gbf_report::report_schemas::{infer_ir_v1::FixtureEquivalenceTag, static_budget_v1};
    use gbf_store::blob::BlobStore;
    use gbf_store::stage_cache::StageCache;
    use serde_json::{Map, Value};
    use tracing::field::{Field, Visit};
    use tracing_subscriber::filter::LevelFilter;
    use tracing_subscriber::prelude::*;

    fn hash(byte: u8) -> Hash256 {
        Hash256::from_bytes([byte; 32])
    }

    fn site(value: &str) -> ReductionSiteId {
        ReductionSiteId(value.to_owned())
    }

    fn projection(site_id: &str, bias_max_abs_q: Option<u32>) -> ReductionSiteProjection {
        ReductionSiteProjection {
            site: site(site_id),
            layer: Some(LayerId::new(3)),
            expert: Some(ExpertId::new(5)),
            term_count: 11,
            input_max_abs_q: 13,
            weight_max_abs_q: 17,
            bias_max_abs_q,
            accumulator_domain: AccumulatorDomain::RawIntegerProducts,
        }
    }

    fn binding(site_id: &str, node_id: u32) -> ReductionSiteBinding {
        ReductionSiteBinding {
            node_id: NodeId::new(node_id),
            site: site(site_id),
            qg_ref: QuantGraphEntityRef::ExpertTensor {
                layer: LayerId::new(3),
                expert: ExpertId::new(5),
                slot: ExpertWeightSlot::FfnDown,
                tensor: 7_u32.into(),
            },
            op_tag: InferOpTag::ExpertMatVec,
            slot: Some(ExpertWeightSlot::FfnDown),
            norm_site: None,
        }
    }

    fn range_ir_with_nodes(nodes: Vec<crate::s3::infer_ir::GbNode>) -> GbInferIR {
        let mut ir = infer_ir_product_fixture().infer_ir;
        ir.nodes = nodes;
        ir.provenance.nodes = ir
            .nodes
            .iter()
            .map(|node| {
                let qg_ref = match node.op {
                    crate::s3::infer_ir::InferOp::Classify => QuantGraphEntityRef::ClassifyHead,
                    crate::s3::infer_ir::InferOp::ExpertMatVec {
                        layer,
                        expert,
                        slot,
                    } => QuantGraphEntityRef::ExpertTensor {
                        layer,
                        expert,
                        slot,
                        tensor: 7_u32.into(),
                    },
                    crate::s3::infer_ir::InferOp::Norm { .. } => QuantGraphEntityRef::NormSite {
                        site: NormSite::LayerFfn {
                            layer: LayerId::new(0),
                        },
                    },
                    _ => QuantGraphEntityRef::Embedding,
                };
                (node.node_id, qg_ref)
            })
            .collect();
        ir
    }

    fn range_node(
        node_id: u32,
        site_id: Option<&str>,
        op: crate::s3::infer_ir::InferOp,
    ) -> crate::s3::infer_ir::GbNode {
        crate::s3::infer_ir::GbNode {
            node_id: NodeId::new(node_id),
            op,
            inputs: Vec::new(),
            effects_in: Vec::new(),
            outputs: Vec::new(),
            effects_out: Vec::new(),
            reduction_site: site_id.map(site),
        }
    }

    struct RecordingStaticBudgetFacts {
        projections: Vec<ReductionSiteProjection>,
        calls: Cell<usize>,
    }

    impl RecordingStaticBudgetFacts {
        fn new(projections: Vec<ReductionSiteProjection>) -> Self {
            Self {
                projections,
                calls: Cell::new(0),
            }
        }
    }

    impl StaticBudgetReductionSiteFacts for RecordingStaticBudgetFacts {
        fn reduction_site_projection(
            &self,
            site: &ReductionSiteId,
        ) -> Option<&ReductionSiteProjection> {
            self.calls.set(self.calls.get() + 1);
            self.projections
                .iter()
                .find(|projection| &projection.site == site)
        }
    }

    fn recurrence(rounding: RenormRounding) -> RenormRecurrence {
        RenormRecurrence {
            input_scale_q16_16: 0x0001_0000,
            output_scale_q16_16: 0x0000_8000,
            rounding,
            saturation: RenormSaturationPolicy::AtNamedNumericBoundary {
                boundary: NamedNumericBoundary::ResidualCombine {
                    layer: Some(LayerId::new(1)),
                    site: ResidualSite::PostFfn,
                },
            },
            max_rounding_error_q16_16: 1,
        }
    }

    fn renorm_spec(strategy: RenormStrategy) -> RenormSpec {
        RenormSpec {
            strategy,
            recurrence: recurrence(RenormRounding::NearestEven),
        }
    }

    fn facts(site: ReductionSiteId) -> ReductionSiteFacts {
        ReductionSiteFacts {
            site,
            layer: Some(LayerId::new(1)),
            expert: Some(ExpertId::new(2)),
            slot: Some(ExpertWeightSlot::FfnDown),
            norm_site: Some(NormSite::LayerFfn {
                layer: LayerId::new(1),
            }),
            term_count: 64,
            input_max_abs_q: 31,
            weight_max_abs_q: 17,
            per_term_abs_max_q: 527,
            bias_max_abs_q: Some(3),
            accumulator_domain: AccumulatorDomain::RawIntegerProducts,
            op_tag: InferOpTag::ExpertMatVec,
        }
    }

    fn entry(site_id: &str, plan: ReductionPlan) -> RangePlanEntry {
        let site = site(site_id);
        RangePlanEntry {
            site: site.clone(),
            plan,
            site_facts: facts(site.clone()),
            effective_ceiling: ReductionPlanCeiling::Adaptive,
            ceiling_provenance: ReductionCeilingProvenance::SiteOverride {
                site,
                source: PolicySource::CompileRequestOverride,
            },
        }
    }

    fn plan_fixture() -> RangePlan {
        RangePlan {
            identity: RangePlanIdentity {
                infer_ir_self_hash: hash(0x10),
                quant_graph_self_hash: hash(0x11),
                static_budget_self_hash: hash(0x12),
                range_policy_projection_hash: hash(0x13),
                determinism: DeterminismClass::BitExact,
            },
            entries: vec![
                entry("classify", ReductionPlan::SingleI16),
                entry(
                    "expert.1.2.down",
                    ReductionPlan::RenormLoop {
                        tile_len: 16,
                        renorm: renorm_spec(RenormStrategy::ExactPostBoundary),
                    },
                ),
            ],
            provenance: RangePlanProvenance {
                site_to_node: BTreeMap::from([
                    (site("classify"), NodeId::new(7)),
                    (site("expert.1.2.down"), NodeId::new(5)),
                ]),
                site_to_qg: BTreeMap::from([
                    (site("classify"), QuantGraphEntityRef::ClassifyHead),
                    (
                        site("expert.1.2.down"),
                        QuantGraphEntityRef::ExpertTensor {
                            layer: LayerId::new(1),
                            expert: ExpertId::new(2),
                            slot: ExpertWeightSlot::FfnDown,
                            tensor: 9_u32.into(),
                        },
                    ),
                ]),
            },
        }
    }

    fn policy_projection_fixture() -> RangePolicyProjection {
        RangePolicyProjection {
            profile_id: CompileProfileId::from("Bringup"),
            range_caps: RangeCapsSpec {
                profile_chunk_max: 64,
                profile_tile_max: 128,
                profile_tile_min: 8,
                renorm_strategy: RenormStrategyPolicy::DynamicMargin {
                    margin_q16_16: 0x0000_4000,
                },
            },
            reduction_ceiling: ReductionPlanCeiling::Adaptive,
            reduction_ceiling_overrides: BTreeMap::from([(
                ReductionSelector::from("site:classify"),
                ReductionPlanCeiling::Conservative,
            )]),
            determinism_class: DeterminismClass::BitExact,
        }
    }

    fn audit_parents_fixture() -> RangePlanAuditParents {
        RangePlanAuditParents {
            policy_resolution_self_hash: hash(0x40),
            compile_request_hash: hash(0x41),
            artifact_aux_hash: hash(0x42),
            locked_range_knobs: LockedRangeKnobs {
                reduction_ceiling_locked: true,
            },
        }
    }

    fn infer_ir_product_fixture() -> GbInferIRProduct {
        let infer_ir = GbInferIR::new(
            InferIrIdentity {
                quant_graph_self_hash: hash(0x11),
                infer_ir_policy_projection_hash: hash(0x43),
                static_budget_self_hash: hash(0x12),
                requested_runtime_modes_hash: hash(0x44),
                determinism: DeterminismClass::BitExact,
                topological_order_hash: hash(0x45),
            },
            vec![
                TokenInput::new(
                    TokenInputId::new(0),
                    ValueId::new(0),
                    BTreeSet::from([TokenIngressMode::Prompt]),
                )
                .expect("token input is valid"),
            ],
            Vec::new(),
            vec![ValueDecl {
                value_id: ValueId::new(0),
                kind: ValueKind::InputToken,
                format: ValueFormat::TokenIdDomain { vocab_size: 257 },
                layout: ValueLayout::scalar(),
            }],
            Vec::new(),
            InferIrProvenance {
                nodes: BTreeMap::new(),
                values: BTreeMap::from([(
                    ValueId::new(0),
                    ValueProducerRef::External {
                        token_input: TokenInputId::new(0),
                    },
                )]),
                effects: BTreeMap::new(),
            },
            BTreeMap::new(),
        )
        .expect("infer ir fixture is valid");

        GbInferIRProduct::new(
            infer_ir,
            InferIrAuditParents {
                policy_resolution_self_hash: hash(0x40),
                compile_request_hash: hash(0x41),
            },
            BTreeSet::<RuntimeMode>::new(),
            FixtureEquivalenceTag::VerifiedFixtureBitExact,
        )
        .expect("infer ir product builds")
    }

    fn static_budget_report_fixture() -> StaticBudgetReport {
        let failure = BudgetFailure::MissingRuntimeChromeBudget;
        let body = static_budget_v1::StaticBudgetReportBody {
            identity: static_budget_v1::BudgetIdentitySection {
                artifact_core_hash: hash(0x50),
                quant_graph_hash: hash(0x11),
                policy_resolution_self_hash: hash(0x40),
                runtime_chrome_budget_hash: None,
                target_profile_hash: hash(0x51),
            },
            policy: static_budget_v1::BudgetPolicySection {
                placement_profile: PlacementProfile::Budgeted,
                objective_hash: hash(0x52),
            },
            runtime_chrome_budget: None,
            projections: static_budget_v1::BudgetProjectionSection::default(),
            decision: static_budget_v1::BudgetDecisionSection {
                fits: false,
                interpretation: static_budget_v1::static_fit_interpretation_for_fits(false),
                placement_model: static_budget_v1::StaticPlacementModel::BudgetedFirstFit,
                failures: vec![failure.clone()],
            },
            diagnostics: vec![budget_failure_diagnostic(&failure)],
        };
        let report = ReportEnvelope::new(ReportOutcome::Failed, body)
            .expect("static budget report envelope")
            .with_computed_self_hash()
            .expect("static budget report hashes");
        StaticBudgetReport {
            static_budget_self_hash: report.report_self_hash,
            static_budget_canonical_bytes_hash: hash(0x53),
            report,
            reduction_site_facts: Vec::new(),
        }
    }

    fn passed_static_budget_report(
        reduction_site_facts: Vec<ReductionSiteProjection>,
    ) -> StaticBudgetReport {
        let runtime_budget = RuntimeChromeBudget {
            target: TargetProfileId::from("dmg-mbc5"),
            profile: CompileProfileId::from("Default"),
            runtime_nucleus_hash: hash(0x60),
            rom_slots: vec![RomBudgetSlot {
                id: BudgetSlotId::new(0),
                class: BudgetSlotClass::CommonBank,
                usable_bytes: 16 * 1024,
                reserved_slack: 128,
                placement_caps: BTreeSet::from([PlacementProfile::Budgeted]),
            }],
            memory_caps: RuntimeMemoryCapSection {
                wram_usable_bytes: 8 * 1024,
                sram_usable_bytes: 32 * 1024,
                hram_usable_bytes: 127,
                source_target_profile_hash: hash(0x61),
            },
            wram_reserved: 128,
            sram_reserved: 512,
        };
        let runtime_budget_section = RuntimeChromeBudgetSection::from(&runtime_budget);
        let runtime_chrome_budget_hash =
            runtime_chrome_budget_section_hash(&runtime_budget_section).expect("budget hashes");
        let mut projections = static_budget_v1::BudgetProjectionSection::default();
        projections.per_bank_occupancy = vec![static_budget_v1::PerBankEntry {
            slot: BudgetSlotId::new(0),
            class: BudgetSlotClass::CommonBank,
            usable_bytes: 16 * 1024,
            reserved_slack: 128,
            effective_cap_bytes: i64::from(16 * 1024 - 128),
            assigned_bytes: 0,
            residual_bytes: 16 * 1024 - 128,
            assigned_components: Vec::new(),
            placement_caps: BTreeSet::from([PlacementProfile::Budgeted]),
        }];
        let body = static_budget_v1::StaticBudgetReportBody {
            identity: static_budget_v1::BudgetIdentitySection {
                artifact_core_hash: hash(0x50),
                quant_graph_hash: hash(0x11),
                policy_resolution_self_hash: hash(0x40),
                runtime_chrome_budget_hash: Some(runtime_chrome_budget_hash),
                target_profile_hash: hash(0x51),
            },
            policy: static_budget_v1::BudgetPolicySection {
                placement_profile: PlacementProfile::Budgeted,
                objective_hash: hash(0x52),
            },
            runtime_chrome_budget: Some(runtime_budget_section),
            projections,
            decision: static_budget_v1::BudgetDecisionSection {
                fits: true,
                interpretation: static_budget_v1::static_fit_interpretation_for_fits(true),
                placement_model: static_budget_v1::StaticPlacementModel::BudgetedFirstFit,
                failures: Vec::new(),
            },
            diagnostics: Vec::new(),
        };
        let report = ReportEnvelope::new(ReportOutcome::Passed, body)
            .expect("static budget report envelope")
            .with_computed_self_hash()
            .expect("static budget report hashes");
        let canonical_bytes = gbf_report::canonicalize(&report).expect("static budget canonical");
        StaticBudgetReport {
            static_budget_self_hash: report.report_self_hash,
            static_budget_canonical_bytes_hash: Hash256::from_bytes(
                Sha256::digest(&canonical_bytes).into(),
            ),
            report,
            reduction_site_facts,
        }
    }

    fn projection_with_shape(
        site_id: &str,
        layer: Option<LayerId>,
        term_count: u32,
        input_max_abs_q: u32,
        weight_max_abs_q: u32,
        bias_max_abs_q: Option<u32>,
    ) -> ReductionSiteProjection {
        ReductionSiteProjection {
            site: site(site_id),
            layer,
            expert: None,
            term_count,
            input_max_abs_q,
            weight_max_abs_q,
            bias_max_abs_q,
            accumulator_domain: AccumulatorDomain::RawIntegerProducts,
        }
    }

    fn infer_ir_product_for(ir: GbInferIR) -> GbInferIRProduct {
        GbInferIRProduct::new(
            ir,
            InferIrAuditParents {
                policy_resolution_self_hash: hash(0x40),
                compile_request_hash: hash(0x41),
            },
            BTreeSet::<RuntimeMode>::new(),
            FixtureEquivalenceTag::VerifiedFixtureBitExact,
        )
        .expect("infer ir product builds")
    }

    fn stage5_inputs_for(
        nodes: Vec<crate::s3::infer_ir::GbNode>,
        reduction_site_facts: Vec<ReductionSiteProjection>,
        determinism: DeterminismClass,
        ceiling: ReductionPlanCeiling,
        caps: RangeCapsSpec,
        overrides: BTreeMap<ReductionSelector, ReductionPlanCeiling>,
    ) -> RangePlanInputs {
        let mut ir = range_ir_with_nodes(nodes);
        ir.identity.determinism = determinism;
        let infer_ir_product = infer_ir_product_for(ir);
        let static_budget_report = passed_static_budget_report(reduction_site_facts);
        let static_budget_self_hash = static_budget_report.static_budget_self_hash;
        RangePlanInputs {
            infer_ir_self_hash: infer_ir_product.infer_ir_self_hash,
            quant_graph_self_hash: infer_ir_product.infer_ir.identity.quant_graph_self_hash,
            infer_ir_product,
            static_budget_report,
            static_budget_self_hash,
            range_policy_projection: RangePolicyProjection {
                profile_id: CompileProfileId::from("Default"),
                range_caps: caps,
                reduction_ceiling: ceiling,
                reduction_ceiling_overrides: overrides,
                determinism_class: determinism,
            },
            audit_parents: audit_parents_fixture(),
        }
    }

    fn single_i16_stage5_inputs() -> RangePlanInputs {
        stage5_inputs_for(
            vec![range_node(
                1,
                Some("dense.matmul.0"),
                crate::s3::infer_ir::InferOp::Classify,
            )],
            vec![projection_with_shape(
                "dense.matmul.0",
                Some(LayerId::new(0)),
                4,
                32,
                16,
                Some(3),
            )],
            DeterminismClass::BitExact,
            ReductionPlanCeiling::ExactOnly,
            RangeCapsSpec::default_v2(),
            BTreeMap::new(),
        )
    }

    fn chunked_i16_stage5_inputs() -> RangePlanInputs {
        stage5_inputs_for(
            vec![range_node(
                1,
                Some("dense.matmul.0"),
                crate::s3::infer_ir::InferOp::Classify,
            )],
            vec![projection_with_shape(
                "dense.matmul.0",
                Some(LayerId::new(0)),
                64,
                100,
                10,
                Some(3),
            )],
            DeterminismClass::BitExact,
            ReductionPlanCeiling::Conservative,
            RangeCapsSpec::default_v2(),
            BTreeMap::new(),
        )
    }

    fn renorm_loop_stage5_inputs() -> RangePlanInputs {
        stage5_inputs_for(
            vec![range_node(
                1,
                Some("dense.matmul.0"),
                crate::s3::infer_ir::InferOp::Classify,
            )],
            vec![projection_with_shape(
                "dense.matmul.0",
                Some(LayerId::new(0)),
                65_539,
                32_767,
                1,
                None,
            )],
            DeterminismClass::Deterministic,
            ReductionPlanCeiling::Adaptive,
            RangeCapsSpec {
                profile_chunk_max: 1,
                profile_tile_min: 1,
                profile_tile_max: 1,
                renorm_strategy: RenormStrategyPolicy::DynamicMargin { margin_q16_16: 0 },
            },
            BTreeMap::new(),
        )
    }

    fn ceiling_override_layer_site_stage5_inputs() -> RangePlanInputs {
        stage5_inputs_for(
            vec![range_node(
                1,
                Some("expert.0.down"),
                crate::s3::infer_ir::InferOp::ExpertMatVec {
                    layer: LayerId::new(0),
                    expert: ExpertId::new(0),
                    slot: ExpertWeightSlot::FfnDown,
                },
            )],
            vec![projection_with_shape(
                "expert.0.down",
                Some(LayerId::new(0)),
                64,
                100,
                10,
                None,
            )],
            DeterminismClass::BitExact,
            ReductionPlanCeiling::ExactOnly,
            RangeCapsSpec::default_v2(),
            BTreeMap::from([
                (
                    ReductionSelector::from("layer:0"),
                    ReductionPlanCeiling::ExactOnly,
                ),
                (
                    ReductionSelector::from("site:expert.0.down"),
                    ReductionPlanCeiling::Conservative,
                ),
            ]),
        )
    }

    fn failed_cert_stage5_inputs() -> RangePlanInputs {
        stage5_inputs_for(
            vec![range_node(
                1,
                Some("dense.matmul.0"),
                crate::s3::infer_ir::InferOp::Classify,
            )],
            vec![projection_with_shape(
                "dense.matmul.0",
                Some(LayerId::new(0)),
                2,
                20_000,
                1,
                None,
            )],
            DeterminismClass::Deterministic,
            ReductionPlanCeiling::ExactOnly,
            RangeCapsSpec::default_v2(),
            BTreeMap::new(),
        )
    }

    fn expect_stage5_failure(
        result: Result<RangePlanStageOutput, RunStage5Error>,
    ) -> RangePlanStageFailure {
        match result {
            Err(RunStage5Error::StageFailure(failure)) => failure,
            other => panic!("expected Stage 5 failure, got {other:?}"),
        }
    }

    #[derive(Clone)]
    struct Stage5ScriptNdjsonLayer {
        writer: Arc<Mutex<std::fs::File>>,
        build_id: String,
        fixture: String,
        event_seq: Arc<std::sync::atomic::AtomicU64>,
        cert_event_seen: Arc<std::sync::atomic::AtomicBool>,
    }

    impl Stage5ScriptNdjsonLayer {
        fn new(path: &std::path::Path, build_id: String, fixture: String) -> Self {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("ndjson parent dir");
            }
            Self {
                writer: Arc::new(Mutex::new(
                    std::fs::File::create(path).expect("ndjson file"),
                )),
                build_id,
                fixture,
                event_seq: Arc::new(std::sync::atomic::AtomicU64::new(0)),
                cert_event_seen: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            }
        }
    }

    impl<S> tracing_subscriber::layer::Layer<S> for Stage5ScriptNdjsonLayer
    where
        S: tracing::Subscriber,
        for<'a> S: tracing_subscriber::registry::LookupSpan<'a>,
    {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            ctx: tracing_subscriber::layer::Context<'_, S>,
        ) {
            let mut visitor = Stage5ScriptFieldVisitor::default();
            event.record(&mut visitor);
            let event_name = visitor
                .fields
                .remove("event")
                .and_then(|value| value.as_str().map(ToOwned::to_owned))
                .unwrap_or_else(|| event.metadata().name().to_owned());
            if !event_name.starts_with("stage5.") && !event_name.starts_with("range_cert.") {
                return;
            }
            if event_name.starts_with("range_cert.verifies.") {
                if event_name == RANGE_CERT_VERIFIES_FAILED_EVENT
                    && !stage5_script_fixture_keeps_failed_cert_event(&self.fixture)
                {
                    return;
                }
                if self
                    .cert_event_seen
                    .swap(true, std::sync::atomic::Ordering::SeqCst)
                {
                    return;
                }
            }

            let seq = self
                .event_seq
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                + 1;
            enrich_stage5_script_fields(
                &mut visitor.fields,
                &event_name,
                &self.build_id,
                &self.fixture,
                seq,
            );
            let span = ctx.lookup_current().map(|span| {
                serde_json::json!({
                    "name": span.name(),
                    "target": span.metadata().target(),
                })
            });
            let line = serde_json::json!({
                "ts": stage5_script_timestamp_string(),
                "event": event_name,
                "level": event.metadata().level().as_str(),
                "target": event.metadata().target(),
                "fields": Value::Object(visitor.fields),
                "span": span,
            });

            let mut writer = self.writer.lock().expect("ndjson writer lock");
            serde_json::to_writer(&mut *writer, &line).expect("ndjson event");
            writer.write_all(b"\n").expect("ndjson newline");
            writer.flush().expect("ndjson flush");
        }
    }

    #[derive(Default)]
    struct Stage5ScriptFieldVisitor {
        fields: Map<String, Value>,
    }

    impl Visit for Stage5ScriptFieldVisitor {
        fn record_bool(&mut self, field: &Field, value: bool) {
            self.fields
                .insert(field.name().to_owned(), Value::Bool(value));
        }

        fn record_i64(&mut self, field: &Field, value: i64) {
            self.fields
                .insert(field.name().to_owned(), Value::Number(value.into()));
        }

        fn record_u64(&mut self, field: &Field, value: u64) {
            self.fields
                .insert(field.name().to_owned(), Value::Number(value.into()));
        }

        fn record_str(&mut self, field: &Field, value: &str) {
            self.fields
                .insert(field.name().to_owned(), Value::String(value.to_owned()));
        }

        fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
            self.fields
                .insert(field.name().to_owned(), Value::String(format!("{value:?}")));
        }
    }

    fn enrich_stage5_script_fields(
        fields: &mut Map<String, Value>,
        event_name: &str,
        build_id: &str,
        fixture: &str,
        seq: u64,
    ) {
        let site_id = fields
            .get("site")
            .and_then(Value::as_str)
            .unwrap_or("dense.matmul.0")
            .to_owned();
        let k5_hash = fields
            .get("k5")
            .and_then(Value::as_str)
            .unwrap_or("not-applicable:stage5")
            .to_owned();
        fields
            .entry("site_id".to_owned())
            .or_insert_with(|| Value::String(site_id));
        fields
            .entry("checkpoint_id".to_owned())
            .or_insert_with(|| Value::String("not-applicable:stage5".to_owned()));
        fields
            .entry("compact_checkpoint_id".to_owned())
            .or_insert_with(|| Value::Number(0_u64.into()));
        fields
            .entry("stratum".to_owned())
            .or_insert_with(|| Value::String("not-applicable:stage5".to_owned()));
        fields
            .entry("probe_instance_id".to_owned())
            .or_insert_with(|| Value::String("not-applicable:stage5".to_owned()));
        fields
            .entry("runtime_probe_id".to_owned())
            .or_insert_with(|| Value::Number(0_u64.into()));
        fields
            .entry("importance_class".to_owned())
            .or_insert_with(|| Value::String("not-applicable:stage5".to_owned()));
        fields
            .entry("build_id".to_owned())
            .or_insert_with(|| Value::String(build_id.to_owned()));
        fields
            .entry("k4_hash".to_owned())
            .or_insert_with(|| Value::String("not-applicable:stage5".to_owned()));
        fields
            .entry("k5_hash".to_owned())
            .or_insert_with(|| Value::String(k5_hash));
        let packet_outcome = if event_name.contains("failed") {
            "failed"
        } else {
            "passed"
        };
        match fields.get("outcome").and_then(Value::as_str) {
            Some("passed" | "failed") => {}
            Some(detail) => {
                fields.insert(
                    "event_outcome_detail".to_owned(),
                    Value::String(detail.to_owned()),
                );
                fields.insert(
                    "outcome".to_owned(),
                    Value::String(packet_outcome.to_owned()),
                );
            }
            None => {
                fields.insert(
                    "outcome".to_owned(),
                    Value::String(packet_outcome.to_owned()),
                );
            }
        }
        fields
            .entry("diag_code".to_owned())
            .or_insert_with(|| Value::String("none".to_owned()));
        fields
            .entry("elapsed_ns".to_owned())
            .or_insert_with(|| Value::Number(seq.into()));
        fields
            .entry("event_seq".to_owned())
            .or_insert_with(|| Value::Number(seq.into()));
        fields
            .entry("fixture".to_owned())
            .or_insert_with(|| Value::String(fixture.to_owned()));
    }

    fn stage5_script_fixture_keeps_failed_cert_event(fixture: &str) -> bool {
        fixture.contains("failed") || fixture.contains("failure") || fixture.contains("tampered")
    }

    fn stage5_script_timestamp_string() -> String {
        let duration = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        format!("unix:{}.{:09}", duration.as_secs(), duration.subsec_nanos())
    }

    fn inputs_fixture() -> RangePlanInputs {
        RangePlanInputs {
            infer_ir_product: infer_ir_product_fixture(),
            infer_ir_self_hash: hash(0x10),
            quant_graph_self_hash: hash(0x11),
            static_budget_report: static_budget_report_fixture(),
            static_budget_self_hash: hash(0x12),
            range_policy_projection: policy_projection_fixture(),
            audit_parents: audit_parents_fixture(),
        }
    }

    fn single_i16_certificate(site: ReductionSiteId) -> AccumulatorCertificate {
        AccumulatorCertificate::SingleI16Proof {
            site,
            term_count: 8,
            per_term_abs_max: 256,
            sum_bound: 2_048,
            bias_abs_max: 4,
            total_abs_max: 2_052,
            i16_envelope: 32_767,
            slack: 30_715,
        }
    }

    fn certified_reduction(site_id: &str) -> CertifiedReduction {
        let site = site(site_id);
        CertifiedReduction {
            site: site.clone(),
            plan: ReductionPlan::SingleI16,
            facts: facts(site.clone()),
            proof: single_i16_certificate(site),
        }
    }

    fn range_cert_body_fixture() -> RangeCertBody {
        RangeCertBody {
            identity: RangeCertIdentity {
                range_plan_self_hash: Some(range_plan_self_hash(&plan_fixture()).expect("hash")),
                infer_ir_self_hash: hash(0x10),
                quant_graph_self_hash: hash(0x11),
                static_budget_self_hash: hash(0x12),
                determinism: DeterminismClass::BitExact,
            },
            cert_outcome: CertOutcome::Verified,
            certificates: vec![certified_reduction("classify")],
            site_to_certificate_index: BTreeMap::from([(site("classify"), 0)]),
            diagnostics: Vec::new(),
        }
    }

    fn range_cert_body_with_two_sites() -> RangeCertBody {
        RangeCertBody {
            certificates: vec![
                certified_reduction("classify"),
                certified_reduction("expert.1.2.down"),
            ],
            site_to_certificate_index: BTreeMap::from([
                (site("expert.1.2.down"), 1),
                (site("classify"), 0),
            ]),
            ..range_cert_body_fixture()
        }
    }

    fn failed_cert_body_fixture() -> RangeCertBody {
        let site_id = site("classify");
        RangeCertBody {
            identity: RangeCertIdentity {
                range_plan_self_hash: None,
                infer_ir_self_hash: hash(0x10),
                quant_graph_self_hash: hash(0x11),
                static_budget_self_hash: hash(0x12),
                determinism: DeterminismClass::BitExact,
            },
            cert_outcome: CertOutcome::Failed,
            certificates: vec![CertifiedReduction {
                site: site_id.clone(),
                plan: ReductionPlan::SingleI16,
                facts: facts(site_id.clone()),
                proof: AccumulatorCertificate::Failed {
                    site: site_id,
                    attempted_plan: ReductionPlan::SingleI16,
                    proof_state: AccumulatorProofState::SumExceedsI16Envelope {
                        sum_bound: 40_000,
                        envelope: 32_767,
                    },
                    witness: AccumulatorFailureWitness::BoundCalculation {
                        input_max_abs_q: 31,
                        weight_max_abs_q: 17,
                        term_count: 64,
                        bias: 3,
                    },
                },
            }],
            site_to_certificate_index: BTreeMap::from([(site("classify"), 0)]),
            diagnostics: Vec::new(),
        }
    }

    fn single_entry_plan_and_cert(
        plan: ReductionPlan,
        facts: ReductionSiteFacts,
        determinism: DeterminismClass,
    ) -> (RangePlan, RangeCertBody) {
        let proof = construct_accumulator_certificate(&plan, &facts, determinism);
        let range_plan = RangePlan {
            identity: RangePlanIdentity {
                determinism,
                ..plan_fixture().identity
            },
            entries: vec![RangePlanEntry {
                site: facts.site.clone(),
                plan: plan.clone(),
                site_facts: facts.clone(),
                effective_ceiling: ReductionPlanCeiling::Adaptive,
                ceiling_provenance: ReductionCeilingProvenance::Global {
                    source: PolicySource::ProfileDefault,
                },
            }],
            provenance: RangePlanProvenance {
                site_to_node: BTreeMap::new(),
                site_to_qg: BTreeMap::new(),
            },
        };
        let range_cert = RangeCertBody {
            identity: RangeCertIdentity {
                determinism,
                range_plan_self_hash: None,
                ..range_cert_body_fixture().identity
            },
            cert_outcome: if matches!(proof, AccumulatorCertificate::Failed { .. }) {
                CertOutcome::Failed
            } else {
                CertOutcome::Verified
            },
            certificates: vec![CertifiedReduction {
                site: facts.site.clone(),
                plan,
                facts: facts.clone(),
                proof,
            }],
            site_to_certificate_index: BTreeMap::from([(facts.site.clone(), 0)]),
            diagnostics: Vec::new(),
        };
        (range_plan, range_cert)
    }

    fn self_consistency_fields(
        range_plan: &RangePlan,
        range_cert: &RangeCertBody,
    ) -> BTreeSet<String> {
        self_consistency_diagnostics(range_plan, range_cert)
            .into_iter()
            .filter_map(|diagnostic| match diagnostic.detail {
                ValidationDetail::Field { field } => Some(field.as_str().to_owned()),
                _ => None,
            })
            .collect()
    }

    fn core_product_fixture() -> RangePlanCoreProduct {
        let range_plan = plan_fixture();
        let range_cert = range_cert_body_fixture();
        RangePlanCoreProduct {
            range_plan_self_hash: range_plan_self_hash(&range_plan).expect("plan hashes"),
            range_cert_body_hash: range_cert_body_hash(&range_cert).expect("cert hashes"),
            range_plan,
            range_cert,
        }
    }

    fn range_plan_report_result_fixture() -> RangePlanReportResult {
        let product = plan_fixture();
        let range_cert_report =
            ReportEnvelope::new(ReportOutcome::Passed, range_cert_body_fixture())
                .expect("range cert report envelope")
                .with_computed_self_hash()
                .expect("range cert report hashes");
        RangePlanReportResult {
            entry_count: product.entries.len() as u32,
            single_i16_count: 1,
            chunked_i16_count: 0,
            renorm_loop_count: 1,
            effective_ceiling_histogram: BTreeMap::from([
                (ReductionPlanCeiling::Adaptive, 1),
                (ReductionPlanCeiling::Conservative, 2),
                (ReductionPlanCeiling::ExactOnly, 0),
            ]),
            ceiling_provenance_histogram: BTreeMap::from([
                (ReductionCeilingProvenanceTag::Global, 1),
                (ReductionCeilingProvenanceTag::LayerOverride, 0),
                (ReductionCeilingProvenanceTag::SiteOverride, 1),
            ]),
            range_cert_report_self_hash: range_cert_report.report_self_hash,
            range_plan_self_hash: range_plan_self_hash(&product).expect("plan hashes"),
            product,
        }
    }

    fn range_plan_report_body_fixture() -> RangePlanReportBody {
        let inputs = inputs_fixture();
        let product = plan_fixture();
        RangePlanReportBody {
            input_identity: RangePlanReportInputIdentity::from_inputs(&inputs, &product.identity),
            result: Some(range_plan_report_result_fixture()),
            diagnostics: Vec::new(),
        }
    }

    fn canonical_bytes<T: Serialize>(value: &T) -> Vec<u8> {
        canonical_json_bytes(value).expect("canonical JSON computes")
    }

    fn round_trip<T>(value: &T) -> T
    where
        T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug,
    {
        let encoded = serde_json::to_vec(value).expect("value serializes");
        let decoded: T = serde_json::from_slice(&encoded).expect("value decodes");
        assert_eq!(&decoded, value);
        decoded
    }

    #[test]
    fn range_plan_inputs_serde_round_trip() {
        round_trip(&inputs_fixture());
    }

    #[test]
    fn run_stage5_single_i16_emits_plan_and_cert() {
        let dir = tempfile::tempdir().expect("tempdir");
        let output = run_stage5(
            single_i16_stage5_inputs(),
            Stage5PassEnvironment::new().with_report_dir(dir.path()),
        )
        .expect("Stage 5 succeeds");

        assert_eq!(output.report.outcome, ReportOutcome::Passed);
        assert_eq!(output.cert_report.outcome, ReportOutcome::Passed);
        assert_eq!(output.cert_report.body.cert_outcome, CertOutcome::Verified);
        assert!(matches!(
            output.product.range_plan.entries[0].plan,
            ReductionPlan::SingleI16
        ));
        assert!(dir.path().join("range_plan.json").exists());
        assert!(dir.path().join("certs/range.cert.json").exists());
    }

    #[test]
    fn run_stage5_chunked_i16_emits_plan_and_cert() {
        let output = run_stage5(chunked_i16_stage5_inputs(), Stage5PassEnvironment::new())
            .expect("Stage 5 succeeds");

        assert!(matches!(
            output.product.range_plan.entries[0].plan,
            ReductionPlan::ChunkedI16 { .. }
        ));
        gbf_report::round_trip_self_hash(&output.report).expect("range plan report round trips");
        gbf_report::round_trip_self_hash(&output.cert_report).expect("cert report round trips");
    }

    #[test]
    fn run_stage5_renorm_loop_non_bitexact_emits_plan_and_cert() {
        let output = run_stage5(renorm_loop_stage5_inputs(), Stage5PassEnvironment::new())
            .expect("Stage 5 succeeds");

        assert!(matches!(
            output.product.range_plan.entries[0].plan,
            ReductionPlan::RenormLoop { .. }
        ));
        assert_eq!(
            output.cert_report.body.identity.range_plan_self_hash,
            Some(output.product.range_plan_self_hash)
        );
    }

    #[test]
    fn run_stage5_ceiling_override_layer_then_site_resolved_correctly() {
        let inputs = ceiling_override_layer_site_stage5_inputs();

        let output = run_stage5(inputs, Stage5PassEnvironment::new()).expect("Stage 5 succeeds");

        assert!(matches!(
            output.product.range_plan.entries[0].plan,
            ReductionPlan::ChunkedI16 { .. }
        ));
        assert!(matches!(
            output.product.range_plan.entries[0].ceiling_provenance,
            ReductionCeilingProvenance::SiteOverride { .. }
        ));
    }

    #[test]
    fn stage5_script_harness_runs_real_driver_fixture() {
        let fixture =
            std::env::var("F_B6_F_B7_STAGE5_FIXTURE").unwrap_or_else(|_| "chunked_i16".to_owned());
        let build_id =
            std::env::var("F_B6_F_B7_BUILD_ID").unwrap_or_else(|_| "unit-stage5-script".to_owned());
        let out_dir = std::env::var("F_B6_F_B7_OUT_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| tempfile::tempdir().expect("out tempdir").keep());
        let report_dir = out_dir.join("reports").join("stage5").join(&fixture);
        let ndjson = out_dir.join("stage5-run.ndjson");
        fs::create_dir_all(&report_dir).expect("report dir");

        let inputs = match fixture.as_str() {
            "single_i16" => single_i16_stage5_inputs(),
            "chunked_i16" => chunked_i16_stage5_inputs(),
            "renorm_loop_non_bitexact" => renorm_loop_stage5_inputs(),
            "ceiling_override_layer_site" => ceiling_override_layer_site_stage5_inputs(),
            other => panic!("unknown Stage 5 fixture {other}"),
        };
        let store_dir = tempfile::tempdir().expect("store tempdir");
        let store = BlobStore::open(store_dir.path().to_path_buf()).expect("blob store");
        let cache = StageCache::new(&store);
        let subscriber = tracing_subscriber::registry()
            .with(LevelFilter::DEBUG)
            .with(Stage5ScriptNdjsonLayer::new(
                &ndjson,
                build_id,
                fixture.clone(),
            ));

        let output = tracing::subscriber::with_default(subscriber, || {
            run_stage5(
                inputs,
                Stage5PassEnvironment::new()
                    .with_report_dir(&report_dir)
                    .with_stage_cache(&cache),
            )
        })
        .expect("Stage 5 script fixture succeeds through real driver");

        let plan_path = report_dir.join("range_plan.json");
        let cert_path = report_dir.join("certs").join("range.cert.json");
        assert!(plan_path.exists(), "missing {}", plan_path.display());
        assert!(cert_path.exists(), "missing {}", cert_path.display());
        let plan_report: ReportEnvelope<RangePlanReportBody> =
            serde_json::from_slice(&fs::read(&plan_path).expect("range plan report"))
                .expect("range plan report parses and hashes");
        let cert_report: ReportEnvelope<RangeCertBody> =
            serde_json::from_slice(&fs::read(&cert_path).expect("range cert report"))
                .expect("range cert report parses and hashes");
        assert_eq!(plan_report.outcome, ReportOutcome::Passed);
        assert_eq!(cert_report.outcome, ReportOutcome::Passed);
        assert_eq!(cert_report.body.cert_outcome, CertOutcome::Verified);
        assert_eq!(
            cert_report.body.identity.range_plan_self_hash,
            Some(output.product.range_plan_self_hash)
        );
        assert!(ndjson.exists(), "missing {}", ndjson.display());
    }

    #[test]
    fn stage5_script_sink_preserves_failed_cert_event_with_typed_common_fields() {
        let dir = tempfile::tempdir().expect("ndjson tempdir");
        let path = dir.path().join("stage5-run.ndjson");
        let subscriber = tracing_subscriber::registry().with(Stage5ScriptNdjsonLayer::new(
            &path,
            "unit-failed-cert".to_owned(),
            "failed_cert".to_owned(),
        ));

        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(
                target: "gbf_verify::range_cert",
                event = RANGE_CERT_VERIFIES_FAILED_EVENT,
                site = "dense.matmul.0",
                proof_state = "SumExceedsI16Envelope",
            );
        });

        let line = fs::read_to_string(&path)
            .expect("ndjson reads")
            .lines()
            .next()
            .expect("failed cert event is not dropped")
            .to_owned();
        let payload: Value = serde_json::from_str(&line).expect("ndjson is JSON");

        assert_eq!(payload["event"], RANGE_CERT_VERIFIES_FAILED_EVENT);
        assert_eq!(payload["target"], "gbf_verify::range_cert");
        assert_eq!(payload["fields"]["outcome"], "failed");
        assert_eq!(payload["fields"]["compact_checkpoint_id"], 0);
        assert_eq!(payload["fields"]["runtime_probe_id"], 0);
        assert_eq!(payload["fields"]["event_seq"], 1);
    }

    #[test]
    fn rp_pre_1_infer_ir_self_hash_mismatch() {
        let mut inputs = single_i16_stage5_inputs();
        inputs.infer_ir_self_hash = hash(0xee);

        let failure = expect_stage5_failure(run_stage5(inputs, Stage5PassEnvironment::new()));

        assert!(matches!(
            failure.diagnostics[0].code,
            ValidationCode::SemanticCoreHashMismatch
        ));
        assert!(failure.cert_report.is_none());
    }

    #[test]
    fn rp_pre_2_quant_graph_self_hash_mismatch() {
        let mut inputs = single_i16_stage5_inputs();
        inputs.quant_graph_self_hash = hash(0xef);

        let failure = expect_stage5_failure(run_stage5(inputs, Stage5PassEnvironment::new()));

        assert!(matches!(
            failure.diagnostics[0].code,
            ValidationCode::SemanticCoreHashMismatch
        ));
        assert!(failure.cert_report.is_none());
    }

    #[test]
    fn rp_pre_3_static_budget_not_passing() {
        let mut inputs = single_i16_stage5_inputs();
        inputs.static_budget_report = static_budget_report_fixture();
        inputs.static_budget_self_hash = inputs.static_budget_report.static_budget_self_hash;

        let failure = expect_stage5_failure(run_stage5(inputs, Stage5PassEnvironment::new()));

        assert_eq!(failure.diagnostics[0].provenance[0].reference, "RP-Pre-3");
        assert!(failure.cert_report.is_none());
    }

    #[test]
    fn rp_pre_3_static_budget_self_hash_mismatch() {
        let mut inputs = single_i16_stage5_inputs();
        inputs.static_budget_self_hash = hash(0xef);

        let failure = expect_stage5_failure(run_stage5(inputs, Stage5PassEnvironment::new()));

        assert!(matches!(
            failure.diagnostics[0].code,
            ValidationCode::SemanticCoreHashMismatch
        ));
        assert!(matches!(
            failure.diagnostics[0].detail,
            ValidationDetail::HashMismatch { .. }
        ));
        assert!(failure.cert_report.is_none());
    }

    #[test]
    fn rp_pre_4_determinism_class_mismatch() {
        let mut inputs = single_i16_stage5_inputs();
        inputs.range_policy_projection.determinism_class = DeterminismClass::Deterministic;

        let failure = expect_stage5_failure(run_stage5(inputs, Stage5PassEnvironment::new()));

        assert_eq!(
            failure.diagnostics[0].provenance[0].reference,
            RANGE_DETERMINISM_MISMATCH_CODE
        );
        assert_eq!(
            failure.diagnostics[0].origin,
            ValidationOrigin::RangePlanConstruction
        );
    }

    #[test]
    fn rp_pre_5_reduction_ceiling_invalid_impossible_for_closed_enum() {
        for ceiling in [
            ReductionPlanCeiling::ExactOnly,
            ReductionPlanCeiling::Conservative,
            ReductionPlanCeiling::Adaptive,
        ] {
            assert!(reduction_ceiling_is_valid(ceiling));
        }
    }

    #[test]
    fn rp_pre_6_override_selector_invalid() {
        let mut inputs = single_i16_stage5_inputs();
        inputs
            .range_policy_projection
            .reduction_ceiling_overrides
            .insert(
                ReductionSelector::from("layer:99"),
                ReductionPlanCeiling::Adaptive,
            );

        let failure = expect_stage5_failure(run_stage5(inputs, Stage5PassEnvironment::new()));

        assert_eq!(
            failure.diagnostics[0].provenance[0].reference,
            RANGE_CEILING_OVERRIDE_INVALID_SELECTOR_CODE
        );
    }

    #[test]
    fn rp_pre_7_duplicate_reduction_site_id_in_g_rejected() {
        let inputs = stage5_inputs_for(
            vec![
                range_node(1, Some("dup.site"), crate::s3::infer_ir::InferOp::Classify),
                range_node(2, Some("dup.site"), crate::s3::infer_ir::InferOp::Classify),
            ],
            vec![projection_with_shape(
                "dup.site",
                Some(LayerId::new(0)),
                4,
                32,
                16,
                None,
            )],
            DeterminismClass::BitExact,
            ReductionPlanCeiling::ExactOnly,
            RangeCapsSpec::default_v2(),
            BTreeMap::new(),
        );

        let failure = expect_stage5_failure(run_stage5(inputs, Stage5PassEnvironment::new()));

        assert_eq!(
            failure.diagnostics[0].provenance[0].reference,
            RANGE_DUPLICATE_REDUCTION_SITE_ID_CODE
        );
    }

    #[test]
    fn failed_cert_emitted_when_at_least_one_attempt_made() {
        let dir = tempfile::tempdir().expect("tempdir");
        let failure = expect_stage5_failure(run_stage5(
            failed_cert_stage5_inputs(),
            Stage5PassEnvironment::new().with_report_dir(dir.path()),
        ));

        let cert_report = failure.cert_report.expect("cert emitted");
        assert_eq!(failure.report.outcome, ReportOutcome::Failed);
        assert_eq!(cert_report.outcome, ReportOutcome::Failed);
        assert_eq!(cert_report.body.cert_outcome, CertOutcome::Failed);
        assert!(matches!(
            cert_report.body.certificates[0].proof,
            AccumulatorCertificate::Failed { .. }
        ));
        assert!(dir.path().join("range_plan.json").exists());
        assert!(dir.path().join("certs/range.cert.json").exists());
    }

    #[test]
    fn early_failure_no_cert_emitted() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut inputs = single_i16_stage5_inputs();
        inputs.infer_ir_self_hash = hash(0xee);

        let failure = expect_stage5_failure(run_stage5(
            inputs,
            Stage5PassEnvironment::new().with_report_dir(dir.path()),
        ));

        assert!(failure.cert_report.is_none());
        assert!(dir.path().join("range_plan.json").exists());
        assert!(!dir.path().join("certs/range.cert.json").exists());
    }

    #[test]
    fn failed_cert_outcome_failed_in_body() {
        let failure = build_range_plan_core(&failed_cert_stage5_inputs())
            .expect_err("failing proof yields core failure");
        let cert = failure.range_cert_body.expect("cert body exists");

        assert_eq!(cert.cert_outcome, CertOutcome::Failed);
        assert!(
            cert.certificates
                .iter()
                .any(|certified| matches!(certified.proof, AccumulatorCertificate::Failed { .. }))
        );
    }

    #[test]
    fn failed_cert_outcome_failed_when_hard_diagnostic_in_body() {
        let mut cert = range_cert_body_fixture();
        cert.diagnostics = vec![plan_choice_diagnostic(
            &site("site.hard"),
            RANGE_NO_PROVEN_PLAN_WITHIN_CEILING_CODE,
        )];
        cert.cert_outcome = range_cert_outcome(&cert.certificates, &cert.diagnostics);

        assert_eq!(cert.cert_outcome, CertOutcome::Failed);
        assert!(cert.validate_semantics(ReportOutcome::Failed).is_ok());
    }

    #[test]
    fn run_stage5_second_run_same_inputs_cache_hit() {
        let store_dir = tempfile::tempdir().expect("store tempdir");
        let store = BlobStore::open(store_dir.path().to_path_buf()).expect("blob store");
        let cache = StageCache::new(&store);
        let inputs = chunked_i16_stage5_inputs();

        let first = run_stage5(
            inputs.clone(),
            Stage5PassEnvironment::new().with_stage_cache(&cache),
        )
        .expect("first run succeeds");
        let second = run_stage5(
            inputs,
            Stage5PassEnvironment::new().with_stage_cache(&cache),
        )
        .expect("second run succeeds");

        assert_eq!(second.product, first.product);
        assert_eq!(
            canonical_bytes(&second.product),
            canonical_bytes(&first.product)
        );
    }

    #[test]
    fn run_stage5_audit_parent_drift_cache_hit_with_rewrap() {
        let store_dir = tempfile::tempdir().expect("store tempdir");
        let store = BlobStore::open(store_dir.path().to_path_buf()).expect("blob store");
        let cache = StageCache::new(&store);
        let first_inputs = chunked_i16_stage5_inputs();
        let mut second_inputs = first_inputs.clone();
        second_inputs.audit_parents.compile_request_hash = hash(0xfa);

        let first = run_stage5(
            first_inputs,
            Stage5PassEnvironment::new().with_stage_cache(&cache),
        )
        .expect("first run succeeds");
        let second = run_stage5(
            second_inputs.clone(),
            Stage5PassEnvironment::new().with_stage_cache(&cache),
        )
        .expect("second run succeeds from cache");

        assert_eq!(second.product, first.product);
        assert_ne!(
            second.report.report_self_hash,
            first.report.report_self_hash
        );
        assert_eq!(
            second.report.body.input_identity.compile_request_hash,
            second_inputs.audit_parents.compile_request_hash
        );
    }

    #[test]
    fn run_stage5_failed_memo_replay_refreshes_audit_parents() {
        let store_dir = tempfile::tempdir().expect("store tempdir");
        let store = BlobStore::open(store_dir.path().to_path_buf()).expect("blob store");
        let cache = StageCache::new(&store);
        let first_inputs = failed_cert_stage5_inputs();
        let mut second_inputs = first_inputs.clone();
        second_inputs.audit_parents.compile_request_hash = hash(0xfb);

        let first = expect_stage5_failure(run_stage5(
            first_inputs,
            Stage5PassEnvironment::new().with_stage_cache(&cache),
        ));
        let second = expect_stage5_failure(run_stage5(
            second_inputs.clone(),
            Stage5PassEnvironment::new().with_stage_cache(&cache),
        ));

        assert_ne!(
            second.report.report_self_hash,
            first.report.report_self_hash
        );
        assert_eq!(
            second.report.body.input_identity.compile_request_hash,
            second_inputs.audit_parents.compile_request_hash
        );
        assert_eq!(
            second.cert_report.as_ref().map(|report| &report.body),
            first.cert_report.as_ref().map(|report| &report.body)
        );
    }

    #[test]
    fn success_emits_both_reports_atomically() {
        let dir = tempfile::tempdir().expect("tempdir");

        run_stage5(
            chunked_i16_stage5_inputs(),
            Stage5PassEnvironment::new().with_report_dir(dir.path()),
        )
        .expect("Stage 5 succeeds");

        assert!(dir.path().join("range_plan.json").exists());
        assert!(dir.path().join("certs/range.cert.json").exists());
        assert!(!dir.path().join("range_plan.json.tmp").exists());
        assert!(!dir.path().join("certs/range.cert.json.tmp").exists());
    }

    #[test]
    fn failure_path_emits_range_plan_json_failed() {
        let dir = tempfile::tempdir().expect("tempdir");

        let failure = expect_stage5_failure(run_stage5(
            failed_cert_stage5_inputs(),
            Stage5PassEnvironment::new().with_report_dir(dir.path()),
        ));

        assert_eq!(failure.report.outcome, ReportOutcome::Failed);
        assert!(dir.path().join("range_plan.json").exists());
    }

    #[test]
    fn r_no_partial_product_cert_rule() {
        let failure = expect_stage5_failure(run_stage5(
            failed_cert_stage5_inputs(),
            Stage5PassEnvironment::new(),
        ));

        assert!(failure.report.body.result.is_none());
        assert_eq!(failure.report.outcome, ReportOutcome::Failed);
        assert_eq!(
            failure
                .cert_report
                .as_ref()
                .expect("failed cert report exists")
                .body
                .identity
                .range_plan_self_hash,
            None
        );
    }

    #[test]
    fn range_policy_projection_hash_deterministic() {
        let first = range_policy_projection_hash(&policy_projection_fixture()).expect("first hash");
        let second =
            range_policy_projection_hash(&policy_projection_fixture()).expect("second hash");

        assert_eq!(first, second);
        assert_eq!(
            first,
            expected_codegen_domain_hash(
                "RangePolicyProjection",
                RANGE_PLAN_SCHEMA_VERSION,
                &policy_projection_fixture()
            )
        );
    }

    #[test]
    fn range_policy_projection_hash_changes_per_field() {
        let base = policy_projection_fixture();
        let base_hash = range_policy_projection_hash(&base).expect("base hash");

        let mut changed_profile = base.clone();
        changed_profile.profile_id = CompileProfileId::from("Trace");
        let mut changed_caps = base.clone();
        changed_caps.range_caps.profile_chunk_max = 32;
        let mut changed_ceiling = base.clone();
        changed_ceiling.reduction_ceiling = ReductionPlanCeiling::Conservative;
        let mut changed_overrides = base.clone();
        changed_overrides.reduction_ceiling_overrides.insert(
            ReductionSelector::from("layer:1"),
            ReductionPlanCeiling::ExactOnly,
        );
        let mut changed_determinism = base.clone();
        changed_determinism.determinism_class = DeterminismClass::Deterministic;

        for projection in [
            changed_profile,
            changed_caps,
            changed_ceiling,
            changed_overrides,
            changed_determinism,
        ] {
            assert_ne!(
                range_policy_projection_hash(&projection).expect("changed hash"),
                base_hash
            );
        }
    }

    #[test]
    fn identity_binding_binds_hashes_and_ir_determinism() {
        let inputs = inputs_fixture();
        let identity = bind_range_plan_identity(&inputs).expect("identity binds");

        assert_eq!(identity.infer_ir_self_hash, inputs.infer_ir_self_hash);
        assert_eq!(identity.quant_graph_self_hash, inputs.quant_graph_self_hash);
        assert_eq!(
            identity.static_budget_self_hash,
            inputs.static_budget_self_hash
        );
        assert_eq!(
            identity.range_policy_projection_hash,
            range_policy_projection_hash(&inputs.range_policy_projection).expect("hash")
        );
        assert_eq!(
            identity.determinism,
            inputs.infer_ir_product.infer_ir.identity.determinism
        );
        assert!(take_recorded_range_construction_events().contains(&RANGE_IDENTITY_BIND_EVENT));
    }

    #[test]
    fn identity_binding_rejects_rp_pre_4_determinism_mismatch() {
        let mut inputs = inputs_fixture();
        inputs.range_policy_projection.determinism_class = DeterminismClass::Deterministic;

        let err = bind_range_plan_identity(&inputs).expect_err("determinism mismatch rejects");

        assert_eq!(err.code(), RANGE_DETERMINISM_MISMATCH_CODE);
    }

    #[test]
    fn reduction_site_enumeration_unique() {
        let ir = range_ir_with_nodes(vec![
            range_node(7, Some("site.b"), crate::s3::infer_ir::InferOp::Classify),
            range_node(
                3,
                None,
                crate::s3::infer_ir::InferOp::Embedding {
                    token_input: TokenInputId::new(0),
                },
            ),
            range_node(
                9,
                Some("site.a"),
                crate::s3::infer_ir::InferOp::ExpertMatVec {
                    layer: LayerId::new(3),
                    expert: ExpertId::new(5),
                    slot: ExpertWeightSlot::FfnDown,
                },
            ),
        ]);

        let sites = enumerate_reduction_sites(&ir).expect("sites enumerate");

        assert_eq!(sites.len(), 2);
        assert_eq!(sites[0].site, site("site.a"));
        assert_eq!(sites[1].site, site("site.b"));
        assert_eq!(sites[0].node_id, NodeId::new(9));
        assert_eq!(sites[1].node_id, NodeId::new(7));
        assert!(
            take_recorded_range_construction_events()
                .contains(&RANGE_REDUCTION_SITE_ENUMERATION_EVENT)
        );
    }

    #[test]
    fn reduction_site_enumeration_canonical_string_order() {
        let ir = range_ir_with_nodes(vec![
            range_node(1, Some("z.last"), crate::s3::infer_ir::InferOp::Classify),
            range_node(2, Some("a.first"), crate::s3::infer_ir::InferOp::Classify),
            range_node(3, Some("m.middle"), crate::s3::infer_ir::InferOp::Classify),
        ]);

        let observed = enumerate_reduction_sites(&ir)
            .expect("sites enumerate")
            .into_iter()
            .map(|binding| binding.site)
            .collect::<Vec<_>>();

        assert_eq!(
            observed,
            vec![site("a.first"), site("m.middle"), site("z.last")]
        );
    }

    #[test]
    fn reduction_site_enumeration_derives_expert_slot_and_non_expert_none() {
        let ir = range_ir_with_nodes(vec![
            range_node(
                1,
                Some("site.classify"),
                crate::s3::infer_ir::InferOp::Classify,
            ),
            range_node(
                2,
                Some("site.expert"),
                crate::s3::infer_ir::InferOp::ExpertMatVec {
                    layer: LayerId::new(4),
                    expert: ExpertId::new(6),
                    slot: ExpertWeightSlot::FfnGate,
                },
            ),
        ]);

        let bindings = enumerate_reduction_sites(&ir).expect("sites enumerate");
        let classify = bindings
            .iter()
            .find(|binding| binding.site == site("site.classify"))
            .expect("classify binding");
        let expert = bindings
            .iter()
            .find(|binding| binding.site == site("site.expert"))
            .expect("expert binding");

        assert_eq!(classify.slot, None);
        assert_eq!(expert.slot, Some(ExpertWeightSlot::FfnGate));
    }

    #[test]
    fn norm_site_provenance_flows_from_enumeration_into_bound_facts() {
        let ir = range_ir_with_nodes(vec![range_node(
            1,
            Some("site.norm"),
            crate::s3::infer_ir::InferOp::Norm {
                plan: crate::s1::quant_graph::NormPlanId::new(0),
            },
        )]);
        let expected_norm_site = Some(NormSite::LayerFfn {
            layer: LayerId::new(0),
        });

        let bindings = enumerate_reduction_sites(&ir).expect("sites enumerate");
        assert_eq!(bindings[0].norm_site, expected_norm_site);

        let bound = bind_reduction_site_facts(
            &bindings,
            &RecordingStaticBudgetFacts::new(vec![projection("site.norm", None)]),
            vec![site("site.norm")],
        )
        .expect("facts bind");

        assert_eq!(bound[0].facts.norm_site, expected_norm_site);
    }

    #[test]
    fn duplicate_reduction_site_id_rejected() {
        let ir = range_ir_with_nodes(vec![
            range_node(1, Some("site.dup"), crate::s3::infer_ir::InferOp::Classify),
            range_node(2, Some("site.dup"), crate::s3::infer_ir::InferOp::Classify),
        ]);

        let err = enumerate_reduction_sites(&ir).expect_err("duplicate rejects");

        assert_eq!(err.code(), RANGE_DUPLICATE_REDUCTION_SITE_ID_CODE);
    }

    #[test]
    fn site_facts_binding_reads_via_trait() {
        let facts_source = RecordingStaticBudgetFacts::new(vec![projection("site.trait", None)]);
        let sites = vec![binding("site.trait", 5)];

        let bound = bind_reduction_site_facts(&sites, &facts_source, vec![site("site.trait")])
            .expect("facts bind");

        assert_eq!(facts_source.calls.get(), 1);
        assert_eq!(bound[0].facts.site, site("site.trait"));
    }

    #[test]
    fn site_facts_binding_copies_field_for_field() {
        let source_projection = projection("site.copy", Some(23));
        let facts_source = RecordingStaticBudgetFacts::new(vec![source_projection.clone()]);
        let sites = vec![binding("site.copy", 5)];

        let bound = bind_reduction_site_facts(&sites, &facts_source, vec![site("site.copy")])
            .expect("facts bind");
        let facts = &bound[0].facts;

        assert_eq!(facts.site, source_projection.site);
        assert_eq!(facts.layer, source_projection.layer);
        assert_eq!(facts.expert, source_projection.expert);
        assert_eq!(facts.term_count, source_projection.term_count);
        assert_eq!(facts.input_max_abs_q, source_projection.input_max_abs_q);
        assert_eq!(facts.weight_max_abs_q, source_projection.weight_max_abs_q);
        assert_eq!(facts.bias_max_abs_q, source_projection.bias_max_abs_q);
        assert_eq!(
            facts.accumulator_domain,
            source_projection.accumulator_domain
        );
        assert_eq!(
            facts.per_term_abs_max_q,
            u64::from(source_projection.input_max_abs_q)
                * u64::from(source_projection.weight_max_abs_q)
        );
        assert_eq!(facts.slot, Some(ExpertWeightSlot::FfnDown));
        assert_eq!(facts.op_tag, InferOpTag::ExpertMatVec);
    }

    #[test]
    fn site_facts_with_bias_none_yields_bias_max_abs_q_none() {
        let facts = reduction_site_facts_from_projection_with_published(
            &binding("site.bias.none", 1),
            &projection("site.bias.none", None),
            None,
        )
        .expect("facts bind");

        assert_eq!(facts.bias_max_abs_q, None);
    }

    #[test]
    fn site_facts_with_bias_some_zero_yields_bias_max_abs_q_some_zero() {
        let facts = reduction_site_facts_from_projection_with_published(
            &binding("site.bias.zero", 1),
            &projection("site.bias.zero", Some(0)),
            None,
        )
        .expect("facts bind");

        assert_eq!(facts.bias_max_abs_q, Some(0));
    }

    #[test]
    fn site_facts_with_bias_some_nonzero_yields_same() {
        let facts = reduction_site_facts_from_projection_with_published(
            &binding("site.bias.nonzero", 1),
            &projection("site.bias.nonzero", Some(19)),
            None,
        )
        .expect("facts bind");

        assert_eq!(facts.bias_max_abs_q, Some(19));
    }

    #[test]
    fn per_term_abs_max_q_computed_via_checked_u128() {
        for (input, weight) in [(0_u128, 99_u128), (1, 1), (13, 17), (u32::MAX as u128, 7)] {
            let expected = u64::try_from(input * weight).expect("test product fits");
            assert_eq!(
                checked_per_term_abs_max_q(input, weight, None).expect("product computes"),
                expected
            );
        }
    }

    #[test]
    fn per_term_abs_max_q_overflow_path() {
        assert_eq!(
            checked_per_term_abs_max_q(u128::MAX, 2, Some(42))
                .expect("published value wins on overflow"),
            42
        );
        assert_eq!(
            checked_per_term_abs_max_q(u128::MAX, 2, None).expect_err("overflow rejects"),
            PerTermAbsMaxError::Overflow
        );
    }

    #[test]
    fn term_count_zero_rejected() {
        let mut source_projection = projection("site.zero_terms", None);
        source_projection.term_count = 0;

        let err = reduction_site_facts_from_projection_with_published(
            &binding("site.zero_terms", 1),
            &source_projection,
            None,
        )
        .expect_err("zero terms reject");

        assert_eq!(err.code(), RANGE_TERM_COUNT_ZERO_CODE);
    }

    #[test]
    fn accumulator_domain_unsupported_v1_rejected() {
        for accumulator_domain in [
            AccumulatorDomain::PostScaleQ8_8,
            AccumulatorDomain::PostScaleQ16_16,
        ] {
            let mut source_projection = projection("site.domain", None);
            source_projection.accumulator_domain = accumulator_domain;

            let err = reduction_site_facts_from_projection_with_published(
                &binding("site.domain", 1),
                &source_projection,
                None,
            )
            .expect_err("non-raw domain rejects");

            assert_eq!(err.code(), RANGE_ACCUMULATOR_DOMAIN_UNSUPPORTED_V1_CODE);
        }
    }

    #[test]
    fn site_missing_from_static_budget_rejected() {
        let facts_source = RecordingStaticBudgetFacts::new(Vec::new());
        let err =
            bind_reduction_site_facts(&[binding("site.missing", 1)], &facts_source, Vec::new())
                .expect_err("missing site rejects");

        assert_eq!(err.code(), RANGE_SITE_MISSING_FROM_STATIC_BUDGET_CODE);
    }

    #[test]
    fn static_budget_site_orphan_rejected() {
        let facts_source = RecordingStaticBudgetFacts::new(vec![projection("site.orphan", None)]);
        let err = bind_reduction_site_facts(&[], &facts_source, vec![site("site.orphan")])
            .expect_err("orphan site rejects");

        assert_eq!(err.code(), RANGE_STATIC_BUDGET_SITE_ORPHANED_CODE);
        assert_eq!(facts_source.calls.get(), 0);
    }

    #[test]
    fn site_facts_inconsistent_optional_maxima_rejected() {
        let err = reduction_site_facts_from_projection_with_published(
            &binding("site.inconsistent", 1),
            &projection("site.inconsistent", None),
            Some(999),
        )
        .expect_err("inconsistent published max rejects");

        assert_eq!(err.code(), RANGE_SITE_FACTS_INCONSISTENT_CODE);
    }

    #[test]
    fn range_plan_audit_parents_carry_locked_knobs() {
        fn accepts_locked_knobs(parents: RangePlanAuditParents) -> LockedRangeKnobs {
            parents.locked_range_knobs
        }

        let locked = accepts_locked_knobs(audit_parents_fixture());
        let projection = serde_json::to_value(policy_projection_fixture()).expect("projection");

        assert!(locked.reduction_ceiling_locked);
        assert!(projection.get("locked_range_knobs").is_none());
    }

    #[test]
    fn range_caps_spec_round_trip_both_renorm_policy_variants() {
        for caps in [
            RangeCapsSpec::default_v2(),
            RangeCapsSpec {
                renorm_strategy: RenormStrategyPolicy::DynamicMargin {
                    margin_q16_16: 0x0000_4000,
                },
                ..RangeCapsSpec::default_v2()
            },
        ] {
            round_trip(&caps);
        }
    }

    #[test]
    fn range_caps_spec_invariants_validated_at_load() {
        fn assert_invalid(source: String, expected: &'static str) {
            match load_compile_profile_spec(&source) {
                Err(CompileProfileSpecLoadError::InvalidInvariant { invariant, .. }) => {
                    assert_eq!(invariant, expected);
                }
                other => panic!("expected invariant {expected}, got {other:?}"),
            }
        }

        let source = BRINGUP_COMPILE_PROFILE_TOML;
        assert_invalid(
            source.replace("profile_chunk_max = 256", "profile_chunk_max = 0"),
            "profile_chunk_max > 0",
        );
        assert_invalid(
            source.replace("profile_tile_max = 256", "profile_tile_max = 0"),
            "profile_tile_max > 0",
        );
        assert_invalid(
            source.replace("profile_tile_min = 16", "profile_tile_min = 0"),
            "profile_tile_min > 0",
        );
        assert_invalid(
            source.replace("profile_tile_max = 256", "profile_tile_max = 8"),
            "profile_tile_min <= profile_tile_max",
        );
        assert_invalid(
            source.replace(
                "renorm_strategy = { kind = \"ExactPostBoundaryOnly\" }",
                "renorm_strategy = { kind = \"DynamicMargin\", margin_q16_16 = 65536 }",
            ),
            "DynamicMargin.margin_q16_16 < 0x1_0000",
        );
    }

    fn caps(
        profile_chunk_max: u16,
        profile_tile_min: u16,
        profile_tile_max: u16,
        renorm_strategy: RenormStrategyPolicy,
    ) -> RangeCapsSpec {
        RangeCapsSpec {
            profile_chunk_max,
            profile_tile_min,
            profile_tile_max,
            renorm_strategy,
        }
    }

    fn facts_with(site_id: &str, term_count: u32, per_term_abs_max_q: u64) -> ReductionSiteFacts {
        ReductionSiteFacts {
            term_count,
            per_term_abs_max_q,
            ..facts(site(site_id))
        }
    }

    fn facts_with_bias(
        site_id: &str,
        term_count: u32,
        per_term_abs_max_q: u64,
        bias_max_abs_q: Option<u32>,
    ) -> ReductionSiteFacts {
        ReductionSiteFacts {
            bias_max_abs_q,
            ..facts_with(site_id, term_count, per_term_abs_max_q)
        }
    }

    fn renorm_recurrence_for(
        rounding: RenormRounding,
        output_scale_q16_16: u32,
        max_rounding_error_q16_16: u32,
    ) -> RenormRecurrence {
        RenormRecurrence {
            input_scale_q16_16: 0x0001_0000,
            output_scale_q16_16,
            rounding,
            saturation: RenormSaturationPolicy::AtNamedNumericBoundary {
                boundary: NamedNumericBoundary::FinalClamp,
            },
            max_rounding_error_q16_16,
        }
    }

    fn damping_renorm_spec() -> RenormSpec {
        RenormSpec {
            strategy: RenormStrategy::DynamicMargin {
                margin_q16_16: 0x0000_4000,
            },
            recurrence: renorm_recurrence_for(RenormRounding::NearestEven, 0x0010_0000, 1),
        }
    }

    fn bound_facts(site_id: &str, layer: Option<LayerId>) -> BoundReductionSiteFacts {
        let mut site_facts = facts(site(site_id));
        site_facts.layer = layer;
        BoundReductionSiteFacts {
            binding: binding(site_id, 1),
            facts: site_facts,
        }
    }

    fn projection_with_overrides(
        global: ReductionPlanCeiling,
        overrides: &[(&str, ReductionPlanCeiling)],
    ) -> RangePolicyProjection {
        RangePolicyProjection {
            reduction_ceiling: global,
            reduction_ceiling_overrides: overrides
                .iter()
                .map(|(selector, ceiling)| (ReductionSelector::from(*selector), *ceiling))
                .collect(),
            ..policy_projection_fixture()
        }
    }

    #[test]
    fn effective_ceiling_global_when_no_override() {
        let bound = vec![bound_facts("site.global", Some(LayerId::new(1)))];
        let projection = projection_with_overrides(ReductionPlanCeiling::Conservative, &[]);

        let ceilings = bind_effective_ceilings(&bound, &projection).expect("ceilings bind");

        assert_eq!(
            ceilings[0].effective_ceiling,
            ReductionPlanCeiling::Conservative
        );
        assert!(matches!(
            ceilings[0].provenance,
            ReductionCeilingProvenance::Global {
                source: PolicySource::ProfileDefault
            }
        ));
    }

    #[test]
    fn effective_ceiling_layer_override_wins_over_global() {
        let bound = vec![bound_facts("site.layer", Some(LayerId::new(7)))];
        let projection = projection_with_overrides(
            ReductionPlanCeiling::ExactOnly,
            &[("layer:7", ReductionPlanCeiling::Adaptive)],
        );

        let ceilings = bind_effective_ceilings(&bound, &projection).expect("ceilings bind");

        assert_eq!(
            ceilings[0].effective_ceiling,
            ReductionPlanCeiling::Adaptive
        );
        assert!(matches!(
            ceilings[0].provenance,
            ReductionCeilingProvenance::LayerOverride {
                layer,
                source: PolicySource::CompileRequestOverride
            } if layer == LayerId::new(7)
        ));
    }

    #[test]
    fn effective_ceiling_site_override_wins_over_layer_and_global() {
        let bound = vec![bound_facts("site.specific", Some(LayerId::new(7)))];
        let projection = projection_with_overrides(
            ReductionPlanCeiling::ExactOnly,
            &[
                ("layer:7", ReductionPlanCeiling::Conservative),
                ("site:site.specific", ReductionPlanCeiling::Adaptive),
            ],
        );

        let ceilings = bind_effective_ceilings(&bound, &projection).expect("ceilings bind");

        assert_eq!(
            ceilings[0].effective_ceiling,
            ReductionPlanCeiling::Adaptive
        );
        assert!(matches!(
            ceilings[0].provenance,
            ReductionCeilingProvenance::SiteOverride {
                site: ref provenance_site,
                source: PolicySource::CompileRequestOverride
            } if provenance_site == &site("site.specific")
        ));
    }

    #[test]
    fn effective_ceiling_provenance_carries_policy_source() {
        let bound = vec![bound_facts("site.source", Some(LayerId::new(2)))];
        let projection = projection_with_overrides(
            ReductionPlanCeiling::Adaptive,
            &[("layer:2", ReductionPlanCeiling::Conservative)],
        );

        let ceilings = bind_effective_ceilings(&bound, &projection).expect("ceilings bind");

        assert_eq!(
            ReductionCeilingProvenanceTag::from(&ceilings[0].provenance),
            ReductionCeilingProvenanceTag::LayerOverride
        );
        assert!(matches!(
            ceilings[0].provenance,
            ReductionCeilingProvenance::LayerOverride {
                source: PolicySource::CompileRequestOverride,
                ..
            }
        ));
        assert!(
            take_recorded_range_construction_events()
                .contains(&RANGE_EFFECTIVE_CEILING_BINDING_EVENT)
        );
    }

    #[test]
    fn effective_ceiling_invalid_selector_rejected() {
        let bound = vec![bound_facts("site.valid", Some(LayerId::new(1)))];
        let projection = projection_with_overrides(
            ReductionPlanCeiling::Adaptive,
            &[("layer:9", ReductionPlanCeiling::ExactOnly)],
        );

        let err = bind_effective_ceilings(&bound, &projection).expect_err("invalid selector");

        assert_eq!(err.code(), RANGE_CEILING_OVERRIDE_INVALID_SELECTOR_CODE);
    }

    #[test]
    fn effective_ceiling_ambiguous_overrides_rejected() {
        let bound = vec![bound_facts("site.ambiguous", Some(LayerId::new(1)))];
        let projection = projection_with_overrides(
            ReductionPlanCeiling::Adaptive,
            &[
                ("site.ambiguous", ReductionPlanCeiling::ExactOnly),
                ("site:site.ambiguous", ReductionPlanCeiling::Conservative),
            ],
        );

        let err = bind_effective_ceilings(&bound, &projection).expect_err("ambiguous selector");

        assert_eq!(err.code(), RANGE_CEILING_OVERRIDE_AMBIGUOUS_CODE);
    }

    #[test]
    fn unprefixed_reduction_selector_is_site_selector() {
        let bound = vec![bound_facts("site.unprefixed", Some(LayerId::new(1)))];
        let projection = projection_with_overrides(
            ReductionPlanCeiling::ExactOnly,
            &[("site.unprefixed", ReductionPlanCeiling::Adaptive)],
        );

        let ceilings = bind_effective_ceilings(&bound, &projection).expect("ceilings bind");

        assert_eq!(
            ceilings[0].effective_ceiling,
            ReductionPlanCeiling::Adaptive
        );
        assert!(matches!(
            ceilings[0].provenance,
            ReductionCeilingProvenance::SiteOverride {
                site: ref provenance_site,
                ..
            } if provenance_site == &site("site.unprefixed")
        ));
    }

    #[test]
    fn effective_ceiling_duplicate_canonical_map_key_rejected_at_deserialize() {
        let encoded = r#"{
            "profile_id": "Bringup",
            "range_caps": {
                "profile_chunk_max": 64,
                "profile_tile_max": 128,
                "profile_tile_min": 8,
                "renorm_strategy": {
                    "kind": "ExactPostBoundaryOnly"
                }
            },
            "reduction_ceiling": {
                "kind": "Adaptive"
            },
            "reduction_ceiling_overrides": {
                "site:classify": {
                    "kind": "ExactOnly"
                },
                "site:classify": {
                    "kind": "Adaptive"
                }
            },
            "determinism_class": {
                "kind": "BitExact"
            }
        }"#;

        let err = serde_json::from_str::<RangePolicyProjection>(encoded)
            .expect_err("duplicate override key rejects");

        assert!(
            err.to_string()
                .contains("duplicate reduction ceiling override key")
        );
    }

    #[test]
    fn plan_candidate_generation_single_i16_only_yields_single_family() {
        let generation = generate_plan_candidates(
            &facts_with("site.single", 64, 1),
            ReductionPlanCeiling::ExactOnly,
            &RangeCapsSpec::default_v2(),
            DeterminismClass::Deterministic,
        );

        assert_eq!(
            generation.candidate_families,
            vec![ReductionPlanFamily::SingleI16]
        );
        assert_eq!(generation.candidates, vec![ReductionPlan::SingleI16]);
    }

    #[test]
    fn plan_candidate_generation_allow_chunked_yields_two_families() {
        let generation = generate_plan_candidates(
            &facts_with("site.chunked", 64, 1),
            ReductionPlanCeiling::Conservative,
            &RangeCapsSpec::default_v2(),
            DeterminismClass::Deterministic,
        );

        assert_eq!(
            generation.candidate_families,
            vec![
                ReductionPlanFamily::SingleI16,
                ReductionPlanFamily::ChunkedI16
            ]
        );
        assert!(matches!(
            generation.candidates.as_slice(),
            [ReductionPlan::SingleI16, ReductionPlan::ChunkedI16 { .. }]
        ));
    }

    #[test]
    fn plan_candidate_generation_allow_renorm_yields_three_families() {
        let generation = generate_plan_candidates(
            &facts_with("site.renorm", 64, 1),
            ReductionPlanCeiling::Adaptive,
            &RangeCapsSpec::default_v2(),
            DeterminismClass::Deterministic,
        );

        assert_eq!(
            generation.candidate_families,
            vec![
                ReductionPlanFamily::SingleI16,
                ReductionPlanFamily::ChunkedI16,
                ReductionPlanFamily::RenormLoop
            ]
        );
        assert!(matches!(
            generation.candidates.as_slice(),
            [
                ReductionPlan::SingleI16,
                ReductionPlan::ChunkedI16 { .. },
                ReductionPlan::RenormLoop { .. }
            ]
        ));
        assert!(
            take_recorded_range_construction_events()
                .contains(&RANGE_PLAN_CANDIDATE_GENERATION_EVENT)
        );
    }

    #[test]
    fn canonical_candidate_for_family_chunked_picks_largest_pow2_le_max_safe() {
        let plan = canonical_candidate_for_family(
            ReductionPlanFamily::ChunkedI16,
            &facts_with("site.chunk.max", 100, 1_000),
            &caps(64, 16, 256, RenormStrategyPolicy::ExactPostBoundaryOnly),
            DeterminismClass::Deterministic,
        )
        .expect("chunked candidate");

        assert_eq!(plan, ReductionPlan::ChunkedI16 { chunk_len: 32 });
    }

    #[test]
    fn choose_chunk_len_per_term_zero_picks_max_pow2_le_chunk_max() {
        let len = choose_chunk_len(
            &facts_with("site.zero", 63, 0),
            &caps(48, 16, 256, RenormStrategyPolicy::ExactPostBoundaryOnly),
            DeterminismClass::Deterministic,
        )
        .expect("chunk len");

        assert_eq!(len, 32);
    }

    #[test]
    fn choose_chunk_len_bitexact_requires_divisor() {
        let len = choose_chunk_len(
            &facts_with("site.bitexact", 96, 1),
            &caps(64, 16, 256, RenormStrategyPolicy::ExactPostBoundaryOnly),
            DeterminismClass::BitExact,
        )
        .expect("chunk len");

        assert_eq!(len, 32);
    }

    #[test]
    fn choose_chunk_len_bitexact_no_divisor_rejected() {
        let err = choose_chunk_len(
            &facts_with("site.zero-terms", 0, 1),
            &RangeCapsSpec::default_v2(),
            DeterminismClass::BitExact,
        )
        .expect_err("bitexact divisor rejects");

        assert_eq!(err.code(), RANGE_BITEXACT_REQUIRES_CHUNK_DIVIDES_CODE);
    }

    #[test]
    fn choose_chunk_len_non_bitexact_picks_max_pow2_le_max_safe() {
        let len = choose_chunk_len(
            &facts_with("site.non-bitexact", 77, 2_000),
            &RangeCapsSpec::default_v2(),
            DeterminismClass::Nondeterministic,
        )
        .expect("chunk len");

        assert_eq!(len, 16);
    }

    #[test]
    fn choose_chunk_len_per_chunk_exceeds_envelope_rejected() {
        let err = choose_chunk_len(
            &facts_with("site.too-wide", 64, 32_768),
            &RangeCapsSpec::default_v2(),
            DeterminismClass::Deterministic,
        )
        .expect_err("chunk envelope rejects");

        assert!(matches!(
            err,
            PlanLengthSelectionError::PerChunkExceedsI16Envelope { .. }
        ));
    }

    #[test]
    fn choose_chunk_len_above_profile_chunk_max_rejected() {
        let err = choose_chunk_len(
            &facts_with("site.bad-cap", 64, 1),
            &caps(0, 16, 256, RenormStrategyPolicy::ExactPostBoundaryOnly),
            DeterminismClass::Deterministic,
        )
        .expect_err("profile max rejects");

        assert_eq!(err.code(), RANGE_CHUNK_LEN_EXCEEDS_PROFILE_MAX_CODE);
    }

    #[test]
    fn choose_tile_len_bitexact_renorm_loop_reserved_v1() {
        let err = choose_tile_len(
            &facts_with("site.bitexact-renorm", 64, 1),
            &RangeCapsSpec::default_v2(),
            DeterminismClass::BitExact,
        )
        .expect_err("bitexact renorm reserved");

        assert_eq!(err.code(), RANGE_BITEXACT_RENORM_LOOP_RESERVED_V1_CODE);
    }

    #[test]
    fn choose_tile_len_non_bitexact_exact_post_boundary_no_margin() {
        let len = choose_tile_len(
            &facts_with("site.tile.exact", 64, 1_000),
            &caps(256, 16, 64, RenormStrategyPolicy::ExactPostBoundaryOnly),
            DeterminismClass::Deterministic,
        )
        .expect("tile len");

        assert_eq!(len, 32);
        assert!(
            take_recorded_range_construction_events().contains(&RANGE_PLAN_LENGTH_SELECTION_EVENT)
        );
    }

    #[test]
    fn choose_tile_len_non_bitexact_dynamic_margin() {
        let len = choose_tile_len(
            &facts_with("site.tile.margin", 64, 1_000),
            &caps(
                256,
                16,
                64,
                RenormStrategyPolicy::DynamicMargin {
                    margin_q16_16: 0x0000_8000,
                },
            ),
            DeterminismClass::Deterministic,
        )
        .expect("tile len");

        assert_eq!(len, 16);
    }

    #[test]
    fn choose_tile_len_per_term_zero_picks_profile_tile_min() {
        let len = choose_tile_len(
            &facts_with("site.tile.zero", 64, 0),
            &caps(256, 24, 256, RenormStrategyPolicy::ExactPostBoundaryOnly),
            DeterminismClass::Deterministic,
        )
        .expect("tile len");

        assert_eq!(len, 24);
    }

    #[test]
    fn choose_tile_len_below_profile_tile_min_rejected() {
        let err = choose_tile_len(
            &facts_with("site.tile.below", 64, 3_000),
            &caps(256, 16, 64, RenormStrategyPolicy::ExactPostBoundaryOnly),
            DeterminismClass::Deterministic,
        )
        .expect_err("tile min rejects");

        assert!(matches!(
            err,
            PlanLengthSelectionError::PerTileExceedsI16Envelope { .. }
        ));
    }

    #[test]
    fn choose_tile_len_above_profile_tile_max_clamped() {
        let len = choose_tile_len(
            &facts_with("site.tile.clamped", 64, 1),
            &caps(256, 16, 32, RenormStrategyPolicy::ExactPostBoundaryOnly),
            DeterminismClass::Deterministic,
        )
        .expect("tile len");

        assert_eq!(len, 32);
    }

    #[test]
    fn choose_tile_len_min_above_profile_tile_max_rejected() {
        let err = choose_tile_len(
            &facts_with("site.tile.bad-bounds", 64, 1),
            &caps(256, 64, 32, RenormStrategyPolicy::ExactPostBoundaryOnly),
            DeterminismClass::Deterministic,
        )
        .expect_err("tile min above max rejects");

        assert_eq!(err.code(), RANGE_TILE_LEN_EXCEEDS_PROFILE_MAX_CODE);
    }

    #[test]
    fn choose_tile_len_per_tile_exceeds_envelope_rejected() {
        let err = choose_tile_len(
            &facts_with("site.tile.exceeds", 64, 40_000),
            &RangeCapsSpec::default_v2(),
            DeterminismClass::Deterministic,
        )
        .expect_err("tile envelope rejects");

        assert!(matches!(
            err,
            PlanLengthSelectionError::PerTileExceedsI16Envelope { .. }
        ));
    }

    #[test]
    fn choose_tile_len_non_bitexact_term_count_above_u16_uses_tile_count() {
        let len = choose_tile_len(
            &facts_with("site.tile.u16", u32::from(u16::MAX) + 1, 1),
            &RangeCapsSpec::default_v2(),
            DeterminismClass::Deterministic,
        )
        .expect("non-bitexact tile len");

        assert_eq!(len, 256);
    }

    #[test]
    fn canonical_chunk_len_picks_largest_when_multiple_safe() {
        let len = choose_chunk_len(
            &facts_with("site.tie", 30, 1_000),
            &caps(128, 16, 256, RenormStrategyPolicy::ExactPostBoundaryOnly),
            DeterminismClass::Deterministic,
        )
        .expect("chunk len");

        assert_eq!(len, 32);
    }

    #[test]
    fn verifies_single_i16_proof_passing() {
        let facts = facts_with_bias("site.single.pass", 8, 256, Some(4));
        let plan = ReductionPlan::SingleI16;
        let cert =
            construct_accumulator_certificate(&plan, &facts, DeterminismClass::Deterministic);

        assert!(verifies(
            &cert,
            &plan,
            &facts,
            DeterminismClass::Deterministic
        ));
        assert!(matches!(
            cert,
            AccumulatorCertificate::SingleI16Proof {
                total_abs_max: 2_052,
                slack: 30_715,
                ..
            }
        ));
    }

    #[test]
    fn verifies_single_i16_proof_failing_sum_overflow() {
        let facts = facts_with_bias("site.single.fail", 2, 20_000, None);
        let plan = ReductionPlan::SingleI16;
        let cert =
            construct_accumulator_certificate(&plan, &facts, DeterminismClass::Deterministic);

        assert!(!verifies(
            &cert,
            &plan,
            &facts,
            DeterminismClass::Deterministic
        ));
        assert!(matches!(
            cert,
            AccumulatorCertificate::Failed {
                proof_state: AccumulatorProofState::SumExceedsI16Envelope { .. },
                ..
            }
        ));
    }

    #[test]
    fn verifies_chunked_i16_proof_passing() {
        let facts = facts_with_bias("site.chunk.pass", 64, 512, Some(7));
        let plan = ReductionPlan::ChunkedI16 { chunk_len: 32 };
        let cert =
            construct_accumulator_certificate(&plan, &facts, DeterminismClass::Deterministic);

        assert!(verifies(
            &cert,
            &plan,
            &facts,
            DeterminismClass::Deterministic
        ));
        assert!(matches!(
            cert,
            AccumulatorCertificate::ChunkedI16Proof {
                per_chunk_sum_bound: 16_384,
                cross_chunk_sum_bound: 32_768,
                ..
            }
        ));
    }

    #[test]
    fn verifies_chunked_i16_proof_per_chunk_overflow() {
        let facts = facts_with_bias("site.chunk.per", 64, 20_000, None);
        let plan = ReductionPlan::ChunkedI16 { chunk_len: 2 };
        let cert =
            construct_accumulator_certificate(&plan, &facts, DeterminismClass::Deterministic);

        assert!(matches!(
            cert,
            AccumulatorCertificate::Failed {
                proof_state: AccumulatorProofState::PerChunkExceedsI16Envelope { .. },
                ..
            }
        ));
    }

    #[test]
    fn verifies_chunked_i16_proof_cross_chunk_overflow() {
        let facts = facts_with_bias("site.chunk.cross", 65_535, 32_767, Some(200_000));
        let plan = ReductionPlan::ChunkedI16 { chunk_len: 1 };
        let cert =
            construct_accumulator_certificate(&plan, &facts, DeterminismClass::Deterministic);

        assert!(matches!(
            cert,
            AccumulatorCertificate::Failed {
                proof_state: AccumulatorProofState::CrossChunkExceedsI32Envelope { .. },
                ..
            }
        ));
    }

    #[test]
    fn verifies_renorm_loop_proof_non_bitexact_passing() {
        let facts = facts_with_bias("site.renorm.pass", 64, 1_000, Some(3));
        let plan = ReductionPlan::RenormLoop {
            tile_len: 16,
            renorm: damping_renorm_spec(),
        };
        let cert =
            construct_accumulator_certificate(&plan, &facts, DeterminismClass::Deterministic);

        assert!(verifies_with_determinism(
            &cert,
            &plan,
            &facts,
            DeterminismClass::Deterministic
        ));
        assert!(matches!(
            cert,
            AccumulatorCertificate::RenormLoopProof {
                per_tile_sum_bound: 16_000,
                ..
            }
        ));
    }

    #[test]
    fn verifies_renorm_loop_requires_explicit_bitexact_rejection() {
        let facts = facts_with_bias("site.renorm.explicit", 64, 1_000, Some(3));
        let plan = ReductionPlan::RenormLoop {
            tile_len: 16,
            renorm: damping_renorm_spec(),
        };
        let cert =
            construct_accumulator_certificate(&plan, &facts, DeterminismClass::Deterministic);

        assert!(verifies(
            &cert,
            &plan,
            &facts,
            DeterminismClass::Deterministic
        ));
        assert!(!verifies(&cert, &plan, &facts, DeterminismClass::BitExact));
        assert!(
            take_recorded_range_construction_events()
                .contains(&RANGE_CERT_REJECTS_BITEXACT_RENORM_LOOP_EVENT)
        );
    }

    #[test]
    fn verifies_renorm_loop_proof_bitexact_v1_reserved() {
        let facts = facts_with_bias("site.renorm.bitexact", 64, 1_000, Some(3));
        let plan = ReductionPlan::RenormLoop {
            tile_len: 16,
            renorm: damping_renorm_spec(),
        };
        let cert = construct_accumulator_certificate(&plan, &facts, DeterminismClass::BitExact);

        assert!(!verifies_with_determinism(
            &cert,
            &plan,
            &facts,
            DeterminismClass::BitExact
        ));
        assert!(matches!(
            cert,
            AccumulatorCertificate::Failed {
                proof_state: AccumulatorProofState::DeterminismRequiresEnforcedRenorm,
                witness: AccumulatorFailureWitness::BitExactSaturationForbidden,
                ..
            }
        ));
    }

    #[test]
    fn verifies_accumulator_domain_must_be_raw_integer_products() {
        for accumulator_domain in [
            AccumulatorDomain::PostScaleQ8_8,
            AccumulatorDomain::PostScaleQ16_16,
        ] {
            let mut facts = facts_with_bias("site.domain.closed", 8, 256, Some(4));
            facts.accumulator_domain = accumulator_domain;
            let plan = ReductionPlan::SingleI16;
            let cert =
                construct_accumulator_certificate(&plan, &facts, DeterminismClass::Deterministic);

            assert!(!verifies(
                &cert,
                &plan,
                &facts,
                DeterminismClass::Deterministic
            ));
        }
    }

    #[test]
    fn verifies_for_dual_matches_verifies() {
        let cases = [
            (
                ReductionPlan::SingleI16,
                facts_with_bias("site.dual.single", 8, 256, Some(4)),
            ),
            (
                ReductionPlan::ChunkedI16 { chunk_len: 32 },
                facts_with_bias("site.dual.chunk", 64, 512, Some(7)),
            ),
            (
                ReductionPlan::RenormLoop {
                    tile_len: 16,
                    renorm: damping_renorm_spec(),
                },
                facts_with_bias("site.dual.renorm", 64, 1_000, Some(3)),
            ),
            (
                ReductionPlan::SingleI16,
                facts_with_bias("site.dual.fail", 8, 10_000, None),
            ),
        ];

        for (plan, facts) in cases {
            let cert =
                construct_accumulator_certificate(&plan, &facts, DeterminismClass::Deterministic);
            let direct = !matches!(cert, AccumulatorCertificate::Failed { .. })
                && verifies(&cert, &plan, &facts, DeterminismClass::Deterministic);
            assert_eq!(
                verifies_for(&plan, &facts, DeterminismClass::Deterministic),
                direct
            );
        }
    }

    #[test]
    fn renorm_recurrence_verifies_raw_integer_products_path() {
        let facts = facts_with_bias("site.renorm.recur", 64, 1_000, Some(3));
        let recurrence = renorm_recurrence_for(RenormRounding::NearestEven, 0x0010_0000, 1);
        let claimed = renorm_closed_form_bound(
            &facts,
            16,
            4,
            &RenormStrategy::DynamicMargin {
                margin_q16_16: 0x0000_4000,
            },
            recurrence,
        )
        .expect("closed form computes");

        assert!(renorm_recurrence_verifies(
            &facts,
            16,
            4,
            RenormStrategy::DynamicMargin {
                margin_q16_16: 0x0000_4000,
            },
            recurrence,
            claimed,
        ));
        assert!(!renorm_recurrence_verifies(
            &facts,
            16,
            4,
            RenormStrategy::ExactPostBoundary,
            RenormRecurrence {
                output_scale_q16_16: 0,
                ..recurrence
            },
            claimed,
        ));
        assert!(renorm_recurrence_verifies(
            &facts,
            16,
            4,
            RenormStrategy::ExactPostBoundary,
            renorm_recurrence_for(RenormRounding::TowardZero, 0x0010_0000, 0),
            renorm_closed_form_bound(
                &facts,
                16,
                4,
                &RenormStrategy::ExactPostBoundary,
                renorm_recurrence_for(RenormRounding::TowardZero, 0x0010_0000, 0),
            )
            .expect("toward zero closed form computes"),
        ));
    }

    #[test]
    fn renorm_recurrence_verifies_non_raw_returns_false() {
        let mut facts = facts_with_bias("site.renorm.nonraw", 64, 1_000, Some(3));
        facts.accumulator_domain = AccumulatorDomain::PostScaleQ16_16;

        assert!(!renorm_recurrence_verifies(
            &facts,
            16,
            4,
            RenormStrategy::ExactPostBoundary,
            renorm_recurrence_for(RenormRounding::NearestEven, 0x0010_0000, 1),
            1,
        ));
    }

    #[test]
    fn plan_choice_picks_single_i16_first_if_verifies() {
        let binding = EffectiveCeilingBinding {
            site: site("site.choice.single"),
            facts: facts_with_bias("site.choice.single", 8, 256, Some(4)),
            effective_ceiling: ReductionPlanCeiling::Adaptive,
            provenance: ReductionCeilingProvenance::Global {
                source: PolicySource::ProfileDefault,
            },
        };
        let generation = generate_plan_candidates(
            &binding.facts,
            binding.effective_ceiling,
            &RangeCapsSpec::default_v2(),
            DeterminismClass::Deterministic,
        );

        let choice = choose_plan(
            &binding,
            &generation,
            &RangeCapsSpec::default_v2(),
            DeterminismClass::Deterministic,
        );

        assert_eq!(
            choice.chosen.as_ref().map(|chosen| &chosen.plan),
            Some(&ReductionPlan::SingleI16)
        );
    }

    #[test]
    fn plan_choice_picks_chunked_i16_when_single_fails_under_allow_chunked_i16() {
        let binding = EffectiveCeilingBinding {
            site: site("site.choice.chunk"),
            facts: facts_with_bias("site.choice.chunk", 64, 1_000, Some(3)),
            effective_ceiling: ReductionPlanCeiling::Conservative,
            provenance: ReductionCeilingProvenance::Global {
                source: PolicySource::ProfileDefault,
            },
        };
        let generation = generate_plan_candidates(
            &binding.facts,
            binding.effective_ceiling,
            &RangeCapsSpec::default_v2(),
            DeterminismClass::Deterministic,
        );

        let choice = choose_plan(
            &binding,
            &generation,
            &RangeCapsSpec::default_v2(),
            DeterminismClass::Deterministic,
        );

        assert!(matches!(
            choice.chosen.as_ref().map(|chosen| &chosen.plan),
            Some(ReductionPlan::ChunkedI16 { .. })
        ));
    }

    #[test]
    fn plan_choice_picks_renorm_loop_when_chunked_fails_under_allow_renorm_loop() {
        let caps = caps(
            1,
            1,
            1,
            RenormStrategyPolicy::DynamicMargin { margin_q16_16: 0 },
        );
        let binding = EffectiveCeilingBinding {
            site: site("site.choice.renorm"),
            facts: facts_with_bias("site.choice.renorm", 65_539, 32_767, None),
            effective_ceiling: ReductionPlanCeiling::Adaptive,
            provenance: ReductionCeilingProvenance::Global {
                source: PolicySource::ProfileDefault,
            },
        };
        let generation = generate_plan_candidates(
            &binding.facts,
            binding.effective_ceiling,
            &caps,
            DeterminismClass::Deterministic,
        );

        let choice = choose_plan(
            &binding,
            &generation,
            &caps,
            DeterminismClass::Deterministic,
        );

        assert!(matches!(
            choice.chosen.as_ref().map(|chosen| &chosen.plan),
            Some(ReductionPlan::RenormLoop { .. })
        ));
    }

    #[test]
    fn plan_choice_ceiling_violation_precedence_single_i16_only() {
        let binding = EffectiveCeilingBinding {
            site: site("site.choice.exact.fail"),
            facts: facts_with_bias("site.choice.exact.fail", 2, 20_000, None),
            effective_ceiling: ReductionPlanCeiling::ExactOnly,
            provenance: ReductionCeilingProvenance::Global {
                source: PolicySource::ProfileDefault,
            },
        };
        let generation = generate_plan_candidates(
            &binding.facts,
            binding.effective_ceiling,
            &RangeCapsSpec::default_v2(),
            DeterminismClass::Deterministic,
        );

        let choice = choose_plan(
            &binding,
            &generation,
            &RangeCapsSpec::default_v2(),
            DeterminismClass::Deterministic,
        );

        assert!(choice.chosen.is_none());
        assert_eq!(
            choice.diagnostics[0].provenance[0].reference,
            format!(
                "{}:{}",
                RANGE_CEILING_VIOLATED_SINGLE_I16_ONLY_CODE, binding.site.0
            )
        );
    }

    #[test]
    fn plan_choice_ceiling_violation_precedence_no_renorm_loop() {
        let caps = caps(
            1,
            1,
            1,
            RenormStrategyPolicy::DynamicMargin { margin_q16_16: 0 },
        );
        let binding = EffectiveCeilingBinding {
            site: site("site.choice.no-renorm"),
            facts: facts_with_bias("site.choice.no-renorm", 65_539, 32_767, None),
            effective_ceiling: ReductionPlanCeiling::Conservative,
            provenance: ReductionCeilingProvenance::Global {
                source: PolicySource::ProfileDefault,
            },
        };
        let generation = generate_plan_candidates(
            &binding.facts,
            binding.effective_ceiling,
            &caps,
            DeterminismClass::Deterministic,
        );

        let choice = choose_plan(
            &binding,
            &generation,
            &caps,
            DeterminismClass::Deterministic,
        );

        assert!(choice.chosen.is_none());
        assert_eq!(
            choice.diagnostics[0].provenance[0].reference,
            format!(
                "{}:{}",
                RANGE_CEILING_VIOLATED_NO_RENORM_LOOP_CODE, binding.site.0
            )
        );
    }

    #[test]
    fn plan_choice_no_proven_plan_within_ceiling() {
        let binding = EffectiveCeilingBinding {
            site: site("site.choice.none"),
            facts: facts_with_bias("site.choice.none", 64, 40_000, None),
            effective_ceiling: ReductionPlanCeiling::Adaptive,
            provenance: ReductionCeilingProvenance::Global {
                source: PolicySource::ProfileDefault,
            },
        };
        let generation = generate_plan_candidates(
            &binding.facts,
            binding.effective_ceiling,
            &RangeCapsSpec::default_v2(),
            DeterminismClass::Deterministic,
        );

        let choice = choose_plan(
            &binding,
            &generation,
            &RangeCapsSpec::default_v2(),
            DeterminismClass::Deterministic,
        );

        assert!(choice.chosen.is_none());
        assert_eq!(
            choice.diagnostics[0].provenance[0].reference,
            format!(
                "{}:{}",
                RANGE_NO_PROVEN_PLAN_WITHIN_CEILING_CODE, binding.site.0
            )
        );
    }

    #[test]
    fn cert_outcome_verified_iff_no_failed_variants_and_no_hard_diag() {
        let body = range_cert_body_fixture();

        assert_eq!(
            range_cert_outcome(&body.certificates, &body.diagnostics),
            CertOutcome::Verified
        );
    }

    #[test]
    fn cert_outcome_failed_when_any_failed_or_hard_diag() {
        let failed = failed_cert_body_fixture();
        assert_eq!(
            range_cert_outcome(&failed.certificates, &failed.diagnostics),
            CertOutcome::Failed
        );

        let hard = vec![plan_choice_diagnostic(
            &site("site.hard"),
            RANGE_NO_PROVEN_PLAN_WITHIN_CEILING_CODE,
        )];
        assert_eq!(
            range_cert_outcome(&range_cert_body_fixture().certificates, &hard),
            CertOutcome::Failed
        );
    }

    #[test]
    fn canonical_sort_builds_bijective_site_to_certificate_index() {
        let mut entries = vec![
            entry("site.z", ReductionPlan::SingleI16),
            entry("site.a", ReductionPlan::SingleI16),
        ];
        let mut certificates = vec![certified_reduction("site.z"), certified_reduction("site.a")];

        let index = canonical_sort_range_plan(&mut entries, &mut certificates);

        assert_eq!(entries[0].site, site("site.a"));
        assert_eq!(certificates[0].site, site("site.a"));
        assert_eq!(
            index,
            BTreeMap::from([(site("site.a"), 0), (site("site.z"), 1)])
        );
    }

    #[test]
    fn provenance_binding_populates_site_to_node_and_qg() {
        let sites = vec![binding("site.prov", 9)];
        let provenance = bind_range_plan_provenance(&sites);

        assert_eq!(provenance.site_to_node[&site("site.prov")], NodeId::new(9));
        assert_eq!(provenance.site_to_qg[&site("site.prov")], sites[0].qg_ref);
    }

    #[test]
    fn self_consistency_accepts_matching_plan_and_certificate() {
        let facts = facts_with_bias("site.sc", 8, 256, Some(4));
        let plan = ReductionPlan::SingleI16;
        let proof =
            construct_accumulator_certificate(&plan, &facts, DeterminismClass::Deterministic);
        let range_plan = RangePlan {
            identity: RangePlanIdentity {
                determinism: DeterminismClass::Deterministic,
                ..plan_fixture().identity
            },
            entries: vec![RangePlanEntry {
                site: facts.site.clone(),
                plan: plan.clone(),
                site_facts: facts.clone(),
                effective_ceiling: ReductionPlanCeiling::Adaptive,
                ceiling_provenance: ReductionCeilingProvenance::Global {
                    source: PolicySource::ProfileDefault,
                },
            }],
            provenance: RangePlanProvenance {
                site_to_node: BTreeMap::new(),
                site_to_qg: BTreeMap::new(),
            },
        };
        let certificates = vec![CertifiedReduction {
            site: facts.site.clone(),
            plan,
            facts,
            proof,
        }];
        let range_cert = RangeCertBody {
            certificates,
            site_to_certificate_index: BTreeMap::from([(site("site.sc"), 0)]),
            ..range_cert_body_fixture()
        };

        assert!(self_consistency_diagnostics(&range_plan, &range_cert).is_empty());
    }

    #[test]
    fn self_consistency_rejects_mismatched_site_facts() {
        let facts = facts_with_bias("site.sc.facts", 8, 256, Some(4));
        let (mut range_plan, range_cert) = single_entry_plan_and_cert(
            ReductionPlan::SingleI16,
            facts,
            DeterminismClass::Deterministic,
        );
        range_plan.entries[0].site_facts.site = site("site.sc.other");

        let fields = self_consistency_fields(&range_plan, &range_cert);
        assert!(fields.contains("range_plan.entries.site_facts"));
    }

    #[test]
    fn self_consistency_rejects_plan_family_above_ceiling() {
        let facts = facts_with_bias("site.sc.ceiling", 64, 512, Some(7));
        let (mut range_plan, range_cert) = single_entry_plan_and_cert(
            ReductionPlan::ChunkedI16 { chunk_len: 32 },
            facts,
            DeterminismClass::Deterministic,
        );
        range_plan.entries[0].effective_ceiling = ReductionPlanCeiling::ExactOnly;

        let fields = self_consistency_fields(&range_plan, &range_cert);
        assert!(fields.contains("range_plan.entries.effective_ceiling"));
    }

    #[test]
    fn self_consistency_rejects_mutated_certificate_index_and_proof() {
        let facts = facts_with_bias("site.sc.proof", 8, 256, Some(4));
        let (range_plan, mut range_cert) = single_entry_plan_and_cert(
            ReductionPlan::SingleI16,
            facts,
            DeterminismClass::Deterministic,
        );
        range_cert.site_to_certificate_index =
            BTreeMap::from([(range_plan.entries[0].site.clone(), 7)]);

        let fields = self_consistency_fields(&range_plan, &range_cert);
        assert!(fields.contains("range_cert.site_to_certificate_index"));

        let (range_plan, mut range_cert) = single_entry_plan_and_cert(
            ReductionPlan::SingleI16,
            facts_with_bias("site.sc.proof2", 8, 256, Some(4)),
            DeterminismClass::Deterministic,
        );
        if let AccumulatorCertificate::SingleI16Proof { slack, .. } =
            &mut range_cert.certificates[0].proof
        {
            *slack += 1;
        }

        let fields = self_consistency_fields(&range_plan, &range_cert);
        assert!(fields.contains("range_cert.certificates.proof"));
    }

    #[test]
    fn self_consistency_rejects_non_smallest_canonical_family() {
        let facts = facts_with_bias("site.sc.smallest", 8, 256, Some(4));
        let (range_plan, range_cert) = single_entry_plan_and_cert(
            ReductionPlan::ChunkedI16 { chunk_len: 4 },
            facts,
            DeterminismClass::Deterministic,
        );

        let fields = self_consistency_fields(&range_plan, &range_cert);
        assert!(fields.contains("range_plan.entries.plan"));
    }

    #[test]
    fn self_consistency_rejects_bitexact_chunk_non_divisor() {
        let facts = facts_with_bias("site.sc.bitexact.chunk", 48, 512, Some(7));
        let (mut range_plan, mut range_cert) = single_entry_plan_and_cert(
            ReductionPlan::ChunkedI16 { chunk_len: 32 },
            facts,
            DeterminismClass::Deterministic,
        );
        range_plan.identity.determinism = DeterminismClass::BitExact;
        range_cert.identity.determinism = DeterminismClass::BitExact;

        let fields = self_consistency_fields(&range_plan, &range_cert);
        assert!(fields.contains("range_plan.entries.plan.determinism"));
        assert!(fields.contains("range_cert.certificates.proof"));
    }

    #[test]
    fn self_consistency_rejects_bitexact_renorm_loop_v1_reserved() {
        let facts = facts_with_bias("site.sc.bitexact.renorm", 64, 1_000, Some(3));
        let (mut range_plan, mut range_cert) = single_entry_plan_and_cert(
            ReductionPlan::RenormLoop {
                tile_len: 16,
                renorm: damping_renorm_spec(),
            },
            facts,
            DeterminismClass::Deterministic,
        );
        range_plan.identity.determinism = DeterminismClass::BitExact;
        range_cert.identity.determinism = DeterminismClass::BitExact;

        let fields = self_consistency_fields(&range_plan, &range_cert);
        assert!(fields.contains("range_plan.entries.plan.determinism"));
        assert!(fields.contains("range_cert.certificates.proof"));
        assert!(
            take_recorded_range_construction_events()
                .contains(&RANGE_CERT_REJECTS_BITEXACT_RENORM_LOOP_EVENT)
        );
    }

    #[test]
    fn certificate_trace_events_are_recorded() {
        let facts = facts_with_bias("site.trace.cert", 8, 256, Some(4));
        let plan = ReductionPlan::SingleI16;
        let cert =
            construct_accumulator_certificate(&plan, &facts, DeterminismClass::Deterministic);
        assert!(verifies(
            &cert,
            &plan,
            &facts,
            DeterminismClass::Deterministic
        ));
        let _ = choose_plan(
            &EffectiveCeilingBinding {
                site: facts.site.clone(),
                facts,
                effective_ceiling: ReductionPlanCeiling::ExactOnly,
                provenance: ReductionCeilingProvenance::Global {
                    source: PolicySource::ProfileDefault,
                },
            },
            &PlanCandidateGeneration {
                site: site("site.trace.cert"),
                candidate_families: vec![ReductionPlanFamily::SingleI16],
                candidates: vec![ReductionPlan::SingleI16],
                rejections: Vec::new(),
            },
            &RangeCapsSpec::default_v2(),
            DeterminismClass::Deterministic,
        );

        let events = take_recorded_range_construction_events();
        assert!(events.contains(&RANGE_CERTIFICATE_CONSTRUCTION_EVENT));
        assert!(events.contains(&RANGE_CERT_VERIFIES_SINGLE_I16_EVENT));
        assert!(events.contains(&RANGE_PLAN_CHOICE_EVENT));
    }

    #[test]
    fn range_plan_core_product_serde_round_trip() {
        round_trip(&core_product_fixture());
    }

    #[test]
    fn range_plan_core_product_includes_range_cert_body() {
        fn accepts_cert(product: RangePlanCoreProduct) -> RangeCertBody {
            product.range_cert
        }

        let cert = accepts_cert(core_product_fixture());
        assert_eq!(cert.cert_outcome, CertOutcome::Verified);
    }

    #[test]
    fn range_plan_core_product_hash_deterministic() {
        let first = range_plan_core_product_hash(&core_product_fixture()).expect("first hash");
        let second = range_plan_core_product_hash(&core_product_fixture()).expect("second hash");

        assert_eq!(first, second);
    }

    #[test]
    fn range_plan_report_result_histogram_keys_are_string_encoded() {
        let result = range_plan_report_result_fixture();
        let value = serde_json::to_value(&result).expect("report result serializes");

        assert_eq!(
            value["effective_ceiling_histogram"],
            serde_json::json!({
                "Adaptive": 1,
                "Conservative": 2,
                "ExactOnly": 0
            })
        );
        assert_eq!(
            value["ceiling_provenance_histogram"],
            serde_json::json!({
                "Global": 1,
                "LayerOverride": 0,
                "SiteOverride": 1
            })
        );

        let decoded: RangePlanReportResult =
            serde_json::from_value(value).expect("report result decodes");
        let canonical = canonical_bytes(&decoded);
        let recanonical = canonical_bytes(&result);

        assert_eq!(decoded, result);
        assert_eq!(canonical, recanonical);
    }

    #[test]
    fn range_plan_report_hash_accepts_histogram_keys() {
        let report = ReportEnvelope::new(ReportOutcome::Passed, range_plan_report_body_fixture())
            .expect("range plan report envelope")
            .with_computed_self_hash()
            .expect("range plan report hashes");
        let canonical = gbf_report::canonicalize(&report).expect("report canonicalizes");
        let decoded: ReportEnvelope<RangePlanReportBody> =
            serde_json::from_slice(&canonical).expect("canonical report decodes");

        gbf_report::round_trip_self_hash(&report).expect("self hash round-trips");
        assert_eq!(decoded.report_self_hash, report.report_self_hash);
    }

    #[test]
    fn range_cert_body_hash_deterministic() {
        let first = range_cert_body_hash(&range_cert_body_fixture()).expect("first hash");
        let second = range_cert_body_hash(&range_cert_body_fixture()).expect("second hash");

        assert_eq!(first, second);
        assert_eq!(
            first,
            expected_codegen_domain_hash(
                "RangeCertBody",
                RANGE_CERT_SCHEMA_VERSION,
                &range_cert_body_fixture()
            )
        );
    }

    #[test]
    fn range_cert_body_hash_byte_stable_independent_of_serialization_order() {
        let first = range_cert_body_with_two_sites();
        let second = RangeCertBody {
            site_to_certificate_index: BTreeMap::from([
                (site("classify"), 0),
                (site("expert.1.2.down"), 1),
            ]),
            ..range_cert_body_with_two_sites()
        };

        assert_eq!(canonical_bytes(&first), canonical_bytes(&second));
        assert_eq!(
            range_cert_body_hash(&first).expect("first hash"),
            range_cert_body_hash(&second).expect("second hash")
        );
    }

    #[test]
    fn range_plan_serde_round_trip() {
        let plan = plan_fixture();
        let first = canonical_bytes(&plan);
        let decoded: RangePlan = serde_json::from_slice(&first).expect("plan decodes");
        let second = canonical_bytes(&decoded);

        assert_eq!(decoded, plan);
        assert_eq!(second, first);
    }

    #[test]
    fn range_plan_body_round_trips_accept_stage5_golden() {
        let golden = include_str!("../../fixtures/accept/stage5/range_plan_body_v1.json");
        let golden = golden.trim_end();
        let decoded: RangePlan = serde_json::from_str(golden).expect("golden plan decodes");
        let canonical = canonical_bytes(&decoded);

        assert_eq!(decoded, plan_fixture());
        assert_eq!(
            String::from_utf8(canonical).expect("canonical bytes are UTF-8"),
            golden
        );
    }

    #[test]
    fn range_plan_self_hash_deterministic() {
        let first = range_plan_self_hash(&plan_fixture()).expect("first hash");
        let second = range_plan_self_hash(&plan_fixture()).expect("second hash");

        assert_eq!(first, second);
        assert_eq!(
            first.to_string(),
            "sha256:37d06fc7674c46529c58390f881c69a664f877360889daa2f1018c63d59b91b7"
        );
        assert_eq!(
            first,
            expected_codegen_domain_hash("RangePlan", RANGE_PLAN_SCHEMA_VERSION, &plan_fixture())
        );
    }

    #[test]
    fn range_plan_entry_no_site_key_field() {
        fn accepts_entry_without_site_key(entry: RangePlanEntry) -> ReductionSiteId {
            entry.site
        }

        let entry = entry("classify", ReductionPlan::SingleI16);
        let site = accepts_entry_without_site_key(entry.clone());
        let value = serde_json::to_value(entry).expect("entry serializes");

        assert_eq!(site, ReductionSiteId("classify".to_owned()));
        assert!(value.get("site").is_some());
        assert!(value.get("site_key").is_none());
    }

    #[test]
    fn reduction_site_facts_serde_round_trip() {
        round_trip(&facts(site("classify")));
    }

    #[test]
    fn reduction_site_facts_bias_max_abs_q_is_option() {
        fn accepts_option(_: Option<u32>) {}

        accepts_option(facts(site("classify")).bias_max_abs_q);
    }

    #[test]
    fn option_none_fields_use_absent_canonical_encoding() {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        struct NoneCaseFixture {
            facts: ReductionSiteFacts,
            boundary: NamedNumericBoundary,
        }

        let fixture = NoneCaseFixture {
            facts: ReductionSiteFacts {
                site: site("classify"),
                layer: None,
                expert: None,
                slot: None,
                norm_site: None,
                term_count: 1,
                input_max_abs_q: 2,
                weight_max_abs_q: 3,
                per_term_abs_max_q: 6,
                bias_max_abs_q: None,
                accumulator_domain: AccumulatorDomain::PostScaleQ8_8,
                op_tag: InferOpTag::Classify,
            },
            boundary: NamedNumericBoundary::ResidualCombine {
                layer: None,
                site: ResidualSite::PostSequence,
            },
        };
        let canonical = canonical_bytes(&fixture);
        let canonical = String::from_utf8(canonical).expect("canonical bytes are UTF-8");

        assert_eq!(
            canonical,
            concat!(
                r#"{"boundary":{"kind":"ResidualCombine","site":{"kind":"PostSequence"}}"#,
                r#","facts":{"accumulator_domain":{"kind":"PostScaleQ8_8"}"#,
                r#","input_max_abs_q":2,"op_tag":{"kind":"Classify"}"#,
                r#","per_term_abs_max_q":6,"site":"classify","term_count":1"#,
                r#","weight_max_abs_q":3}}"#
            )
        );

        let decoded: NoneCaseFixture =
            serde_json::from_str(&canonical).expect("omitted option fields decode");
        assert_eq!(decoded, fixture);
    }

    #[test]
    fn reduction_site_facts_per_term_abs_max_q_is_u64() {
        fn accepts_u64(_: u64) {}

        accepts_u64(facts(site("classify")).per_term_abs_max_q);
    }

    #[test]
    fn reduction_site_facts_accumulator_domain_round_trip_all_variants() {
        for accumulator_domain in [
            AccumulatorDomain::RawIntegerProducts,
            AccumulatorDomain::PostScaleQ8_8,
            AccumulatorDomain::PostScaleQ16_16,
        ] {
            let mut facts = facts(site("classify"));
            facts.accumulator_domain = accumulator_domain;
            round_trip(&facts);
        }
    }

    #[test]
    fn reduction_plan_variants_round_trip() {
        for plan in [
            ReductionPlan::SingleI16,
            ReductionPlan::ChunkedI16 { chunk_len: 8 },
            ReductionPlan::RenormLoop {
                tile_len: 16,
                renorm: renorm_spec(RenormStrategy::ExactPostBoundary),
            },
        ] {
            round_trip(&plan);
        }
    }

    #[test]
    fn accumulator_certificate_single_i16_proof_round_trip() {
        fn accepts_u64(_: u64) {}

        let proof = single_i16_certificate(site("classify"));
        if let AccumulatorCertificate::SingleI16Proof {
            term_count,
            per_term_abs_max,
            sum_bound,
            bias_abs_max,
            total_abs_max,
            i16_envelope,
            slack,
            ..
        } = proof.clone()
        {
            accepts_u64(term_count);
            accepts_u64(per_term_abs_max);
            accepts_u64(sum_bound);
            accepts_u64(bias_abs_max);
            accepts_u64(total_abs_max);
            accepts_u64(i16_envelope);
            accepts_u64(slack);
        } else {
            panic!("expected SingleI16Proof");
        }
        round_trip(&proof);
    }

    #[test]
    fn accumulator_certificate_chunked_i16_proof_round_trip() {
        round_trip(&AccumulatorCertificate::ChunkedI16Proof {
            site: site("classify"),
            chunk_len: 8,
            chunk_count: 8,
            per_term_abs_max: 256,
            per_chunk_sum_bound: 2_048,
            per_chunk_i16_slack: 30_719,
            cross_chunk_sum_bound: 16_384,
            bias_abs_max: 4,
            total_abs_max: 16_388,
            i32_envelope: 2_147_483_647,
            slack: 2_147_467_259,
        });
    }

    #[test]
    fn accumulator_certificate_renorm_loop_proof_round_trip() {
        round_trip(&AccumulatorCertificate::RenormLoopProof {
            site: site("expert.1.2.down"),
            tile_len: 16,
            tile_count: 4,
            per_term_abs_max: 256,
            per_tile_sum_bound: 4_096,
            per_tile_i16_slack: 28_671,
            renorm: renorm_spec(RenormStrategy::ExactPostBoundary),
            bias_abs_max: 4,
            total_abs_max: 4_100,
            slack: 28_667,
        });
    }

    #[test]
    fn accumulator_certificate_failed_round_trip() {
        round_trip(&failed_cert_body_fixture().certificates[0].proof);
    }

    #[test]
    fn certified_reduction_packs_site_plan_facts_proof() {
        fn accepts_certified(
            certified: CertifiedReduction,
        ) -> (
            ReductionSiteId,
            ReductionPlan,
            ReductionSiteFacts,
            AccumulatorCertificate,
        ) {
            (
                certified.site,
                certified.plan,
                certified.facts,
                certified.proof,
            )
        }

        let (site, plan, facts, proof) = accepts_certified(certified_reduction("classify"));
        assert_eq!(site, facts.site);
        assert_eq!(plan, ReductionPlan::SingleI16);
        assert!(matches!(
            proof,
            AccumulatorCertificate::SingleI16Proof { .. }
        ));
    }

    #[test]
    fn range_cert_body_with_failed_cert_outcome_failed() {
        let body = failed_cert_body_fixture();

        assert_eq!(body.cert_outcome, CertOutcome::Failed);
        assert!(matches!(
            body.certificates[0].proof,
            AccumulatorCertificate::Failed { .. }
        ));
        assert!(body.validate_semantics(ReportOutcome::Failed).is_ok());
    }

    #[test]
    fn range_cert_identity_range_plan_self_hash_is_optional() {
        fn accepts_optional_hash(_: Option<Hash256>) {}

        let identity = failed_cert_body_fixture().identity;
        accepts_optional_hash(identity.range_plan_self_hash);
        round_trip(&identity);
    }

    #[test]
    fn accumulator_proof_state_all_variants_round_trip() {
        fn accepts_u64(_: u64) {}

        for state in [
            AccumulatorProofState::SumExceedsI16Envelope {
                sum_bound: 40_000,
                envelope: 32_767,
            },
            AccumulatorProofState::PerChunkExceedsI16Envelope {
                per_chunk_sum_bound: 40_000,
                envelope: 32_767,
            },
            AccumulatorProofState::CrossChunkExceedsI32Envelope {
                cross_chunk_sum_bound: 3_000_000_000,
                envelope: 2_147_483_647,
            },
            AccumulatorProofState::PerTileExceedsI16Envelope {
                per_tile_sum_bound: 40_000,
                envelope: 32_767,
            },
            AccumulatorProofState::LengthZero {
                length_field: LengthField::ChunkLen,
            },
            AccumulatorProofState::LengthZero {
                length_field: LengthField::TileLen,
            },
            AccumulatorProofState::ChunkLenExceedsProfileMax {
                chunk_len: 512,
                profile_chunk_max: 256,
            },
            AccumulatorProofState::TileLenBelowProfileMin {
                tile_len: 8,
                profile_tile_min: 16,
            },
            AccumulatorProofState::TileLenExceedsProfileMax {
                tile_len: 512,
                profile_tile_max: 256,
            },
            AccumulatorProofState::BitExactRequiresChunkDivides {
                term_count: 65,
                chunk_len: 8,
            },
            AccumulatorProofState::TileLenExceedsU16 { term_count: 70_000 },
            AccumulatorProofState::DeterminismRequiresEnforcedRenorm,
        ] {
            match &state {
                AccumulatorProofState::SumExceedsI16Envelope { envelope, .. }
                | AccumulatorProofState::PerChunkExceedsI16Envelope { envelope, .. }
                | AccumulatorProofState::CrossChunkExceedsI32Envelope { envelope, .. }
                | AccumulatorProofState::PerTileExceedsI16Envelope { envelope, .. } => {
                    accepts_u64(*envelope);
                }
                AccumulatorProofState::LengthZero { .. }
                | AccumulatorProofState::ChunkLenExceedsProfileMax { .. }
                | AccumulatorProofState::TileLenBelowProfileMin { .. }
                | AccumulatorProofState::TileLenExceedsProfileMax { .. }
                | AccumulatorProofState::BitExactRequiresChunkDivides { .. }
                | AccumulatorProofState::TileLenExceedsU16 { .. }
                | AccumulatorProofState::DeterminismRequiresEnforcedRenorm => {}
            }
            round_trip(&state);
        }
    }

    #[test]
    fn length_field_enum_two_variants_round_trip() {
        for field in [LengthField::ChunkLen, LengthField::TileLen] {
            round_trip(&field);
        }
    }

    #[test]
    fn renorm_spec_round_trip_both_strategies() {
        for spec in [
            renorm_spec(RenormStrategy::ExactPostBoundary),
            renorm_spec(RenormStrategy::DynamicMargin {
                margin_q16_16: 0x0000_4000,
            }),
        ] {
            round_trip(&spec);
        }
    }

    #[test]
    fn renorm_recurrence_round_trip_all_rounding_modes() {
        for rounding in [RenormRounding::TowardZero, RenormRounding::NearestEven] {
            round_trip(&recurrence(rounding));
        }
    }

    #[test]
    fn renorm_saturation_policy_round_trip() {
        for saturation in [
            RenormSaturationPolicy::Forbidden,
            RenormSaturationPolicy::AtNamedNumericBoundary {
                boundary: NamedNumericBoundary::FfnActivationOutput {
                    layer: LayerId::new(1),
                    expert: ExpertId::new(2),
                },
            },
        ] {
            round_trip(&saturation);
        }
    }

    #[test]
    fn named_numeric_boundary_v1_variants_round_trip() {
        for boundary in [
            NamedNumericBoundary::ResidualCombine {
                layer: Some(LayerId::new(1)),
                site: ResidualSite::PostSequence,
            },
            NamedNumericBoundary::ClassifyLogit,
            NamedNumericBoundary::FfnActivationOutput {
                layer: LayerId::new(1),
                expert: ExpertId::new(2),
            },
            NamedNumericBoundary::FinalClamp,
        ] {
            round_trip(&boundary);
        }
    }

    #[test]
    fn reduction_ceiling_provenance_round_trip_all_three_variants() {
        for provenance in [
            ReductionCeilingProvenance::Global {
                source: PolicySource::ProfileDefault,
            },
            ReductionCeilingProvenance::LayerOverride {
                layer: LayerId::new(1),
                source: PolicySource::HintBundle,
            },
            ReductionCeilingProvenance::SiteOverride {
                site: site("classify"),
                source: PolicySource::CompileRequestOverride,
            },
        ] {
            round_trip(&provenance);
        }
    }

    #[test]
    fn range_plan_provenance_keys_are_reduction_site_id_strings() {
        let provenance = plan_fixture().provenance;
        let value = serde_json::to_value(provenance).expect("provenance serializes");

        assert_eq!(value["site_to_node"]["classify"], serde_json::json!(7));
        assert_eq!(
            value["site_to_node"]["expert.1.2.down"],
            serde_json::json!(5)
        );
        assert_eq!(value["site_to_qg"]["classify"]["kind"], "ClassifyHead");
    }

    #[test]
    fn range_plan_independent_of_observation_plan() {
        fn assert_stage5_shape(
            _: RangePlan,
            _: RangePlanEntry,
            _: ReductionSiteFacts,
            _: RangePlanProvenance,
        ) {
        }

        let plan = plan_fixture();
        assert_stage5_shape(
            plan.clone(),
            plan.entries[0].clone(),
            plan.entries[0].site_facts.clone(),
            plan.provenance.clone(),
        );

        let value = serde_json::to_value(plan).expect("plan serializes");
        let encoded = serde_json::to_string(&value).expect("value encodes");
        let production_source = include_str!("range_plan.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("production section exists");

        assert!(!production_source.contains("crate::s4"));
        assert!(!production_source.contains("ObservationPlan"));
        assert!(!encoded.contains("observation_plan"));
        assert!(!encoded.contains("ObservationPlan"));
    }

    fn expected_codegen_domain_hash<T: Serialize>(
        type_name: &str,
        schema_version: &str,
        value: &T,
    ) -> Hash256 {
        let canonical = canonical_json_bytes(value).expect("value canonicalizes");
        domain_hash_from_canonical(type_name, schema_version, &canonical)
    }
}
