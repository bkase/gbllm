//! Stage 6 `StoragePlan` types and helpers.

use gbf_foundation::{DomainHash, Hash256};
use gbf_store::stage_cache::StageCache as StoreStageCache;

pub mod alias_engine;
pub mod cache;
pub mod diagnostics;
pub mod driver;
pub mod emitter;
pub mod invariants;
pub mod lifetime;
pub mod overlay_lens;
pub mod persist;
pub mod policy_view;
pub mod predicates;
pub mod rules;
pub mod types;

pub use alias_engine::*;
pub use cache::*;
pub use diagnostics::*;
pub use driver::*;
pub use emitter::*;
pub use invariants::*;
pub use lifetime::*;
pub use overlay_lens::*;
pub use persist::*;
pub use policy_view::*;
pub use predicates::*;
pub use rules::*;
pub use types::*;

use crate::stage_cache::{
    CodegenStageCacheError, StoreBackedStageCacheKeys, StoreBackedStageCellKind,
    StoreBackedStageExpectedHashes, StoreBackedStageRunOutput, StoreBackedStageRunResult,
    run_store_backed_stage_with_cache, stage6_storage_plan_store_key,
};

pub fn run_storage_plan_with_cache(
    cache: &StoreStageCache<'_>,
    input: &StoragePlanCoreInput,
    expected_hashes: StoreBackedStageExpectedHashes,
) -> Result<StoreBackedStageRunOutput<StoragePlanReportResult>, CodegenStageCacheError> {
    let cache_key = StoragePlanCacheKeyInputs::from_input_identity(&input.input_identity)
        .and_then(|key_inputs| key_inputs.cache_key())
        .map_err(|error| CodegenStageCacheError::StageCacheKey {
            stage_id: "6",
            message: error.to_string(),
        })?;
    let keys = StoreBackedStageCacheKeys::new(
        "6",
        stage6_storage_plan_store_key(cache_key, StoreBackedStageCellKind::Success),
        stage6_storage_plan_store_key(cache_key, StoreBackedStageCellKind::FailureMemo),
    );
    run_store_backed_stage_with_cache(cache, &keys, cache_key.0, expected_hashes, || {
        let output = build_storage_plan_core(input);
        let report = emit_storage_plan_report(&output).map_err(|error| {
            CodegenStageCacheError::StageEmit {
                stage_id: "6",
                message: error.to_string(),
            }
        })?;
        let report_self_hash = report.report_self_hash;
        match output.outcome {
            StoragePlanCoreOutcome::Succeeded => {
                let result = output.result.as_ref().ok_or_else(|| {
                    CodegenStageCacheError::StageOutputInvariant {
                        stage_id: "6",
                        message: "succeeded output is missing StoragePlanCoreResult".to_owned(),
                    }
                })?;
                let product =
                    StoragePlanReportResult::from_core_result(&output.input_identity, result);
                let product_self_hash =
                    storage_plan_report_result_self_hash(&product).map_err(|error| {
                        CodegenStageCacheError::StageEmit {
                            stage_id: "6",
                            message: error.to_string(),
                        }
                    })?;
                Ok(StoreBackedStageRunResult::Success {
                    product,
                    product_self_hash,
                    report_self_hash,
                })
            }
            StoragePlanCoreOutcome::Failed => Ok(StoreBackedStageRunResult::FailureMemo {
                diagnostics: StoragePlanReportBody::from_core_output(&output).diagnostics,
                report_self_hash,
            }),
        }
    })
}

pub fn storage_plan_report_result_self_hash(
    result: &StoragePlanReportResult,
) -> Result<Hash256, gbf_foundation::CanonicalJsonError> {
    DomainHash::new(
        "gbf-codegen",
        "StoragePlanReportResult",
        STORAGE_PLAN_SCHEMA_ID,
        "1.0.0",
    )
    .hash(result)
}

#[cfg(test)]
mod tests {
    fn closed_key(parts: &[&str]) -> String {
        parts.concat()
    }

    #[test]
    fn sc11_static_scan_rejects_forbidden_storage_plan_type_surface_names() {
        let source = include_str!("types.rs");
        let forbidden = [
            closed_key(&["byte_", "offset"]),
            closed_key(&["byte_", "alignment"]),
            closed_key(&["byte_", "address"]),
            closed_key(&["concrete_", "bank"]),
            closed_key(&["rom_", "bank"]),
            closed_key(&["sram_", "bank"]),
            closed_key(&["slice_", "id"]),
            closed_key(&["lease_", "id"]),
            closed_key(&["overlay_", "region"]),
            closed_key(&["overlay_", "install"]),
            closed_key(&["page_", "byte_", "address"]),
            closed_key(&["kernel_", "residency"]),
            closed_key(&["SRAM page ", "family"]),
            closed_key(&["working-", "set"]),
            closed_key(&["Resource", "Vector"]),
            closed_key(&["Sched", "Slice"]),
            closed_key(&["Resid", "ency", "Epoch"]),
            closed_key(&["Overlay", "Id"]),
            closed_key(&["Overlay", "Install"]),
            closed_key(&["Kernel", "Resid", "ency"]),
            closed_key(&["Bank", "Class"]),
            closed_key(&["Resid", "ency"]),
        ];

        for key in forbidden {
            assert!(
                !source.contains(&key),
                "storage_plan::types contains forbidden SC11 type-surface name {key:?}"
            );
        }
    }
}
