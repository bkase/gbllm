//! Stage-cache key construction and payload cells for F-B2 Stage 0/0.5 and F-B4 Stage 2.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::time::Instant;

use gbf_foundation::{CompileProfileId, Hash256, SemVer};
use gbf_policy::{ProbeImportanceClass, ReductionPlanCeiling, ValidationDiagnostic};
use gbf_report::report_schemas::{
    artifact_validation_v1, infer_ir_v1, policy_resolution_v1, quant_graph_v1, static_budget_v1,
};
use gbf_report::{
    ReportBody, ReportEnvelope, ReportEnvelopeError, ReportOutcome, ReportSelfHashError,
};
use gbf_report::{canonicalize as canonicalize_report, canonicalize_value, compute_self_hash};
use gbf_store::stage_cache::{
    ComponentDigestSet, ComponentId, FeatureFlag, StageCache as StoreStageCache, StageCacheError,
    StageCacheKey, StageId, StageKey, compose_key,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::budget::StaticBudgetReport;
use crate::policy::{PolicyResolutionStageFailure, ResolvedPolicyProduct};
use crate::s1::quant_graph::{PASS_VERSION_QUANT_GRAPH, QuantGraphProduct};
use crate::s3::infer_ir::{
    GbInferIR, GbInferIRProduct, InferIrAuditParents, PASS_VERSION_INFER_IR,
};
use crate::s4::observation_plan::{
    BUILD_ACTIVE_CHECKPOINT_SCHEMA_VERSION, NonEmptyList, OBSERVATION_PLAN_SCHEMA_VERSION,
    OPERATIONAL_PROBE_SCHEMA_VERSION, ObservationPlanAuditParents, ObservationPlanCoreFailure,
    ObservationPlanCoreProduct, ObservationPlanReportBody, ObservationPlanReportInputIdentity,
    ObservationPlanReportResult, ObservationPlanStageOutput, OperationalProbeSchemaBody,
    OperationalProbeSchemaInputIdentity, OperationalProbeSchemaResult, PerClassCount,
    SemanticCheckpointRole, SemanticCheckpointSchemaReEmitBody,
    SemanticCheckpointSchemaReEmitInputIdentity, SemanticCheckpointSchemaReEmitResult,
    observation_plan_self_hash, observation_policy_projection_hash,
};
use crate::s5::range_plan::{
    RANGE_CERT_SCHEMA_VERSION, RANGE_PLAN_SCHEMA_VERSION, RangeCertBody, RangePlanAuditParents,
    RangePlanCoreProduct, RangePlanReportBody, RangePlanReportInputIdentity, RangePlanReportResult,
    RangePlanStageOutput, ReductionCeilingProvenanceTag, ReductionPlan, range_cert_body_hash,
    range_plan_self_hash, range_policy_projection_hash,
};
use crate::validate::{
    CachedValidationProduct, CachedValidationProductRehydrateError, ValidatedInputHashes,
    ValidationStageFailure,
};

pub const PASS_VERSION_VALIDATE: SemVer = SemVer::new(1, 0, 0);
pub const PASS_VERSION_RESOLVE: SemVer = SemVer::new(2, 0, 0);
pub const PASS_VERSION_QUANT_GRAPH_STAGE1: SemVer = SemVer::new(1, 0, 0);
pub const PASS_VERSION_BUDGET: SemVer = SemVer::new(1, 1, 0);
pub const PASS_VERSION_INFER_IR_STAGE3: SemVer = SemVer::new(1, 0, 0);
pub const PASS_VERSION_OBSERVATION_PLAN: SemVer = SemVer::new(1, 0, 0);
pub const PASS_VERSION_RANGE_PLAN: SemVer = SemVer::new(1, 0, 0);

const STAGE0_VALIDATE_SUCCESS_ID: &str = "gbf-codegen.stage0.validate.success";
const STAGE0_VALIDATE_FAILURE_ID: &str = "gbf-codegen.stage0.validate.failure";
const STAGE05_RESOLVE_SUCCESS_ID: &str = "gbf-codegen.stage0_5.resolve_policy.success";
const STAGE05_RESOLVE_FAILURE_ID: &str = "gbf-codegen.stage0_5.resolve_policy.failure";
const STAGE1_QUANT_GRAPH_SUCCESS_ID: &str = "gbf-codegen.stage1.quant_graph.success";
const STAGE1_QUANT_GRAPH_FAILURE_ID: &str = "gbf-codegen.stage1.quant_graph.failure";
const STAGE2_BUDGET_SUCCESS_ID: &str = "gbf-codegen.stage2.static_budget.success";
const STAGE2_BUDGET_FAILURE_ID: &str = "gbf-codegen.stage2.static_budget.failure";
const STAGE3_INFER_IR_SUCCESS_ID: &str = "gbf-codegen.stage3.infer_ir.success";
const STAGE3_INFER_IR_FAILURE_ID: &str = "gbf-codegen.stage3.infer_ir.failure";
const STAGE4_OBSERVATION_PLAN_SUCCESS_ID: &str = "observation_plan.v1.success";
const STAGE4_OBSERVATION_PLAN_FAILURE_ID: &str = "observation_plan.v1.failure";
const STAGE5_RANGE_PLAN_SUCCESS_ID: &str = "range_plan.v1.success";
const STAGE5_RANGE_PLAN_FAILURE_ID: &str = "range_plan.v1.failure";
pub const STAGE3_CACHE_LOOKUP_EVENT: &str = "stage3.cache.lookup";
pub const STAGE3_CACHE_AUDIT_REWRAP_EVENT: &str = "stage3.cache.audit_rewrap";
pub const STAGE4_DRIVER_CACHE_LOOKUP_EVENT: &str = "stage4.driver.cache_lookup";
pub const STAGE4_DRIVER_CACHE_HIT_EVENT: &str = "stage4.driver.cache_hit";
pub const STAGE4_DRIVER_CACHE_MISS_EVENT: &str = "stage4.driver.cache_miss";
pub const STAGE4_DRIVER_AUDIT_PARENT_REWRAP_EVENT: &str = "stage4.driver.audit_parent_rewrap";
pub const STAGE5_DRIVER_CACHE_LOOKUP_EVENT: &str = "stage5.driver.cache_lookup";
pub const STAGE5_DRIVER_CACHE_HIT_EVENT: &str = "stage5.driver.cache_hit";
pub const STAGE5_DRIVER_CACHE_MISS_EVENT: &str = "stage5.driver.cache_miss";
pub const STAGE5_DRIVER_AUDIT_PARENT_REWRAP_EVENT: &str = "stage5.driver.audit_parent_rewrap";

fn stage4_pass_version_string() -> String {
    PASS_VERSION_OBSERVATION_PLAN.to_string()
}

fn stage5_pass_version_string() -> String {
    PASS_VERSION_RANGE_PLAN.to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage0CellKind {
    Success,
    FailureMemo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage05CellKind {
    Success,
    FailureMemo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage1CellKind {
    Success,
    FailureMemo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage2CellKind {
    Success,
    FailureMemo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage3CellKind {
    Success,
    FailureMemo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage4CellKind {
    Success,
    FailureMemo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage5CellKind {
    Success,
    FailureMemo,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stage0CacheKeyMaterial {
    pub artifact_source_hash: Hash256,
    pub artifact_effective_core_hash: Option<Hash256>,
    pub artifact_manifest_hash: Option<Hash256>,
    pub artifact_aux_hash: Option<Hash256>,
    pub lowering_manifest_hash: Option<Hash256>,
    pub hint_bundle_hash: Hash256,
    pub compile_request_hash: Hash256,
    pub target_profile_hash: Hash256,
    pub compile_profile_hash: Hash256,
    pub calibration_hash: Option<Hash256>,
    pub compatibility_adapter_registry_hash: Hash256,
    pub crate_feature_set_hash: Hash256,
    pub artifact_validation_schema_hash: Hash256,
}

impl Stage0CacheKeyMaterial {
    #[must_use]
    pub fn success(
        input_hashes: ValidatedInputHashes,
        compatibility_adapter_registry_hash: Hash256,
    ) -> Self {
        Self {
            artifact_source_hash: input_hashes.artifact_source_hash,
            artifact_effective_core_hash: Some(input_hashes.artifact_effective_core_hash),
            artifact_manifest_hash: Some(input_hashes.artifact_manifest_hash),
            artifact_aux_hash: Some(input_hashes.artifact_aux_hash),
            lowering_manifest_hash: Some(input_hashes.lowering_manifest_hash),
            hint_bundle_hash: input_hashes.hint_bundle_hash,
            compile_request_hash: input_hashes.compile_request_hash,
            target_profile_hash: input_hashes.target_profile_hash,
            compile_profile_hash: input_hashes.compile_profile_hash,
            calibration_hash: Some(input_hashes.calibration_hash),
            compatibility_adapter_registry_hash,
            crate_feature_set_hash: crate_feature_set_hash(),
            artifact_validation_schema_hash: artifact_validation_schema_hash(),
        }
    }

    #[must_use]
    pub fn partial_failure(
        artifact_source_hash: Hash256,
        hint_bundle_hash: Hash256,
        compile_request_hash: Hash256,
        target_profile_hash: Hash256,
        compile_profile_hash: Hash256,
        compatibility_adapter_registry_hash: Hash256,
    ) -> Self {
        Self {
            artifact_source_hash,
            artifact_effective_core_hash: None,
            artifact_manifest_hash: None,
            artifact_aux_hash: None,
            lowering_manifest_hash: None,
            hint_bundle_hash,
            compile_request_hash,
            target_profile_hash,
            compile_profile_hash,
            calibration_hash: None,
            compatibility_adapter_registry_hash,
            crate_feature_set_hash: crate_feature_set_hash(),
            artifact_validation_schema_hash: artifact_validation_schema_hash(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stage05CacheKeyMaterial {
    pub artifact_validation_self_hash: Hash256,
    pub input_hashes: ValidatedInputHashes,
    pub target_defaults_hash: Hash256,
    pub compile_profile_hash: Hash256,
    pub profile_defaults_hash: Hash256,
    pub compile_objective_hash: Hash256,
    pub crate_feature_set_hash: Hash256,
    pub policy_resolution_schema_hash: Hash256,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stage2CacheKeyMaterial {
    pub policy_resolution_self_hash: Hash256,
    pub quant_graph_hash: Hash256,
    pub runtime_chrome_budget_hash: Option<Hash256>,
    pub target_profile_hash: Hash256,
    pub crate_feature_set_hash: Hash256,
    pub static_budget_schema_hash: Hash256,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stage1CacheKeyMaterial {
    pub artifact_validation_self_hash: Hash256,
    /// Stage 1 intentionally keys by the full policy-resolution report hash.
    /// A76: a cache hit therefore already has the report audit parent it was
    /// built with; unlike Stage 3, there is no policy-projection key or audit
    /// rewrap.
    pub policy_resolution_self_hash: Hash256,
    pub artifact_effective_core_hash: Hash256,
    pub lowering_manifest_hash: Hash256,
    pub resolved_blob_index_hash: Hash256,
    pub pass_version_quant_graph: String,
    pub crate_feature_set_hash: Hash256,
    pub quant_graph_schema_hash: Hash256,
}

impl Stage1CacheKeyMaterial {
    #[must_use]
    pub fn new(
        artifact_validation_self_hash: Hash256,
        policy_resolution_self_hash: Hash256,
        artifact_effective_core_hash: Hash256,
        lowering_manifest_hash: Hash256,
        resolved_blob_index_hash: Hash256,
    ) -> Self {
        Self {
            artifact_validation_self_hash,
            policy_resolution_self_hash,
            artifact_effective_core_hash,
            lowering_manifest_hash,
            resolved_blob_index_hash,
            pass_version_quant_graph: PASS_VERSION_QUANT_GRAPH.to_owned(),
            crate_feature_set_hash: crate_feature_set_hash(),
            quant_graph_schema_hash: quant_graph_schema_hash(),
        }
    }
}

impl Stage2CacheKeyMaterial {
    #[must_use]
    pub fn new(
        policy_resolution_self_hash: Hash256,
        quant_graph_hash: Hash256,
        runtime_chrome_budget_hash: Option<Hash256>,
        target_profile_hash: Hash256,
    ) -> Self {
        Self {
            policy_resolution_self_hash,
            quant_graph_hash,
            runtime_chrome_budget_hash,
            target_profile_hash,
            crate_feature_set_hash: crate_feature_set_hash(),
            static_budget_schema_hash: static_budget_schema_hash(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stage3CacheKeyMaterial {
    pub quant_graph_self_hash: Hash256,
    pub infer_ir_policy_projection_hash: Hash256,
    pub static_budget_self_hash: Hash256,
    pub pass_version_infer_ir: String,
    pub crate_feature_set_hash: Hash256,
    pub infer_ir_schema_hash: Hash256,
}

impl Stage3CacheKeyMaterial {
    #[must_use]
    pub fn new(
        quant_graph_self_hash: Hash256,
        infer_ir_policy_projection_hash: Hash256,
        static_budget_self_hash: Hash256,
    ) -> Self {
        Self {
            quant_graph_self_hash,
            infer_ir_policy_projection_hash,
            static_budget_self_hash,
            pass_version_infer_ir: PASS_VERSION_INFER_IR.to_owned(),
            crate_feature_set_hash: crate_feature_set_hash(),
            infer_ir_schema_hash: infer_ir_schema_hash(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stage4CacheKeyMaterial {
    pub infer_ir_self_hash: Hash256,
    pub quant_graph_self_hash: Hash256,
    pub semantic_checkpoint_schema_hash: Hash256,
    pub observation_policy_projection_hash: Hash256,
    pub pass_version_observation_plan: String,
    pub crate_feature_set_hash: Hash256,
    pub observation_plan_schema_hash: Hash256,
    pub build_active_semantic_checkpoint_schema_schema_hash: Hash256,
    pub operational_probe_schema_schema_hash: Hash256,
    pub probe_registry_hash: Hash256,
    pub metric_registry_hash: Hash256,
    pub trace_event_layout_registry_hash: Hash256,
}

impl Stage4CacheKeyMaterial {
    #[must_use]
    pub fn new(
        infer_ir_self_hash: Hash256,
        quant_graph_self_hash: Hash256,
        semantic_checkpoint_schema_hash: Hash256,
        observation_policy_projection_hash: Hash256,
        probe_registry_hash: Hash256,
        metric_registry_hash: Hash256,
        trace_event_layout_registry_hash: Hash256,
    ) -> Self {
        Self {
            infer_ir_self_hash,
            quant_graph_self_hash,
            semantic_checkpoint_schema_hash,
            observation_policy_projection_hash,
            pass_version_observation_plan: stage4_pass_version_string(),
            crate_feature_set_hash: crate_feature_set_hash(),
            observation_plan_schema_hash: observation_plan_schema_hash(),
            build_active_semantic_checkpoint_schema_schema_hash:
                build_active_semantic_checkpoint_schema_schema_hash(),
            operational_probe_schema_schema_hash: operational_probe_schema_schema_hash(),
            probe_registry_hash,
            metric_registry_hash,
            trace_event_layout_registry_hash,
        }
    }

    pub fn from_inputs(
        inputs: &crate::s4::observation_plan::ObservationPlanInputs,
    ) -> Result<Self, serde_json::Error> {
        Ok(Self::new(
            inputs.infer_ir_self_hash,
            inputs.quant_graph_self_hash,
            inputs.semantic_checkpoint_schema_hash,
            observation_policy_projection_hash(&inputs.op_policy_projection)?,
            inputs.probe_registry_hash,
            inputs.metric_registry_hash,
            inputs.trace_event_layout_registry_hash,
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stage5CacheKeyMaterial {
    pub infer_ir_self_hash: Hash256,
    pub quant_graph_self_hash: Hash256,
    pub static_budget_self_hash: Hash256,
    pub range_policy_projection_hash: Hash256,
    pub pass_version_range_plan: String,
    pub crate_feature_set_hash: Hash256,
    pub range_plan_schema_hash: Hash256,
    pub range_cert_schema_hash: Hash256,
}

impl Stage5CacheKeyMaterial {
    #[must_use]
    pub fn new(
        infer_ir_self_hash: Hash256,
        quant_graph_self_hash: Hash256,
        static_budget_self_hash: Hash256,
        range_policy_projection_hash: Hash256,
    ) -> Self {
        Self {
            infer_ir_self_hash,
            quant_graph_self_hash,
            static_budget_self_hash,
            range_policy_projection_hash,
            pass_version_range_plan: stage5_pass_version_string(),
            crate_feature_set_hash: crate_feature_set_hash(),
            range_plan_schema_hash: range_plan_schema_hash(),
            range_cert_schema_hash: range_cert_schema_hash(),
        }
    }

    pub fn from_inputs(
        inputs: &crate::s5::range_plan::RangePlanInputs,
    ) -> Result<Self, serde_json::Error> {
        Ok(Self::new(
            inputs.infer_ir_self_hash,
            inputs.quant_graph_self_hash,
            inputs.static_budget_self_hash,
            range_policy_projection_hash(&inputs.range_policy_projection)?,
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stage4ReportRewrapContext {
    pub profile_id: CompileProfileId,
    pub audit_parents: ObservationPlanAuditParents,
}

impl Stage4ReportRewrapContext {
    #[must_use]
    pub fn from_inputs(inputs: &crate::s4::observation_plan::ObservationPlanInputs) -> Self {
        Self {
            profile_id: inputs.op_policy_projection.profile_id.clone(),
            audit_parents: inputs.audit_parents.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stage5ReportRewrapContext {
    pub audit_parents: RangePlanAuditParents,
}

impl Stage5ReportRewrapContext {
    #[must_use]
    pub const fn new(audit_parents: RangePlanAuditParents) -> Self {
        Self { audit_parents }
    }

    #[must_use]
    pub const fn from_inputs(inputs: &crate::s5::range_plan::RangePlanInputs) -> Self {
        Self {
            audit_parents: inputs.audit_parents,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stage4FailureReplay {
    pub report: ReportEnvelope<ObservationPlanReportBody>,
    pub sc_re_emit_report: Option<ReportEnvelope<SemanticCheckpointSchemaReEmitBody>>,
    pub operational_probe_report: Option<ReportEnvelope<OperationalProbeSchemaBody>>,
    pub diagnostics: NonEmptyList<ValidationDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RangePlanCoreFailure {
    pub range_plan_body: RangePlanReportBody,
    pub range_cert_body: Option<RangeCertBody>,
    pub diagnostics: Vec<ValidationDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stage5FailureReplay {
    pub report: ReportEnvelope<RangePlanReportBody>,
    pub cert_report: Option<ReportEnvelope<RangeCertBody>>,
    pub diagnostics: Vec<ValidationDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CachedReportBytes {
    pub report_self_hash: Hash256,
    pub canonical_bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cell", deny_unknown_fields)]
pub enum Stage0CacheCell {
    ValidationSuccess {
        product: Box<CachedValidationProduct>,
        report: CachedReportBytes,
    },
    FailureMemo {
        report: CachedReportBytes,
        diagnostics: Vec<ValidationDiagnostic>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "cell", deny_unknown_fields)]
pub enum Stage05CacheCell {
    ResolvePolicySuccess {
        product: Box<ResolvedPolicyProduct>,
        report: CachedReportBytes,
    },
    FailureMemo {
        report: CachedReportBytes,
        diagnostics: Vec<ValidationDiagnostic>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cell", deny_unknown_fields)]
pub enum Stage1CacheCell {
    QuantGraphSuccess {
        product: Box<QuantGraphProduct>,
        report: CachedReportBytes,
    },
    FailureMemo {
        report: CachedReportBytes,
        diagnostics: Vec<ValidationDiagnostic>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "cell", deny_unknown_fields)]
pub enum Stage2CacheCell {
    StaticBudgetSuccess {
        product: Box<StaticBudgetReport>,
        report: CachedReportBytes,
    },
    FailureMemo {
        report: CachedReportBytes,
        diagnostics: Vec<ValidationDiagnostic>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cell", deny_unknown_fields)]
pub enum Stage3CacheCell {
    InferIrSuccess {
        product: Box<GbInferIRProduct>,
        report: CachedReportBytes,
    },
    FailureMemo {
        report: CachedReportBytes,
        diagnostics: Vec<ValidationDiagnostic>,
    },
}

#[derive(Debug)]
pub enum CodegenStageCacheError {
    Store(StageCacheError),
    Json(serde_json::Error),
    ReportEnvelope(ReportEnvelopeError),
    ReportSelfHash(ReportSelfHashError),
    CachedValidationProduct(CachedValidationProductRehydrateError),
    CachedReportBytes(CachedReportBytesError),
    CachedPolicyProduct(CachedPolicyProductError),
    UnexpectedCell {
        expected: &'static str,
        observed: &'static str,
    },
}

impl fmt::Display for CodegenStageCacheError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Store(err) => write!(f, "{err}"),
            Self::Json(err) => write!(f, "stage cache payload JSON error: {err}"),
            Self::ReportEnvelope(err) => write!(f, "stage cache report envelope error: {err}"),
            Self::ReportSelfHash(err) => write!(f, "stage cache report self-hash error: {err}"),
            Self::CachedValidationProduct(err) => write!(f, "{err}"),
            Self::CachedReportBytes(err) => write!(f, "{err}"),
            Self::CachedPolicyProduct(err) => write!(f, "{err}"),
            Self::UnexpectedCell { expected, observed } => {
                write!(f, "expected {expected} cache cell, observed {observed}")
            }
        }
    }
}

impl std::error::Error for CodegenStageCacheError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Store(err) => Some(err),
            Self::Json(err) => Some(err),
            Self::ReportEnvelope(err) => Some(err),
            Self::ReportSelfHash(err) => Some(err),
            Self::CachedValidationProduct(err) => Some(err),
            Self::CachedReportBytes(err) => Some(err),
            Self::CachedPolicyProduct(err) => Some(err),
            Self::UnexpectedCell { .. } => None,
        }
    }
}

impl From<StageCacheError> for CodegenStageCacheError {
    fn from(value: StageCacheError) -> Self {
        Self::Store(value)
    }
}

impl From<serde_json::Error> for CodegenStageCacheError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

impl From<ReportEnvelopeError> for CodegenStageCacheError {
    fn from(value: ReportEnvelopeError) -> Self {
        Self::ReportEnvelope(value)
    }
}

impl From<ReportSelfHashError> for CodegenStageCacheError {
    fn from(value: ReportSelfHashError) -> Self {
        Self::ReportSelfHash(value)
    }
}

impl From<CachedValidationProductRehydrateError> for CodegenStageCacheError {
    fn from(value: CachedValidationProductRehydrateError) -> Self {
        Self::CachedValidationProduct(value)
    }
}

impl From<CachedReportBytesError> for CodegenStageCacheError {
    fn from(value: CachedReportBytesError) -> Self {
        Self::CachedReportBytes(value)
    }
}

impl From<CachedPolicyProductError> for CodegenStageCacheError {
    fn from(value: CachedPolicyProductError) -> Self {
        Self::CachedPolicyProduct(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CachedReportBytesError {
    CanonicalBytesHashMismatch {
        canonical_bytes_hash: Hash256,
        expected_canonical_bytes_hash: Hash256,
    },
}

impl fmt::Display for CachedReportBytesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CanonicalBytesHashMismatch {
                canonical_bytes_hash,
                expected_canonical_bytes_hash,
            } => write!(
                f,
                "cached report canonical bytes hash mismatch: report bytes have {canonical_bytes_hash}, product has {expected_canonical_bytes_hash}"
            ),
        }
    }
}

impl std::error::Error for CachedReportBytesError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CachedPolicyProductError {
    ReportSelfHashMismatch {
        report_self_hash: Hash256,
        policy_resolution_self_hash: Hash256,
    },
    CachedReportSelfHashMismatch {
        cached_report_self_hash: Hash256,
        policy_resolution_self_hash: Hash256,
    },
    ReportSelfHashUncomputable {
        message: String,
    },
    ReportSemanticValidation {
        message: String,
    },
    UnexpectedReportOutcome {
        outcome: ReportOutcome,
    },
    CanonicalBytesUncomputable {
        message: String,
    },
    CanonicalBytesHashMismatch {
        canonical_bytes_hash: Hash256,
        policy_resolution_canonical_bytes_hash: Hash256,
    },
    KeyMaterialMismatch,
    ReportIdentityMismatch,
}

impl fmt::Display for CachedPolicyProductError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReportSelfHashMismatch {
                report_self_hash,
                policy_resolution_self_hash,
            } => write!(
                f,
                "cached policy report self-hash mismatch: report has {report_self_hash}, product has {policy_resolution_self_hash}"
            ),
            Self::CachedReportSelfHashMismatch {
                cached_report_self_hash,
                policy_resolution_self_hash,
            } => write!(
                f,
                "cached policy report bytes self-hash mismatch: report bytes have {cached_report_self_hash}, product has {policy_resolution_self_hash}"
            ),
            Self::ReportSelfHashUncomputable { message } => {
                write!(
                    f,
                    "cached policy report self-hash is not computable: {message}"
                )
            }
            Self::ReportSemanticValidation { message } => {
                write!(f, "cached policy report is semantically invalid: {message}")
            }
            Self::UnexpectedReportOutcome { outcome } => {
                write!(f, "cached policy success report has outcome {outcome:?}")
            }
            Self::CanonicalBytesUncomputable { message } => {
                write!(
                    f,
                    "cached policy report canonical bytes are not computable: {message}"
                )
            }
            Self::CanonicalBytesHashMismatch {
                canonical_bytes_hash,
                policy_resolution_canonical_bytes_hash,
            } => write!(
                f,
                "cached policy report canonical bytes hash mismatch: report has {canonical_bytes_hash}, product has {policy_resolution_canonical_bytes_hash}"
            ),
            Self::KeyMaterialMismatch => {
                f.write_str("cached policy product does not match Stage 0.5 key material")
            }
            Self::ReportIdentityMismatch => {
                f.write_str("cached policy report identity does not match cached product")
            }
        }
    }
}

impl std::error::Error for CachedPolicyProductError {}

#[must_use]
pub fn stage0_validation_store_key(
    material: &Stage0CacheKeyMaterial,
    cell_kind: Stage0CellKind,
) -> StageKey {
    stage0_validation_store_key_with_pass_version(material, cell_kind, PASS_VERSION_VALIDATE)
}

#[must_use]
pub fn stage0_validation_store_key_with_pass_version(
    material: &Stage0CacheKeyMaterial,
    cell_kind: Stage0CellKind,
    pass_version: SemVer,
) -> StageKey {
    stage_key(
        match cell_kind {
            Stage0CellKind::Success => STAGE0_VALIDATE_SUCCESS_ID,
            Stage0CellKind::FailureMemo => STAGE0_VALIDATE_FAILURE_ID,
        },
        material,
        pass_version,
    )
}

#[must_use]
pub fn stage05_resolve_policy_store_key(
    material: &Stage05CacheKeyMaterial,
    cell_kind: Stage05CellKind,
) -> StageKey {
    stage05_resolve_policy_store_key_with_pass_version(material, cell_kind, PASS_VERSION_RESOLVE)
}

#[must_use]
pub fn stage05_resolve_policy_store_key_with_pass_version(
    material: &Stage05CacheKeyMaterial,
    cell_kind: Stage05CellKind,
    pass_version: SemVer,
) -> StageKey {
    stage_key(
        match cell_kind {
            Stage05CellKind::Success => STAGE05_RESOLVE_SUCCESS_ID,
            Stage05CellKind::FailureMemo => STAGE05_RESOLVE_FAILURE_ID,
        },
        material,
        pass_version,
    )
}

#[must_use]
pub fn stage1_quant_graph_store_key(
    material: &Stage1CacheKeyMaterial,
    cell_kind: Stage1CellKind,
) -> StageKey {
    stage1_quant_graph_store_key_with_pass_version(
        material,
        cell_kind,
        PASS_VERSION_QUANT_GRAPH_STAGE1,
    )
}

#[must_use]
pub fn stage1_quant_graph_store_key_with_pass_version(
    material: &Stage1CacheKeyMaterial,
    cell_kind: Stage1CellKind,
    pass_version: SemVer,
) -> StageKey {
    stage_key(
        match cell_kind {
            Stage1CellKind::Success => STAGE1_QUANT_GRAPH_SUCCESS_ID,
            Stage1CellKind::FailureMemo => STAGE1_QUANT_GRAPH_FAILURE_ID,
        },
        material,
        pass_version,
    )
}

#[must_use]
pub fn stage2_static_budget_store_key(
    material: &Stage2CacheKeyMaterial,
    cell_kind: Stage2CellKind,
) -> StageKey {
    stage2_static_budget_store_key_with_pass_version(material, cell_kind, PASS_VERSION_BUDGET)
}

#[must_use]
pub fn stage2_static_budget_store_key_with_pass_version(
    material: &Stage2CacheKeyMaterial,
    cell_kind: Stage2CellKind,
    pass_version: SemVer,
) -> StageKey {
    stage_key(
        match cell_kind {
            Stage2CellKind::Success => STAGE2_BUDGET_SUCCESS_ID,
            Stage2CellKind::FailureMemo => STAGE2_BUDGET_FAILURE_ID,
        },
        material,
        pass_version,
    )
}

#[must_use]
pub fn stage3_infer_ir_store_key(
    material: &Stage3CacheKeyMaterial,
    cell_kind: Stage3CellKind,
) -> StageKey {
    stage3_infer_ir_store_key_with_pass_version(material, cell_kind, PASS_VERSION_INFER_IR_STAGE3)
}

#[must_use]
pub fn stage3_infer_ir_store_key_with_pass_version(
    material: &Stage3CacheKeyMaterial,
    cell_kind: Stage3CellKind,
    pass_version: SemVer,
) -> StageKey {
    stage_key(
        match cell_kind {
            Stage3CellKind::Success => STAGE3_INFER_IR_SUCCESS_ID,
            Stage3CellKind::FailureMemo => STAGE3_INFER_IR_FAILURE_ID,
        },
        material,
        pass_version,
    )
}

#[must_use]
pub fn stage4_observation_plan_store_key(
    material: &Stage4CacheKeyMaterial,
    cell_kind: Stage4CellKind,
) -> StageKey {
    stage4_observation_plan_store_key_with_pass_version(
        material,
        cell_kind,
        PASS_VERSION_OBSERVATION_PLAN,
    )
}

#[must_use]
pub fn stage4_observation_plan_store_key_with_pass_version(
    material: &Stage4CacheKeyMaterial,
    cell_kind: Stage4CellKind,
    pass_version: SemVer,
) -> StageKey {
    stage_key(
        match cell_kind {
            Stage4CellKind::Success => STAGE4_OBSERVATION_PLAN_SUCCESS_ID,
            Stage4CellKind::FailureMemo => STAGE4_OBSERVATION_PLAN_FAILURE_ID,
        },
        material,
        pass_version,
    )
}

#[must_use]
pub fn stage5_range_plan_store_key(
    material: &Stage5CacheKeyMaterial,
    cell_kind: Stage5CellKind,
) -> StageKey {
    stage5_range_plan_store_key_with_pass_version(material, cell_kind, PASS_VERSION_RANGE_PLAN)
}

#[must_use]
pub fn stage5_range_plan_store_key_with_pass_version(
    material: &Stage5CacheKeyMaterial,
    cell_kind: Stage5CellKind,
    pass_version: SemVer,
) -> StageKey {
    stage_key(
        match cell_kind {
            Stage5CellKind::Success => STAGE5_RANGE_PLAN_SUCCESS_ID,
            Stage5CellKind::FailureMemo => STAGE5_RANGE_PLAN_FAILURE_ID,
        },
        material,
        pass_version,
    )
}

pub fn get_stage0_success(
    cache: &StoreStageCache<'_>,
    material: &Stage0CacheKeyMaterial,
) -> Result<Option<Stage0CacheCell>, CodegenStageCacheError> {
    let Some(bytes) = cache.get(&stage0_validation_store_key(
        material,
        Stage0CellKind::Success,
    ))?
    else {
        return Ok(None);
    };
    let cell: Stage0CacheCell = serde_json::from_slice(&bytes)?;
    match &cell {
        Stage0CacheCell::ValidationSuccess { product, report } => {
            product.rehydrate_checked()?;
            validate_cached_report_bytes(
                report,
                product.artifact_validation_canonical_bytes_hash(),
            )?;
        }
        Stage0CacheCell::FailureMemo { .. } => {
            return Err(unexpected_stage0_cell("Stage 0 validation success", &cell));
        }
    }
    Ok(Some(cell))
}

pub fn get_stage0_failure_memo(
    cache: &StoreStageCache<'_>,
    material: &Stage0CacheKeyMaterial,
) -> Result<Option<Stage0CacheCell>, CodegenStageCacheError> {
    let Some(bytes) = cache.get(&stage0_validation_store_key(
        material,
        Stage0CellKind::FailureMemo,
    ))?
    else {
        return Ok(None);
    };
    let cell: Stage0CacheCell = serde_json::from_slice(&bytes)?;
    if !matches!(cell, Stage0CacheCell::FailureMemo { .. }) {
        return Err(unexpected_stage0_cell(
            "Stage 0 validation failure memo",
            &cell,
        ));
    }
    Ok(Some(cell))
}

pub fn put_stage0_success(
    cache: &StoreStageCache<'_>,
    material: &Stage0CacheKeyMaterial,
    product: impl Into<CachedValidationProduct>,
    report_bytes: Vec<u8>,
) -> Result<StageCacheKey, CodegenStageCacheError> {
    let product = product.into();
    debug_assert_eq!(
        product.report().report_self_hash,
        product.artifact_validation_self_hash(),
        "CachedValidationProduct self-hash mirrors its report envelope"
    );
    debug_assert_eq!(
        product.report().outcome,
        ReportOutcome::Passed,
        "Stage 0 success cells only store passed validation reports"
    );
    let report_self_hash = product.artifact_validation_self_hash();
    let cell = Stage0CacheCell::ValidationSuccess {
        product: Box::new(product),
        report: CachedReportBytes {
            report_self_hash,
            canonical_bytes: report_bytes,
        },
    };
    put_cell(
        cache,
        &stage0_validation_store_key(material, Stage0CellKind::Success),
        &cell,
    )
}

pub fn put_stage0_failure_memo(
    cache: &StoreStageCache<'_>,
    material: &Stage0CacheKeyMaterial,
    failure: &ValidationStageFailure,
    report_bytes: Vec<u8>,
) -> Result<StageCacheKey, CodegenStageCacheError> {
    debug_assert_eq!(
        failure.report.report_self_hash, failure.artifact_validation_self_hash,
        "ValidationStageFailure self-hash mirrors its report envelope"
    );
    let cell = Stage0CacheCell::FailureMemo {
        report: CachedReportBytes {
            report_self_hash: failure.report.report_self_hash,
            canonical_bytes: report_bytes,
        },
        diagnostics: failure.diagnostics.clone(),
    };
    put_cell(
        cache,
        &stage0_validation_store_key(material, Stage0CellKind::FailureMemo),
        &cell,
    )
}

pub fn get_stage05_success(
    cache: &StoreStageCache<'_>,
    material: &Stage05CacheKeyMaterial,
) -> Result<Option<Stage05CacheCell>, CodegenStageCacheError> {
    let Some(bytes) = cache.get(&stage05_resolve_policy_store_key(
        material,
        Stage05CellKind::Success,
    ))?
    else {
        return Ok(None);
    };
    let cell: Stage05CacheCell = serde_json::from_slice(&bytes)?;
    match &cell {
        Stage05CacheCell::ResolvePolicySuccess { product, report } => {
            validate_cached_policy_product(product, report, material)?;
        }
        Stage05CacheCell::FailureMemo { .. } => {
            return Err(unexpected_stage05_cell("Stage 0.5 policy success", &cell));
        }
    }
    Ok(Some(cell))
}

pub fn get_stage05_failure_memo(
    cache: &StoreStageCache<'_>,
    material: &Stage05CacheKeyMaterial,
) -> Result<Option<Stage05CacheCell>, CodegenStageCacheError> {
    let Some(bytes) = cache.get(&stage05_resolve_policy_store_key(
        material,
        Stage05CellKind::FailureMemo,
    ))?
    else {
        return Ok(None);
    };
    let cell: Stage05CacheCell = serde_json::from_slice(&bytes)?;
    if !matches!(cell, Stage05CacheCell::FailureMemo { .. }) {
        return Err(unexpected_stage05_cell(
            "Stage 0.5 policy failure memo",
            &cell,
        ));
    }
    Ok(Some(cell))
}

pub fn put_stage05_success(
    cache: &StoreStageCache<'_>,
    material: &Stage05CacheKeyMaterial,
    product: &ResolvedPolicyProduct,
    report_bytes: Vec<u8>,
) -> Result<StageCacheKey, CodegenStageCacheError> {
    debug_assert_eq!(
        product.report.report_self_hash, product.policy_resolution_self_hash,
        "ResolvedPolicyProduct self-hash mirrors its report envelope"
    );
    debug_assert_eq!(
        product.report.outcome,
        ReportOutcome::Passed,
        "Stage 0.5 success cells only store passed policy reports"
    );
    let cell = Stage05CacheCell::ResolvePolicySuccess {
        product: Box::new(product.clone()),
        report: CachedReportBytes {
            report_self_hash: product.policy_resolution_self_hash,
            canonical_bytes: report_bytes,
        },
    };
    put_cell(
        cache,
        &stage05_resolve_policy_store_key(material, Stage05CellKind::Success),
        &cell,
    )
}

pub fn put_stage05_failure_memo(
    cache: &StoreStageCache<'_>,
    material: &Stage05CacheKeyMaterial,
    failure: &PolicyResolutionStageFailure,
    report_bytes: Vec<u8>,
) -> Result<StageCacheKey, CodegenStageCacheError> {
    let cell = Stage05CacheCell::FailureMemo {
        report: CachedReportBytes {
            report_self_hash: failure.report.report_self_hash,
            canonical_bytes: report_bytes,
        },
        diagnostics: failure.diagnostics.clone(),
    };
    put_cell(
        cache,
        &stage05_resolve_policy_store_key(material, Stage05CellKind::FailureMemo),
        &cell,
    )
}

pub fn get_stage1_success(
    cache: &StoreStageCache<'_>,
    material: &Stage1CacheKeyMaterial,
) -> Result<Option<Stage1CacheCell>, CodegenStageCacheError> {
    let Some(bytes) = cache.get(&stage1_quant_graph_store_key(
        material,
        Stage1CellKind::Success,
    ))?
    else {
        return Ok(None);
    };
    let cell: Stage1CacheCell = serde_json::from_slice(&bytes)?;
    match &cell {
        Stage1CacheCell::QuantGraphSuccess { product, report } => {
            validate_cached_quant_graph_product(product, report, material)?;
        }
        Stage1CacheCell::FailureMemo { .. } => {
            return Err(CodegenStageCacheError::UnexpectedCell {
                expected: "Stage 1 quant_graph success",
                observed: "Stage 1 quant_graph failure memo",
            });
        }
    }
    Ok(Some(cell))
}

pub fn get_stage1_failure_memo(
    cache: &StoreStageCache<'_>,
    material: &Stage1CacheKeyMaterial,
) -> Result<Option<Stage1CacheCell>, CodegenStageCacheError> {
    let Some(bytes) = cache.get(&stage1_quant_graph_store_key(
        material,
        Stage1CellKind::FailureMemo,
    ))?
    else {
        return Ok(None);
    };
    let cell: Stage1CacheCell = serde_json::from_slice(&bytes)?;
    if !matches!(cell, Stage1CacheCell::FailureMemo { .. }) {
        return Err(CodegenStageCacheError::UnexpectedCell {
            expected: "Stage 1 quant_graph failure memo",
            observed: "Stage 1 quant_graph success",
        });
    }
    Ok(Some(cell))
}

pub fn put_stage1_success(
    cache: &StoreStageCache<'_>,
    material: &Stage1CacheKeyMaterial,
    product: &QuantGraphProduct,
    report_bytes: Vec<u8>,
) -> Result<StageCacheKey, CodegenStageCacheError> {
    debug_assert_eq!(
        product.report.outcome,
        ReportOutcome::Passed,
        "Stage 1 success cells only store passed quant_graph reports"
    );
    debug_assert_eq!(
        product
            .report
            .body
            .result
            .as_ref()
            .map(|result| result.quant_graph_self_hash),
        Some(product.quant_graph_self_hash),
        "Stage 1 report result carries the product quant_graph_self_hash"
    );
    let cell = Stage1CacheCell::QuantGraphSuccess {
        product: Box::new(product.clone()),
        report: CachedReportBytes {
            report_self_hash: product.report.report_self_hash,
            canonical_bytes: report_bytes,
        },
    };
    put_cell(
        cache,
        &stage1_quant_graph_store_key(material, Stage1CellKind::Success),
        &cell,
    )
}

pub fn put_stage1_failure_memo(
    cache: &StoreStageCache<'_>,
    material: &Stage1CacheKeyMaterial,
    report: CachedReportBytes,
    diagnostics: Vec<ValidationDiagnostic>,
) -> Result<StageCacheKey, CodegenStageCacheError> {
    let cell = Stage1CacheCell::FailureMemo {
        report,
        diagnostics,
    };
    put_cell(
        cache,
        &stage1_quant_graph_store_key(material, Stage1CellKind::FailureMemo),
        &cell,
    )
}

pub fn get_stage2_success(
    cache: &StoreStageCache<'_>,
    material: &Stage2CacheKeyMaterial,
) -> Result<Option<Stage2CacheCell>, CodegenStageCacheError> {
    let Some(bytes) = cache.get(&stage2_static_budget_store_key(
        material,
        Stage2CellKind::Success,
    ))?
    else {
        return Ok(None);
    };
    let cell: Stage2CacheCell = serde_json::from_slice(&bytes)?;
    if !matches!(cell, Stage2CacheCell::StaticBudgetSuccess { .. }) {
        return Err(unexpected_stage2_cell(
            "Stage 2 static budget success",
            &cell,
        ));
    }
    Ok(Some(cell))
}

pub fn get_stage2_failure_memo(
    cache: &StoreStageCache<'_>,
    material: &Stage2CacheKeyMaterial,
) -> Result<Option<Stage2CacheCell>, CodegenStageCacheError> {
    let Some(bytes) = cache.get(&stage2_static_budget_store_key(
        material,
        Stage2CellKind::FailureMemo,
    ))?
    else {
        return Ok(None);
    };
    let cell: Stage2CacheCell = serde_json::from_slice(&bytes)?;
    if !matches!(cell, Stage2CacheCell::FailureMemo { .. }) {
        return Err(unexpected_stage2_cell(
            "Stage 2 static budget failure memo",
            &cell,
        ));
    }
    Ok(Some(cell))
}

pub fn put_stage2_success(
    cache: &StoreStageCache<'_>,
    material: &Stage2CacheKeyMaterial,
    product: &StaticBudgetReport,
    report_bytes: Vec<u8>,
) -> Result<StageCacheKey, CodegenStageCacheError> {
    debug_assert_eq!(
        product.report.report_self_hash, product.static_budget_self_hash,
        "StaticBudgetReport self-hash mirrors its report envelope"
    );
    debug_assert_eq!(
        product.report.outcome,
        ReportOutcome::Passed,
        "Stage 2 success cells only store passed static-budget reports"
    );
    let cell = Stage2CacheCell::StaticBudgetSuccess {
        product: Box::new(product.clone()),
        report: CachedReportBytes {
            report_self_hash: product.static_budget_self_hash,
            canonical_bytes: report_bytes,
        },
    };
    put_cell(
        cache,
        &stage2_static_budget_store_key(material, Stage2CellKind::Success),
        &cell,
    )
}

pub fn put_stage2_failure_memo(
    cache: &StoreStageCache<'_>,
    material: &Stage2CacheKeyMaterial,
    failure: &StaticBudgetReport,
    report_bytes: Vec<u8>,
) -> Result<StageCacheKey, CodegenStageCacheError> {
    debug_assert_eq!(
        failure.report.report_self_hash, failure.static_budget_self_hash,
        "StaticBudgetReport self-hash mirrors its report envelope"
    );
    debug_assert_eq!(
        failure.report.outcome,
        ReportOutcome::Failed,
        "Stage 2 failure memos only store failed static-budget reports"
    );
    let cell = Stage2CacheCell::FailureMemo {
        report: CachedReportBytes {
            report_self_hash: failure.static_budget_self_hash,
            canonical_bytes: report_bytes,
        },
        diagnostics: failure.report.body.diagnostics.clone(),
    };
    put_cell(
        cache,
        &stage2_static_budget_store_key(material, Stage2CellKind::FailureMemo),
        &cell,
    )
}

pub fn get_stage3_success(
    cache: &StoreStageCache<'_>,
    material: &Stage3CacheKeyMaterial,
) -> Result<Option<Stage3CacheCell>, CodegenStageCacheError> {
    let key = stage3_infer_ir_store_key(material, Stage3CellKind::Success);
    let bytes = cache.get(&key)?;
    tracing::debug!(
        event = STAGE3_CACHE_LOOKUP_EVENT,
        stage_id = STAGE3_INFER_IR_SUCCESS_ID,
        cell_kind = ?Stage3CellKind::Success,
        hit = bytes.is_some(),
        "stage3.cache.lookup"
    );
    let Some(bytes) = bytes else {
        return Ok(None);
    };
    let cell: Stage3CacheCell = serde_json::from_slice(&bytes)?;
    match &cell {
        Stage3CacheCell::InferIrSuccess { product, report } => {
            validate_cached_infer_ir_product(product, report, material)?;
        }
        Stage3CacheCell::FailureMemo { .. } => {
            return Err(unexpected_stage3_cell("Stage 3 infer_ir success", &cell));
        }
    }
    Ok(Some(cell))
}

pub fn get_stage3_failure_memo(
    cache: &StoreStageCache<'_>,
    material: &Stage3CacheKeyMaterial,
) -> Result<Option<Stage3CacheCell>, CodegenStageCacheError> {
    let key = stage3_infer_ir_store_key(material, Stage3CellKind::FailureMemo);
    let bytes = cache.get(&key)?;
    tracing::debug!(
        event = STAGE3_CACHE_LOOKUP_EVENT,
        stage_id = STAGE3_INFER_IR_FAILURE_ID,
        cell_kind = ?Stage3CellKind::FailureMemo,
        hit = bytes.is_some(),
        "stage3.cache.lookup"
    );
    let Some(bytes) = bytes else {
        return Ok(None);
    };
    let cell: Stage3CacheCell = serde_json::from_slice(&bytes)?;
    if !matches!(cell, Stage3CacheCell::FailureMemo { .. }) {
        return Err(unexpected_stage3_cell(
            "Stage 3 infer_ir failure memo",
            &cell,
        ));
    }
    Ok(Some(cell))
}

pub fn put_stage3_success(
    cache: &StoreStageCache<'_>,
    material: &Stage3CacheKeyMaterial,
    product: &GbInferIRProduct,
    report_bytes: Vec<u8>,
) -> Result<StageCacheKey, CodegenStageCacheError> {
    debug_assert_eq!(
        product.report.outcome,
        ReportOutcome::Passed,
        "Stage 3 success cells only store passed infer_ir reports"
    );
    debug_assert_eq!(
        product
            .report
            .body
            .result
            .as_ref()
            .map(|result| result.infer_ir_self_hash),
        Some(product.infer_ir_self_hash),
        "Stage 3 report result carries the product infer_ir_self_hash"
    );
    let cell = Stage3CacheCell::InferIrSuccess {
        product: Box::new(product.clone()),
        report: CachedReportBytes {
            report_self_hash: product.report.report_self_hash,
            canonical_bytes: report_bytes,
        },
    };
    put_cell(
        cache,
        &stage3_infer_ir_store_key(material, Stage3CellKind::Success),
        &cell,
    )
}

pub fn put_stage3_failure_memo(
    cache: &StoreStageCache<'_>,
    material: &Stage3CacheKeyMaterial,
    report: CachedReportBytes,
    diagnostics: Vec<ValidationDiagnostic>,
) -> Result<StageCacheKey, CodegenStageCacheError> {
    let cell = Stage3CacheCell::FailureMemo {
        report,
        diagnostics,
    };
    put_cell(
        cache,
        &stage3_infer_ir_store_key(material, Stage3CellKind::FailureMemo),
        &cell,
    )
}

pub fn get_stage4_success(
    cache: &StoreStageCache<'_>,
    material: &Stage4CacheKeyMaterial,
) -> Result<Option<ObservationPlanCoreProduct>, CodegenStageCacheError> {
    let key = stage4_observation_plan_store_key(material, Stage4CellKind::Success);
    let replay_started = Instant::now();
    emit_stage4_cache_lookup(&key, Stage4CellKind::Success);
    let bytes = cache.get(&key)?;
    let Some(bytes) = bytes else {
        emit_stage4_cache_miss(&key, Stage4CellKind::Success);
        return Ok(None);
    };
    let product: ObservationPlanCoreProduct = serde_json::from_slice(&bytes)?;
    validate_cached_observation_plan_product(&product, material)?;
    emit_stage4_cache_hit(
        &key,
        Stage4CellKind::Success,
        replay_elapsed_ns(replay_started),
    );
    Ok(Some(product))
}

pub fn get_stage4_failure_memo(
    cache: &StoreStageCache<'_>,
    material: &Stage4CacheKeyMaterial,
) -> Result<Option<ObservationPlanCoreFailure>, CodegenStageCacheError> {
    let key = stage4_observation_plan_store_key(material, Stage4CellKind::FailureMemo);
    let replay_started = Instant::now();
    emit_stage4_cache_lookup(&key, Stage4CellKind::FailureMemo);
    let bytes = cache.get(&key)?;
    let Some(bytes) = bytes else {
        emit_stage4_cache_miss(&key, Stage4CellKind::FailureMemo);
        return Ok(None);
    };
    let failure: ObservationPlanCoreFailure = serde_json::from_slice(&bytes)?;
    emit_stage4_cache_hit(
        &key,
        Stage4CellKind::FailureMemo,
        replay_elapsed_ns(replay_started),
    );
    Ok(Some(failure))
}

pub fn put_stage4_success(
    cache: &StoreStageCache<'_>,
    material: &Stage4CacheKeyMaterial,
    product: &ObservationPlanCoreProduct,
) -> Result<StageCacheKey, CodegenStageCacheError> {
    validate_cached_observation_plan_product(product, material)?;
    put_cell(
        cache,
        &stage4_observation_plan_store_key(material, Stage4CellKind::Success),
        product,
    )
}

pub fn put_stage4_failure_memo(
    cache: &StoreStageCache<'_>,
    material: &Stage4CacheKeyMaterial,
    failure: &ObservationPlanCoreFailure,
) -> Result<StageCacheKey, CodegenStageCacheError> {
    put_cell(
        cache,
        &stage4_observation_plan_store_key(material, Stage4CellKind::FailureMemo),
        failure,
    )
}

pub fn get_stage5_success(
    cache: &StoreStageCache<'_>,
    material: &Stage5CacheKeyMaterial,
) -> Result<Option<RangePlanCoreProduct>, CodegenStageCacheError> {
    let key = stage5_range_plan_store_key(material, Stage5CellKind::Success);
    let replay_started = Instant::now();
    let bytes = cache.get(&key)?;
    emit_stage5_cache_lookup(&key, Stage5CellKind::Success);
    let Some(bytes) = bytes else {
        emit_stage5_cache_miss(&key, Stage5CellKind::Success);
        return Ok(None);
    };
    let product: RangePlanCoreProduct = serde_json::from_slice(&bytes)?;
    validate_cached_range_plan_product(&product, material)?;
    emit_stage5_cache_hit(
        &key,
        Stage5CellKind::Success,
        replay_elapsed_ns(replay_started),
    );
    Ok(Some(product))
}

pub fn get_stage5_failure_memo(
    cache: &StoreStageCache<'_>,
    material: &Stage5CacheKeyMaterial,
) -> Result<Option<RangePlanCoreFailure>, CodegenStageCacheError> {
    let key = stage5_range_plan_store_key(material, Stage5CellKind::FailureMemo);
    let replay_started = Instant::now();
    let bytes = cache.get(&key)?;
    emit_stage5_cache_lookup(&key, Stage5CellKind::FailureMemo);
    let Some(bytes) = bytes else {
        emit_stage5_cache_miss(&key, Stage5CellKind::FailureMemo);
        return Ok(None);
    };
    let failure: RangePlanCoreFailure = serde_json::from_slice(&bytes)?;
    emit_stage5_cache_hit(
        &key,
        Stage5CellKind::FailureMemo,
        replay_elapsed_ns(replay_started),
    );
    Ok(Some(failure))
}

pub fn put_stage5_success(
    cache: &StoreStageCache<'_>,
    material: &Stage5CacheKeyMaterial,
    product: &RangePlanCoreProduct,
) -> Result<StageCacheKey, CodegenStageCacheError> {
    validate_cached_range_plan_product(product, material)?;
    put_cell(
        cache,
        &stage5_range_plan_store_key(material, Stage5CellKind::Success),
        product,
    )
}

pub fn put_stage5_failure_memo(
    cache: &StoreStageCache<'_>,
    material: &Stage5CacheKeyMaterial,
    failure: &RangePlanCoreFailure,
) -> Result<StageCacheKey, CodegenStageCacheError> {
    put_cell(
        cache,
        &stage5_range_plan_store_key(material, Stage5CellKind::FailureMemo),
        failure,
    )
}

#[must_use]
pub fn materialize_stage0_cached_report(cell: &Stage0CacheCell) -> CachedReportBytes {
    match cell {
        Stage0CacheCell::ValidationSuccess { report, .. }
        | Stage0CacheCell::FailureMemo { report, .. } => report.clone(),
    }
}

#[must_use]
pub fn materialize_stage2_cached_report(cell: &Stage2CacheCell) -> CachedReportBytes {
    match cell {
        Stage2CacheCell::StaticBudgetSuccess { report, .. }
        | Stage2CacheCell::FailureMemo { report, .. } => report.clone(),
    }
}

#[must_use]
pub fn materialize_stage05_cached_report(cell: &Stage05CacheCell) -> CachedReportBytes {
    match cell {
        Stage05CacheCell::ResolvePolicySuccess { report, .. }
        | Stage05CacheCell::FailureMemo { report, .. } => report.clone(),
    }
}

#[must_use]
pub fn materialize_stage1_cached_report(cell: &Stage1CacheCell) -> CachedReportBytes {
    match cell {
        Stage1CacheCell::QuantGraphSuccess { report, .. }
        | Stage1CacheCell::FailureMemo { report, .. } => report.clone(),
    }
}

#[must_use]
pub fn materialize_stage3_cached_report(cell: &Stage3CacheCell) -> CachedReportBytes {
    match cell {
        Stage3CacheCell::InferIrSuccess { report, .. }
        | Stage3CacheCell::FailureMemo { report, .. } => report.clone(),
    }
}

pub fn rewrap_stage3_cached_report_audit_parents(
    report: &ReportEnvelope<infer_ir_v1::InferIrReportBody<GbInferIR>>,
    audit_parents: InferIrAuditParents,
) -> Result<
    ReportEnvelope<infer_ir_v1::InferIrReportBody<GbInferIR>>,
    gbf_report::ReportSelfHashError,
> {
    let pre_audit_hash = report.report_self_hash;
    let embedded_product_hash = report
        .body
        .result
        .as_ref()
        .map(|result| result.infer_ir_self_hash);
    let mut rewrapped = report.clone();
    rewrapped.body.input_identity.policy_resolution_self_hash =
        audit_parents.policy_resolution_self_hash;
    rewrapped.body.input_identity.compile_request_hash = audit_parents.compile_request_hash;
    rewrapped = rewrapped.with_computed_self_hash()?;
    tracing::debug!(
        event = STAGE3_CACHE_AUDIT_REWRAP_EVENT,
        pre_audit_hash = %pre_audit_hash,
        post_audit_hash = %rewrapped.report_self_hash,
        embedded_product_hash_unchanged = embedded_product_hash
            == rewrapped
                .body
                .result
                .as_ref()
                .map(|result| result.infer_ir_self_hash),
        "stage3.cache.audit_rewrap"
    );
    Ok(rewrapped)
}

pub fn rewrap_stage4_cached_success(
    product: &ObservationPlanCoreProduct,
    context: &Stage4ReportRewrapContext,
) -> Result<ObservationPlanStageOutput, CodegenStageCacheError> {
    let sc_re_emit_body = stage4_sc_re_emit_body(product, context);
    let sc_re_emit_report = report_envelope(ReportOutcome::Passed, sc_re_emit_body)?;
    let operational_probe_body = stage4_operational_probe_body(product, context);
    let operational_probe_report = report_envelope(ReportOutcome::Passed, operational_probe_body)?;
    let report_body = stage4_observation_report_body(
        product,
        context,
        sc_re_emit_report.report_self_hash,
        operational_probe_report.report_self_hash,
    );
    let report = report_envelope(ReportOutcome::Passed, report_body)?;

    // Success cache cells store audit-free core products, so there is no
    // cached compile-request hash to compare against during rewrap.
    tracing::debug!(
        event = STAGE4_DRIVER_AUDIT_PARENT_REWRAP_EVENT,
        current_compile_request_hash = %context.audit_parents.compile_request_hash
    );

    Ok(ObservationPlanStageOutput {
        product: product.clone(),
        report,
        sc_re_emit_report,
        operational_probe_report,
    })
}

pub fn rewrap_stage4_cached_failure(
    failure: &ObservationPlanCoreFailure,
    context: &Stage4ReportRewrapContext,
) -> Result<Stage4FailureReplay, CodegenStageCacheError> {
    let cached_compile_request_hash = failure
        .observation_plan_body
        .input_identity
        .compile_request_hash;
    let mut observation_plan_body = failure.observation_plan_body.clone();
    refresh_stage4_observation_input_identity(
        &mut observation_plan_body.input_identity,
        &context.audit_parents,
    );
    let report = report_envelope(ReportOutcome::Failed, observation_plan_body)?;

    let sc_re_emit_report = failure
        .sc_re_emit_body
        .as_ref()
        .map(|body| {
            let mut body = body.clone();
            body.input_identity.artifact_aux_hash = context.audit_parents.artifact_aux_hash;
            report_envelope(ReportOutcome::Failed, body)
        })
        .transpose()?;
    let operational_probe_report = failure
        .operational_probe_body
        .as_ref()
        .map(|body| report_envelope(ReportOutcome::Failed, body.clone()))
        .transpose()?;

    tracing::debug!(
        event = STAGE4_DRIVER_AUDIT_PARENT_REWRAP_EVENT,
        cached_compile_request_hash = %cached_compile_request_hash,
        current_compile_request_hash = %context.audit_parents.compile_request_hash
    );

    Ok(Stage4FailureReplay {
        report,
        sc_re_emit_report,
        operational_probe_report,
        diagnostics: failure.diagnostics.clone(),
    })
}

pub fn rewrap_stage5_cached_success(
    product: &RangePlanCoreProduct,
    context: &Stage5ReportRewrapContext,
) -> Result<RangePlanStageOutput, CodegenStageCacheError> {
    let cert_report = report_envelope(ReportOutcome::Passed, product.range_cert.clone())?;
    let report_body = stage5_range_plan_report_body(product, context, cert_report.report_self_hash);
    let report = report_envelope(ReportOutcome::Passed, report_body)?;

    // Success cache cells store audit-free core products, so there is no
    // cached compile-request hash to compare against during rewrap.
    tracing::info!(
        target: "gbf_codegen::s5",
        event = STAGE5_DRIVER_AUDIT_PARENT_REWRAP_EVENT,
        current_compile_request_hash = %context.audit_parents.compile_request_hash
    );

    Ok(RangePlanStageOutput {
        product: product.clone(),
        report,
        cert_report,
    })
}

pub fn rewrap_stage5_cached_failure(
    failure: &RangePlanCoreFailure,
    context: &Stage5ReportRewrapContext,
) -> Result<Stage5FailureReplay, CodegenStageCacheError> {
    let cached_compile_request_hash = failure.range_plan_body.input_identity.compile_request_hash;
    let mut range_plan_body = failure.range_plan_body.clone();
    refresh_stage5_range_input_identity(
        &mut range_plan_body.input_identity,
        &context.audit_parents,
    );
    let report = report_envelope(ReportOutcome::Failed, range_plan_body)?;
    let cert_report = failure
        .range_cert_body
        .as_ref()
        .map(|body| report_envelope(ReportOutcome::Failed, body.clone()))
        .transpose()?;

    tracing::info!(
        target: "gbf_codegen::s5",
        event = STAGE5_DRIVER_AUDIT_PARENT_REWRAP_EVENT,
        cached_compile_request_hash = %cached_compile_request_hash,
        current_compile_request_hash = %context.audit_parents.compile_request_hash
    );

    Ok(Stage5FailureReplay {
        report,
        cert_report,
        diagnostics: failure.diagnostics.clone(),
    })
}

#[must_use]
pub fn artifact_validation_schema_hash() -> Hash256 {
    schema_hash(
        artifact_validation_v1::SCHEMA_ID,
        artifact_validation_v1::SCHEMA_VERSION,
        "artifact_validation_v1::ArtifactValidationReportBody::validate_semantics",
    )
}

#[must_use]
pub fn policy_resolution_schema_hash() -> Hash256 {
    schema_hash(
        policy_resolution_v1::SCHEMA_ID,
        policy_resolution_v1::SCHEMA_VERSION,
        "policy_resolution_v1::PolicyResolutionReportBody::validate_semantics",
    )
}

#[must_use]
pub fn static_budget_schema_hash() -> Hash256 {
    schema_hash(
        static_budget_v1::SCHEMA_ID,
        static_budget_v1::SCHEMA_VERSION,
        "static_budget_v1::StaticBudgetReportBody::validate_semantics",
    )
}

#[must_use]
pub fn quant_graph_schema_hash() -> Hash256 {
    schema_hash(
        quant_graph_v1::SCHEMA_ID,
        quant_graph_v1::SCHEMA_VERSION,
        "quant_graph_v1::QuantGraphReportBody::validate_semantics",
    )
}

#[must_use]
pub fn infer_ir_schema_hash() -> Hash256 {
    schema_hash(
        infer_ir_v1::SCHEMA_ID,
        infer_ir_v1::SCHEMA_VERSION,
        "infer_ir_v1::InferIrReportBody::validate_semantics",
    )
}

#[must_use]
pub fn observation_plan_schema_hash() -> Hash256 {
    schema_hash(
        OBSERVATION_PLAN_SCHEMA_VERSION,
        crate::s4::observation_plan::OBSERVATION_REPORT_SCHEMA_SEMVER,
        "observation_plan::ObservationPlanReportBody::validate_semantics",
    )
}

#[must_use]
pub fn build_active_semantic_checkpoint_schema_schema_hash() -> Hash256 {
    schema_hash(
        BUILD_ACTIVE_CHECKPOINT_SCHEMA_VERSION,
        crate::s4::observation_plan::OBSERVATION_REPORT_SCHEMA_SEMVER,
        "observation_plan::SemanticCheckpointSchemaReEmitBody::validate_semantics",
    )
}

#[must_use]
pub fn operational_probe_schema_schema_hash() -> Hash256 {
    schema_hash(
        OPERATIONAL_PROBE_SCHEMA_VERSION,
        crate::s4::observation_plan::OBSERVATION_REPORT_SCHEMA_SEMVER,
        "observation_plan::OperationalProbeSchemaBody::validate_semantics",
    )
}

#[must_use]
pub fn range_plan_schema_hash() -> Hash256 {
    schema_hash(
        RANGE_PLAN_SCHEMA_VERSION,
        crate::s5::range_plan::RANGE_REPORT_SCHEMA_SEMVER,
        "range_plan::RangePlanReportBody::validate_semantics",
    )
}

#[must_use]
pub fn range_cert_schema_hash() -> Hash256 {
    schema_hash(
        RANGE_CERT_SCHEMA_VERSION,
        crate::s5::range_plan::RANGE_REPORT_SCHEMA_SEMVER,
        "range_plan::RangeCertBody::validate_semantics",
    )
}

#[must_use]
pub fn crate_feature_set_hash() -> Hash256 {
    #[derive(Serialize)]
    struct FeatureSet<'a> {
        crate_name: &'a str,
        features: BTreeSet<&'a str>,
    }

    canonical_hash(
        "gbf-codegen:crate-feature-set.v1",
        &FeatureSet {
            crate_name: "gbf-codegen",
            features: BTreeSet::new(),
        },
    )
}

fn put_cell<T: Serialize>(
    cache: &StoreStageCache<'_>,
    key: &StageKey,
    cell: &T,
) -> Result<StageCacheKey, CodegenStageCacheError> {
    let payload = serde_json::to_vec(cell)?;
    Ok(cache.put(key, &payload)?.key)
}

fn stage_key<T: Serialize>(stage_id: &str, material: &T, pass_version: SemVer) -> StageKey {
    let material_hash = canonical_hash("gbf-codegen:stage-cache-key-material.v1", material);
    StageKey {
        stage_id: StageId::from(stage_id),
        shard_local: ComponentDigestSet {
            components: BTreeMap::from([(ComponentId::from_static("key_material"), material_hash)]),
        },
        // F-B2 Stage 0/0.5 keys are wholly captured in the canonical material hash above.
        // `global` stays zero so gbf-store receives one deterministic component instead of
        // duplicating the same digest in both StageKey halves.
        global: Hash256::ZERO,
        feature_flags: BTreeSet::<FeatureFlag>::new(),
        pass_version,
    }
}

fn schema_hash(schema_id: &str, schema_version: &str, semantic_validator: &str) -> Hash256 {
    #[derive(Serialize)]
    struct SchemaMaterial<'a> {
        schema_id: &'a str,
        schema_version: &'a str,
        canonicalization: &'a str,
        semantic_validator: &'a str,
    }

    canonical_hash(
        "gbf-codegen:report-schema-hash.v1",
        &SchemaMaterial {
            schema_id,
            schema_version,
            canonicalization: "gbf-report::canonical_json",
            semantic_validator,
        },
    )
}

fn canonical_hash<T: Serialize>(domain: &str, value: &T) -> Hash256 {
    let encoded = serde_json::to_value(value).expect("stage-cache material serializes");
    let bytes = canonicalize_value(&encoded).expect("stage-cache material canonicalizes");
    let mut hasher = Sha256::new();
    hasher.update(domain.as_bytes());
    hasher.update([0]);
    hasher.update(bytes);
    Hash256::from_bytes(hasher.finalize().into())
}

fn report_envelope<B>(
    outcome: ReportOutcome,
    body: B,
) -> Result<ReportEnvelope<B>, CodegenStageCacheError>
where
    B: ReportBody + Serialize,
{
    Ok(ReportEnvelope::new(outcome, body)?.with_computed_self_hash()?)
}

fn validate_cached_observation_plan_product(
    product: &ObservationPlanCoreProduct,
    material: &Stage4CacheKeyMaterial,
) -> Result<(), CodegenStageCacheError> {
    let observation_plan_self_hash = observation_plan_self_hash(&product.observation_plan)
        .map_err(CodegenStageCacheError::Json)?;
    let build_active_checkpoint_schema_hash =
        crate::s4::observation_plan::build_active_checkpoint_schema_hash(
            &product.build_active_checkpoint_schema,
        )
        .map_err(CodegenStageCacheError::Json)?;
    let operational_probe_schema_hash = crate::s4::observation_plan::operational_probe_schema_hash(
        &product.operational_probe_schema,
    )
    .map_err(CodegenStageCacheError::Json)?;

    if product.observation_plan_self_hash != observation_plan_self_hash
        || product.build_active_checkpoint_schema_hash != build_active_checkpoint_schema_hash
        || product.operational_probe_schema_hash != operational_probe_schema_hash
    {
        return Err(CodegenStageCacheError::UnexpectedCell {
            expected: "hash-consistent Stage 4 observation_plan core product",
            observed: "Stage 4 observation_plan core product with stale body hash",
        });
    }

    let identity = &product.observation_plan.identity;
    if identity.infer_ir_self_hash != material.infer_ir_self_hash
        || identity.quant_graph_self_hash != material.quant_graph_self_hash
        || identity.semantic_checkpoint_schema_hash != material.semantic_checkpoint_schema_hash
        || identity.observation_policy_projection_hash
            != material.observation_policy_projection_hash
        || identity.probe_registry_hash != material.probe_registry_hash
        || identity.metric_registry_hash != material.metric_registry_hash
        || identity.trace_event_layout_registry_hash != material.trace_event_layout_registry_hash
    {
        return Err(CodegenStageCacheError::UnexpectedCell {
            expected: "Stage 4 key-compatible observation_plan core product",
            observed: "Stage 4 observation_plan core product for different key material",
        });
    }

    Ok(())
}

fn validate_cached_range_plan_product(
    product: &RangePlanCoreProduct,
    material: &Stage5CacheKeyMaterial,
) -> Result<(), CodegenStageCacheError> {
    let range_plan_self_hash =
        range_plan_self_hash(&product.range_plan).map_err(CodegenStageCacheError::Json)?;
    let range_cert_body_hash =
        range_cert_body_hash(&product.range_cert).map_err(CodegenStageCacheError::Json)?;

    if product.range_plan_self_hash != range_plan_self_hash
        || product.range_cert_body_hash != range_cert_body_hash
    {
        return Err(CodegenStageCacheError::UnexpectedCell {
            expected: "hash-consistent Stage 5 range_plan core product",
            observed: "Stage 5 range_plan core product with stale body hash",
        });
    }

    let identity = product.range_plan.identity;
    if identity.infer_ir_self_hash != material.infer_ir_self_hash
        || identity.quant_graph_self_hash != material.quant_graph_self_hash
        || identity.static_budget_self_hash != material.static_budget_self_hash
        || identity.range_policy_projection_hash != material.range_policy_projection_hash
    {
        return Err(CodegenStageCacheError::UnexpectedCell {
            expected: "Stage 5 key-compatible range_plan core product",
            observed: "Stage 5 range_plan core product for different key material",
        });
    }

    if product.range_cert.identity.infer_ir_self_hash != material.infer_ir_self_hash
        || product.range_cert.identity.quant_graph_self_hash != material.quant_graph_self_hash
        || product.range_cert.identity.static_budget_self_hash != material.static_budget_self_hash
        || product.range_cert.identity.range_plan_self_hash != Some(product.range_plan_self_hash)
    {
        return Err(CodegenStageCacheError::UnexpectedCell {
            expected: "Stage 5 range certificate body bound to cached range_plan product",
            observed: "Stage 5 range certificate body for different product identity",
        });
    }

    Ok(())
}

fn stage4_observation_report_body(
    product: &ObservationPlanCoreProduct,
    context: &Stage4ReportRewrapContext,
    sc_re_emit_report_self_hash: Hash256,
    operational_probe_schema_report_self_hash: Hash256,
) -> ObservationPlanReportBody {
    let mut per_class_probe_count = PerClassCount::default();
    for probe in &product.observation_plan.probes {
        increment_importance_count(&mut per_class_probe_count, probe.importance);
    }
    let mut per_class_metric_count = PerClassCount::default();
    for metric in &product.observation_plan.metrics {
        increment_importance_count(&mut per_class_metric_count, metric.importance);
    }
    let mandatory_semantic_count = product
        .observation_plan
        .semantic
        .iter()
        .filter(|semantic| semantic.artifact_role == SemanticCheckpointRole::Mandatory)
        .count();
    let optional_semantic_count = product
        .observation_plan
        .semantic
        .iter()
        .filter(|semantic| semantic.artifact_role == SemanticCheckpointRole::Optional)
        .count();

    ObservationPlanReportBody {
        input_identity: stage4_observation_input_identity(product, context),
        result: Some(ObservationPlanReportResult {
            product: product.observation_plan.clone(),
            semantic_count: checked_u16(product.observation_plan.semantic.len(), "semantic_count"),
            probe_count: checked_u16(product.observation_plan.probes.len(), "probe_count"),
            metric_count: checked_u16(product.observation_plan.metrics.len(), "metric_count"),
            mandatory_semantic_count: checked_u16(
                mandatory_semantic_count,
                "mandatory_semantic_count",
            ),
            optional_semantic_count: checked_u16(
                optional_semantic_count,
                "optional_semantic_count",
            ),
            per_class_probe_count,
            per_class_metric_count,
            sc_re_emit_report_self_hash,
            operational_probe_schema_report_self_hash,
            observation_plan_self_hash: product.observation_plan_self_hash,
        }),
        diagnostics: Vec::new(),
    }
}

fn stage4_observation_input_identity(
    product: &ObservationPlanCoreProduct,
    context: &Stage4ReportRewrapContext,
) -> ObservationPlanReportInputIdentity {
    let identity = &product.observation_plan.identity;
    ObservationPlanReportInputIdentity {
        infer_ir_self_hash: identity.infer_ir_self_hash,
        quant_graph_self_hash: identity.quant_graph_self_hash,
        semantic_checkpoint_schema_hash: identity.semantic_checkpoint_schema_hash,
        observation_policy_projection_hash: identity.observation_policy_projection_hash,
        static_budget_self_hash: context.audit_parents.static_budget_self_hash,
        policy_resolution_self_hash: context.audit_parents.policy_resolution_self_hash,
        compile_request_hash: context.audit_parents.compile_request_hash,
        artifact_aux_hash: context.audit_parents.artifact_aux_hash,
        determinism: identity.determinism,
        observability_mode: identity.observability_mode,
        trace_budget: identity.trace_budget,
        profile_id: context.profile_id.clone(),
        workload_id: identity.workload_id.clone(),
    }
}

fn stage4_sc_re_emit_body(
    product: &ObservationPlanCoreProduct,
    context: &Stage4ReportRewrapContext,
) -> SemanticCheckpointSchemaReEmitBody {
    let identity = &product.observation_plan.identity;
    SemanticCheckpointSchemaReEmitBody {
        input_identity: SemanticCheckpointSchemaReEmitInputIdentity {
            observation_plan_self_hash: Some(product.observation_plan_self_hash),
            original_schema_hash: identity.semantic_checkpoint_schema_hash,
            infer_ir_self_hash: identity.infer_ir_self_hash,
            quant_graph_self_hash: identity.quant_graph_self_hash,
            artifact_aux_hash: context.audit_parents.artifact_aux_hash,
            determinism: identity.determinism,
            workload_id: identity.workload_id.clone(),
        },
        result: Some(SemanticCheckpointSchemaReEmitResult {
            schema_hash: product.build_active_checkpoint_schema_hash,
            checkpoints: product.build_active_checkpoint_schema.checkpoints.clone(),
            build_active_count: product.build_active_checkpoint_schema.build_active_count,
            mandatory_count: product.build_active_checkpoint_schema.mandatory_count,
            optional_count: product.build_active_checkpoint_schema.optional_count,
        }),
        diagnostics: Vec::new(),
    }
}

fn stage4_operational_probe_body(
    product: &ObservationPlanCoreProduct,
    context: &Stage4ReportRewrapContext,
) -> OperationalProbeSchemaBody {
    let identity = &product.observation_plan.identity;
    OperationalProbeSchemaBody {
        input_identity: OperationalProbeSchemaInputIdentity {
            observation_plan_self_hash: Some(product.observation_plan_self_hash),
            infer_ir_self_hash: identity.infer_ir_self_hash,
            quant_graph_self_hash: identity.quant_graph_self_hash,
            determinism: identity.determinism,
            observability_mode: identity.observability_mode,
            trace_budget: identity.trace_budget,
            profile_id: context.profile_id.clone(),
            workload_id: identity.workload_id.clone(),
        },
        result: Some(OperationalProbeSchemaResult {
            schema_hash: product.operational_probe_schema_hash,
            probes: product.operational_probe_schema.probes.clone(),
            metrics: product.operational_probe_schema.metrics.clone(),
            probe_count: product.operational_probe_schema.probe_count,
            metric_count: product.operational_probe_schema.metric_count,
            per_class_probe_weight_total: product
                .operational_probe_schema
                .per_class_probe_weight_total,
            per_class_metric_weight_total: product
                .operational_probe_schema
                .per_class_metric_weight_total,
            per_class_total_weight: product.operational_probe_schema.per_class_total_weight,
        }),
        diagnostics: Vec::new(),
    }
}

fn stage5_range_plan_report_body(
    product: &RangePlanCoreProduct,
    context: &Stage5ReportRewrapContext,
    range_cert_report_self_hash: Hash256,
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
        input_identity: stage5_range_input_identity(product, context),
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
        diagnostics: Vec::new(),
    }
}

fn stage5_range_input_identity(
    product: &RangePlanCoreProduct,
    context: &Stage5ReportRewrapContext,
) -> RangePlanReportInputIdentity {
    let identity = product.range_plan.identity;
    RangePlanReportInputIdentity {
        infer_ir_self_hash: identity.infer_ir_self_hash,
        quant_graph_self_hash: identity.quant_graph_self_hash,
        static_budget_self_hash: identity.static_budget_self_hash,
        range_policy_projection_hash: identity.range_policy_projection_hash,
        policy_resolution_self_hash: context.audit_parents.policy_resolution_self_hash,
        compile_request_hash: context.audit_parents.compile_request_hash,
        artifact_aux_hash: context.audit_parents.artifact_aux_hash,
        determinism: identity.determinism,
    }
}

fn refresh_stage4_observation_input_identity(
    identity: &mut ObservationPlanReportInputIdentity,
    audit_parents: &ObservationPlanAuditParents,
) {
    identity.static_budget_self_hash = audit_parents.static_budget_self_hash;
    identity.policy_resolution_self_hash = audit_parents.policy_resolution_self_hash;
    identity.compile_request_hash = audit_parents.compile_request_hash;
    identity.artifact_aux_hash = audit_parents.artifact_aux_hash;
}

fn refresh_stage5_range_input_identity(
    identity: &mut RangePlanReportInputIdentity,
    audit_parents: &RangePlanAuditParents,
) {
    identity.policy_resolution_self_hash = audit_parents.policy_resolution_self_hash;
    identity.compile_request_hash = audit_parents.compile_request_hash;
    identity.artifact_aux_hash = audit_parents.artifact_aux_hash;
}

fn increment_importance_count(count: &mut PerClassCount, importance: ProbeImportanceClass) {
    match importance {
        ProbeImportanceClass::Required => count.required += 1,
        ProbeImportanceClass::Important => count.important += 1,
        ProbeImportanceClass::Diagnostic => count.diagnostic += 1,
        ProbeImportanceClass::BestEffort => count.best_effort += 1,
    }
}

fn checked_u16(value: usize, field: &'static str) -> u16 {
    u16::try_from(value).unwrap_or_else(|_| panic!("Stage 4 {field} fits u16"))
}

fn replay_elapsed_ns(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

fn emit_stage4_cache_lookup(key: &StageKey, cell_kind: Stage4CellKind) {
    let k4 = compose_key(key);
    tracing::debug!(
        event = STAGE4_DRIVER_CACHE_LOOKUP_EVENT,
        k4 = %k4,
        cell_kind = ?cell_kind
    );
}

fn emit_stage4_cache_hit(key: &StageKey, cell_kind: Stage4CellKind, replay_ns: u64) {
    let k4 = compose_key(key);
    tracing::debug!(
        event = STAGE4_DRIVER_CACHE_HIT_EVENT,
        k4 = %k4,
        cell_kind = ?cell_kind,
        replay_ns = replay_ns
    );
}

fn emit_stage4_cache_miss(key: &StageKey, cell_kind: Stage4CellKind) {
    let k4 = compose_key(key);
    tracing::debug!(
        event = STAGE4_DRIVER_CACHE_MISS_EVENT,
        k4 = %k4,
        cell_kind = ?cell_kind
    );
}

fn emit_stage5_cache_lookup(key: &StageKey, cell_kind: Stage5CellKind) {
    let k5 = compose_key(key);
    tracing::debug!(
        target: "gbf_codegen::s5",
        event = STAGE5_DRIVER_CACHE_LOOKUP_EVENT,
        k5 = %k5,
        cell_kind = ?cell_kind
    );
}

fn emit_stage5_cache_hit(key: &StageKey, cell_kind: Stage5CellKind, replay_ns: u64) {
    let k5 = compose_key(key);
    tracing::debug!(
        target: "gbf_codegen::s5",
        event = STAGE5_DRIVER_CACHE_HIT_EVENT,
        k5 = %k5,
        cell_kind = ?cell_kind,
        replay_ns = replay_ns
    );
}

fn emit_stage5_cache_miss(key: &StageKey, cell_kind: Stage5CellKind) {
    let k5 = compose_key(key);
    tracing::debug!(
        target: "gbf_codegen::s5",
        event = STAGE5_DRIVER_CACHE_MISS_EVENT,
        k5 = %k5,
        cell_kind = ?cell_kind
    );
}

fn validate_cached_policy_product(
    product: &ResolvedPolicyProduct,
    cached_report: &CachedReportBytes,
    material: &Stage05CacheKeyMaterial,
) -> Result<(), CachedPolicyProductError> {
    if product.report.report_self_hash != product.policy_resolution_self_hash {
        return Err(CachedPolicyProductError::ReportSelfHashMismatch {
            report_self_hash: product.report.report_self_hash,
            policy_resolution_self_hash: product.policy_resolution_self_hash,
        });
    }
    if cached_report.report_self_hash != product.policy_resolution_self_hash {
        return Err(CachedPolicyProductError::CachedReportSelfHashMismatch {
            cached_report_self_hash: cached_report.report_self_hash,
            policy_resolution_self_hash: product.policy_resolution_self_hash,
        });
    }
    let computed_self_hash = compute_self_hash(&product.report).map_err(|err| {
        CachedPolicyProductError::ReportSelfHashUncomputable {
            message: err.to_string(),
        }
    })?;
    if computed_self_hash != product.policy_resolution_self_hash {
        return Err(CachedPolicyProductError::ReportSelfHashMismatch {
            report_self_hash: computed_self_hash,
            policy_resolution_self_hash: product.policy_resolution_self_hash,
        });
    }
    product
        .report
        .body
        .validate_semantics(product.report.outcome)
        .map_err(|err| CachedPolicyProductError::ReportSemanticValidation {
            message: format!("{err:?}"),
        })?;
    if product.report.outcome != ReportOutcome::Passed {
        return Err(CachedPolicyProductError::UnexpectedReportOutcome {
            outcome: product.report.outcome,
        });
    }

    let canonical_bytes = canonicalize_report(&product.report).map_err(|err| {
        CachedPolicyProductError::CanonicalBytesUncomputable {
            message: err.to_string(),
        }
    })?;
    let canonical_bytes_hash = Hash256::from_bytes(Sha256::digest(&canonical_bytes).into());
    if canonical_bytes_hash != product.policy_resolution_canonical_bytes_hash {
        return Err(CachedPolicyProductError::CanonicalBytesHashMismatch {
            canonical_bytes_hash,
            policy_resolution_canonical_bytes_hash: product.policy_resolution_canonical_bytes_hash,
        });
    }
    validate_cached_report_bytes(
        cached_report,
        product.policy_resolution_canonical_bytes_hash,
    )
    .map_err(|err| match err {
        CachedReportBytesError::CanonicalBytesHashMismatch {
            canonical_bytes_hash,
            expected_canonical_bytes_hash,
        } => CachedPolicyProductError::CanonicalBytesHashMismatch {
            canonical_bytes_hash,
            policy_resolution_canonical_bytes_hash: expected_canonical_bytes_hash,
        },
    })?;

    if product.artifact_validation_self_hash != material.artifact_validation_self_hash
        || product.input_hashes != material.input_hashes
        || product.policy.provenance.target_defaults != material.target_defaults_hash
        || product.input_hashes.compile_profile_hash != material.compile_profile_hash
        || product.policy.provenance.profile_defaults != material.profile_defaults_hash
    {
        return Err(CachedPolicyProductError::KeyMaterialMismatch);
    }
    if !cached_policy_report_identity_matches_product(product) {
        return Err(CachedPolicyProductError::ReportIdentityMismatch);
    }

    Ok(())
}

fn validate_cached_report_bytes(
    cached_report: &CachedReportBytes,
    expected_canonical_bytes_hash: Hash256,
) -> Result<(), CachedReportBytesError> {
    let canonical_bytes_hash =
        Hash256::from_bytes(Sha256::digest(&cached_report.canonical_bytes).into());
    if canonical_bytes_hash != expected_canonical_bytes_hash {
        return Err(CachedReportBytesError::CanonicalBytesHashMismatch {
            canonical_bytes_hash,
            expected_canonical_bytes_hash,
        });
    }
    Ok(())
}

fn validate_cached_quant_graph_product(
    product: &QuantGraphProduct,
    cached_report: &CachedReportBytes,
    material: &Stage1CacheKeyMaterial,
) -> Result<(), CodegenStageCacheError> {
    if cached_report.report_self_hash != product.report.report_self_hash
        || product
            .report
            .body
            .result
            .as_ref()
            .map(|result| result.quant_graph_self_hash)
            != Some(product.quant_graph_self_hash)
    {
        return Err(CodegenStageCacheError::CachedReportBytes(
            CachedReportBytesError::CanonicalBytesHashMismatch {
                canonical_bytes_hash: cached_report.report_self_hash,
                expected_canonical_bytes_hash: product.report.report_self_hash,
            },
        ));
    }
    product
        .report
        .body
        .validate_semantics(product.report.outcome)
        .map_err(|_| CodegenStageCacheError::UnexpectedCell {
            expected: "semantically valid Stage 1 quant_graph success",
            observed: "semantically invalid Stage 1 quant_graph success",
        })?;
    if product.report.outcome != ReportOutcome::Passed {
        return Err(CodegenStageCacheError::UnexpectedCell {
            expected: "passed Stage 1 quant_graph success",
            observed: "failed Stage 1 quant_graph report",
        });
    }
    let canonical_bytes = canonicalize_report(&product.report).map_err(|err| {
        CodegenStageCacheError::Json(serde_json::Error::io(std::io::Error::other(
            err.to_string(),
        )))
    })?;
    let canonical_bytes_hash = Hash256::from_bytes(Sha256::digest(&canonical_bytes).into());
    validate_cached_report_bytes(cached_report, canonical_bytes_hash)?;
    if product
        .report
        .body
        .input_identity
        .artifact_validation_self_hash
        != material.artifact_validation_self_hash
        || product
            .report
            .body
            .input_identity
            .policy_resolution_self_hash
            != material.policy_resolution_self_hash
        || product.report.body.input_identity.semantic_core_hash
            != material.artifact_effective_core_hash
        || product.report.body.input_identity.lowering_manifest_hash
            != material.lowering_manifest_hash
        || product.report.body.input_identity.resolved_blob_index_hash
            != material.resolved_blob_index_hash
    {
        return Err(CodegenStageCacheError::UnexpectedCell {
            expected: "Stage 1 key-compatible quant_graph product",
            observed: "Stage 1 quant_graph product for different key material",
        });
    }
    Ok(())
}

fn validate_cached_infer_ir_product(
    product: &GbInferIRProduct,
    cached_report: &CachedReportBytes,
    material: &Stage3CacheKeyMaterial,
) -> Result<(), CodegenStageCacheError> {
    if cached_report.report_self_hash != product.report.report_self_hash
        || product
            .report
            .body
            .result
            .as_ref()
            .map(|result| result.infer_ir_self_hash)
            != Some(product.infer_ir_self_hash)
    {
        return Err(CodegenStageCacheError::CachedReportBytes(
            CachedReportBytesError::CanonicalBytesHashMismatch {
                canonical_bytes_hash: cached_report.report_self_hash,
                expected_canonical_bytes_hash: product.report.report_self_hash,
            },
        ));
    }
    product
        .report
        .body
        .validate_semantics(product.report.outcome)
        .map_err(|_| CodegenStageCacheError::UnexpectedCell {
            expected: "semantically valid Stage 3 infer_ir success",
            observed: "semantically invalid Stage 3 infer_ir success",
        })?;
    if product.report.outcome != ReportOutcome::Passed {
        return Err(CodegenStageCacheError::UnexpectedCell {
            expected: "passed Stage 3 infer_ir success",
            observed: "failed Stage 3 infer_ir report",
        });
    }
    let canonical_bytes = canonicalize_report(&product.report).map_err(|err| {
        CodegenStageCacheError::Json(serde_json::Error::io(std::io::Error::other(
            err.to_string(),
        )))
    })?;
    let canonical_bytes_hash = Hash256::from_bytes(Sha256::digest(&canonical_bytes).into());
    validate_cached_report_bytes(cached_report, canonical_bytes_hash)?;
    if product.infer_ir.identity.quant_graph_self_hash != material.quant_graph_self_hash
        || product.infer_ir.identity.infer_ir_policy_projection_hash
            != material.infer_ir_policy_projection_hash
        || product.infer_ir.identity.static_budget_self_hash != material.static_budget_self_hash
    {
        return Err(CodegenStageCacheError::UnexpectedCell {
            expected: "Stage 3 key-compatible infer_ir product",
            observed: "Stage 3 infer_ir product for different key material",
        });
    }
    Ok(())
}

fn cached_policy_report_identity_matches_product(product: &ResolvedPolicyProduct) -> bool {
    let input_hashes = product.input_hashes;
    let report = &product.report.body;
    let Some(result) = &report.result else {
        return false;
    };
    let expected_resolved = policy_resolution_v1::ResolvedSection::from(&product.policy);
    let expected_compile_knobs =
        policy_resolution_v1::CompileKnobsSection::from(&product.policy.knobs);
    let provenance = &product.policy.provenance;

    report.artifact_identity.artifact_core_hash == input_hashes.artifact_effective_core_hash
        && report.artifact_identity.artifact_manifest_hash == input_hashes.artifact_manifest_hash
        && report.artifact_identity.lowering_manifest_hash == input_hashes.lowering_manifest_hash
        && report.artifact_identity.hint_bundle_hash == input_hashes.hint_bundle_hash
        && report.compile_request.compile_request_hash == input_hashes.compile_request_hash
        && report.compile_request.target == product.policy.target
        && report.compile_request.target_profile_hash == input_hashes.target_profile_hash
        && report.compile_request.profile == product.policy.profile
        && report.compile_request.objective == product.policy.objective
        && report.compile_request.required_features
            == product.policy.effective_constraints.required_features
        && report.compile_request.requested_runtime_modes == product.policy.requested_runtime_modes
        && report.compile_request.requested_runtime_modes
            == product.policy.effective_constraints.requested_runtime_modes
        && report.compile_request.calibration_hash == input_hashes.calibration_hash
        && result.provenance.hint_bundle_hash == input_hashes.hint_bundle_hash
        && result.provenance.compile_request_hash == input_hashes.compile_request_hash
        && result.provenance.calibration_hash == input_hashes.calibration_hash
        && result.provenance.target_defaults == provenance.target_defaults
        && result.provenance.profile_defaults == provenance.profile_defaults
        && provenance.hint_bundle_hash == Some(result.provenance.hint_bundle_hash)
        && provenance.compile_request_hash == result.provenance.compile_request_hash
        && provenance.calibration_hash == Some(result.provenance.calibration_hash)
        && result.resolved == expected_resolved
        && result.compile_knobs == expected_compile_knobs
}

fn unexpected_stage0_cell(
    expected: &'static str,
    observed: &Stage0CacheCell,
) -> CodegenStageCacheError {
    CodegenStageCacheError::UnexpectedCell {
        expected,
        observed: match observed {
            Stage0CacheCell::ValidationSuccess { .. } => "Stage 0 validation success",
            Stage0CacheCell::FailureMemo { .. } => "Stage 0 validation failure memo",
        },
    }
}

fn unexpected_stage05_cell(
    expected: &'static str,
    observed: &Stage05CacheCell,
) -> CodegenStageCacheError {
    CodegenStageCacheError::UnexpectedCell {
        expected,
        observed: match observed {
            Stage05CacheCell::ResolvePolicySuccess { .. } => "Stage 0.5 policy success",
            Stage05CacheCell::FailureMemo { .. } => "Stage 0.5 policy failure memo",
        },
    }
}

fn unexpected_stage2_cell(
    expected: &'static str,
    observed: &Stage2CacheCell,
) -> CodegenStageCacheError {
    CodegenStageCacheError::UnexpectedCell {
        expected,
        observed: match observed {
            Stage2CacheCell::StaticBudgetSuccess { .. } => "Stage 2 static budget success",
            Stage2CacheCell::FailureMemo { .. } => "Stage 2 static budget failure memo",
        },
    }
}

fn unexpected_stage3_cell(
    expected: &'static str,
    observed: &Stage3CacheCell,
) -> CodegenStageCacheError {
    CodegenStageCacheError::UnexpectedCell {
        expected,
        observed: match observed {
            Stage3CacheCell::InferIrSuccess { .. } => "Stage 3 infer_ir success",
            Stage3CacheCell::FailureMemo { .. } => "Stage 3 infer_ir failure memo",
        },
    }
}

#[cfg(test)]
mod tests {
    use std::fmt;
    use std::sync::{Arc, Mutex};

    use gbf_abi::{TraceBudget, TraceDropPolicy};
    use gbf_foundation::WorkloadId;
    use gbf_hw::target::dmg_mbc5_8mib_128kib;
    use gbf_policy::{
        BudgetFailure, BudgetSlotClass, DEFAULT_COMPILE_PROFILE_ID, ObservabilityMode,
        PlacementProfile, RuntimeMode, budget_failure_diagnostic,
    };
    use gbf_report::ReportEnvelope;
    use gbf_report::report_schemas::infer_ir_v1::{
        FixtureEquivalenceSkippedReason, FixtureEquivalenceTag,
    };
    use gbf_store::blob::BlobStore;
    use gbf_store::stage_cache::{StageCache, compose_key};
    use tempfile::TempDir;
    use tracing_subscriber::filter::LevelFilter;
    use tracing_subscriber::prelude::*;

    use crate::budget::{
        BudgetInputs, QuantGraphBudgetSource, QuantGraphBudgetView, QuantGraphBudgetViewError,
        static_budget_report as run_stage2_static_budget,
    };
    use crate::policy::resolve_policy;
    use crate::s1::quant_graph::{DeterminismClass, QuantFormat};
    use crate::s3::infer_ir::{
        GbInferIR, InferIrIdentity, InferIrProvenance, InferOp, NodeAnchorMap, NodeId,
        QuantGraphEntityRef, SemanticAnchor, TokenIngressMode, TokenInput, TokenInputId, ValueAxis,
        ValueDecl, ValueFormat, ValueId, ValueKind, ValueLayout, ValueProducerRef,
    };
    use crate::s4::observation_plan::{
        AnchorAttachmentTable, BuildActiveCheckpointSchema, LockedObservationKnobs,
        ObservationPlan, ObservationPlanIdentity, ObservationProvenance, OperationalProbeSchema,
        PerClassWeightTotal, TraceBudgetProjection,
    };
    use crate::s5::range_plan::{
        CertOutcome, LockedRangeKnobs, RangeCertIdentity, RangePlan, RangePlanIdentity,
        RangePlanProvenance,
    };

    use super::*;

    #[test]
    fn stage_cache_key_validate_is_deterministic() {
        let material = stage0_material();

        assert_eq!(
            compose_key(&stage0_validation_store_key(
                &material,
                Stage0CellKind::Success
            )),
            compose_key(&stage0_validation_store_key(
                &material,
                Stage0CellKind::Success
            ))
        );
    }

    #[test]
    fn stage_cache_key_validate_changes_with_inputs() {
        let left = stage0_material();
        let mut right = left.clone();
        right.compile_request_hash = hash(42);

        assert_ne!(
            compose_key(&stage0_validation_store_key(&left, Stage0CellKind::Success)),
            compose_key(&stage0_validation_store_key(
                &right,
                Stage0CellKind::Success
            ))
        );
    }

    #[test]
    fn stage_cache_key_validate_changes_with_pass_version() {
        let material = stage0_material();

        assert_ne!(
            compose_key(&stage0_validation_store_key_with_pass_version(
                &material,
                Stage0CellKind::Success,
                SemVer::new(1, 0, 0)
            )),
            compose_key(&stage0_validation_store_key_with_pass_version(
                &material,
                Stage0CellKind::Success,
                SemVer::new(1, 0, 1)
            ))
        );
    }

    #[test]
    fn stage_cache_key_resolve_policy_is_deterministic() {
        let material = stage05_material();

        assert_eq!(
            compose_key(&stage05_resolve_policy_store_key(
                &material,
                Stage05CellKind::Success
            )),
            compose_key(&stage05_resolve_policy_store_key(
                &material,
                Stage05CellKind::Success
            ))
        );
    }

    #[test]
    fn stage_cache_key_resolve_policy_changes_with_pass_version() {
        let material = stage05_material();

        assert_ne!(
            compose_key(&stage05_resolve_policy_store_key_with_pass_version(
                &material,
                Stage05CellKind::Success,
                SemVer::new(1, 0, 0)
            )),
            compose_key(&stage05_resolve_policy_store_key_with_pass_version(
                &material,
                Stage05CellKind::Success,
                SemVer::new(1, 0, 1)
            ))
        );
    }

    #[test]
    fn stage_cache_key_budget_is_deterministic() {
        let material = stage2_material(Some(hash(43)));

        assert_eq!(
            compose_key(&stage2_static_budget_store_key(
                &material,
                Stage2CellKind::Success
            )),
            compose_key(&stage2_static_budget_store_key(
                &material,
                Stage2CellKind::Success
            ))
        );
    }

    #[test]
    fn stage_cache_key_budget_distinguishes_missing_runtime_budget_from_zero_hash() {
        let none_material = stage2_material(None);
        let zero_material = stage2_material(Some(Hash256::ZERO));

        let none_value = serde_json::to_value(&none_material).expect("key serializes");
        assert_eq!(
            none_value["runtime_chrome_budget_hash"],
            serde_json::Value::Null
        );
        assert_ne!(
            compose_key(&stage2_static_budget_store_key(
                &none_material,
                Stage2CellKind::FailureMemo
            )),
            compose_key(&stage2_static_budget_store_key(
                &zero_material,
                Stage2CellKind::FailureMemo
            ))
        );
    }

    #[test]
    fn stage_cache_key_budget_changes_with_pass_version() {
        let material = stage2_material(Some(hash(43)));

        assert_ne!(
            compose_key(&stage2_static_budget_store_key_with_pass_version(
                &material,
                Stage2CellKind::Success,
                SemVer::new(1, 0, 0)
            )),
            compose_key(&stage2_static_budget_store_key_with_pass_version(
                &material,
                Stage2CellKind::Success,
                SemVer::new(1, 0, 1)
            ))
        );
    }

    #[test]
    fn stage_cache_key_quant_graph_is_deterministic() {
        let material = stage1_material();

        assert_eq!(
            compose_key(&stage1_quant_graph_store_key(
                &material,
                Stage1CellKind::Success
            )),
            compose_key(&stage1_quant_graph_store_key(
                &material,
                Stage1CellKind::Success
            ))
        );
    }

    #[test]
    fn stage_cache_key_quant_graph_changes_with_resolved_blob_index() {
        let left = stage1_material();
        let mut right = left.clone();
        right.resolved_blob_index_hash = hash(0x9a);

        assert_ne!(
            compose_key(&stage1_quant_graph_store_key(
                &left,
                Stage1CellKind::Success
            )),
            compose_key(&stage1_quant_graph_store_key(
                &right,
                Stage1CellKind::Success
            ))
        );
    }

    #[test]
    fn stage_cache_key_quant_graph_changes_with_full_policy_resolution_hash() {
        let left = stage1_material();
        let mut right = left.clone();
        right.policy_resolution_self_hash = hash(0x99);

        assert_ne!(
            compose_key(&stage1_quant_graph_store_key(
                &left,
                Stage1CellKind::Success
            )),
            compose_key(&stage1_quant_graph_store_key(
                &right,
                Stage1CellKind::Success
            ))
        );
    }

    #[test]
    fn stage_cache_key_quant_graph_changes_with_schema_hash() {
        let left = stage1_material();
        let mut right = left.clone();
        right.quant_graph_schema_hash = hash(0x9b);

        assert_ne!(
            compose_key(&stage1_quant_graph_store_key(
                &left,
                Stage1CellKind::Success
            )),
            compose_key(&stage1_quant_graph_store_key(
                &right,
                Stage1CellKind::Success
            ))
        );
    }

    #[test]
    fn stage_cache_key_quant_graph_changes_with_lowering_manifest_hash() {
        let left = stage1_material();
        let mut right = left.clone();
        right.lowering_manifest_hash = hash(0x9c);

        assert_ne!(
            compose_key(&stage1_quant_graph_store_key(
                &left,
                Stage1CellKind::Success
            )),
            compose_key(&stage1_quant_graph_store_key(
                &right,
                Stage1CellKind::Success
            ))
        );
    }

    #[test]
    fn stage_cache_key_quant_graph_changes_with_artifact_effective_core_hash() {
        let left = stage1_material();
        let mut right = left.clone();
        right.artifact_effective_core_hash = hash(0x9d);

        assert_ne!(
            compose_key(&stage1_quant_graph_store_key(
                &left,
                Stage1CellKind::Success
            )),
            compose_key(&stage1_quant_graph_store_key(
                &right,
                Stage1CellKind::Success
            ))
        );
    }

    #[test]
    fn stage_cache_key_quant_graph_changes_with_feature_set_hash() {
        let left = stage1_material();
        let mut right = left.clone();
        right.crate_feature_set_hash = hash(0x9e);

        assert_ne!(
            compose_key(&stage1_quant_graph_store_key(
                &left,
                Stage1CellKind::Success
            )),
            compose_key(&stage1_quant_graph_store_key(
                &right,
                Stage1CellKind::Success
            ))
        );
    }

    #[test]
    fn stage_cache_key_quant_graph_excludes_sequence_semantics_hash() {
        let material = stage1_material();
        let material_value =
            serde_json::to_value(&material).expect("stage1 material serializes to JSON");

        assert!(material_value.get("sequence_semantics_hash").is_none());
        assert!(
            !serde_json::to_string(&material_value)
                .expect("material JSON renders")
                .contains("sequence_semantics")
        );
    }

    #[test]
    fn stage_cache_key_quant_graph_success_and_failure_memo_are_distinct() {
        let material = stage1_material();

        assert_ne!(
            compose_key(&stage1_quant_graph_store_key(
                &material,
                Stage1CellKind::Success
            )),
            compose_key(&stage1_quant_graph_store_key(
                &material,
                Stage1CellKind::FailureMemo
            ))
        );
    }

    #[test]
    fn stage_cache_key_quant_graph_changes_with_pass_version() {
        let material = stage1_material();

        assert_ne!(
            compose_key(&stage1_quant_graph_store_key_with_pass_version(
                &material,
                Stage1CellKind::Success,
                SemVer::new(1, 0, 0)
            )),
            compose_key(&stage1_quant_graph_store_key_with_pass_version(
                &material,
                Stage1CellKind::Success,
                SemVer::new(1, 0, 1)
            ))
        );
    }

    #[test]
    fn k3_excludes_policy_resolution_self_hash() {
        let material = stage3_material();
        let value = serde_json::to_value(&material).expect("stage3 material serializes");

        assert!(value.get("policy_resolution_self_hash").is_none());
        assert!(
            !serde_json::to_string(&value)
                .expect("material JSON renders")
                .contains("policy_resolution_self_hash")
        );
    }

    #[test]
    fn k3_excludes_compile_request_hash() {
        let material = stage3_material();
        let value = serde_json::to_value(&material).expect("stage3 material serializes");

        assert!(value.get("compile_request_hash").is_none());
        assert!(
            !serde_json::to_string(&value)
                .expect("material JSON renders")
                .contains("compile_request_hash")
        );
    }

    #[test]
    fn k3_includes_infer_ir_policy_projection_hash() {
        let left = stage3_material();
        let mut right = left.clone();
        right.infer_ir_policy_projection_hash = hash(0xa0);

        assert_ne!(
            compose_key(&stage3_infer_ir_store_key(&left, Stage3CellKind::Success)),
            compose_key(&stage3_infer_ir_store_key(&right, Stage3CellKind::Success))
        );
    }

    #[test]
    fn k3_excludes_requested_runtime_modes_hash_double_count() {
        let material = stage3_material();
        let value = serde_json::to_value(&material).expect("stage3 material serializes");

        assert!(value.get("requested_runtime_modes_hash").is_none());
        assert!(
            !serde_json::to_string(&value)
                .expect("material JSON renders")
                .contains("requested_runtime_modes_hash")
        );
    }

    #[test]
    fn stage3_cache_success_store_iff_passed() {
        let (_dir, store) = store();
        let cache = StageCache::new(&store);
        let product = infer_ir_product();
        let material = stage3_material_for(&product);
        let report_bytes = canonical_report_bytes(&product.report);

        put_stage3_success(&cache, &material, &product, report_bytes.clone())
            .expect("put Stage 3 success cell");
        let cell = get_stage3_success(&cache, &material)
            .expect("Stage 3 success lookup")
            .expect("Stage 3 cache hit");
        let Stage3CacheCell::InferIrSuccess {
            product: cached, ..
        } = cell
        else {
            panic!("expected Stage 3 success product");
        };

        assert_eq!(cached.report.outcome, ReportOutcome::Passed);
        assert_eq!(cached.infer_ir_self_hash, product.infer_ir_self_hash);
    }

    #[test]
    fn stage3_cache_no_false_success_on_failure() {
        let (_dir, store) = store();
        let cache = StageCache::new(&store);
        let material = stage3_material();
        put_stage3_failure_memo(
            &cache,
            &material,
            CachedReportBytes {
                report_self_hash: hash(0xa1),
                canonical_bytes: b"cached infer_ir failure".to_vec(),
            },
            Vec::new(),
        )
        .expect("put Stage 3 failure memo");

        assert!(
            get_stage3_success(&cache, &material)
                .expect("Stage 3 success lookup")
                .is_none()
        );
        assert!(matches!(
            get_stage3_failure_memo(&cache, &material).expect("Stage 3 failure lookup"),
            Some(Stage3CacheCell::FailureMemo { .. })
        ));
    }

    #[test]
    fn stage3_cache_lookup_and_audit_rewrap_traces_are_subscriber_captured() {
        let (_dir, store) = store();
        let cache = StageCache::new(&store);
        let product = infer_ir_product();
        let material = stage3_material_for(&product);
        let report_bytes = canonical_report_bytes(&product.report);
        put_stage3_success(&cache, &material, &product, report_bytes)
            .expect("put Stage 3 success cell");
        let capture = TraceCapture::default();
        let subscriber = tracing_subscriber::registry()
            .with(LevelFilter::TRACE)
            .with(capture.clone());

        tracing::callsite::rebuild_interest_cache();
        let rewrapped = tracing::subscriber::with_default(subscriber, || {
            tracing::callsite::rebuild_interest_cache();
            get_stage3_success(&cache, &material)
                .expect("Stage 3 success lookup")
                .expect("Stage 3 success cache hit");
            assert!(
                get_stage3_failure_memo(&cache, &material)
                    .expect("Stage 3 failure memo lookup")
                    .is_none()
            );
            let rewrapped = rewrap_stage3_cached_report_audit_parents(
                &product.report,
                InferIrAuditParents {
                    policy_resolution_self_hash: hash(0xb9),
                    compile_request_hash: hash(0xba),
                },
            )
            .expect("audit rewrap hashes");
            tracing::callsite::rebuild_interest_cache();
            rewrapped
        });
        tracing::callsite::rebuild_interest_cache();

        let records = capture.records();
        assert!(records.iter().any(|record| {
            record.level == "DEBUG"
                && record.field_equals("event", STAGE3_CACHE_LOOKUP_EVENT)
                && record.field_equals("stage_id", STAGE3_INFER_IR_SUCCESS_ID)
                && record.field_equals("cell_kind", "Success")
                && record.field_equals("hit", "true")
        }));
        assert!(records.iter().any(|record| {
            record.level == "DEBUG"
                && record.field_equals("event", STAGE3_CACHE_LOOKUP_EVENT)
                && record.field_equals("stage_id", STAGE3_INFER_IR_FAILURE_ID)
                && record.field_equals("cell_kind", "FailureMemo")
                && record.field_equals("hit", "false")
        }));
        assert!(records.iter().any(|record| {
            record.level == "DEBUG"
                && record.field_equals("event", STAGE3_CACHE_AUDIT_REWRAP_EVENT)
                && record.field_contains(
                    "pre_audit_hash",
                    &product.report.report_self_hash.to_string(),
                )
                && record.field_contains("post_audit_hash", &rewrapped.report_self_hash.to_string())
                && record.field_equals("embedded_product_hash_unchanged", "true")
        }));
    }

    #[test]
    fn stage3_cache_failure_memo_distinct_from_success() {
        let material = stage3_material();

        assert_ne!(
            compose_key(&stage3_infer_ir_store_key(
                &material,
                Stage3CellKind::Success
            )),
            compose_key(&stage3_infer_ir_store_key(
                &material,
                Stage3CellKind::FailureMemo
            ))
        );
    }

    #[test]
    fn stage3_cache_pass_version_drift_invalidates() {
        let material = stage3_material();

        assert_ne!(
            compose_key(&stage3_infer_ir_store_key_with_pass_version(
                &material,
                Stage3CellKind::Success,
                SemVer::new(1, 0, 0)
            )),
            compose_key(&stage3_infer_ir_store_key_with_pass_version(
                &material,
                Stage3CellKind::Success,
                SemVer::new(1, 0, 1)
            ))
        );
    }

    #[test]
    fn stage3_cache_schema_drift_invalidates() {
        let left = stage3_material();
        let mut right = left.clone();
        right.infer_ir_schema_hash = hash(0xa2);

        assert_ne!(
            compose_key(&stage3_infer_ir_store_key(&left, Stage3CellKind::Success)),
            compose_key(&stage3_infer_ir_store_key(&right, Stage3CellKind::Success))
        );
    }

    #[test]
    fn stage3_cache_feature_set_drift_invalidates() {
        let left = stage3_material();
        let mut right = left.clone();
        right.crate_feature_set_hash = hash(0xa3);

        assert_ne!(
            compose_key(&stage3_infer_ir_store_key(&left, Stage3CellKind::Success)),
            compose_key(&stage3_infer_ir_store_key(&right, Stage3CellKind::Success))
        );
    }

    #[test]
    fn unrelated_policy_edit_does_not_invalidate_cache() {
        let left = stage3_material();
        let right = stage3_material();
        let value = serde_json::to_value(&left).expect("stage3 material serializes");

        assert_eq!(
            compose_key(&stage3_infer_ir_store_key(&left, Stage3CellKind::Success)),
            compose_key(&stage3_infer_ir_store_key(&right, Stage3CellKind::Success))
        );
        assert!(value.get("policy_resolution_self_hash").is_none());
        assert!(value.get("compile_request_hash").is_none());
    }

    #[test]
    fn stage3_cache_hit_replays_byte_identical_embedded_product() {
        let product = infer_ir_product();
        let rewrapped = rewrap_stage3_cached_report_audit_parents(
            &product.report,
            InferIrAuditParents {
                policy_resolution_self_hash: hash(0xb1),
                compile_request_hash: hash(0xb2),
            },
        )
        .expect("audit rewrap hashes");
        let before_product =
            serde_json::to_value(&product.report.body.result.as_ref().expect("result").product)
                .expect("product serializes");
        let after_product = serde_json::to_value(
            &rewrapped
                .body
                .result
                .as_ref()
                .expect("rewrapped result")
                .product,
        )
        .expect("product serializes");

        assert_eq!(
            canonicalize_value(&before_product).expect("before canonicalizes"),
            canonicalize_value(&after_product).expect("after canonicalizes")
        );
    }

    #[test]
    fn stage3_cache_hit_audit_parents_refreshed_on_envelope() {
        let product = infer_ir_product();
        let audit_parents = InferIrAuditParents {
            policy_resolution_self_hash: hash(0xb3),
            compile_request_hash: hash(0xb4),
        };

        let rewrapped = rewrap_stage3_cached_report_audit_parents(&product.report, audit_parents)
            .expect("audit rewrap hashes");

        assert_eq!(
            rewrapped.body.input_identity.policy_resolution_self_hash,
            audit_parents.policy_resolution_self_hash
        );
        assert_eq!(
            rewrapped.body.input_identity.compile_request_hash,
            audit_parents.compile_request_hash
        );
    }

    #[test]
    fn stage3_cache_hit_audit_rewrap_does_not_change_embedded_hash() {
        let product = infer_ir_product();
        let rewrapped = rewrap_stage3_cached_report_audit_parents(
            &product.report,
            InferIrAuditParents {
                policy_resolution_self_hash: hash(0xb5),
                compile_request_hash: hash(0xb6),
            },
        )
        .expect("audit rewrap hashes");

        assert_eq!(
            product
                .report
                .body
                .result
                .as_ref()
                .map(|result| result.infer_ir_self_hash),
            rewrapped
                .body
                .result
                .as_ref()
                .map(|result| result.infer_ir_self_hash)
        );
    }

    #[test]
    fn stage3_cache_hit_audit_rewrap_changes_envelope_self_hash() {
        let product = infer_ir_product();
        let rewrapped = rewrap_stage3_cached_report_audit_parents(
            &product.report,
            InferIrAuditParents {
                policy_resolution_self_hash: hash(0xb7),
                compile_request_hash: hash(0xb8),
            },
        )
        .expect("audit rewrap hashes");

        assert_ne!(product.report.report_self_hash, rewrapped.report_self_hash);
    }

    #[test]
    fn k4_includes_required_inputs_and_excludes_audit_only_fields() {
        let material = stage4_material();
        let value = serde_json::to_value(&material).expect("stage4 material serializes");
        let rendered = serde_json::to_string(&value).expect("material JSON renders");

        assert_eq!(
            material.pass_version_observation_plan,
            PASS_VERSION_OBSERVATION_PLAN.to_string()
        );
        assert_eq!(value["probe_registry_hash"], serde_json::json!(hash(0x86)));
        assert_eq!(value["metric_registry_hash"], serde_json::json!(hash(0x87)));
        assert_eq!(
            value["trace_event_layout_registry_hash"],
            serde_json::json!(hash(0x88))
        );
        assert!(value.get("observation_plan_schema_hash").is_some());
        assert!(
            value
                .get("build_active_semantic_checkpoint_schema_schema_hash")
                .is_some()
        );
        assert!(value.get("operational_probe_schema_schema_hash").is_some());
        for audit_only in [
            "policy_resolution_self_hash",
            "compile_request_hash",
            "artifact_aux_hash",
        ] {
            assert!(value.get(audit_only).is_none());
            assert!(!rendered.contains(audit_only));
        }
    }

    #[test]
    fn k4_changes_on_pass_feature_projection_and_stays_stable_under_audit_drift() {
        let left = stage4_material();
        let mut projection_changed = left.clone();
        projection_changed.observation_policy_projection_hash = hash(0x91);
        let mut feature_changed = left.clone();
        feature_changed.crate_feature_set_hash = hash(0x92);

        assert_ne!(
            compose_key(&stage4_observation_plan_store_key(
                &left,
                Stage4CellKind::Success
            )),
            compose_key(&stage4_observation_plan_store_key(
                &projection_changed,
                Stage4CellKind::Success
            ))
        );
        assert_ne!(
            compose_key(&stage4_observation_plan_store_key(
                &left,
                Stage4CellKind::Success
            )),
            compose_key(&stage4_observation_plan_store_key(
                &feature_changed,
                Stage4CellKind::Success
            ))
        );
        assert_ne!(
            compose_key(&stage4_observation_plan_store_key_with_pass_version(
                &left,
                Stage4CellKind::Success,
                SemVer::new(1, 0, 0)
            )),
            compose_key(&stage4_observation_plan_store_key_with_pass_version(
                &left,
                Stage4CellKind::Success,
                SemVer::new(1, 0, 1)
            ))
        );
        assert_eq!(
            compose_key(&stage4_observation_plan_store_key(
                &left,
                Stage4CellKind::Success
            )),
            compose_key(&stage4_observation_plan_store_key(
                &stage4_material(),
                Stage4CellKind::Success
            ))
        );
    }

    #[test]
    fn k5_includes_static_budget_and_excludes_stage4_and_audit_only_fields() {
        let material = stage5_material();
        let value = serde_json::to_value(&material).expect("stage5 material serializes");
        let rendered = serde_json::to_string(&value).expect("material JSON renders");

        assert_eq!(
            material.pass_version_range_plan,
            PASS_VERSION_RANGE_PLAN.to_string()
        );
        assert_eq!(
            value["static_budget_self_hash"],
            serde_json::json!(hash(0x93))
        );
        assert!(value.get("range_plan_schema_hash").is_some());
        assert!(value.get("range_cert_schema_hash").is_some());
        for excluded in [
            "observation_plan_self_hash",
            "policy_resolution_self_hash",
            "compile_request_hash",
            "artifact_aux_hash",
        ] {
            assert!(value.get(excluded).is_none());
            assert!(!rendered.contains(excluded));
        }
    }

    #[test]
    fn k5_changes_on_pass_feature_range_policy_and_static_budget() {
        let left = stage5_material();
        let mut range_policy_changed = left.clone();
        range_policy_changed.range_policy_projection_hash = hash(0x97);
        let mut static_budget_changed = left.clone();
        static_budget_changed.static_budget_self_hash = hash(0x95);
        let mut feature_changed = left.clone();
        feature_changed.crate_feature_set_hash = hash(0x96);

        for changed in [range_policy_changed, static_budget_changed, feature_changed] {
            assert_ne!(
                compose_key(&stage5_range_plan_store_key(&left, Stage5CellKind::Success)),
                compose_key(&stage5_range_plan_store_key(
                    &changed,
                    Stage5CellKind::Success
                ))
            );
        }
        assert_ne!(
            compose_key(&stage5_range_plan_store_key_with_pass_version(
                &left,
                Stage5CellKind::Success,
                SemVer::new(1, 0, 0)
            )),
            compose_key(&stage5_range_plan_store_key_with_pass_version(
                &left,
                Stage5CellKind::Success,
                SemVer::new(1, 0, 1)
            ))
        );
        assert_eq!(
            compose_key(&stage5_range_plan_store_key(&left, Stage5CellKind::Success)),
            compose_key(&stage5_range_plan_store_key(
                &stage5_material(),
                Stage5CellKind::Success
            ))
        );
    }

    #[test]
    fn stage4_success_cache_stores_core_product_not_envelope() {
        let (_dir, store) = store();
        let cache = StageCache::new(&store);
        let product = stage4_product();
        let material = stage4_material_for(&product);

        put_stage4_success(&cache, &material, &product).expect("put Stage 4 core product");
        let raw = cache
            .get(&stage4_observation_plan_store_key(
                &material,
                Stage4CellKind::Success,
            ))
            .expect("raw Stage 4 cache get")
            .expect("raw Stage 4 cache hit");
        let decoded: ObservationPlanCoreProduct =
            serde_json::from_slice(&raw).expect("raw payload is core product");

        assert_eq!(decoded, product);
        assert!(serde_json::from_slice::<ReportEnvelope<ObservationPlanReportBody>>(&raw).is_err());
        assert_eq!(
            get_stage4_success(&cache, &material)
                .expect("Stage 4 success lookup")
                .expect("Stage 4 cache hit"),
            product
        );
    }

    #[test]
    fn stage5_success_cache_stores_core_product_including_cert_body() {
        let (_dir, store) = store();
        let cache = StageCache::new(&store);
        let product = stage5_product();
        let material = stage5_material_for(&product);

        put_stage5_success(&cache, &material, &product).expect("put Stage 5 core product");
        let raw = cache
            .get(&stage5_range_plan_store_key(
                &material,
                Stage5CellKind::Success,
            ))
            .expect("raw Stage 5 cache get")
            .expect("raw Stage 5 cache hit");
        let decoded: RangePlanCoreProduct =
            serde_json::from_slice(&raw).expect("raw payload is core product");

        assert_eq!(decoded.range_cert.cert_outcome, CertOutcome::Verified);
        assert_eq!(decoded, product);
        assert!(serde_json::from_slice::<ReportEnvelope<RangePlanReportBody>>(&raw).is_err());
    }

    #[test]
    fn stage4_and_stage5_failure_memos_store_bodies_not_envelopes_and_never_fake_success() {
        let (_dir, store) = store();
        let cache = StageCache::new(&store);
        let stage4_product = stage4_product();
        let stage4_material = stage4_material_for(&stage4_product);
        let stage4_failure = stage4_failure(&stage4_product, &stage4_context(0x97));

        put_stage4_failure_memo(&cache, &stage4_material, &stage4_failure)
            .expect("put Stage 4 failure memo");
        assert!(
            get_stage4_success(&cache, &stage4_material)
                .expect("Stage 4 success lookup")
                .is_none()
        );
        let raw_stage4 = cache
            .get(&stage4_observation_plan_store_key(
                &stage4_material,
                Stage4CellKind::FailureMemo,
            ))
            .expect("raw Stage 4 failure get")
            .expect("raw Stage 4 failure hit");
        assert!(serde_json::from_slice::<ObservationPlanCoreFailure>(&raw_stage4).is_ok());
        assert!(
            serde_json::from_slice::<ReportEnvelope<ObservationPlanReportBody>>(&raw_stage4)
                .is_err()
        );

        let stage5_product = stage5_product();
        let stage5_material = stage5_material_for(&stage5_product);
        let stage5_failure = stage5_failure(&stage5_product, &stage5_context(0x98));
        put_stage5_failure_memo(&cache, &stage5_material, &stage5_failure)
            .expect("put Stage 5 failure memo");
        assert!(
            get_stage5_success(&cache, &stage5_material)
                .expect("Stage 5 success lookup")
                .is_none()
        );
        let raw_stage5 = cache
            .get(&stage5_range_plan_store_key(
                &stage5_material,
                Stage5CellKind::FailureMemo,
            ))
            .expect("raw Stage 5 failure get")
            .expect("raw Stage 5 failure hit");
        let decoded_stage5: RangePlanCoreFailure =
            serde_json::from_slice(&raw_stage5).expect("raw Stage 5 memo is failure body");
        assert!(decoded_stage5.range_cert_body.is_some());
        assert!(
            serde_json::from_slice::<ReportEnvelope<RangePlanReportBody>>(&raw_stage5).is_err()
        );
    }

    #[test]
    fn stage4_and_stage5_success_replay_rewraps_fresh_audit_parents() {
        let stage4_product = stage4_product();
        let first_stage4 = rewrap_stage4_cached_success(&stage4_product, &stage4_context(0xa0))
            .expect("first Stage 4 rewrap");
        let second_stage4 = rewrap_stage4_cached_success(&stage4_product, &stage4_context(0xa1))
            .expect("second Stage 4 rewrap");

        assert_eq!(
            canonical_json_bytes(&first_stage4.product).expect("first Stage 4 product"),
            canonical_json_bytes(&second_stage4.product).expect("second Stage 4 product")
        );
        assert_ne!(
            first_stage4.report.body.input_identity.compile_request_hash,
            second_stage4
                .report
                .body
                .input_identity
                .compile_request_hash
        );
        assert_ne!(
            first_stage4.report.report_self_hash,
            second_stage4.report.report_self_hash
        );
        assert_ne!(
            first_stage4
                .sc_re_emit_report
                .body
                .input_identity
                .artifact_aux_hash,
            second_stage4
                .sc_re_emit_report
                .body
                .input_identity
                .artifact_aux_hash
        );

        let stage5_product = stage5_product();
        let first_stage5 = rewrap_stage5_cached_success(&stage5_product, &stage5_context(0xa2))
            .expect("first Stage 5 rewrap");
        let second_stage5 = rewrap_stage5_cached_success(&stage5_product, &stage5_context(0xa3))
            .expect("second Stage 5 rewrap");

        assert_eq!(
            canonical_json_bytes(&first_stage5.product).expect("first Stage 5 product"),
            canonical_json_bytes(&second_stage5.product).expect("second Stage 5 product")
        );
        assert_ne!(
            first_stage5.report.body.input_identity.compile_request_hash,
            second_stage5
                .report
                .body
                .input_identity
                .compile_request_hash
        );
        assert_ne!(
            first_stage5.report.report_self_hash,
            second_stage5.report.report_self_hash
        );
        assert_eq!(
            first_stage5.cert_report.body.identity.range_plan_self_hash,
            Some(stage5_product.range_plan_self_hash)
        );
    }

    #[test]
    fn stage4_and_stage5_failure_replay_rewraps_fresh_audit_parents() {
        let stage4_product = stage4_product();
        let cached_stage4 = stage4_context(0xb0);
        let current_stage4 = stage4_context(0xb1);
        let mut stage4_failure = stage4_failure(&stage4_product, &cached_stage4);
        let mut cached_sc_re_emit_body = stage4_sc_re_emit_body(&stage4_product, &cached_stage4);
        cached_sc_re_emit_body.result = None;
        cached_sc_re_emit_body.diagnostics = vec![hard_diagnostic()];
        stage4_failure.sc_re_emit_body = Some(cached_sc_re_emit_body);
        let stage4_replay = rewrap_stage4_cached_failure(&stage4_failure, &current_stage4)
            .expect("Stage 4 failure rewrap");

        let stage4_identity = &stage4_replay.report.body.input_identity;
        assert_eq!(
            stage4_identity.static_budget_self_hash,
            current_stage4.audit_parents.static_budget_self_hash
        );
        assert_eq!(
            stage4_identity.policy_resolution_self_hash,
            current_stage4.audit_parents.policy_resolution_self_hash
        );
        assert_eq!(
            stage4_identity.compile_request_hash,
            current_stage4.audit_parents.compile_request_hash
        );
        assert_eq!(
            stage4_identity.artifact_aux_hash,
            current_stage4.audit_parents.artifact_aux_hash
        );
        assert_ne!(
            stage4_failure
                .observation_plan_body
                .input_identity
                .compile_request_hash,
            stage4_identity.compile_request_hash
        );
        let stage4_sc_re_emit = stage4_replay
            .sc_re_emit_report
            .as_ref()
            .expect("Stage 4 failure SC re-emit replay");
        assert_eq!(
            stage4_sc_re_emit.body.input_identity.artifact_aux_hash,
            current_stage4.audit_parents.artifact_aux_hash
        );
        assert_ne!(
            stage4_failure
                .sc_re_emit_body
                .as_ref()
                .expect("cached Stage 4 SC re-emit body")
                .input_identity
                .artifact_aux_hash,
            stage4_sc_re_emit.body.input_identity.artifact_aux_hash
        );

        let stage5_product = stage5_product();
        let cached_stage5 = stage5_context(0xb2);
        let current_stage5 = stage5_context(0xb3);
        let stage5_failure = stage5_failure(&stage5_product, &cached_stage5);
        let stage5_replay = rewrap_stage5_cached_failure(&stage5_failure, &current_stage5)
            .expect("Stage 5 failure rewrap");

        let stage5_identity = &stage5_replay.report.body.input_identity;
        assert_eq!(
            stage5_identity.policy_resolution_self_hash,
            current_stage5.audit_parents.policy_resolution_self_hash
        );
        assert_eq!(
            stage5_identity.compile_request_hash,
            current_stage5.audit_parents.compile_request_hash
        );
        assert_eq!(
            stage5_identity.artifact_aux_hash,
            current_stage5.audit_parents.artifact_aux_hash
        );
        assert_ne!(
            stage5_failure
                .range_plan_body
                .input_identity
                .compile_request_hash,
            stage5_identity.compile_request_hash
        );
        assert!(stage5_replay.cert_report.is_some());
    }

    #[test]
    fn stage4_stage5_cache_and_rewrap_events_are_subscriber_captured() {
        let (_dir, store) = store();
        let cache = StageCache::new(&store);
        let stage4_product = stage4_product();
        let stage4_material = stage4_material_for(&stage4_product);
        put_stage4_success(&cache, &stage4_material, &stage4_product).expect("put Stage 4 success");
        let stage5_product = stage5_product();
        let stage5_material = stage5_material_for(&stage5_product);
        put_stage5_success(&cache, &stage5_material, &stage5_product).expect("put Stage 5 success");
        let capture = TraceCapture::default();
        let subscriber = tracing_subscriber::registry()
            .with(LevelFilter::TRACE)
            .with(capture.clone());

        tracing::callsite::rebuild_interest_cache();
        tracing::subscriber::with_default(subscriber, || {
            tracing::callsite::rebuild_interest_cache();
            get_stage4_success(&cache, &stage4_material)
                .expect("Stage 4 lookup")
                .expect("Stage 4 hit");
            assert!(
                get_stage4_failure_memo(&cache, &stage4_material)
                    .expect("Stage 4 failure memo lookup")
                    .is_none()
            );
            get_stage5_success(&cache, &stage5_material)
                .expect("Stage 5 lookup")
                .expect("Stage 5 hit");
            assert!(
                get_stage5_failure_memo(&cache, &stage5_material)
                    .expect("Stage 5 failure memo lookup")
                    .is_none()
            );
            rewrap_stage4_cached_success(&stage4_product, &stage4_context(0xc0))
                .expect("Stage 4 rewrap");
            rewrap_stage5_cached_success(&stage5_product, &stage5_context(0xc1))
                .expect("Stage 5 rewrap");
            tracing::callsite::rebuild_interest_cache();
        });
        tracing::callsite::rebuild_interest_cache();

        let records = capture.records();
        let stage4_hit = records
            .iter()
            .find(|record| {
                record.level == "DEBUG"
                    && record.field_equals("event", STAGE4_DRIVER_CACHE_HIT_EVENT)
                    && record.field_equals("cell_kind", "Success")
            })
            .expect("Stage 4 cache hit trace");
        assert!(stage4_hit.field_u64("replay_ns").is_some());
        let stage5_hit = records
            .iter()
            .find(|record| {
                record.level == "DEBUG"
                    && record.field_equals("event", STAGE5_DRIVER_CACHE_HIT_EVENT)
                    && record.field_equals("cell_kind", "Success")
            })
            .expect("Stage 5 cache hit trace");
        assert!(stage5_hit.field_u64("replay_ns").is_some());

        for (event, cell_kind) in [
            (STAGE4_DRIVER_CACHE_LOOKUP_EVENT, Some("Success")),
            (STAGE4_DRIVER_CACHE_MISS_EVENT, Some("FailureMemo")),
            (STAGE4_DRIVER_AUDIT_PARENT_REWRAP_EVENT, None),
            (STAGE5_DRIVER_CACHE_LOOKUP_EVENT, Some("Success")),
            (STAGE5_DRIVER_CACHE_MISS_EVENT, Some("FailureMemo")),
            (STAGE5_DRIVER_AUDIT_PARENT_REWRAP_EVENT, None),
        ] {
            assert!(
                records.iter().any(|record| {
                    (record.level == "DEBUG"
                        || (event == STAGE5_DRIVER_AUDIT_PARENT_REWRAP_EVENT
                            && record.level == "INFO"))
                        && record.field_equals("event", event)
                        && cell_kind.map_or(true, |cell_kind| {
                            record.field_equals("cell_kind", cell_kind)
                        })
                }),
                "missing trace event {event}"
            );
        }
    }

    #[test]
    fn stage_cache_failed_pass_does_not_enter_success_cache() {
        let (_dir, store) = store();
        let cache = StageCache::new(&store);
        let material = stage0_material();
        let failure = Stage0CacheCell::FailureMemo {
            report: CachedReportBytes {
                report_self_hash: hash(90),
                canonical_bytes: br#"{"outcome":"Failed","report_self_hash":"cached"}"#.to_vec(),
            },
            diagnostics: Vec::new(),
        };
        put_cell(
            &cache,
            &stage0_validation_store_key(&material, Stage0CellKind::FailureMemo),
            &failure,
        )
        .expect("put failure memo");

        assert!(
            get_stage0_success(&cache, &material)
                .expect("success lookup")
                .is_none()
        );
        assert!(matches!(
            get_stage0_failure_memo(&cache, &material).expect("failure lookup"),
            Some(Stage0CacheCell::FailureMemo { .. })
        ));

        let budget_material = stage2_material(None);
        let budget_failure = static_budget_report(ReportOutcome::Failed);
        put_stage2_failure_memo(
            &cache,
            &budget_material,
            &budget_failure,
            br#"{"outcome":"Failed","report_self_hash":"cached"}"#.to_vec(),
        )
        .expect("put Stage 2 failure memo");

        assert!(
            get_stage2_success(&cache, &budget_material)
                .expect("Stage 2 success lookup")
                .is_none()
        );
        assert!(matches!(
            get_stage2_failure_memo(&cache, &budget_material).expect("Stage 2 failure lookup"),
            Some(Stage2CacheCell::FailureMemo { .. })
        ));
    }

    #[test]
    fn stage_cache_failure_memo_replays_only_on_exact_input_match() {
        let (_dir, store) = store();
        let cache = StageCache::new(&store);
        let material = stage05_material();
        let memo = Stage05CacheCell::FailureMemo {
            report: CachedReportBytes {
                report_self_hash: hash(91),
                canonical_bytes: b"cached policy failure".to_vec(),
            },
            diagnostics: Vec::new(),
        };
        put_cell(
            &cache,
            &stage05_resolve_policy_store_key(&material, Stage05CellKind::FailureMemo),
            &memo,
        )
        .expect("put failure memo");

        let mut changed = material.clone();
        changed.input_hashes.hint_bundle_hash = hash(92);

        assert!(matches!(
            get_stage05_failure_memo(&cache, &material).expect("exact lookup"),
            Some(Stage05CacheCell::FailureMemo { .. })
        ));
        assert!(
            get_stage05_failure_memo(&cache, &changed)
                .expect("changed lookup")
                .is_none()
        );

        let budget_material = stage2_material(None);
        let budget_failure = static_budget_report(ReportOutcome::Failed);
        put_stage2_failure_memo(
            &cache,
            &budget_material,
            &budget_failure,
            b"cached static-budget failure".to_vec(),
        )
        .expect("put Stage 2 failure memo");

        let mut changed_budget = budget_material.clone();
        changed_budget.runtime_chrome_budget_hash = Some(hash(94));

        assert!(matches!(
            get_stage2_failure_memo(&cache, &budget_material).expect("exact Stage 2 lookup"),
            Some(Stage2CacheCell::FailureMemo { .. })
        ));
        assert!(
            get_stage2_failure_memo(&cache, &changed_budget)
                .expect("changed Stage 2 lookup")
                .is_none()
        );
    }

    #[test]
    fn stage_cache_hit_materializes_cached_report_bytes() {
        let (_dir, store) = store();
        let cache = StageCache::new(&store);
        let material = stage0_material();
        let fixture = crate::policy::tests::Fixture::new(DEFAULT_COMPILE_PROFILE_ID);
        let validation = fixture.validation();
        let report_bytes = canonical_report_bytes(&validation.report);

        put_stage0_success(&cache, &material, &validation, report_bytes.clone())
            .expect("put success cell");
        let cell = get_stage0_success(&cache, &material)
            .expect("success lookup")
            .expect("cache hit");
        let materialized = materialize_stage0_cached_report(&cell);

        assert_eq!(
            materialized.report_self_hash,
            validation.artifact_validation_self_hash
        );
        assert_eq!(materialized.canonical_bytes, report_bytes);

        let budget_material = stage2_material(Some(hash(43)));
        let budget_report = static_budget_report(ReportOutcome::Passed);
        let budget_report_bytes =
            br#"{"cached":["static-budget"],"nested":{"order":"preserved"},"z":true}"#.to_vec();
        put_stage2_success(
            &cache,
            &budget_material,
            &budget_report,
            budget_report_bytes.clone(),
        )
        .expect("put Stage 2 success cell");
        let budget_cell = get_stage2_success(&cache, &budget_material)
            .expect("Stage 2 success lookup")
            .expect("Stage 2 cache hit");
        let materialized_budget = materialize_stage2_cached_report(&budget_cell);

        assert_eq!(
            materialized_budget.report_self_hash,
            budget_report.static_budget_self_hash
        );
        assert_eq!(materialized_budget.canonical_bytes, budget_report_bytes);
    }

    #[test]
    fn stage_cache_hit_rehydrates_stage0_product_for_policy_resume() {
        let (_dir, store) = store();
        let cache = StageCache::new(&store);
        let fixture = crate::policy::tests::Fixture::new(DEFAULT_COMPILE_PROFILE_ID);
        let validation = fixture.validation();
        let material = Stage0CacheKeyMaterial::success(validation.validated.input_hashes, hash(11));
        let report_bytes = canonical_report_bytes(&validation.report);

        put_stage0_success(&cache, &material, &validation, report_bytes)
            .expect("put Stage 0 success product");
        let cell = get_stage0_success(&cache, &material)
            .expect("Stage 0 success lookup")
            .expect("Stage 0 cache hit");
        let Stage0CacheCell::ValidationSuccess { product, .. } = cell else {
            panic!("expected Stage 0 success product");
        };

        let resumed_validation = product
            .rehydrate_checked()
            .expect("cached Stage 0 product passes rehydration checks");
        assert_eq!(
            resumed_validation.validated.input_hashes,
            validation.validated.input_hashes
        );
        assert_eq!(
            resumed_validation.validated.compile_request,
            validation.validated.compile_request
        );

        let resumed_policy = resolve_policy(&resumed_validation)
            .expect("policy resolves from cached Stage 0 product");
        assert_eq!(
            resumed_policy.input_hashes,
            validation.validated.input_hashes
        );
    }

    #[test]
    fn stage_cache_rejects_stage0_product_with_tampered_self_hash() {
        let (_dir, store) = store();
        let cache = StageCache::new(&store);
        let fixture = crate::policy::tests::Fixture::new(DEFAULT_COMPILE_PROFILE_ID);
        let validation = fixture.validation();
        let material = Stage0CacheKeyMaterial::success(validation.validated.input_hashes, hash(11));
        let cell = Stage0CacheCell::ValidationSuccess {
            product: Box::new(CachedValidationProduct::from(&validation)),
            report: CachedReportBytes {
                report_self_hash: validation.artifact_validation_self_hash,
                canonical_bytes: canonical_report_bytes(&validation.report),
            },
        };
        let mut value = serde_json::to_value(cell).expect("cache cell serializes");
        value["product"]["artifact_validation_self_hash"] =
            serde_json::to_value(hash(222)).expect("hash serializes");
        let payload = serde_json::to_vec(&value).expect("tampered cache cell serializes");
        cache
            .put(
                &stage0_validation_store_key(&material, Stage0CellKind::Success),
                &payload,
            )
            .expect("put tampered Stage 0 product");

        assert!(matches!(
            get_stage0_success(&cache, &material),
            Err(CodegenStageCacheError::CachedValidationProduct(
                CachedValidationProductRehydrateError::ReportSelfHashMismatch { .. }
            ))
        ));
    }

    #[test]
    fn stage_cache_rejects_stage0_success_with_tampered_cached_report_bytes() {
        let (_dir, store) = store();
        let cache = StageCache::new(&store);
        let fixture = crate::policy::tests::Fixture::new(DEFAULT_COMPILE_PROFILE_ID);
        let validation = fixture.validation();
        let material = Stage0CacheKeyMaterial::success(validation.validated.input_hashes, hash(11));
        let cell = Stage0CacheCell::ValidationSuccess {
            product: Box::new(CachedValidationProduct::from(&validation)),
            report: CachedReportBytes {
                report_self_hash: validation.artifact_validation_self_hash,
                canonical_bytes: b"tampered artifact validation report".to_vec(),
            },
        };
        put_cell(
            &cache,
            &stage0_validation_store_key(&material, Stage0CellKind::Success),
            &cell,
        )
        .expect("put tampered Stage 0 report bytes");

        assert!(matches!(
            get_stage0_success(&cache, &material),
            Err(CodegenStageCacheError::CachedReportBytes(
                CachedReportBytesError::CanonicalBytesHashMismatch { .. }
            ))
        ));
    }

    #[test]
    fn stage_cache_rejects_stage0_success_with_consistently_tampered_canonical_bytes() {
        let (_dir, store) = store();
        let cache = StageCache::new(&store);
        let fixture = crate::policy::tests::Fixture::new(DEFAULT_COMPILE_PROFILE_ID);
        let validation = fixture.validation();
        let material = Stage0CacheKeyMaterial::success(validation.validated.input_hashes, hash(11));
        let tampered_report_bytes = b"tampered artifact validation report".to_vec();
        let tampered_report_bytes_hash =
            Hash256::from_bytes(Sha256::digest(&tampered_report_bytes).into());
        let cell = Stage0CacheCell::ValidationSuccess {
            product: Box::new(CachedValidationProduct::from(&validation)),
            report: CachedReportBytes {
                report_self_hash: validation.artifact_validation_self_hash,
                canonical_bytes: tampered_report_bytes,
            },
        };
        let mut value = serde_json::to_value(cell).expect("cache cell serializes");
        value["product"]["artifact_validation_canonical_bytes_hash"] =
            serde_json::to_value(tampered_report_bytes_hash).expect("hash serializes");
        let payload = serde_json::to_vec(&value).expect("tampered cache cell serializes");
        cache
            .put(
                &stage0_validation_store_key(&material, Stage0CellKind::Success),
                &payload,
            )
            .expect("put consistently tampered Stage 0 product");

        assert!(matches!(
            get_stage0_success(&cache, &material),
            Err(CodegenStageCacheError::CachedValidationProduct(
                CachedValidationProductRehydrateError::CanonicalBytesHashMismatch { .. }
            ))
        ));
    }

    #[test]
    fn stage_cache_hit_replays_stage05_policy_product_for_budget_resume() {
        let (_dir, store) = store();
        let cache = StageCache::new(&store);
        let fixture = crate::policy::tests::Fixture::new(DEFAULT_COMPILE_PROFILE_ID);
        let validation = fixture.validation();
        let policy = resolve_policy(&validation).expect("policy resolves");
        let material = stage05_material_for(&policy);
        let report_bytes = canonical_report_bytes(&policy.report);

        put_stage05_success(&cache, &material, &policy, report_bytes)
            .expect("put Stage 0.5 success product");
        let cell = get_stage05_success(&cache, &material)
            .expect("Stage 0.5 success lookup")
            .expect("Stage 0.5 cache hit");
        let Stage05CacheCell::ResolvePolicySuccess { product, .. } = cell else {
            panic!("expected Stage 0.5 success product");
        };

        assert_eq!(product.policy, policy.policy);
        assert_eq!(
            product.policy_resolution_self_hash,
            policy.policy_resolution_self_hash
        );

        let quant_graph = CacheHitQuantGraph {
            quant_graph_hash: hash(0xe0),
        };
        let target_profile = dmg_mbc5_8mib_128kib();
        let budget = run_stage2_static_budget(BudgetInputs {
            policy: &product,
            quant_graph: &quant_graph,
            runtime_chrome_budget: None,
            target_profile: &target_profile,
        });

        assert_eq!(budget.report.outcome, ReportOutcome::Failed);
        assert_eq!(
            budget.report.body.identity.policy_resolution_self_hash,
            policy.policy_resolution_self_hash
        );
        assert!(
            budget
                .report
                .body
                .decision
                .failures
                .contains(&BudgetFailure::MissingRuntimeChromeBudget)
        );
    }

    #[test]
    fn stage_cache_rejects_stage05_product_with_tampered_report_self_hash() {
        let (_dir, store) = store();
        let cache = StageCache::new(&store);
        let fixture = crate::policy::tests::Fixture::new(DEFAULT_COMPILE_PROFILE_ID);
        let validation = fixture.validation();
        let policy = resolve_policy(&validation).expect("policy resolves");
        let material = stage05_material_for(&policy);
        let cell = Stage05CacheCell::ResolvePolicySuccess {
            product: Box::new(policy.clone()),
            report: CachedReportBytes {
                report_self_hash: policy.policy_resolution_self_hash,
                canonical_bytes: canonical_report_bytes(&policy.report),
            },
        };
        let mut value = serde_json::to_value(cell).expect("cache cell serializes");
        value["product"]["policy_resolution_self_hash"] =
            serde_json::to_value(hash(222)).expect("hash serializes");
        let payload = serde_json::to_vec(&value).expect("tampered cache cell serializes");
        cache
            .put(
                &stage05_resolve_policy_store_key(&material, Stage05CellKind::Success),
                &payload,
            )
            .expect("put tampered Stage 0.5 product");

        assert!(matches!(
            get_stage05_success(&cache, &material),
            Err(CodegenStageCacheError::CachedPolicyProduct(
                CachedPolicyProductError::ReportSelfHashMismatch { .. }
            ))
        ));
    }

    #[test]
    fn stage_cache_rejects_stage05_product_with_tampered_policy_field() {
        let (_dir, store) = store();
        let cache = StageCache::new(&store);
        let fixture = crate::policy::tests::Fixture::new(DEFAULT_COMPILE_PROFILE_ID);
        let validation = fixture.validation();
        let policy = resolve_policy(&validation).expect("policy resolves");
        let material = stage05_material_for(&policy);
        let mut tampered = policy.clone();
        tampered.policy.objective.min_ui_headroom_pct = tampered
            .policy
            .objective
            .min_ui_headroom_pct
            .wrapping_add(1);
        let cell = Stage05CacheCell::ResolvePolicySuccess {
            product: Box::new(tampered),
            report: CachedReportBytes {
                report_self_hash: policy.policy_resolution_self_hash,
                canonical_bytes: canonical_report_bytes(&policy.report),
            },
        };
        put_cell(
            &cache,
            &stage05_resolve_policy_store_key(&material, Stage05CellKind::Success),
            &cell,
        )
        .expect("put tampered Stage 0.5 policy product");

        assert!(matches!(
            get_stage05_success(&cache, &material),
            Err(CodegenStageCacheError::CachedPolicyProduct(
                CachedPolicyProductError::ReportIdentityMismatch
            ))
        ));
    }

    #[test]
    fn stage_cache_rejects_stage05_product_with_tampered_runtime_modes() {
        let (_dir, store) = store();
        let cache = StageCache::new(&store);
        let fixture = crate::policy::tests::Fixture::new(DEFAULT_COMPILE_PROFILE_ID);
        let validation = fixture.validation();
        let policy = resolve_policy(&validation).expect("policy resolves");
        let material = stage05_material_for(&policy);
        let mut tampered = policy.clone();
        tampered
            .policy
            .requested_runtime_modes
            .insert(RuntimeMode::Trace);
        let cell = Stage05CacheCell::ResolvePolicySuccess {
            product: Box::new(tampered),
            report: CachedReportBytes {
                report_self_hash: policy.policy_resolution_self_hash,
                canonical_bytes: canonical_report_bytes(&policy.report),
            },
        };
        put_cell(
            &cache,
            &stage05_resolve_policy_store_key(&material, Stage05CellKind::Success),
            &cell,
        )
        .expect("put tampered Stage 0.5 runtime modes");

        assert!(matches!(
            get_stage05_success(&cache, &material),
            Err(CodegenStageCacheError::CachedPolicyProduct(
                CachedPolicyProductError::ReportIdentityMismatch
            ))
        ));
    }

    #[test]
    fn stage_cache_rejects_stage05_success_with_tampered_cached_report_bytes() {
        let (_dir, store) = store();
        let cache = StageCache::new(&store);
        let fixture = crate::policy::tests::Fixture::new(DEFAULT_COMPILE_PROFILE_ID);
        let validation = fixture.validation();
        let policy = resolve_policy(&validation).expect("policy resolves");
        let material = stage05_material_for(&policy);
        let cell = Stage05CacheCell::ResolvePolicySuccess {
            product: Box::new(policy.clone()),
            report: CachedReportBytes {
                report_self_hash: policy.policy_resolution_self_hash,
                canonical_bytes: b"tampered policy resolution report".to_vec(),
            },
        };
        put_cell(
            &cache,
            &stage05_resolve_policy_store_key(&material, Stage05CellKind::Success),
            &cell,
        )
        .expect("put tampered Stage 0.5 report bytes");

        assert!(matches!(
            get_stage05_success(&cache, &material),
            Err(CodegenStageCacheError::CachedPolicyProduct(
                CachedPolicyProductError::CanonicalBytesHashMismatch { .. }
            ))
        ));
    }

    #[test]
    fn stage_cache_validate_allows_partial_failure_key_without_fake_hashes() {
        let mut material = Stage0CacheKeyMaterial::partial_failure(
            hash(1),
            hash(6),
            hash(7),
            hash(8),
            hash(9),
            hash(11),
        );

        let value = serde_json::to_value(&material).expect("key serializes");

        assert_eq!(
            value["artifact_source_hash"],
            serde_json::to_value(hash(1)).expect("hash serializes")
        );
        assert_eq!(
            value["artifact_effective_core_hash"],
            serde_json::Value::Null
        );
        assert_eq!(value["artifact_manifest_hash"], serde_json::Value::Null);
        assert_eq!(value["artifact_aux_hash"], serde_json::Value::Null);
        assert_eq!(value["lowering_manifest_hash"], serde_json::Value::Null);
        assert_eq!(value["calibration_hash"], serde_json::Value::Null);

        let none_key = compose_key(&stage0_validation_store_key(
            &material,
            Stage0CellKind::FailureMemo,
        ));
        material.artifact_manifest_hash = Some(Hash256::ZERO);
        let zero_key = compose_key(&stage0_validation_store_key(
            &material,
            Stage0CellKind::FailureMemo,
        ));

        assert_ne!(none_key, zero_key);
    }

    fn store() -> (TempDir, BlobStore) {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = BlobStore::open(dir.path().to_path_buf()).expect("blob store");
        (dir, store)
    }

    fn canonical_report_bytes<B: ReportBody + Serialize>(report: &ReportEnvelope<B>) -> Vec<u8> {
        canonicalize_report(report).expect("report canonicalizes")
    }

    fn canonical_json_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, String> {
        let value = serde_json::to_value(value).map_err(|err| err.to_string())?;
        canonicalize_value(&value).map_err(|err| err.to_string())
    }

    fn stage0_material() -> Stage0CacheKeyMaterial {
        Stage0CacheKeyMaterial {
            artifact_source_hash: hash(1),
            artifact_effective_core_hash: Some(hash(2)),
            artifact_manifest_hash: Some(hash(3)),
            artifact_aux_hash: Some(hash(4)),
            lowering_manifest_hash: Some(hash(5)),
            hint_bundle_hash: hash(6),
            compile_request_hash: hash(7),
            target_profile_hash: hash(8),
            compile_profile_hash: hash(9),
            calibration_hash: Some(hash(10)),
            compatibility_adapter_registry_hash: hash(11),
            crate_feature_set_hash: crate_feature_set_hash(),
            artifact_validation_schema_hash: artifact_validation_schema_hash(),
        }
    }

    fn stage05_material() -> Stage05CacheKeyMaterial {
        Stage05CacheKeyMaterial {
            artifact_validation_self_hash: hash(20),
            input_hashes: input_hashes(),
            target_defaults_hash: hash(32),
            compile_profile_hash: hash(33),
            profile_defaults_hash: hash(34),
            compile_objective_hash: hash(35),
            crate_feature_set_hash: crate_feature_set_hash(),
            policy_resolution_schema_hash: policy_resolution_schema_hash(),
        }
    }

    fn stage05_material_for(policy: &ResolvedPolicyProduct) -> Stage05CacheKeyMaterial {
        Stage05CacheKeyMaterial {
            artifact_validation_self_hash: policy.artifact_validation_self_hash,
            input_hashes: policy.input_hashes,
            target_defaults_hash: policy.policy.provenance.target_defaults,
            compile_profile_hash: policy.input_hashes.compile_profile_hash,
            profile_defaults_hash: policy.policy.provenance.profile_defaults,
            compile_objective_hash: policy.policy.provenance.compile_request_hash,
            crate_feature_set_hash: crate_feature_set_hash(),
            policy_resolution_schema_hash: policy_resolution_schema_hash(),
        }
    }

    fn stage1_material() -> Stage1CacheKeyMaterial {
        Stage1CacheKeyMaterial {
            artifact_validation_self_hash: hash(36),
            policy_resolution_self_hash: hash(37),
            artifact_effective_core_hash: hash(38),
            lowering_manifest_hash: hash(39),
            resolved_blob_index_hash: hash(40),
            pass_version_quant_graph: PASS_VERSION_QUANT_GRAPH.to_owned(),
            crate_feature_set_hash: crate_feature_set_hash(),
            quant_graph_schema_hash: quant_graph_schema_hash(),
        }
    }

    fn stage2_material(runtime_chrome_budget_hash: Option<Hash256>) -> Stage2CacheKeyMaterial {
        Stage2CacheKeyMaterial {
            policy_resolution_self_hash: hash(40),
            quant_graph_hash: hash(41),
            runtime_chrome_budget_hash,
            target_profile_hash: hash(42),
            crate_feature_set_hash: crate_feature_set_hash(),
            static_budget_schema_hash: static_budget_schema_hash(),
        }
    }

    fn static_budget_report(outcome: ReportOutcome) -> StaticBudgetReport {
        let mut projections = static_budget_v1::BudgetProjectionSection::default();
        let mut runtime_chrome_budget = None;
        let mut runtime_chrome_budget_hash = None;
        let mut failures = Vec::new();
        let mut diagnostics = Vec::new();
        if outcome == ReportOutcome::Passed {
            let budget = runtime_budget_section();
            runtime_chrome_budget_hash =
                Some(static_budget_v1::runtime_chrome_budget_hash(&budget).expect("budget hash"));
            projections.per_bank_occupancy = budget
                .rom_slots
                .iter()
                .map(|slot| static_budget_v1::PerBankEntry {
                    slot: slot.id,
                    class: slot.class,
                    usable_bytes: slot.usable_bytes,
                    reserved_slack: slot.reserved_slack,
                    effective_cap_bytes: i64::from(slot.usable_bytes)
                        - i64::from(slot.reserved_slack),
                    assigned_bytes: 0,
                    residual_bytes: i32::try_from(
                        i64::from(slot.usable_bytes) - i64::from(slot.reserved_slack),
                    )
                    .expect("fixture residual fits"),
                    assigned_components: Vec::new(),
                    placement_caps: slot.placement_caps.clone(),
                })
                .collect();
            runtime_chrome_budget = Some(budget);
        } else {
            let failure = BudgetFailure::MissingRuntimeChromeBudget;
            diagnostics.push(budget_failure_diagnostic(&failure));
            failures.push(failure);
        }

        let body = static_budget_v1::StaticBudgetReportBody {
            identity: static_budget_v1::BudgetIdentitySection {
                artifact_core_hash: hash(1),
                quant_graph_hash: hash(41),
                policy_resolution_self_hash: hash(40),
                runtime_chrome_budget_hash,
                target_profile_hash: hash(42),
            },
            policy: static_budget_v1::BudgetPolicySection {
                placement_profile: PlacementProfile::Budgeted,
                objective_hash: hash(44),
            },
            runtime_chrome_budget,
            projections,
            decision: static_budget_v1::BudgetDecisionSection {
                fits: outcome == ReportOutcome::Passed,
                interpretation: static_budget_v1::static_fit_interpretation_for_fits(
                    outcome == ReportOutcome::Passed,
                ),
                placement_model: static_budget_v1::StaticPlacementModel::BudgetedFirstFit,
                failures,
            },
            diagnostics,
        };
        let report = ReportEnvelope::new(outcome, body)
            .expect("report envelope")
            .with_computed_self_hash()
            .expect("fixture report hashes");
        let self_hash = report.report_self_hash;
        StaticBudgetReport {
            report,
            static_budget_self_hash: self_hash,
            static_budget_canonical_bytes_hash: hash(if outcome == ReportOutcome::Passed {
                100
            } else {
                101
            }),
            reduction_site_facts: Vec::new(),
        }
    }

    fn runtime_budget_section() -> static_budget_v1::RuntimeChromeBudgetSection {
        static_budget_v1::RuntimeChromeBudgetSection {
            target: "dmg-mbc5-8mib-128kib".into(),
            profile: "Bringup".into(),
            runtime_nucleus_hash: hash(45),
            rom_slots: vec![static_budget_v1::RomBudgetSlotEntry {
                id: gbf_foundation::BudgetSlotId::new(1),
                class: BudgetSlotClass::ExpertBank,
                usable_bytes: 1024,
                reserved_slack: 128,
                placement_caps: BTreeSet::from([PlacementProfile::Budgeted]),
            }],
            memory_caps: static_budget_v1::RuntimeMemoryCapSection {
                wram_usable_bytes: 8192,
                sram_usable_bytes: 32768,
                hram_usable_bytes: 127,
                source_target_profile_hash: hash(42),
            },
            wram_reserved: 64,
            sram_reserved: 128,
        }
    }

    fn stage3_material() -> Stage3CacheKeyMaterial {
        Stage3CacheKeyMaterial {
            quant_graph_self_hash: hash(0x71),
            infer_ir_policy_projection_hash: hash(0x72),
            static_budget_self_hash: hash(0x73),
            pass_version_infer_ir: PASS_VERSION_INFER_IR.to_owned(),
            crate_feature_set_hash: crate_feature_set_hash(),
            infer_ir_schema_hash: infer_ir_schema_hash(),
        }
    }

    fn stage3_material_for(product: &GbInferIRProduct) -> Stage3CacheKeyMaterial {
        Stage3CacheKeyMaterial {
            quant_graph_self_hash: product.infer_ir.identity.quant_graph_self_hash,
            infer_ir_policy_projection_hash: product
                .infer_ir
                .identity
                .infer_ir_policy_projection_hash,
            static_budget_self_hash: product.infer_ir.identity.static_budget_self_hash,
            pass_version_infer_ir: PASS_VERSION_INFER_IR.to_owned(),
            crate_feature_set_hash: crate_feature_set_hash(),
            infer_ir_schema_hash: infer_ir_schema_hash(),
        }
    }

    fn stage4_material() -> Stage4CacheKeyMaterial {
        Stage4CacheKeyMaterial {
            infer_ir_self_hash: hash(0x81),
            quant_graph_self_hash: hash(0x82),
            semantic_checkpoint_schema_hash: hash(0x83),
            observation_policy_projection_hash: hash(0x84),
            pass_version_observation_plan: stage4_pass_version_string(),
            crate_feature_set_hash: crate_feature_set_hash(),
            observation_plan_schema_hash: observation_plan_schema_hash(),
            build_active_semantic_checkpoint_schema_schema_hash:
                build_active_semantic_checkpoint_schema_schema_hash(),
            operational_probe_schema_schema_hash: operational_probe_schema_schema_hash(),
            probe_registry_hash: hash(0x86),
            metric_registry_hash: hash(0x87),
            trace_event_layout_registry_hash: hash(0x88),
        }
    }

    fn stage4_material_for(product: &ObservationPlanCoreProduct) -> Stage4CacheKeyMaterial {
        let identity = &product.observation_plan.identity;
        Stage4CacheKeyMaterial {
            infer_ir_self_hash: identity.infer_ir_self_hash,
            quant_graph_self_hash: identity.quant_graph_self_hash,
            semantic_checkpoint_schema_hash: identity.semantic_checkpoint_schema_hash,
            observation_policy_projection_hash: identity.observation_policy_projection_hash,
            pass_version_observation_plan: stage4_pass_version_string(),
            crate_feature_set_hash: crate_feature_set_hash(),
            observation_plan_schema_hash: observation_plan_schema_hash(),
            build_active_semantic_checkpoint_schema_schema_hash:
                build_active_semantic_checkpoint_schema_schema_hash(),
            operational_probe_schema_schema_hash: operational_probe_schema_schema_hash(),
            probe_registry_hash: identity.probe_registry_hash,
            metric_registry_hash: identity.metric_registry_hash,
            trace_event_layout_registry_hash: identity.trace_event_layout_registry_hash,
        }
    }

    fn stage5_material() -> Stage5CacheKeyMaterial {
        Stage5CacheKeyMaterial {
            infer_ir_self_hash: hash(0x91),
            quant_graph_self_hash: hash(0x92),
            static_budget_self_hash: hash(0x93),
            range_policy_projection_hash: hash(0x94),
            pass_version_range_plan: stage5_pass_version_string(),
            crate_feature_set_hash: crate_feature_set_hash(),
            range_plan_schema_hash: range_plan_schema_hash(),
            range_cert_schema_hash: range_cert_schema_hash(),
        }
    }

    fn stage5_material_for(product: &RangePlanCoreProduct) -> Stage5CacheKeyMaterial {
        let identity = product.range_plan.identity;
        Stage5CacheKeyMaterial {
            infer_ir_self_hash: identity.infer_ir_self_hash,
            quant_graph_self_hash: identity.quant_graph_self_hash,
            static_budget_self_hash: identity.static_budget_self_hash,
            range_policy_projection_hash: identity.range_policy_projection_hash,
            pass_version_range_plan: stage5_pass_version_string(),
            crate_feature_set_hash: crate_feature_set_hash(),
            range_plan_schema_hash: range_plan_schema_hash(),
            range_cert_schema_hash: range_cert_schema_hash(),
        }
    }

    fn stage4_context(byte: u8) -> Stage4ReportRewrapContext {
        Stage4ReportRewrapContext {
            profile_id: CompileProfileId::from("Default"),
            audit_parents: ObservationPlanAuditParents {
                policy_resolution_self_hash: hash(byte),
                compile_request_hash: hash(byte.wrapping_add(1)),
                static_budget_self_hash: hash(byte.wrapping_add(2)),
                artifact_aux_hash: hash(byte.wrapping_add(3)),
                locked_observation_knobs: LockedObservationKnobs {
                    trace_demotion_locked: false,
                    optional_probe_floor_locked: false,
                    probe_selection_locked: false,
                },
            },
        }
    }

    fn stage5_context(byte: u8) -> Stage5ReportRewrapContext {
        Stage5ReportRewrapContext {
            audit_parents: RangePlanAuditParents {
                policy_resolution_self_hash: hash(byte),
                compile_request_hash: hash(byte.wrapping_add(1)),
                artifact_aux_hash: hash(byte.wrapping_add(2)),
                locked_range_knobs: LockedRangeKnobs {
                    reduction_ceiling_locked: false,
                },
            },
        }
    }

    fn stage4_product() -> ObservationPlanCoreProduct {
        let observation_plan = ObservationPlan {
            identity: ObservationPlanIdentity {
                infer_ir_self_hash: hash(0x81),
                quant_graph_self_hash: hash(0x82),
                semantic_checkpoint_schema_hash: hash(0x83),
                observation_policy_projection_hash: hash(0x84),
                determinism: DeterminismClass::BitExact,
                observability_mode: ObservabilityMode::Invariant,
                trace_budget: TraceBudget::new(8, 128, TraceDropPolicy::DropOldest)
                    .expect("trace budget"),
                workload_id: WorkloadId::from("stage4.cache.fixture"),
                probe_registry_hash: hash(0x86),
                metric_registry_hash: hash(0x87),
                trace_event_layout_registry_hash: hash(0x88),
            },
            semantic: Vec::new(),
            probes: Vec::new(),
            metrics: Vec::new(),
            anchor_table: AnchorAttachmentTable {
                semantic: BTreeMap::new(),
                probes: BTreeMap::new(),
                metrics: BTreeMap::new(),
            },
            provenance: ObservationProvenance {
                semantic_provenance: BTreeMap::new(),
                probe_provenance: BTreeMap::new(),
                metric_provenance: BTreeMap::new(),
            },
            trace_budget_projection: TraceBudgetProjection {
                projected_max_events_per_slice: 0,
                projected_max_bytes_per_frame: 0,
                fits_declared_budget: true,
            },
        };
        let build_active_checkpoint_schema = BuildActiveCheckpointSchema {
            checkpoints: Vec::new(),
            build_active_count: 0,
            mandatory_count: 0,
            optional_count: 0,
        };
        let operational_probe_schema = OperationalProbeSchema {
            probes: Vec::new(),
            metrics: Vec::new(),
            probe_count: 0,
            metric_count: 0,
            per_class_probe_weight_total: PerClassWeightTotal::default(),
            per_class_metric_weight_total: PerClassWeightTotal::default(),
            per_class_total_weight: PerClassWeightTotal::default(),
        };

        ObservationPlanCoreProduct {
            observation_plan_self_hash: observation_plan_self_hash(&observation_plan)
                .expect("observation plan hashes"),
            build_active_checkpoint_schema_hash:
                crate::s4::observation_plan::build_active_checkpoint_schema_hash(
                    &build_active_checkpoint_schema,
                )
                .expect("checkpoint schema hashes"),
            operational_probe_schema_hash:
                crate::s4::observation_plan::operational_probe_schema_hash(
                    &operational_probe_schema,
                )
                .expect("operational probe schema hashes"),
            observation_plan,
            build_active_checkpoint_schema,
            operational_probe_schema,
        }
    }

    fn stage5_product() -> RangePlanCoreProduct {
        let range_plan = RangePlan {
            identity: RangePlanIdentity {
                infer_ir_self_hash: hash(0x91),
                quant_graph_self_hash: hash(0x92),
                static_budget_self_hash: hash(0x93),
                range_policy_projection_hash: hash(0x94),
                determinism: DeterminismClass::BitExact,
            },
            entries: Vec::new(),
            provenance: RangePlanProvenance {
                site_to_node: BTreeMap::new(),
                site_to_qg: BTreeMap::new(),
            },
        };
        let range_plan_self_hash = range_plan_self_hash(&range_plan).expect("range plan hashes");
        let range_cert = RangeCertBody {
            identity: RangeCertIdentity {
                range_plan_self_hash: Some(range_plan_self_hash),
                infer_ir_self_hash: range_plan.identity.infer_ir_self_hash,
                quant_graph_self_hash: range_plan.identity.quant_graph_self_hash,
                static_budget_self_hash: range_plan.identity.static_budget_self_hash,
                determinism: range_plan.identity.determinism,
            },
            cert_outcome: CertOutcome::Verified,
            certificates: Vec::new(),
            site_to_certificate_index: BTreeMap::new(),
            diagnostics: Vec::new(),
        };

        RangePlanCoreProduct {
            range_plan_self_hash,
            range_cert_body_hash: range_cert_body_hash(&range_cert).expect("range cert hashes"),
            range_plan,
            range_cert,
        }
    }

    fn stage4_failure(
        product: &ObservationPlanCoreProduct,
        context: &Stage4ReportRewrapContext,
    ) -> ObservationPlanCoreFailure {
        let diagnostic = hard_diagnostic();
        ObservationPlanCoreFailure {
            observation_plan_body: ObservationPlanReportBody {
                input_identity: stage4_observation_input_identity(product, context),
                result: None,
                diagnostics: vec![diagnostic.clone()],
            },
            sc_re_emit_body: None,
            operational_probe_body: None,
            diagnostics: NonEmptyList::new(vec![diagnostic])
                .expect("failure diagnostics are non-empty"),
        }
    }

    fn stage5_failure(
        product: &RangePlanCoreProduct,
        context: &Stage5ReportRewrapContext,
    ) -> RangePlanCoreFailure {
        let diagnostic = hard_diagnostic();
        let mut failed_cert = product.range_cert.clone();
        failed_cert.identity.range_plan_self_hash = None;
        failed_cert.cert_outcome = CertOutcome::Failed;
        failed_cert.diagnostics = vec![diagnostic.clone()];
        RangePlanCoreFailure {
            range_plan_body: RangePlanReportBody {
                input_identity: stage5_range_input_identity(product, context),
                result: None,
                diagnostics: vec![diagnostic.clone()],
            },
            range_cert_body: Some(failed_cert),
            diagnostics: vec![diagnostic],
        }
    }

    fn hard_diagnostic() -> ValidationDiagnostic {
        budget_failure_diagnostic(&BudgetFailure::MissingRuntimeChromeBudget)
    }

    fn infer_ir_product() -> GbInferIRProduct {
        let infer_ir = infer_ir_fixture();
        GbInferIRProduct::new(
            infer_ir,
            InferIrAuditParents {
                policy_resolution_self_hash: hash(0x74),
                compile_request_hash: hash(0x75),
            },
            BTreeSet::from([RuntimeMode::Interactive, RuntimeMode::Safe]),
            FixtureEquivalenceTag::Skipped {
                reason: FixtureEquivalenceSkippedReason::NonFixtureBuild,
            },
        )
        .expect("infer_ir product builds")
    }

    fn infer_ir_fixture() -> GbInferIR {
        let token_input = TokenInput::new(
            TokenInputId::new(0),
            ValueId::new(0),
            BTreeSet::from([TokenIngressMode::Prompt]),
        )
        .expect("token input builds");
        let node = crate::s3::infer_ir::GbNode {
            node_id: NodeId::new(0),
            op: InferOp::Embedding {
                token_input: TokenInputId::new(0),
            },
            inputs: vec![ValueId::new(0)],
            effects_in: Vec::new(),
            outputs: vec![ValueId::new(1)],
            effects_out: Vec::new(),
            reduction_site: None,
        };
        let mut provenance = InferIrProvenance::default();
        provenance.nodes.insert(
            NodeId::new(0),
            QuantGraphEntityRef::TokenInput {
                token_input: TokenInputId::new(0),
            },
        );
        provenance.values.insert(
            ValueId::new(0),
            ValueProducerRef::External {
                token_input: TokenInputId::new(0),
            },
        );
        provenance.values.insert(
            ValueId::new(1),
            ValueProducerRef::Node {
                node: NodeId::new(0),
            },
        );
        let anchors = NodeAnchorMap::from([(NodeId::new(0), SemanticAnchor::new(hash(0x76)))]);

        GbInferIR::new(
            InferIrIdentity {
                quant_graph_self_hash: hash(0x71),
                infer_ir_policy_projection_hash: hash(0x72),
                static_budget_self_hash: hash(0x73),
                requested_runtime_modes_hash: hash(0x77),
                determinism: DeterminismClass::BitExact,
                topological_order_hash: hash(0x78),
            },
            vec![token_input],
            vec![node],
            vec![
                ValueDecl {
                    value_id: ValueId::new(0),
                    kind: ValueKind::InputToken,
                    format: ValueFormat::TokenIdDomain { vocab_size: 16 },
                    layout: ValueLayout::scalar(),
                },
                ValueDecl {
                    value_id: ValueId::new(1),
                    kind: ValueKind::EmbeddingOutput,
                    format: ValueFormat::Quant {
                        format: QuantFormat::I8,
                    },
                    layout: ValueLayout {
                        shape: vec![ValueAxis::Model],
                    },
                },
            ],
            Vec::new(),
            provenance,
            anchors,
        )
        .expect("infer_ir fixture builds")
    }

    fn input_hashes() -> ValidatedInputHashes {
        ValidatedInputHashes {
            artifact_source_hash: hash(21),
            artifact_effective_core_hash: hash(22),
            artifact_manifest_hash: hash(23),
            artifact_aux_hash: hash(24),
            lowering_manifest_hash: hash(25),
            hint_bundle_hash: hash(26),
            compile_request_hash: hash(27),
            target_profile_hash: hash(28),
            compile_profile_hash: hash(29),
            calibration_hash: hash(30),
            compatibility_adapter_hash: Some(hash(31)),
        }
    }

    struct CacheHitQuantGraph {
        quant_graph_hash: Hash256,
    }

    impl QuantGraphBudgetSource for CacheHitQuantGraph {
        fn quant_graph_hash(&self) -> Hash256 {
            self.quant_graph_hash
        }

        fn semantic_core_hash(&self) -> Hash256 {
            hash(0x02)
        }

        fn to_budget_view(&self) -> Result<QuantGraphBudgetView, QuantGraphBudgetViewError> {
            panic!("missing runtime chrome budget must not evaluate the quant graph")
        }
    }

    fn hash(byte: u8) -> Hash256 {
        Hash256::from_bytes([byte; 32])
    }

    #[derive(Clone, Debug, Default)]
    struct TraceCapture {
        records: Arc<Mutex<Vec<TraceRecord>>>,
    }

    impl TraceCapture {
        fn records(&self) -> Vec<TraceRecord> {
            self.records
                .lock()
                .expect("trace capture mutex is not poisoned")
                .clone()
        }
    }

    impl<S> tracing_subscriber::layer::Layer<S> for TraceCapture
    where
        S: tracing::Subscriber,
    {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _ctx: tracing_subscriber::layer::Context<'_, S>,
        ) {
            let mut visitor = TraceFieldVisitor::default();
            event.record(&mut visitor);
            self.records
                .lock()
                .expect("trace capture mutex is not poisoned")
                .push(TraceRecord {
                    level: event.metadata().level().as_str().to_owned(),
                    fields: visitor.fields,
                });
        }
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct TraceRecord {
        level: String,
        fields: BTreeMap<String, String>,
    }

    impl TraceRecord {
        fn field_contains(&self, field: &str, needle: &str) -> bool {
            self.fields
                .get(field)
                .is_some_and(|value| value.contains(needle))
        }

        fn field_equals(&self, field: &str, expected: &str) -> bool {
            self.fields
                .get(field)
                .is_some_and(|value| value == expected)
        }

        fn field_u64(&self, field: &str) -> Option<u64> {
            self.fields
                .get(field)
                .and_then(|value| value.parse::<u64>().ok())
        }
    }

    #[derive(Debug, Default)]
    struct TraceFieldVisitor {
        fields: BTreeMap<String, String>,
    }

    impl TraceFieldVisitor {
        fn insert(&mut self, field: &tracing::field::Field, value: String) {
            self.fields.insert(field.name().to_owned(), value);
        }
    }

    impl tracing::field::Visit for TraceFieldVisitor {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn fmt::Debug) {
            self.insert(field, format!("{value:?}"));
        }

        fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
            self.insert(field, value.to_owned());
        }

        fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
            self.insert(field, value.to_string());
        }

        fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
            self.insert(field, value.to_string());
        }
    }
}
