//! Stage-cache key construction and payload cells for F-B2 Stage 0/0.5 and F-B4 Stage 2.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use gbf_foundation::{Hash256, SemVer};
use gbf_policy::ValidationDiagnostic;
use gbf_report::report_schemas::{artifact_validation_v1, policy_resolution_v1, static_budget_v1};
use gbf_report::{ReportBody, ReportOutcome};
use gbf_report::{canonicalize as canonicalize_report, canonicalize_value, compute_self_hash};
use gbf_store::stage_cache::{
    ComponentDigestSet, ComponentId, FeatureFlag, StageCache as StoreStageCache, StageCacheError,
    StageCacheKey, StageId, StageKey,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::budget::StaticBudgetReport;
use crate::policy::{PolicyResolutionStageFailure, ResolvedPolicyProduct};
use crate::validate::{
    CachedValidationProduct, CachedValidationProductRehydrateError, ValidatedInputHashes,
    ValidationStageFailure,
};

pub const PASS_VERSION_VALIDATE: SemVer = SemVer::new(1, 0, 0);
pub const PASS_VERSION_RESOLVE: SemVer = SemVer::new(1, 0, 0);
pub const PASS_VERSION_BUDGET: SemVer = SemVer::new(1, 0, 0);

const STAGE0_VALIDATE_SUCCESS_ID: &str = "gbf-codegen.stage0.validate.success";
const STAGE0_VALIDATE_FAILURE_ID: &str = "gbf-codegen.stage0.validate.failure";
const STAGE05_RESOLVE_SUCCESS_ID: &str = "gbf-codegen.stage0_5.resolve_policy.success";
const STAGE05_RESOLVE_FAILURE_ID: &str = "gbf-codegen.stage0_5.resolve_policy.failure";
const STAGE2_BUDGET_SUCCESS_ID: &str = "gbf-codegen.stage2.static_budget.success";
const STAGE2_BUDGET_FAILURE_ID: &str = "gbf-codegen.stage2.static_budget.failure";

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
pub enum Stage2CellKind {
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

#[derive(Debug)]
pub enum CodegenStageCacheError {
    Store(StageCacheError),
    Json(serde_json::Error),
    CachedValidationProduct(CachedValidationProductRehydrateError),
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
            Self::CachedValidationProduct(err) => write!(f, "{err}"),
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
            Self::CachedValidationProduct(err) => Some(err),
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

impl From<CachedValidationProductRehydrateError> for CodegenStageCacheError {
    fn from(value: CachedValidationProductRehydrateError) -> Self {
        Self::CachedValidationProduct(value)
    }
}

impl From<CachedPolicyProductError> for CodegenStageCacheError {
    fn from(value: CachedPolicyProductError) -> Self {
        Self::CachedPolicyProduct(value)
    }
}

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
        Stage0CacheCell::ValidationSuccess { product, .. } => {
            product.rehydrate_checked()?;
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

    if product.artifact_validation_self_hash != material.artifact_validation_self_hash
        || product.input_hashes != material.input_hashes
    {
        return Err(CachedPolicyProductError::KeyMaterialMismatch);
    }
    if !cached_policy_report_identity_matches_product(product) {
        return Err(CachedPolicyProductError::ReportIdentityMismatch);
    }

    Ok(())
}

fn cached_policy_report_identity_matches_product(product: &ResolvedPolicyProduct) -> bool {
    let input_hashes = product.input_hashes;
    let report = &product.report.body;
    let Some(result) = &report.result else {
        return false;
    };

    report.artifact_identity.artifact_core_hash == input_hashes.artifact_effective_core_hash
        && report.artifact_identity.artifact_manifest_hash == input_hashes.artifact_manifest_hash
        && report.artifact_identity.lowering_manifest_hash == input_hashes.lowering_manifest_hash
        && report.artifact_identity.hint_bundle_hash == input_hashes.hint_bundle_hash
        && report.compile_request.compile_request_hash == input_hashes.compile_request_hash
        && report.compile_request.target_profile_hash == input_hashes.target_profile_hash
        && report.compile_request.calibration_hash == input_hashes.calibration_hash
        && result.provenance.hint_bundle_hash == input_hashes.hint_bundle_hash
        && result.provenance.compile_request_hash == input_hashes.compile_request_hash
        && result.provenance.calibration_hash == input_hashes.calibration_hash
        && result.provenance.target_defaults == product.policy.provenance.target_defaults
        && result.provenance.profile_defaults == product.policy.provenance.profile_defaults
        && result.compile_knobs.global == product.policy.knobs.global
        && result.compile_knobs.bounds == product.policy.knobs.bounds
        && result.compile_knobs.locks == product.policy.knobs.locks
        && result.compile_knobs.overrides == product.policy.knobs.overrides
        && result.compile_knobs.provenance == product.policy.knobs.provenance
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

#[cfg(test)]
mod tests {
    use gbf_hw::target::dmg_mbc5_8mib_128kib;
    use gbf_policy::{
        BudgetFailure, BudgetSlotClass, DEFAULT_COMPILE_PROFILE_ID, PlacementProfile,
        budget_failure_diagnostic,
    };
    use gbf_report::ReportEnvelope;
    use gbf_store::blob::BlobStore;
    use gbf_store::stage_cache::{StageCache, compose_key};
    use tempfile::TempDir;

    use crate::budget::{
        BudgetInputs, QuantGraphBudgetSource, QuantGraphBudgetView, QuantGraphBudgetViewError,
        static_budget_report as run_stage2_static_budget,
    };
    use crate::policy::resolve_policy;

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
        let report_bytes =
            br#"{"cached":["bytes",17],"nested":{"order":"preserved"},"z":true}"#.to_vec();
        let fixture = crate::policy::tests::Fixture::new(DEFAULT_COMPILE_PROFILE_ID);
        let validation = fixture.validation();

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
        let report_bytes = b"cached artifact validation report".to_vec();

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
                canonical_bytes: b"cached artifact validation report".to_vec(),
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
    fn stage_cache_hit_replays_stage05_policy_product_for_budget_resume() {
        let (_dir, store) = store();
        let cache = StageCache::new(&store);
        let fixture = crate::policy::tests::Fixture::new(DEFAULT_COMPILE_PROFILE_ID);
        let validation = fixture.validation();
        let policy = resolve_policy(&validation).expect("policy resolves");
        let material = stage05_material_for(&policy);
        let report_bytes = b"cached policy resolution report".to_vec();

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
                canonical_bytes: b"cached policy resolution report".to_vec(),
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
}
