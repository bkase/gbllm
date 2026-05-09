//! Stage-cache key construction and payload cells for F-B2 Stage 0/0.5.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use gbf_foundation::{Hash256, SemVer};
use gbf_policy::ValidationDiagnostic;
use gbf_report::canonicalize_value;
use gbf_report::report_schemas::{artifact_validation_v1, policy_resolution_v1};
use gbf_store::stage_cache::{
    ComponentDigestSet, ComponentId, FeatureFlag, StageCache as StoreStageCache, StageCacheError,
    StageCacheKey, StageId, StageKey,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::policy::{PolicyResolutionStageFailure, ResolvedPolicyProduct};
use crate::validate::{ValidatedInputHashes, ValidationProduct, ValidationStageFailure};

pub const PASS_VERSION_VALIDATE: SemVer = SemVer::new(1, 0, 0);
pub const PASS_VERSION_RESOLVE: SemVer = SemVer::new(1, 0, 0);

const STAGE0_VALIDATE_SUCCESS_ID: &str = "gbf-codegen.stage0.validate.success";
const STAGE0_VALIDATE_FAILURE_ID: &str = "gbf-codegen.stage0.validate.failure";
const STAGE05_RESOLVE_SUCCESS_ID: &str = "gbf-codegen.stage0_5.resolve_policy.success";
const STAGE05_RESOLVE_FAILURE_ID: &str = "gbf-codegen.stage0_5.resolve_policy.failure";

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
pub struct CachedReportBytes {
    pub report_self_hash: Hash256,
    pub canonical_bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CachedValidationProduct {
    pub input_hashes: ValidatedInputHashes,
    pub artifact_validation_self_hash: Hash256,
    pub artifact_validation_canonical_bytes_hash: Hash256,
}

impl From<&ValidationProduct<'_>> for CachedValidationProduct {
    fn from(product: &ValidationProduct<'_>) -> Self {
        Self {
            input_hashes: product.validated.input_hashes,
            artifact_validation_self_hash: product.artifact_validation_self_hash,
            artifact_validation_canonical_bytes_hash: product
                .artifact_validation_canonical_bytes_hash,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CachedResolvedPolicyProduct {
    pub input_hashes: ValidatedInputHashes,
    pub artifact_validation_self_hash: Hash256,
    pub policy_resolution_self_hash: Hash256,
    pub policy_resolution_canonical_bytes_hash: Hash256,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
        product: Box<CachedResolvedPolicyProduct>,
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
    if !matches!(cell, Stage0CacheCell::ValidationSuccess { .. }) {
        return Err(unexpected_stage0_cell("Stage 0 validation success", &cell));
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
    let report_self_hash = product.artifact_validation_self_hash;
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
    if !matches!(cell, Stage05CacheCell::ResolvePolicySuccess { .. }) {
        return Err(unexpected_stage05_cell("Stage 0.5 policy success", &cell));
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
    let cell = Stage05CacheCell::ResolvePolicySuccess {
        product: Box::new(CachedResolvedPolicyProduct {
            input_hashes: product.input_hashes,
            artifact_validation_self_hash: product.artifact_validation_self_hash,
            policy_resolution_self_hash: product.policy_resolution_self_hash,
            policy_resolution_canonical_bytes_hash: product.policy_resolution_canonical_bytes_hash,
        }),
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

#[must_use]
pub fn materialize_stage0_cached_report(cell: &Stage0CacheCell) -> CachedReportBytes {
    match cell {
        Stage0CacheCell::ValidationSuccess { report, .. }
        | Stage0CacheCell::FailureMemo { report, .. } => report.clone(),
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

#[cfg(test)]
mod tests {
    use gbf_store::blob::BlobStore;
    use gbf_store::stage_cache::{StageCache, compose_key};
    use tempfile::TempDir;

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
    }

    #[test]
    fn stage_cache_hit_materializes_cached_report_bytes() {
        let (_dir, store) = store();
        let cache = StageCache::new(&store);
        let material = stage0_material();
        let report_bytes =
            br#"{"cached":["bytes",17],"nested":{"order":"preserved"},"z":true}"#.to_vec();
        let product = CachedValidationProduct {
            input_hashes: input_hashes(),
            artifact_validation_self_hash: hash(97),
            artifact_validation_canonical_bytes_hash: hash(96),
        };

        put_stage0_success(&cache, &material, product, report_bytes.clone())
            .expect("put success cell");
        let cell = get_stage0_success(&cache, &material)
            .expect("success lookup")
            .expect("cache hit");
        let materialized = materialize_stage0_cached_report(&cell);

        assert_eq!(materialized.report_self_hash, hash(97));
        assert_eq!(materialized.canonical_bytes, report_bytes);
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

    fn hash(byte: u8) -> Hash256 {
        Hash256::from_bytes([byte; 32])
    }
}
