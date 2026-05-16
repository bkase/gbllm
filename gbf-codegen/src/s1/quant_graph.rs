//! Stage 1 `QuantGraph` public type surface.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::io;
use std::path::Path;

use gbf_artifact::norm_plan::NormPlan;
use gbf_artifact::tensor::{CanonicalTensorId, CanonicalTensorLayout};
use gbf_artifact::weight_plan::{
    ScaleFormat as WeightScaleFormat, ScaleGranularity as WeightScaleGranularity,
    TernaryWeightPlan, ThresholdPlan, WeightEncoding,
};
use gbf_foundation::{BlobCodec, BlobRef, ExpertId, FieldPath, Hash256, LayerId};
use gbf_policy::{
    ReductionSiteId, ValidationCode, ValidationDetail, ValidationDiagnostic, ValidationOrigin,
};
use gbf_report::canonicalize as canonicalize_report;
use gbf_report::canonicalize_value;
use gbf_report::report_schemas::quant_graph_v1 as report_schema;
use gbf_report::{ReportEnvelope, ReportOutcome};
use gbf_store::stage_cache::StageCache as StoreStageCache;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::budget::{
    AccumulatorDomain, ExpertProjection, QuantGraphBudgetSource, QuantGraphBudgetView,
    QuantGraphBudgetViewError, ReductionSiteProjection, RoutingProjection, SequenceStateProjection,
};
use crate::stage_cache::{
    CachedReportBytes, CodegenStageCacheError, Stage1CacheCell, Stage1CacheKeyMaterial,
    get_stage1_failure_memo, get_stage1_success, materialize_stage1_cached_report,
    put_stage1_failure_memo, put_stage1_success,
};
use crate::validate::ArtifactResolver;

pub const QUANT_GRAPH_SCHEMA_ID: &str = "quant_graph.v1";
pub const QUANT_GRAPH_SCHEMA_VERSION: &str = "1.0.0";

pub type ExportTensorId = CanonicalTensorId;
pub type TensorProvenanceMap = BTreeMap<TensorId, ExportTensorId>;

pub const PASS_VERSION_QUANT_GRAPH: &str = "1.0.0";
pub const STAGE1_BINDING_IDENTITY_EVENT: &str = "stage1.binding.identity";
pub const STAGE1_BINDING_SEQUENCE_SEMANTICS_EVENT: &str = "stage1.binding.sequence_semantics";
pub const STAGE1_BINDING_NORM_PLAN_ID_PRE_EVENT: &str = "stage1.binding.norm_plan_id_pre";
pub const STAGE1_BINDING_TENSOR_EVENT: &str = "stage1.binding.tensor";
pub const STAGE1_BINDING_TENSOR_ONE_EVENT: &str = "stage1.binding.tensor_one";
pub const STAGE1_BINDING_FIRST_HARD_EVENT: &str = "stage1.binding.first_hard";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuantGraph {
    pub identity: QuantGraphIdentity,
    pub tensors: Vec<QuantTensorRef>,
    pub norm_plans: Vec<NormPlanRecord>,
    #[serde(with = "layer_map_entries")]
    pub layer_norms: BTreeMap<LayerId, LayerNorms>,
    pub routing_table: Option<RoutingTable>,
    pub expert_sections: Vec<ExpertSection>,
    #[serde(with = "layer_map_entries")]
    pub ffn_plans: BTreeMap<LayerId, FfnPlan>,
    pub decode_spec: DecodeSpecRecord,
    pub sequence_semantics: SequenceSemanticsSpec,
    #[serde(with = "tensor_map_entries")]
    pub provenance: TensorProvenanceMap,
    pub classify_head: ClassifyHead,
    pub residual_plan: ResidualPlan,
}

impl QuantGraph {
    #[must_use]
    pub fn norm_plan(&self, site: NormSite) -> Option<&NormPlanRecord> {
        self.norm_plans.iter().find(|record| record.site == site)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuantGraphProduct {
    pub quant_graph: QuantGraph,
    pub report: ReportEnvelope<report_schema::QuantGraphReportBody<QuantGraph>>,
    pub quant_graph_self_hash: Hash256,
    pub quant_graph_canonical_bytes_hash: Hash256,
}

impl QuantGraphProduct {
    pub fn new(
        quant_graph: QuantGraph,
        input_identity: report_schema::QuantGraphInputIdentity,
    ) -> Result<Self, QuantGraphProductError> {
        let quant_graph_self_hash = quant_graph_self_hash(&quant_graph)?;
        let quant_graph_canonical_bytes_hash = quant_graph_canonical_bytes_hash(&quant_graph)?;
        let result = quant_graph_report_result(
            quant_graph.clone(),
            quant_graph_self_hash,
            quant_graph_canonical_bytes_hash,
        )?;
        let report = ReportEnvelope::new(
            ReportOutcome::Passed,
            report_schema::QuantGraphReportBody::new(input_identity, Some(result), Vec::new()),
        )
        .map_err(|err| QuantGraphProductError::ReportEnvelope(err.to_string()))?
        .with_computed_self_hash()
        .map_err(|err| QuantGraphProductError::ReportSelfHash(err.to_string()))?;

        Ok(Self {
            quant_graph,
            report,
            quant_graph_self_hash,
            quant_graph_canonical_bytes_hash,
        })
    }
}

impl QuantGraphBudgetSource for QuantGraphProduct {
    fn quant_graph_hash(&self) -> Hash256 {
        self.quant_graph_self_hash
    }

    fn semantic_core_hash(&self) -> Hash256 {
        self.quant_graph.identity.semantic_core_hash
    }

    fn to_budget_view(&self) -> Result<QuantGraphBudgetView, QuantGraphBudgetViewError> {
        quant_graph_budget_view(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuantGraphProductError {
    CanonicalJson(String),
    ReportEnvelope(String),
    ReportSelfHash(String),
    CountOverflow(&'static str),
}

impl fmt::Display for QuantGraphProductError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CanonicalJson(message) => {
                write!(f, "quant_graph canonical JSON error: {message}")
            }
            Self::ReportEnvelope(message) => {
                write!(f, "quant_graph report envelope error: {message}")
            }
            Self::ReportSelfHash(message) => {
                write!(f, "quant_graph report self-hash error: {message}")
            }
            Self::CountOverflow(field) => write!(f, "quant_graph report count overflows {field}"),
        }
    }
}

impl std::error::Error for QuantGraphProductError {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuantGraphInputs {
    pub identity: IdentityBindingInputs,
    pub resolved_blob_index: ResolvedBlobIndex,
    pub tensor_bindings: Vec<QuantTensorBindingInput>,
    pub norm_plan_bindings: Vec<NormPlanBindingInput>,
    pub router_layers: Vec<RouterLayerBindingInput>,
    pub ffn_plans: BTreeMap<LayerId, FfnPlan>,
    pub decode_plan_id: DecodePlanId,
    pub decode_source: DecodeBindingSource,
    pub decode_caps: DecodeCapabilitySet,
    pub sequence_semantics: SequenceSemanticsBindingInputs,
    pub residual_plan: ResidualPlanInput,
    pub classify_head: ClassifyHead,
    pub tensor_exports: BTreeMap<TensorId, ExportTensorId>,
    /// Upstream artifact/export facts: no training-only tensors may be bound
    /// into the storage-free QuantGraph surface.
    pub training_residue_absent: bool,
    /// Upstream sequence-state facts match the v1 identity-only tensor set.
    pub sequence_semantics_tensors_match: bool,
    pub required_features_supported: bool,
    /// BitExact reductions require an explicitly enforced reduction order.
    pub reduction_order_policy_enforced: bool,
    pub bit_exact_mid_reduction_saturation_absent: bool,
    /// QuantGraph input facts must not carry storage locality/offset metadata.
    pub forbidden_storage_metadata_absent: bool,
    /// Stage 0.5 decode facts agree with the bound decode spec RNG needs.
    pub decode_requires_rng_matches_spec: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouterLayerBindingInput {
    pub layer: LayerId,
    pub n_experts: u16,
    pub router_weight: TensorId,
    pub router_bias: Option<TensorId>,
    pub semantics: RouterSemanticsBindingInput,
}

#[derive(Debug, Clone, PartialEq)]
pub struct QuantGraphStageFailure {
    pub kind: QuantGraphStageFailureKind,
    pub message: String,
    pub report: Option<Box<ReportEnvelope<report_schema::QuantGraphReportBody<QuantGraph>>>>,
    pub diagnostics: Vec<ValidationDiagnostic>,
    pub binding_diagnostics: Vec<QuantGraphBindingDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuantGraphStageFailureKind {
    Rejected,
    BlobResolution,
    CacheHitFailureMemo,
    Product,
    ReportIo,
    StageCache,
}

impl fmt::Display for QuantGraphStageFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for QuantGraphStageFailure {}

impl QuantGraphStageFailure {
    fn rejected(
        inputs: &QuantGraphInputs,
        binding_diagnostics: Vec<QuantGraphBindingDiagnostic>,
    ) -> Self {
        let diagnostics = binding_diagnostics
            .iter()
            .map(validation_diagnostic_from_binding)
            .collect::<Vec<_>>();
        let report = failure_report(inputs, diagnostics.clone())
            .ok()
            .map(Box::new);
        Self {
            kind: QuantGraphStageFailureKind::Rejected,
            message: "Stage 1 QuantGraph binding rejected the inputs".to_owned(),
            report,
            diagnostics,
            binding_diagnostics,
        }
    }

    fn cache_hit_failure_memo(
        report: ReportEnvelope<report_schema::QuantGraphReportBody<QuantGraph>>,
        diagnostics: Vec<ValidationDiagnostic>,
    ) -> Self {
        Self {
            kind: QuantGraphStageFailureKind::CacheHitFailureMemo,
            message: "Stage 1 QuantGraph failure memo replayed from cache".to_owned(),
            report: Some(Box::new(report)),
            diagnostics,
            binding_diagnostics: Vec::new(),
        }
    }

    fn product(inputs: &QuantGraphInputs, err: QuantGraphProductError) -> Self {
        let field = FieldPath::from("quant_graph.report");
        let diagnostics = vec![ValidationDiagnostic::hard(
            ValidationOrigin::Schema,
            ValidationCode::ReportSemanticInvariantViolated {
                field: field.clone(),
            },
            ValidationDetail::Field { field },
            Vec::new(),
        )];
        let report = failure_report(inputs, diagnostics.clone())
            .ok()
            .map(Box::new);
        Self {
            kind: QuantGraphStageFailureKind::Product,
            message: err.to_string(),
            report,
            diagnostics,
            binding_diagnostics: Vec::new(),
        }
    }

    fn blob_resolution(
        inputs: &QuantGraphInputs,
        blob_ref: BlobRef,
        message: impl Into<String>,
    ) -> Self {
        let binding_diagnostics = vec![QuantGraphBindingDiagnostic::new(
            QuantGraphBindingDiagnosticCode::QuantGraphBlobRefUnresolvable { blob_ref },
            "tensors.blob_ref",
        )];
        let diagnostics = binding_diagnostics
            .iter()
            .map(validation_diagnostic_from_binding)
            .collect::<Vec<_>>();
        let report = failure_report(inputs, diagnostics.clone())
            .ok()
            .map(Box::new);
        Self {
            kind: QuantGraphStageFailureKind::BlobResolution,
            message: message.into(),
            report,
            diagnostics,
            binding_diagnostics,
        }
    }

    fn blob_len_mismatch(inputs: &QuantGraphInputs, blob_ref: BlobRef, actual: u64) -> Self {
        let binding_diagnostics = vec![QuantGraphBindingDiagnostic::new(
            QuantGraphBindingDiagnosticCode::QuantGraphBlobRefSizeMismatch {
                blob_ref,
                expected_decoded_size_bytes: u64::from(blob_ref.len),
                observed_decoded_size_bytes: actual,
            },
            "tensors.blob_ref.len",
        )];
        let diagnostics = binding_diagnostics
            .iter()
            .map(validation_diagnostic_from_binding)
            .collect::<Vec<_>>();
        let report = failure_report(inputs, diagnostics.clone())
            .ok()
            .map(Box::new);
        Self {
            kind: QuantGraphStageFailureKind::BlobResolution,
            message: format!(
                "Stage 1 blob {} length mismatch: expected {}, observed {actual}",
                blob_ref.hash, blob_ref.len
            ),
            report,
            diagnostics,
            binding_diagnostics,
        }
    }

    fn blob_hash_mismatch(inputs: &QuantGraphInputs, blob_ref: BlobRef, observed: Hash256) -> Self {
        let field = FieldPath::from("tensors.blob_ref.hash");
        let diagnostics = vec![ValidationDiagnostic::hard(
            ValidationOrigin::SemanticCore,
            ValidationCode::ArtifactBlobDigestMismatch {
                blob: blob_ref,
                expected: blob_ref.hash,
                observed,
            },
            ValidationDetail::HashMismatch {
                expected: blob_ref.hash,
                observed,
            },
            Vec::new(),
        )];
        let binding_diagnostics = vec![QuantGraphBindingDiagnostic::new(
            QuantGraphBindingDiagnosticCode::QuantGraphBlobRefUnresolvable { blob_ref },
            field.to_string(),
        )];
        let report = failure_report(inputs, diagnostics.clone())
            .ok()
            .map(Box::new);
        Self {
            kind: QuantGraphStageFailureKind::BlobResolution,
            message: format!(
                "Stage 1 blob {} hash mismatch: expected {}, observed {observed}",
                blob_ref.hash, blob_ref.hash
            ),
            report,
            diagnostics,
            binding_diagnostics,
        }
    }

    fn report_io(message: impl Into<String>) -> Self {
        Self {
            kind: QuantGraphStageFailureKind::ReportIo,
            message: message.into(),
            report: None,
            diagnostics: Vec::new(),
            binding_diagnostics: Vec::new(),
        }
    }

    fn stage_cache(err: CodegenStageCacheError) -> Self {
        Self {
            kind: QuantGraphStageFailureKind::StageCache,
            message: err.to_string(),
            report: None,
            diagnostics: Vec::new(),
            binding_diagnostics: Vec::new(),
        }
    }
}

#[derive(Clone, Copy)]
pub struct PassEnvironment<'a> {
    pub resolver: &'a dyn ArtifactResolver,
    pub report_dir: Option<&'a Path>,
    pub stage_cache: Option<&'a StoreStageCache<'a>>,
}

impl<'a> PassEnvironment<'a> {
    #[must_use]
    pub const fn new(resolver: &'a dyn ArtifactResolver) -> Self {
        Self {
            resolver,
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

#[allow(clippy::result_large_err)]
pub fn build_quant_graph_core(
    inputs: QuantGraphInputs,
) -> Result<QuantGraphProduct, QuantGraphStageFailure> {
    let class_1_to_4 = bind_construction_classes_1_to_4(&inputs)
        .map_err(|diagnostics| QuantGraphStageFailure::rejected(&inputs, diagnostics))?;
    let norm_plans = bind_norm_plans(&inputs.norm_plan_bindings, &class_1_to_4.norm_plan_ids)
        .map_err(|diagnostics| QuantGraphStageFailure::rejected(&inputs, diagnostics))?;
    let layer_norms = bind_layer_norms(
        class_1_to_4.identity.model_spec_summary.n_layers,
        &norm_plans,
    )
    .map_err(|diagnostics| QuantGraphStageFailure::rejected(&inputs, diagnostics))?;
    let router_layers = bind_router_layers(&inputs.router_layers)
        .map_err(|diagnostics| QuantGraphStageFailure::rejected(&inputs, diagnostics))?;
    let routing_table =
        bind_routing_table(&class_1_to_4.identity.model_spec_summary, router_layers)
            .map_err(|diagnostics| QuantGraphStageFailure::rejected(&inputs, diagnostics))?;
    let expert_sections = bind_expert_sections(
        &class_1_to_4.identity.model_spec_summary,
        &inputs.ffn_plans,
        &class_1_to_4.tensors,
    )
    .map_err(|diagnostics| QuantGraphStageFailure::rejected(&inputs, diagnostics))?;
    let residual_plan = bind_residual_plan(inputs.residual_plan.clone())
        .map_err(|diagnostics| QuantGraphStageFailure::rejected(&inputs, diagnostics))?;
    let decode_spec = bind_decode_spec(
        inputs.decode_plan_id,
        inputs.decode_source.clone(),
        &inputs.decode_caps,
    )
    .map_err(|diagnostics| QuantGraphStageFailure::rejected(&inputs, diagnostics))?;
    let classify_head = bind_classify_head(
        inputs.classify_head.clone(),
        embedding_tensor_id(&class_1_to_4.tensors),
    )
    .map_err(|diagnostics| QuantGraphStageFailure::rejected(&inputs, diagnostics))?;
    let provenance = bind_provenance(&class_1_to_4.tensors, &inputs.tensor_exports)
        .map_err(|diagnostics| QuantGraphStageFailure::rejected(&inputs, diagnostics))?;

    let mut quant_graph = QuantGraph {
        identity: class_1_to_4.identity,
        tensors: class_1_to_4.tensors,
        norm_plans,
        layer_norms,
        routing_table,
        expert_sections,
        ffn_plans: inputs.ffn_plans.clone(),
        decode_spec,
        sequence_semantics: class_1_to_4.sequence_semantics,
        provenance,
        classify_head,
        residual_plan,
    };
    canonical_sort_quant_graph(&mut quant_graph);

    let self_consistency_diagnostics = validate_quant_graph_self_consistency(
        &quant_graph,
        QuantGraphSelfConsistencyContext {
            decode_caps: &inputs.decode_caps,
            blob_index: Some(&inputs.resolved_blob_index),
            training_residue_absent: inputs.training_residue_absent,
            sequence_semantics_tensors_match: inputs.sequence_semantics_tensors_match,
            required_features_supported: inputs.required_features_supported,
            reduction_order_policy_enforced: inputs.reduction_order_policy_enforced,
            bit_exact_mid_reduction_saturation_absent: inputs
                .bit_exact_mid_reduction_saturation_absent,
            forbidden_storage_metadata_absent: inputs.forbidden_storage_metadata_absent,
            decode_requires_rng_matches_spec: inputs.decode_requires_rng_matches_spec,
        },
    );
    if !self_consistency_diagnostics.is_empty() {
        return Err(QuantGraphStageFailure::rejected(
            &inputs,
            self_consistency_diagnostics,
        ));
    }

    let input_identity =
        quant_graph_input_identity(&quant_graph, inputs.resolved_blob_index.self_hash);
    QuantGraphProduct::new(quant_graph, input_identity)
        .map_err(|err| QuantGraphStageFailure::product(&inputs, err))
}

#[allow(clippy::result_large_err)]
pub fn run_stage1(
    mut inputs: QuantGraphInputs,
    env: PassEnvironment<'_>,
) -> Result<QuantGraphProduct, QuantGraphStageFailure> {
    let resolved_blob_index = build_stage1_resolved_blob_index(&inputs, env.resolver);
    inputs.resolved_blob_index = match resolved_blob_index {
        Ok(index) => {
            tracing::debug!(
                n_entries = index.entries.len() as u64,
                self_hash = %index.self_hash,
                "stage1.driver.blob_index_built"
            );
            index
        }
        Err(failure) => {
            emit_failure_report(env.report_dir, &failure)?;
            return Err(failure);
        }
    };
    let material = stage1_cache_key_material(&inputs);

    if let Some(cache) = env.stage_cache {
        tracing::debug!(k1_hash = ?material, state = "lookup", "stage1.driver.cache_lookup");
        if let Some(cell) =
            get_stage1_success(cache, &material).map_err(QuantGraphStageFailure::stage_cache)?
        {
            let cached_report = materialize_stage1_cached_report(&cell);
            emit_quant_graph_report_bytes(env.report_dir, &cached_report.canonical_bytes)?;
            if let Stage1CacheCell::QuantGraphSuccess { product, .. } = cell {
                tracing::info!(
                    quant_graph_self_hash = %product.quant_graph_self_hash,
                    cache_state = "hit_success",
                    total_ms = 0_u64,
                    "stage1.driver.run"
                );
                return Ok(*product);
            }
        }
        if let Some(cell) = get_stage1_failure_memo(cache, &material)
            .map_err(QuantGraphStageFailure::stage_cache)?
        {
            let cached_report = materialize_stage1_cached_report(&cell);
            emit_quant_graph_report_bytes(env.report_dir, &cached_report.canonical_bytes)?;
            if let Stage1CacheCell::FailureMemo { diagnostics, .. } = cell {
                let report = serde_json::from_slice::<
                    ReportEnvelope<report_schema::QuantGraphReportBody<QuantGraph>>,
                >(&cached_report.canonical_bytes)
                .map_err(|err| {
                    QuantGraphStageFailure::report_io(format!(
                        "cached Stage 1 failure report did not decode: {err}"
                    ))
                })?;
                return Err(QuantGraphStageFailure::cache_hit_failure_memo(
                    report,
                    diagnostics,
                ));
            }
        }
    }

    match build_quant_graph_core(inputs.clone()) {
        Ok(product) => {
            let report_bytes = canonicalize_quant_graph_report(&product.report)?;
            emit_quant_graph_report_bytes(env.report_dir, &report_bytes)?;
            if let Some(cache) = env.stage_cache {
                put_stage1_success(cache, &material, &product, report_bytes)
                    .map_err(QuantGraphStageFailure::stage_cache)?;
            }
            tracing::info!(
                quant_graph_self_hash = %product.quant_graph_self_hash,
                cache_state = "miss_success",
                total_ms = 0_u64,
                "stage1.driver.run"
            );
            Ok(product)
        }
        Err(failure) => {
            if let Some(report) = failure.report.as_deref() {
                let report_bytes = canonicalize_quant_graph_report(report)?;
                emit_quant_graph_report_bytes(env.report_dir, &report_bytes)?;
                if let Some(cache) = env.stage_cache {
                    put_stage1_failure_memo(
                        cache,
                        &material,
                        CachedReportBytes {
                            report_self_hash: report.report_self_hash,
                            canonical_bytes: report_bytes,
                        },
                        failure.diagnostics.clone(),
                    )
                    .map_err(QuantGraphStageFailure::stage_cache)?;
                }
            }
            Err(failure)
        }
    }
}

#[must_use]
pub fn stage1_cache_key_material(inputs: &QuantGraphInputs) -> Stage1CacheKeyMaterial {
    // A76 reconciliation: Stage 1 keys on the full policy-resolution hash.
    // `QuantGraph` has no compile_request_hash audit parent, so cache hits do
    // not need the Stage 3 policy-projection/audit-rewrap split.
    Stage1CacheKeyMaterial::new(
        inputs.identity.artifact_validation_self_hash,
        inputs.identity.policy_resolution_self_hash,
        inputs.identity.validated_artifact_effective_core_hash,
        inputs.identity.lowering_manifest_hash,
        inputs.resolved_blob_index.self_hash,
    )
}

pub fn build_stage1_resolved_blob_index(
    inputs: &QuantGraphInputs,
    resolver: &dyn ArtifactResolver,
) -> Result<ResolvedBlobIndex, QuantGraphStageFailure> {
    let mut entries = BTreeMap::new();
    for blob_ref in quant_graph_blob_refs(inputs) {
        let resolved = resolver.resolve_blob(&blob_ref).map_err(|err| {
            QuantGraphStageFailure::blob_resolution(
                inputs,
                blob_ref,
                format!("Stage 1 failed to resolve blob {}: {err}", blob_ref.hash),
            )
        })?;
        if resolved.content_hash != blob_ref.hash {
            return Err(QuantGraphStageFailure::blob_hash_mismatch(
                inputs,
                blob_ref,
                resolved.content_hash,
            ));
        }
        let encoded_size_bytes = u64::try_from(resolved.bytes.len()).map_err(|_| {
            QuantGraphStageFailure::blob_resolution(
                inputs,
                blob_ref,
                format!("Stage 1 blob {} length exceeds u64", blob_ref.hash),
            )
        })?;
        if encoded_size_bytes != u64::from(blob_ref.len) {
            return Err(QuantGraphStageFailure::blob_len_mismatch(
                inputs,
                blob_ref,
                encoded_size_bytes,
            ));
        }
        entries.insert(
            blob_ref,
            BlobMetadata {
                content_hash: resolved.content_hash,
                encoded_size_bytes,
                decoded_size_bytes: encoded_size_bytes,
                codec: blob_ref.codec,
            },
        );
    }
    Ok(ResolvedBlobIndex::new(entries))
}

fn bind_router_layers(
    inputs: &[RouterLayerBindingInput],
) -> Result<Vec<RouterLayer>, Vec<QuantGraphBindingDiagnostic>> {
    let mut diagnostics = Vec::new();
    let mut router_layers = Vec::new();
    for input in inputs {
        match bind_router_semantics_v1(input.semantics.clone()) {
            Ok(semantics) => router_layers.push(RouterLayer {
                layer: input.layer,
                n_experts: input.n_experts,
                router_weight: input.router_weight,
                router_bias: input.router_bias,
                semantics,
            }),
            Err(mut semantic_diagnostics) => diagnostics.append(&mut semantic_diagnostics),
        }
    }

    if diagnostics.is_empty() {
        Ok(router_layers)
    } else {
        Err(diagnostics)
    }
}

fn embedding_tensor_id(tensors: &[QuantTensorRef]) -> TensorId {
    tensors
        .iter()
        .find(|tensor| tensor.role == QuantTensorRole::EmbeddingTable)
        .map(|tensor| tensor.tensor_id)
        .unwrap_or_else(|| TensorId::new(u32::MAX))
}

fn quant_graph_blob_refs(inputs: &QuantGraphInputs) -> BTreeSet<BlobRef> {
    let mut refs = BTreeSet::new();
    for tensor in &inputs.tensor_bindings {
        refs.insert(tensor.blob_ref);
        refs.extend(tensor.aux_blob_refs.iter().map(|aux| aux.blob_ref));
    }
    refs
}

fn failure_report(
    inputs: &QuantGraphInputs,
    diagnostics: Vec<ValidationDiagnostic>,
) -> Result<ReportEnvelope<report_schema::QuantGraphReportBody<QuantGraph>>, QuantGraphProductError>
{
    ReportEnvelope::new(
        ReportOutcome::Failed,
        report_schema::QuantGraphReportBody::<QuantGraph>::new(
            quant_graph_input_identity_from_inputs(inputs),
            None,
            diagnostics,
        ),
    )
    .map_err(|err| QuantGraphProductError::ReportEnvelope(err.to_string()))?
    .with_computed_self_hash()
    .map_err(|err| QuantGraphProductError::ReportSelfHash(err.to_string()))
}

fn quant_graph_input_identity_from_inputs(
    inputs: &QuantGraphInputs,
) -> report_schema::QuantGraphInputIdentity {
    report_schema::QuantGraphInputIdentity {
        artifact_core_hash: inputs.identity.artifact_core_hash,
        artifact_validation_self_hash: inputs.identity.artifact_validation_self_hash,
        policy_resolution_self_hash: inputs.identity.policy_resolution_self_hash,
        semantic_core_hash: inputs.identity.semantic_core_hash,
        lowering_manifest_hash: inputs.identity.lowering_manifest_hash,
        resolved_blob_index_hash: inputs.resolved_blob_index.self_hash,
        determinism: match inputs.identity.artifact_determinism {
            DeterminismClass::BitExact => report_schema::DeterminismClassTag::BitExact,
            DeterminismClass::Deterministic => report_schema::DeterminismClassTag::Deterministic,
            DeterminismClass::Nondeterministic => {
                report_schema::DeterminismClassTag::Nondeterministic
            }
        },
        model_spec_summary: report_model_spec_summary(&inputs.identity.model_spec_summary),
        sequence_semantics_kind: match inputs.sequence_semantics.artifact_sequence.kind {
            SequenceSemanticsKind::Identity => report_schema::SequenceSemanticsKindTag::Identity,
            SequenceSemanticsKind::LinearState => {
                report_schema::SequenceSemanticsKindTag::LinearState
            }
            SequenceSemanticsKind::BoundedKv => report_schema::SequenceSemanticsKindTag::BoundedKv,
        },
        ffn_topology_kind: match compute_ffn_topology_kind(&inputs.identity.model_spec_summary) {
            FfnTopologyKindTag::Dense => report_schema::FfnTopologyKindTag::Dense,
            FfnTopologyKindTag::Routed => report_schema::FfnTopologyKindTag::Routed,
            FfnTopologyKindTag::Mixed => report_schema::FfnTopologyKindTag::Mixed,
        },
    }
}

fn validation_diagnostic_from_binding(
    diagnostic: &QuantGraphBindingDiagnostic,
) -> ValidationDiagnostic {
    ValidationDiagnostic::hard(
        ValidationOrigin::SemanticCore,
        ValidationCode::ArtifactPayloadMalformed {
            field: diagnostic.field.clone(),
        },
        ValidationDetail::Field {
            field: diagnostic.field.clone(),
        },
        Vec::new(),
    )
}

fn emit_first_hard_binding_diagnostic(
    class: QuantGraphBindingClass,
    diagnostics: &[QuantGraphBindingDiagnostic],
) {
    let Some(first) = diagnostics.first() else {
        return;
    };
    tracing::error!(
        event = STAGE1_BINDING_FIRST_HARD_EVENT,
        class = ?class,
        field = %first.field,
        code = ?first.code,
        "stage1.binding.first_hard"
    );
}

fn canonicalize_quant_graph_report(
    report: &ReportEnvelope<report_schema::QuantGraphReportBody<QuantGraph>>,
) -> Result<Vec<u8>, QuantGraphStageFailure> {
    canonicalize_report(report).map_err(|err| {
        QuantGraphStageFailure::report_io(format!(
            "Stage 1 quant_graph report did not canonicalize: {err}"
        ))
    })
}

fn emit_failure_report(
    report_dir: Option<&Path>,
    failure: &QuantGraphStageFailure,
) -> Result<(), QuantGraphStageFailure> {
    if let Some(report) = failure.report.as_deref() {
        let bytes = canonicalize_quant_graph_report(report)?;
        emit_quant_graph_report_bytes(report_dir, &bytes)?;
    }
    Ok(())
}

fn emit_quant_graph_report_bytes(
    report_dir: Option<&Path>,
    bytes: &[u8],
) -> Result<(), QuantGraphStageFailure> {
    let Some(report_dir) = report_dir else {
        return Ok(());
    };
    fs::create_dir_all(report_dir).map_err(|err| {
        report_io_error(format!(
            "failed to create Stage 1 report directory {}: {err}",
            report_dir.display()
        ))
    })?;
    let path = report_dir.join("quant_graph.json");
    fs::write(&path, bytes).map_err(|err| {
        report_io_error(format!(
            "failed to write Stage 1 report {}: {err}",
            path.display()
        ))
    })?;
    tracing::debug!(
        canonical_bytes_len = bytes.len() as u64,
        report_path = %path.display(),
        "stage1.driver.report_emit"
    );
    Ok(())
}

fn report_io_error(message: String) -> QuantGraphStageFailure {
    QuantGraphStageFailure::report_io(io::Error::other(message).to_string())
}

#[derive(Debug)]
struct ConstructionClass1To4Bindings {
    identity: QuantGraphIdentity,
    sequence_semantics: SequenceSemanticsSpec,
    norm_plan_ids: NormPlanIdPreBinding,
    tensors: Vec<QuantTensorRef>,
}

fn bind_construction_classes_1_to_4(
    inputs: &QuantGraphInputs,
) -> Result<ConstructionClass1To4Bindings, Vec<QuantGraphBindingDiagnostic>> {
    let mut diagnostics = Vec::new();
    let identity = match bind_identity(inputs.identity.clone()) {
        Ok(identity) => Some(identity),
        Err(mut identity_diagnostics) => {
            diagnostics.append(&mut identity_diagnostics);
            None
        }
    };
    let sequence_semantics = match bind_sequence_semantics(inputs.sequence_semantics.clone()) {
        Ok(sequence_semantics) => Some(sequence_semantics),
        Err(mut sequence_diagnostics) => {
            diagnostics.append(&mut sequence_diagnostics);
            None
        }
    };
    let norm_plan_ids = bind_norm_plan_ids(inputs.identity.model_spec_summary.n_layers);
    let tensors = match bind_quant_tensors(&inputs.tensor_bindings, &inputs.resolved_blob_index) {
        Ok(tensors) => Some(tensors),
        Err(mut tensor_diagnostics) => {
            diagnostics.append(&mut tensor_diagnostics);
            None
        }
    };

    if diagnostics.is_empty() {
        Ok(ConstructionClass1To4Bindings {
            identity: identity.expect("identity exists when diagnostics are empty"),
            sequence_semantics: sequence_semantics
                .expect("sequence semantics exists when diagnostics are empty"),
            norm_plan_ids,
            tensors: tensors.expect("tensors exist when diagnostics are empty"),
        })
    } else {
        Err(diagnostics)
    }
}

#[must_use]
pub fn quant_graph_input_identity(
    quant_graph: &QuantGraph,
    resolved_blob_index_hash: Hash256,
) -> report_schema::QuantGraphInputIdentity {
    report_schema::QuantGraphInputIdentity {
        artifact_core_hash: quant_graph.identity.artifact_core_hash,
        artifact_validation_self_hash: quant_graph.identity.artifact_validation_self_hash,
        policy_resolution_self_hash: quant_graph.identity.policy_resolution_self_hash,
        semantic_core_hash: quant_graph.identity.semantic_core_hash,
        lowering_manifest_hash: quant_graph.identity.lowering_manifest_hash,
        resolved_blob_index_hash,
        determinism: match quant_graph.identity.determinism {
            DeterminismClass::BitExact => report_schema::DeterminismClassTag::BitExact,
            DeterminismClass::Deterministic => report_schema::DeterminismClassTag::Deterministic,
            DeterminismClass::Nondeterministic => {
                report_schema::DeterminismClassTag::Nondeterministic
            }
        },
        model_spec_summary: report_model_spec_summary(&quant_graph.identity.model_spec_summary),
        sequence_semantics_kind: match quant_graph.sequence_semantics.kind {
            SequenceSemanticsKind::Identity => report_schema::SequenceSemanticsKindTag::Identity,
            SequenceSemanticsKind::LinearState => {
                report_schema::SequenceSemanticsKindTag::LinearState
            }
            SequenceSemanticsKind::BoundedKv => report_schema::SequenceSemanticsKindTag::BoundedKv,
        },
        ffn_topology_kind: match compute_ffn_topology_kind(&quant_graph.identity.model_spec_summary)
        {
            FfnTopologyKindTag::Dense => report_schema::FfnTopologyKindTag::Dense,
            FfnTopologyKindTag::Routed => report_schema::FfnTopologyKindTag::Routed,
            FfnTopologyKindTag::Mixed => report_schema::FfnTopologyKindTag::Mixed,
        },
    }
}

fn report_model_spec_summary(model: &ModelSpecSummary) -> report_schema::ModelSpecSummary {
    report_schema::ModelSpecSummary {
        n_layers: model.n_layers,
        n_experts: model.n_experts.clone(),
        d_model: model.d_model,
        d_ff: model.d_ff,
        vocab_size: model.vocab_size,
        ffn_kind: model
            .ffn_kind
            .iter()
            .map(|(layer, kind)| {
                (
                    *layer,
                    match kind {
                        FfnKindTag::Dense => report_schema::FfnKindTag::Dense,
                        FfnKindTag::Routed => report_schema::FfnKindTag::Routed,
                    },
                )
            })
            .collect(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuantGraphIdentity {
    pub artifact_core_hash: Hash256,
    pub policy_resolution_self_hash: Hash256,
    pub artifact_validation_self_hash: Hash256,
    pub semantic_core_hash: Hash256,
    pub lowering_manifest_hash: Hash256,
    pub determinism: DeterminismClass,
    pub model_spec_summary: ModelSpecSummary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdentityBindingInputs {
    pub artifact_core_hash: Hash256,
    pub policy_resolution_self_hash: Hash256,
    pub artifact_validation_self_hash: Hash256,
    pub semantic_core_hash: Hash256,
    pub validated_artifact_effective_core_hash: Hash256,
    pub lowering_manifest_hash: Hash256,
    pub artifact_determinism: DeterminismClass,
    /// Audit-only context from Stage 0.5. Stage 1 identity binding deliberately
    /// uses `artifact_determinism` as the product determinism source.
    pub policy_determinism: Option<DeterminismClass>,
    pub model_spec_summary: ModelSpecSummary,
}

pub fn bind_identity(
    inputs: IdentityBindingInputs,
) -> Result<QuantGraphIdentity, Vec<QuantGraphBindingDiagnostic>> {
    let mut diagnostics = Vec::new();
    tracing::info!(
        event = STAGE1_BINDING_IDENTITY_EVENT,
        artifact_core_hash = %inputs.artifact_core_hash,
        policy_resolution_self_hash = %inputs.policy_resolution_self_hash,
        n_layers = inputs.model_spec_summary.n_layers as u64,
        policy_determinism_present = inputs.policy_determinism.is_some(),
        "stage1.binding.identity"
    );
    if inputs.semantic_core_hash != inputs.validated_artifact_effective_core_hash {
        diagnostics.push(QuantGraphBindingDiagnostic::new(
            QuantGraphBindingDiagnosticCode::QuantGraphIdentityHashMismatch {
                expected: inputs.validated_artifact_effective_core_hash,
                observed: inputs.semantic_core_hash,
            },
            "identity.semantic_core_hash",
        ));
    }

    if diagnostics.is_empty() {
        Ok(QuantGraphIdentity {
            artifact_core_hash: inputs.artifact_core_hash,
            policy_resolution_self_hash: inputs.policy_resolution_self_hash,
            artifact_validation_self_hash: inputs.artifact_validation_self_hash,
            semantic_core_hash: inputs.semantic_core_hash,
            lowering_manifest_hash: inputs.lowering_manifest_hash,
            determinism: inputs.artifact_determinism,
            model_spec_summary: inputs.model_spec_summary,
        })
    } else {
        emit_first_hard_binding_diagnostic(QuantGraphBindingClass::IdentityBinding, &diagnostics);
        Err(diagnostics)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelSpecSummary {
    pub n_layers: u16,
    #[serde(with = "layer_map_entries")]
    pub n_experts: BTreeMap<LayerId, u16>,
    pub d_model: u32,
    pub d_ff: u32,
    pub vocab_size: u32,
    #[serde(with = "layer_map_entries")]
    pub ffn_kind: BTreeMap<LayerId, FfnKindTag>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TensorId(u32);

impl TensorId {
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl From<u32> for TensorId {
    fn from(value: u32) -> Self {
        Self::new(value)
    }
}

impl From<TensorId> for u32 {
    fn from(value: TensorId) -> Self {
        value.get()
    }
}

impl fmt::Display for TensorId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum DeterminismClass {
    BitExact,
    Deterministic,
    Nondeterministic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
// `*Binding` tags are RFC construction-class names.
#[allow(clippy::enum_variant_names)]
pub enum QuantGraphBindingClass {
    IdentityBinding,
    SequenceSemanticsBinding,
    NormPlanIdPreBinding,
    TensorBinding,
    NormPlanBinding,
    LayerNormsBinding,
    RoutingBinding,
    ExpertBinding,
    ResidualPlanBinding,
    DecodeBinding,
    ClassifyHeadBinding,
    ProvenanceBinding,
    CanonicalSort,
    SelfConsistency,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum FfnKindTag {
    Dense,
    Routed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum FfnTopologyKindTag {
    Dense,
    Routed,
    Mixed,
}

#[must_use]
pub fn compute_ffn_topology_kind(model_spec: &ModelSpecSummary) -> FfnTopologyKindTag {
    let has_dense = model_spec
        .ffn_kind
        .values()
        .any(|kind| matches!(kind, FfnKindTag::Dense));
    let has_routed = model_spec
        .ffn_kind
        .values()
        .any(|kind| matches!(kind, FfnKindTag::Routed));

    match (has_dense, has_routed) {
        (true, true) => FfnTopologyKindTag::Mixed,
        (false, true) => FfnTopologyKindTag::Routed,
        _ => FfnTopologyKindTag::Dense,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuantTensorRef {
    pub tensor_id: TensorId,
    pub layout: CanonicalTensorLayout,
    pub quant_format: QuantFormat,
    pub role: QuantTensorRole,
    pub blob: ResolvedBlobRef,
    pub aux_blob_refs: Vec<QuantAuxBlobRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedBlobRef {
    pub blob_ref: BlobRef,
    pub content_hash: Hash256,
    pub encoded_size_bytes: u64,
    pub decoded_size_bytes: u64,
    pub codec: BlobCodec,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuantAuxBlobRef {
    pub kind: QuantAuxKind,
    pub layout: CanonicalTensorLayout,
    pub format: AuxFormat,
    pub blob: ResolvedBlobRef,
    pub export_tensor_id: ExportTensorId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum QuantAuxKind {
    Scale,
    Threshold,
    SparseMeta,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum AuxFormat {
    Q8_8,
    Q4_4,
    Pow2,
    I8,
    I16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum QuantTensorRole {
    EmbeddingTable,
    NormScale {
        norm_plan: NormPlanId,
    },
    NormBias {
        norm_plan: NormPlanId,
    },
    RouterWeight {
        layer: LayerId,
    },
    RouterBias {
        layer: LayerId,
    },
    ExpertWeight {
        layer: LayerId,
        expert: ExpertId,
        slot: ExpertWeightSlot,
    },
    ExpertBias {
        layer: LayerId,
        expert: ExpertId,
        slot: ExpertWeightSlot,
    },
    ClassifyWeight,
    ClassifyBias,
}

// `Ffn*` tags are schema/RFC terms, not local naming noise.
#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ExpertWeightSlot {
    FfnGate,
    FfnUp,
    FfnDown,
}

pub const EXPERT_WEIGHT_SLOT_CANONICAL_ORDER: [ExpertWeightSlot; 3] = [
    ExpertWeightSlot::FfnGate,
    ExpertWeightSlot::FfnUp,
    ExpertWeightSlot::FfnDown,
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum QuantFormat {
    Ternary2 {
        scale_granularity: ScaleGranularity,
        scale_format: AuxFormat,
        threshold_granularity: ThresholdGranularity,
    },
    Binary1 {
        scale_granularity: ScaleGranularity,
        scale_format: AuxFormat,
    },
    SparseTernaryBitplanes {
        scale_granularity: ScaleGranularity,
        scale_format: AuxFormat,
        sparse_meta_kind: SparseMetaKind,
    },
    Q8_8,
    Q4_4,
    I8,
    I16,
}

#[must_use]
pub fn role_format_allowed(role: &QuantTensorRole, format: &QuantFormat) -> bool {
    match role {
        QuantTensorRole::EmbeddingTable | QuantTensorRole::ClassifyWeight => {
            matches!(format, QuantFormat::I8 | QuantFormat::Q8_8)
        }
        QuantTensorRole::NormScale { .. } | QuantTensorRole::NormBias { .. } => {
            matches!(
                format,
                QuantFormat::Q8_8 | QuantFormat::Q4_4 | QuantFormat::I16
            )
        }
        QuantTensorRole::RouterWeight { .. } => {
            matches!(format, QuantFormat::Q8_8 | QuantFormat::I8)
        }
        QuantTensorRole::RouterBias { .. }
        | QuantTensorRole::ExpertBias { .. }
        | QuantTensorRole::ClassifyBias => {
            matches!(
                format,
                QuantFormat::Q8_8 | QuantFormat::I8 | QuantFormat::I16
            )
        }
        QuantTensorRole::ExpertWeight { .. } => matches!(
            format,
            QuantFormat::Ternary2 { .. }
                | QuantFormat::Binary1 { .. }
                | QuantFormat::SparseTernaryBitplanes { .. }
        ),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ScaleGranularity {
    Global,
    PerOutputRow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ThresholdGranularity {
    Global,
    PerOutputRow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum SparseMetaKind {
    RowOffsets,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedBlobIndex {
    #[serde(with = "resolved_blob_entries")]
    pub entries: BTreeMap<BlobRef, BlobMetadata>,
    pub self_hash: Hash256,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlobMetadata {
    pub content_hash: Hash256,
    pub encoded_size_bytes: u64,
    pub decoded_size_bytes: u64,
    pub codec: BlobCodec,
}

impl ResolvedBlobIndex {
    #[must_use]
    pub fn new(entries: BTreeMap<BlobRef, BlobMetadata>) -> Self {
        let self_hash = resolved_blob_index_self_hash(&entries);
        Self { entries, self_hash }
    }

    #[must_use]
    pub fn resolve(&self, blob_ref: BlobRef) -> Option<ResolvedBlobRef> {
        self.entries
            .get(&blob_ref)
            .map(|metadata| resolved_blob_ref_from_metadata(blob_ref, metadata))
    }
}

#[must_use]
pub fn resolved_blob_ref_from_metadata(
    blob_ref: BlobRef,
    metadata: &BlobMetadata,
) -> ResolvedBlobRef {
    ResolvedBlobRef {
        blob_ref,
        content_hash: metadata.content_hash,
        encoded_size_bytes: metadata.encoded_size_bytes,
        decoded_size_bytes: metadata.decoded_size_bytes,
        codec: metadata.codec,
    }
}

#[must_use]
pub fn resolved_blob_index_self_hash(entries: &BTreeMap<BlobRef, BlobMetadata>) -> Hash256 {
    #[derive(Serialize)]
    struct Entry<'a> {
        blob_ref: &'a BlobRef,
        metadata: &'a BlobMetadata,
    }

    let ordered_entries = entries
        .iter()
        .map(|(blob_ref, metadata)| Entry { blob_ref, metadata })
        .collect::<Vec<_>>();
    let value = serde_json::to_value(&ordered_entries).expect("blob index entries serialize");
    let canonical = canonicalize_value(&value).expect("blob index entries canonicalize");
    domain_hash(
        "ResolvedBlobIndex",
        QUANT_GRAPH_SCHEMA_ID,
        QUANT_GRAPH_SCHEMA_VERSION,
        &canonical,
    )
}

pub fn injective_provenance_image<'a>(
    provenance: impl IntoIterator<Item = (&'a TensorId, &'a ExportTensorId)>,
    aux_refs: impl IntoIterator<Item = &'a QuantAuxBlobRef>,
) -> Result<(), ProvenanceImageError> {
    let mut seen = BTreeSet::new();
    for (_tensor_id, export_tensor_id) in provenance {
        if !seen.insert(export_tensor_id.clone()) {
            return Err(ProvenanceImageError::NotInjective {
                field: FieldPath::from("provenance"),
                export_tensor_id: export_tensor_id.clone(),
            });
        }
    }
    for aux_ref in aux_refs {
        if !seen.insert(aux_ref.export_tensor_id.clone()) {
            return Err(ProvenanceImageError::NotInjective {
                field: FieldPath::from("aux_blob_refs.export_tensor_id"),
                export_tensor_id: aux_ref.export_tensor_id.clone(),
            });
        }
    }

    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuantTensorBindingInput {
    pub tensor_id: TensorId,
    pub layout: CanonicalTensorLayout,
    pub quant_format: QuantFormat,
    pub role: QuantTensorRole,
    pub blob_ref: BlobRef,
    pub aux_blob_refs: Vec<QuantAuxBlobBindingInput>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuantAuxBlobBindingInput {
    pub kind: QuantAuxKind,
    pub layout: CanonicalTensorLayout,
    pub format: AuxFormat,
    pub blob_ref: BlobRef,
    pub export_tensor_id: ExportTensorId,
}

pub fn bind_quant_tensors(
    inputs: &[QuantTensorBindingInput],
    blob_index: &ResolvedBlobIndex,
) -> Result<Vec<QuantTensorRef>, Vec<QuantGraphBindingDiagnostic>> {
    let mut diagnostics = Vec::new();
    let mut seen_tensor_ids = BTreeSet::new();
    let mut tensors = Vec::new();

    for input in inputs {
        tracing::debug!(
            event = STAGE1_BINDING_TENSOR_ONE_EVENT,
            tensor_id = input.tensor_id.get() as u64,
            role = ?input.role,
            aux_blob_ref_count = input.aux_blob_refs.len() as u64,
            "stage1.binding.tensor_one"
        );
        if !seen_tensor_ids.insert(input.tensor_id) {
            diagnostics.push(QuantGraphBindingDiagnostic::new(
                QuantGraphBindingDiagnosticCode::QuantGraphTensorIdNotUnique {
                    tensor_id: input.tensor_id,
                },
                "tensors.tensor_id",
            ));
        }
        if !role_format_allowed(&input.role, &input.quant_format) {
            diagnostics.push(QuantGraphBindingDiagnostic::new(
                QuantGraphBindingDiagnosticCode::QuantGraphRoleFormatMismatch,
                "tensors.quant_format",
            ));
        }

        let expected_tensor_size =
            expected_decoded_tensor_payload_size(&input.layout, &input.quant_format, &input.role);
        let Some(blob) = resolve_and_check_blob(
            input.blob_ref,
            expected_tensor_size,
            blob_index,
            false,
            &mut diagnostics,
        ) else {
            continue;
        };

        let mut aux_blob_refs = Vec::new();
        for aux_input in &input.aux_blob_refs {
            let expected_aux_size =
                expected_decoded_aux_payload_size(&aux_input.layout, aux_input.format);
            if let Some(blob) = resolve_and_check_blob(
                aux_input.blob_ref,
                expected_aux_size,
                blob_index,
                true,
                &mut diagnostics,
            ) {
                aux_blob_refs.push(QuantAuxBlobRef {
                    kind: aux_input.kind,
                    layout: aux_input.layout.clone(),
                    format: aux_input.format,
                    blob,
                    export_tensor_id: aux_input.export_tensor_id.clone(),
                });
            }
        }

        diagnostics.extend(validate_aux_blob_refs_for_format(
            &input.quant_format,
            &aux_blob_refs,
        ));

        tensors.push(QuantTensorRef {
            tensor_id: input.tensor_id,
            layout: input.layout.clone(),
            quant_format: input.quant_format.clone(),
            role: input.role.clone(),
            blob,
            aux_blob_refs,
        });
    }

    tracing::info!(
        event = STAGE1_BINDING_TENSOR_EVENT,
        n_inputs = inputs.len() as u64,
        n_tensors = tensors.len() as u64,
        n_diagnostics = diagnostics.len() as u64,
        blob_index_hash = %blob_index.self_hash,
        "stage1.binding.tensor"
    );
    if diagnostics.is_empty() {
        Ok(tensors)
    } else {
        emit_first_hard_binding_diagnostic(QuantGraphBindingClass::TensorBinding, &diagnostics);
        Err(diagnostics)
    }
}

fn resolve_and_check_blob(
    blob_ref: BlobRef,
    expected_decoded_size_bytes: u64,
    blob_index: &ResolvedBlobIndex,
    is_aux: bool,
    diagnostics: &mut Vec<QuantGraphBindingDiagnostic>,
) -> Option<ResolvedBlobRef> {
    let Some(blob) = blob_index.resolve(blob_ref) else {
        diagnostics.push(QuantGraphBindingDiagnostic::new(
            QuantGraphBindingDiagnosticCode::QuantGraphBlobRefUnresolvable { blob_ref },
            "blob_ref",
        ));
        return None;
    };

    if blob.decoded_size_bytes != expected_decoded_size_bytes {
        let code = if is_aux {
            QuantGraphBindingDiagnosticCode::QuantGraphAuxBlobRefSizeMismatch {
                blob_ref,
                expected_decoded_size_bytes,
                observed_decoded_size_bytes: blob.decoded_size_bytes,
            }
        } else {
            QuantGraphBindingDiagnosticCode::QuantGraphBlobRefSizeMismatch {
                blob_ref,
                expected_decoded_size_bytes,
                observed_decoded_size_bytes: blob.decoded_size_bytes,
            }
        };
        diagnostics.push(QuantGraphBindingDiagnostic::new(
            code,
            "blob.decoded_size_bytes",
        ));
    }

    Some(blob)
}

#[must_use]
pub fn validate_aux_blob_refs_for_format(
    format: &QuantFormat,
    aux_blob_refs: &[QuantAuxBlobRef],
) -> Vec<QuantGraphBindingDiagnostic> {
    let mut diagnostics = Vec::new();
    let mut counts = BTreeMap::<QuantAuxKind, usize>::new();
    for aux_ref in aux_blob_refs {
        *counts.entry(aux_ref.kind).or_default() += 1;
    }

    let expected: &[QuantAuxKind] = match format {
        QuantFormat::Ternary2 { .. } => &[QuantAuxKind::Scale, QuantAuxKind::Threshold],
        QuantFormat::Binary1 { .. } => &[QuantAuxKind::Scale],
        QuantFormat::SparseTernaryBitplanes { .. } => {
            &[QuantAuxKind::Scale, QuantAuxKind::SparseMeta]
        }
        QuantFormat::Q8_8 | QuantFormat::Q4_4 | QuantFormat::I8 | QuantFormat::I16 => &[],
    };

    for kind in expected {
        if counts.get(kind).copied().unwrap_or_default() != 1 {
            diagnostics.push(QuantGraphBindingDiagnostic::new(
                QuantGraphBindingDiagnosticCode::QuantGraphAuxBlobKindMismatch,
                "aux_blob_refs.kind",
            ));
        }
    }
    for (kind, count) in counts {
        if count > 1 || !expected.contains(&kind) {
            diagnostics.push(QuantGraphBindingDiagnostic::new(
                QuantGraphBindingDiagnosticCode::QuantGraphAuxBlobKindMismatch,
                "aux_blob_refs.kind",
            ));
        }
    }

    diagnostics
}

#[must_use]
pub fn expected_decoded_tensor_payload_size(
    layout: &CanonicalTensorLayout,
    format: &QuantFormat,
    _role: &QuantTensorRole,
) -> u64 {
    let elements = layout.shape.element_count() as u64;
    match format {
        QuantFormat::Ternary2 { .. } => ceil_div(elements * 2, 8),
        QuantFormat::Binary1 { .. } => ceil_div(elements, 8),
        QuantFormat::SparseTernaryBitplanes { .. } => ceil_div(elements * 2, 8),
        QuantFormat::Q8_8 | QuantFormat::I16 => elements * 2,
        QuantFormat::Q4_4 | QuantFormat::I8 => elements,
    }
}

#[must_use]
pub fn expected_decoded_aux_payload_size(layout: &CanonicalTensorLayout, format: AuxFormat) -> u64 {
    let elements = layout.shape.element_count() as u64;
    match format {
        AuxFormat::Q8_8 | AuxFormat::I16 => elements * 2,
        AuxFormat::Q4_4 | AuxFormat::Pow2 | AuxFormat::I8 => elements,
    }
}

fn ceil_div(numerator: u64, denominator: u64) -> u64 {
    numerator.div_ceil(denominator)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProvenanceImageError {
    NotInjective {
        field: FieldPath,
        export_tensor_id: ExportTensorId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuantGraphBindingDiagnostic {
    pub code: QuantGraphBindingDiagnosticCode,
    pub field: FieldPath,
}

impl QuantGraphBindingDiagnostic {
    #[must_use]
    pub fn new(code: QuantGraphBindingDiagnosticCode, field: impl Into<String>) -> Self {
        Self {
            code,
            field: FieldPath::from(field.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum QuantGraphBindingDiagnosticCode {
    QuantGraphIdentityHashMismatch {
        expected: Hash256,
        observed: Hash256,
    },
    QuantGraphSequenceSemanticsUnsupportedV1,
    QuantGraphSequenceSemanticsSidecarRebind,
    QuantGraphTensorIdNotUnique {
        tensor_id: TensorId,
    },
    QuantGraphRoleFormatMismatch,
    QuantGraphBlobRefUnresolvable {
        blob_ref: BlobRef,
    },
    QuantGraphBlobRefSizeMismatch {
        blob_ref: BlobRef,
        expected_decoded_size_bytes: u64,
        observed_decoded_size_bytes: u64,
    },
    QuantGraphAuxBlobKindMismatch,
    QuantGraphAuxBlobRefSizeMismatch {
        blob_ref: BlobRef,
        expected_decoded_size_bytes: u64,
        observed_decoded_size_bytes: u64,
    },
    QuantGraphTrainingResidue,
    QuantGraphNormPlanReferenceUnresolved,
    QuantGraphNormSiteDuplicate,
    QuantGraphFinalNormMissing,
    QuantGraphMissingLayerNorms {
        layer: LayerId,
    },
    QuantGraphLayerNormsIncomplete {
        layer: LayerId,
    },
    QuantGraphRoutingMissingForRoutedLayer {
        layer: LayerId,
    },
    QuantGraphRoutingPresentForDenseLayer {
        layer: LayerId,
    },
    QuantGraphRoutingExpertCoverageMismatch {
        layer: LayerId,
    },
    QuantGraphRoutingExpertCoverageGap {
        layer: LayerId,
        expert: ExpertId,
    },
    QuantGraphRoutingExpertCoverageExtra {
        layer: LayerId,
        expert: ExpertId,
    },
    QuantGraphExpertSectionWeightMissing {
        layer: LayerId,
        expert: ExpertId,
    },
    QuantGraphFfnGatePresenceMismatch {
        layer: LayerId,
        expert: ExpertId,
    },
    QuantGraphExportProvenanceMissing {
        tensor_id: TensorId,
    },
    QuantGraphProvenanceImageNotInjective {
        export_tensor_id: ExportTensorId,
    },
    QuantGraphLayoutInconsistentWithModelSpec,
    QuantGraphSequenceSemanticsTensorMismatch,
    QuantGraphRequiredFeatureUnsupported,
    QuantGraphEmbeddingMissing,
    QuantGraphEmbeddingNotUnique,
    QuantGraphDecodeRequiresRngMismatch,
    QuantGraphDeterminismRequiresEnforcedReductionOrder,
    QuantGraphForbiddenStorageMetadata,
    QuantGraphRouterGateWeightSemanticsUnsupported,
    QuantGraphRouterTieBreakUnsupported,
    QuantGraphBitExactMidReductionSaturationForbidden,
    QuantGraphResidualPlanInvalid,
    QuantGraphDecodeSpecNotInCapabilitySet,
    QuantGraphDecodeSpecUnboundDefault,
    QuantGraphClassifyHeadTiedMismatch,
    QuantGraphClassifyHeadFormatMismatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuantGraphDiagnosticSeverity {
    Hard,
}

impl QuantGraphBindingDiagnosticCode {
    #[must_use]
    pub const fn severity(&self) -> QuantGraphDiagnosticSeverity {
        QuantGraphDiagnosticSeverity::Hard
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct NormPlanId(u32);

impl NormPlanId {
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl From<u32> for NormPlanId {
    fn from(value: u32) -> Self {
        Self::new(value)
    }
}

impl From<NormPlanId> for u32 {
    fn from(value: NormPlanId) -> Self {
        value.get()
    }
}

impl fmt::Display for NormPlanId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NormPlanRecord {
    pub norm_plan_id: NormPlanId,
    pub site: NormSite,
    pub plan: NormPlan,
    pub input_format: QuantFormat,
    pub output_format: QuantFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum NormSite {
    LayerSequence { layer: LayerId },
    LayerFfn { layer: LayerId },
    Final,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NormPlanIdPreBinding {
    pub by_site: BTreeMap<NormSite, NormPlanId>,
}

impl NormPlanIdPreBinding {
    #[must_use]
    pub fn norm_plan_id(&self, site: NormSite) -> Option<NormPlanId> {
        self.by_site.get(&site).copied()
    }
}

#[must_use]
pub fn bind_norm_plan_ids(n_layers: u16) -> NormPlanIdPreBinding {
    let mut by_site = BTreeMap::new();
    let mut next_id = 0_u32;
    for layer in 0..n_layers {
        by_site.insert(
            NormSite::LayerSequence {
                layer: LayerId::new(layer),
            },
            NormPlanId::new(next_id),
        );
        next_id += 1;
    }
    for layer in 0..n_layers {
        by_site.insert(
            NormSite::LayerFfn {
                layer: LayerId::new(layer),
            },
            NormPlanId::new(next_id),
        );
        next_id += 1;
    }
    by_site.insert(NormSite::Final, NormPlanId::new(next_id));

    tracing::info!(
        event = STAGE1_BINDING_NORM_PLAN_ID_PRE_EVENT,
        n_layers = n_layers as u64,
        n_norm_plan_ids = by_site.len() as u64,
        "stage1.binding.norm_plan_id_pre"
    );
    NormPlanIdPreBinding { by_site }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NormPlanBindingInput {
    pub site: NormSite,
    pub plan: NormPlan,
    pub input_format: QuantFormat,
    pub output_format: QuantFormat,
}

pub fn bind_norm_plans(
    inputs: &[NormPlanBindingInput],
    norm_plan_ids: &NormPlanIdPreBinding,
) -> Result<Vec<NormPlanRecord>, Vec<QuantGraphBindingDiagnostic>> {
    let mut diagnostics = Vec::new();
    let mut seen_sites = BTreeSet::new();
    let mut records = Vec::new();

    for input in inputs {
        if !seen_sites.insert(input.site) {
            diagnostics.push(QuantGraphBindingDiagnostic::new(
                QuantGraphBindingDiagnosticCode::QuantGraphNormSiteDuplicate,
                "norm_plans.site",
            ));
            continue;
        }
        let Some(norm_plan_id) = norm_plan_ids.norm_plan_id(input.site) else {
            diagnostics.push(QuantGraphBindingDiagnostic::new(
                QuantGraphBindingDiagnosticCode::QuantGraphNormPlanReferenceUnresolved,
                "norm_plans.norm_plan_id",
            ));
            continue;
        };
        records.push(NormPlanRecord {
            norm_plan_id,
            site: input.site,
            plan: input.plan.clone(),
            input_format: input.input_format.clone(),
            output_format: input.output_format.clone(),
        });
    }

    if !seen_sites.contains(&NormSite::Final) {
        diagnostics.push(QuantGraphBindingDiagnostic::new(
            QuantGraphBindingDiagnosticCode::QuantGraphFinalNormMissing,
            "norm_plans.site",
        ));
    }

    tracing::info!(
        n_norm_plans = records.len() as u64,
        has_final_norm = seen_sites.contains(&NormSite::Final),
        "stage1.types.bind_norm_plans"
    );
    for record in &records {
        tracing::debug!(
            norm_plan_id = record.norm_plan_id.get() as u64,
            site = ?record.site,
            "stage1.types.norm_site"
        );
    }

    if diagnostics.is_empty() {
        Ok(records)
    } else {
        Err(diagnostics)
    }
}

pub fn bind_layer_norms(
    n_layers: u16,
    norm_plans: &[NormPlanRecord],
) -> Result<BTreeMap<LayerId, LayerNorms>, Vec<QuantGraphBindingDiagnostic>> {
    let mut diagnostics = Vec::new();
    let mut by_layer = BTreeMap::<LayerId, (Option<NormPlanId>, Option<NormPlanId>)>::new();

    for record in norm_plans {
        match record.site {
            NormSite::LayerSequence { layer } => {
                by_layer.entry(layer).or_default().0 = Some(record.norm_plan_id);
            }
            NormSite::LayerFfn { layer } => {
                by_layer.entry(layer).or_default().1 = Some(record.norm_plan_id);
            }
            NormSite::Final => {}
        }
    }

    let mut layer_norms = BTreeMap::new();
    for layer_index in 0..n_layers {
        let layer = LayerId::new(layer_index);
        match by_layer.get(&layer).copied().unwrap_or_default() {
            (Some(pre_sequence), Some(pre_ffn)) => {
                layer_norms.insert(
                    layer,
                    LayerNorms {
                        pre_sequence,
                        pre_ffn,
                    },
                );
            }
            (None, None) => diagnostics.push(QuantGraphBindingDiagnostic::new(
                QuantGraphBindingDiagnosticCode::QuantGraphMissingLayerNorms { layer },
                "layer_norms",
            )),
            _ => diagnostics.push(QuantGraphBindingDiagnostic::new(
                QuantGraphBindingDiagnosticCode::QuantGraphLayerNormsIncomplete { layer },
                "layer_norms",
            )),
        }
    }

    if diagnostics.is_empty() {
        Ok(layer_norms)
    } else {
        Err(diagnostics)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayerNorms {
    pub pre_sequence: NormPlanId,
    pub pre_ffn: NormPlanId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutingTable {
    pub layers: Vec<RouterLayer>,
}

pub fn bind_routing_table(
    model_spec: &ModelSpecSummary,
    router_layers: Vec<RouterLayer>,
) -> Result<Option<RoutingTable>, Vec<QuantGraphBindingDiagnostic>> {
    let mut diagnostics = Vec::new();
    let routed_layers = model_spec
        .ffn_kind
        .iter()
        .filter_map(|(layer, kind)| matches!(kind, FfnKindTag::Routed).then_some(*layer))
        .collect::<BTreeSet<_>>();
    let mut by_layer = BTreeMap::new();

    for router_layer in router_layers {
        if !routed_layers.contains(&router_layer.layer) {
            diagnostics.push(QuantGraphBindingDiagnostic::new(
                QuantGraphBindingDiagnosticCode::QuantGraphRoutingPresentForDenseLayer {
                    layer: router_layer.layer,
                },
                "routing_table.layers",
            ));
        }
        if model_spec
            .n_experts
            .get(&router_layer.layer)
            .is_some_and(|expected| *expected != router_layer.n_experts)
        {
            diagnostics.push(QuantGraphBindingDiagnostic::new(
                QuantGraphBindingDiagnosticCode::QuantGraphRoutingExpertCoverageMismatch {
                    layer: router_layer.layer,
                },
                "routing_table.layers.n_experts",
            ));
        }
        by_layer.insert(router_layer.layer, router_layer);
    }

    for layer in routed_layers {
        if !by_layer.contains_key(&layer) {
            diagnostics.push(QuantGraphBindingDiagnostic::new(
                QuantGraphBindingDiagnosticCode::QuantGraphRoutingMissingForRoutedLayer { layer },
                "routing_table.layers",
            ));
        }
    }

    if diagnostics.is_empty() {
        let layers = by_layer.into_values().collect::<Vec<_>>();
        Ok((!layers.is_empty()).then_some(RoutingTable { layers }))
    } else {
        Err(diagnostics)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum RouterSemanticsBindingInput {
    Top1Hard {
        gate_weight: RouterGateWeightSemanticsBindingInput,
        tie_break: RouterTieBreakBindingInput,
    },
    UnsupportedV1 {
        tag: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum RouterGateWeightSemanticsBindingInput {
    One,
    SelectedScore,
    UnsupportedV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum RouterTieBreakBindingInput {
    LowestExpertId,
    UnsupportedV1,
}

pub fn bind_router_semantics_v1(
    input: RouterSemanticsBindingInput,
) -> Result<RouterSemantics, Vec<QuantGraphBindingDiagnostic>> {
    let mut diagnostics = Vec::new();
    let RouterSemanticsBindingInput::Top1Hard {
        gate_weight,
        tie_break,
    } = input
    else {
        diagnostics.push(QuantGraphBindingDiagnostic::new(
            QuantGraphBindingDiagnosticCode::QuantGraphRouterGateWeightSemanticsUnsupported,
            "routing_table.layers.semantics",
        ));
        return Err(diagnostics);
    };

    let gate_weight = match gate_weight {
        RouterGateWeightSemanticsBindingInput::One => Some(RouterGateWeightSemantics::One),
        RouterGateWeightSemanticsBindingInput::SelectedScore => {
            Some(RouterGateWeightSemantics::SelectedScore)
        }
        RouterGateWeightSemanticsBindingInput::UnsupportedV1 => {
            diagnostics.push(QuantGraphBindingDiagnostic::new(
                QuantGraphBindingDiagnosticCode::QuantGraphRouterGateWeightSemanticsUnsupported,
                "routing_table.layers.semantics.gate_weight",
            ));
            None
        }
    };
    let tie_break = match tie_break {
        RouterTieBreakBindingInput::LowestExpertId => Some(RouterTieBreak::LowestExpertId),
        RouterTieBreakBindingInput::UnsupportedV1 => {
            diagnostics.push(QuantGraphBindingDiagnostic::new(
                QuantGraphBindingDiagnosticCode::QuantGraphRouterTieBreakUnsupported,
                "routing_table.layers.semantics.tie_break",
            ));
            None
        }
    };

    match (gate_weight, tie_break, diagnostics.is_empty()) {
        (Some(gate_weight), Some(tie_break), true) => Ok(RouterSemantics::Top1Hard {
            gate_weight,
            tie_break,
        }),
        _ => Err(diagnostics),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouterLayer {
    pub layer: LayerId,
    pub n_experts: u16,
    pub router_weight: TensorId,
    pub router_bias: Option<TensorId>,
    pub semantics: RouterSemantics,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum RouterSemantics {
    Top1Hard {
        gate_weight: RouterGateWeightSemantics,
        tie_break: RouterTieBreak,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum RouterGateWeightSemantics {
    One,
    SelectedScore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum RouterTieBreak {
    LowestExpertId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpertSection {
    pub layer: LayerId,
    pub expert: ExpertId,
    pub tensor_refs: Vec<TensorId>,
}

pub fn bind_expert_sections(
    model_spec: &ModelSpecSummary,
    ffn_plans: &BTreeMap<LayerId, FfnPlan>,
    tensors: &[QuantTensorRef],
) -> Result<Vec<ExpertSection>, Vec<QuantGraphBindingDiagnostic>> {
    let mut diagnostics = Vec::new();
    let mut grouped =
        BTreeMap::<(LayerId, ExpertId), Vec<(&ExpertWeightSlot, bool, TensorId)>>::new();

    for tensor in tensors {
        match &tensor.role {
            QuantTensorRole::ExpertWeight {
                layer,
                expert,
                slot,
            } => {
                grouped
                    .entry((*layer, *expert))
                    .or_default()
                    .push((slot, false, tensor.tensor_id))
            }
            QuantTensorRole::ExpertBias {
                layer,
                expert,
                slot,
            } => grouped
                .entry((*layer, *expert))
                .or_default()
                .push((slot, true, tensor.tensor_id)),
            _ => {}
        }
    }

    for (layer, kind) in &model_spec.ffn_kind {
        let expected_experts = if matches!(kind, FfnKindTag::Dense) {
            1
        } else {
            *model_spec.n_experts.get(layer).unwrap_or(&0)
        };
        for expert_index in 0..expected_experts {
            let expert = ExpertId::new(expert_index);
            if !grouped.contains_key(&(*layer, expert)) {
                diagnostics.push(QuantGraphBindingDiagnostic::new(
                    QuantGraphBindingDiagnosticCode::QuantGraphRoutingExpertCoverageGap {
                        layer: *layer,
                        expert,
                    },
                    "expert_sections",
                ));
            }
        }
    }
    for (layer, expert) in grouped.keys().copied() {
        let expected_experts = match model_spec.ffn_kind.get(&layer) {
            Some(FfnKindTag::Dense) => 1,
            Some(FfnKindTag::Routed) => *model_spec.n_experts.get(&layer).unwrap_or(&0),
            None => 0,
        };
        if expert.get() >= expected_experts {
            diagnostics.push(QuantGraphBindingDiagnostic::new(
                QuantGraphBindingDiagnosticCode::QuantGraphRoutingExpertCoverageExtra {
                    layer,
                    expert,
                },
                "expert_sections",
            ));
        }
    }

    let mut sections = Vec::new();
    for ((layer, expert), refs) in grouped {
        let tensor_refs = canonical_expert_tensor_refs(&refs);
        let has_up = refs
            .iter()
            .any(|(slot, is_bias, _)| **slot == ExpertWeightSlot::FfnUp && !*is_bias);
        let has_down = refs
            .iter()
            .any(|(slot, is_bias, _)| **slot == ExpertWeightSlot::FfnDown && !*is_bias);
        if !has_up || !has_down {
            diagnostics.push(QuantGraphBindingDiagnostic::new(
                QuantGraphBindingDiagnosticCode::QuantGraphExpertSectionWeightMissing {
                    layer,
                    expert,
                },
                "expert_sections.tensor_refs",
            ));
        }

        let has_gate = refs
            .iter()
            .any(|(slot, is_bias, _)| **slot == ExpertWeightSlot::FfnGate && !*is_bias);
        let wants_gate = ffn_plans
            .get(&layer)
            .is_some_and(|plan| plan.activation_kind == FfnActivationKind::SwiGLU);
        if has_gate != wants_gate {
            diagnostics.push(QuantGraphBindingDiagnostic::new(
                QuantGraphBindingDiagnosticCode::QuantGraphFfnGatePresenceMismatch {
                    layer,
                    expert,
                },
                "expert_sections.tensor_refs",
            ));
        }

        sections.push(ExpertSection {
            layer,
            expert,
            tensor_refs,
        });
    }

    if diagnostics.is_empty() {
        Ok(sections)
    } else {
        Err(diagnostics)
    }
}

fn canonical_expert_tensor_refs(refs: &[(&ExpertWeightSlot, bool, TensorId)]) -> Vec<TensorId> {
    let mut ordered = Vec::new();
    for slot in EXPERT_WEIGHT_SLOT_CANONICAL_ORDER {
        if let Some((_, _, tensor_id)) = refs
            .iter()
            .find(|(candidate_slot, is_bias, _)| **candidate_slot == slot && !*is_bias)
        {
            ordered.push(*tensor_id);
        }
        if let Some((_, _, tensor_id)) = refs
            .iter()
            .find(|(candidate_slot, is_bias, _)| **candidate_slot == slot && *is_bias)
        {
            ordered.push(*tensor_id);
        }
    }
    ordered
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FfnPlan {
    pub layer: LayerId,
    pub activation_kind: FfnActivationKind,
    pub intermediate_format: QuantFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum FfnActivationKind {
    Relu,
    Gelu,
    SiLU,
    SwiGLU,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecodeSpecRecord {
    pub decode_plan_id: DecodePlanId,
    pub spec: DecodeSpec,
    pub requires_rng: bool,
}

impl DecodeSpecRecord {
    #[must_use]
    pub fn requires_rng_matches_spec(&self) -> bool {
        self.requires_rng == self.spec.requires_rng()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum DecodeBindingSource {
    Explicit { spec: DecodeSpec },
    ArtifactDefault { spec: DecodeSpec, hash_bound: bool },
    UnboundDefault,
}

pub fn bind_decode_spec(
    decode_plan_id: DecodePlanId,
    source: DecodeBindingSource,
    decode_caps: &DecodeCapabilitySet,
) -> Result<DecodeSpecRecord, Vec<QuantGraphBindingDiagnostic>> {
    let mut diagnostics = Vec::new();
    let spec = match source {
        DecodeBindingSource::Explicit { spec } => Some(spec),
        DecodeBindingSource::ArtifactDefault { spec, hash_bound } => {
            if !hash_bound {
                diagnostics.push(QuantGraphBindingDiagnostic::new(
                    QuantGraphBindingDiagnosticCode::QuantGraphDecodeSpecUnboundDefault,
                    "decode_spec",
                ));
            }
            Some(spec)
        }
        DecodeBindingSource::UnboundDefault => {
            diagnostics.push(QuantGraphBindingDiagnostic::new(
                QuantGraphBindingDiagnosticCode::QuantGraphDecodeSpecUnboundDefault,
                "decode_spec",
            ));
            None
        }
    };

    if let Some(spec) = spec {
        if !check_decode_spec_in_capabilities(&spec, decode_caps) {
            diagnostics.push(QuantGraphBindingDiagnostic::new(
                QuantGraphBindingDiagnosticCode::QuantGraphDecodeSpecNotInCapabilitySet,
                "decode_spec.spec",
            ));
        }

        if diagnostics.is_empty() {
            Ok(DecodeSpecRecord {
                requires_rng: spec.requires_rng(),
                decode_plan_id,
                spec,
            })
        } else {
            Err(diagnostics)
        }
    } else {
        Err(diagnostics)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct DecodePlanId(u32);

impl DecodePlanId {
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl From<u32> for DecodePlanId {
    fn from(value: u32) -> Self {
        Self::new(value)
    }
}

impl From<DecodePlanId> for u32 {
    fn from(value: DecodePlanId) -> Self {
        value.get()
    }
}

impl fmt::Display for DecodePlanId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum DecodeSpec {
    Argmax,
    TopKTemperature { k: u16, temperature_q8_8: u16 },
}

impl DecodeSpec {
    #[must_use]
    pub const fn mode(&self) -> DecodeMode {
        match self {
            Self::Argmax => DecodeMode::Argmax,
            Self::TopKTemperature { .. } => DecodeMode::TopKTemperature,
        }
    }

    #[must_use]
    pub const fn requires_rng(&self) -> bool {
        matches!(self, Self::TopKTemperature { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum DecodeMode {
    Argmax,
    TopKTemperature,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecodeCapabilitySet {
    pub supported: BTreeSet<DecodeMode>,
}

#[must_use]
pub fn check_decode_spec_in_capabilities(
    spec: &DecodeSpec,
    decode_caps: &DecodeCapabilitySet,
) -> bool {
    decode_caps.supported.contains(&spec.mode())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SequenceSemanticsSpec {
    pub kind: SequenceSemanticsKind,
    #[serde(with = "layer_map_entries")]
    pub state_slots: BTreeMap<LayerId, Vec<SequenceStateSlot>>,
}

impl SequenceSemanticsSpec {
    #[must_use]
    pub fn identity() -> Self {
        Self {
            kind: SequenceSemanticsKind::Identity,
            state_slots: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn has_state_slots(&self) -> bool {
        self.state_slots.values().any(|slots| !slots.is_empty())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum SequenceSemanticsKind {
    Identity,
    LinearState,
    BoundedKv,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SequenceStateSlot {
    pub slot_id: u16,
    pub width_bytes: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SequenceSemanticsBindingInputs {
    pub artifact_sequence: SequenceSemanticsSpec,
    pub requested_sequence: SequenceSemanticsSpec,
}

pub fn bind_sequence_semantics(
    inputs: SequenceSemanticsBindingInputs,
) -> Result<SequenceSemanticsSpec, Vec<QuantGraphBindingDiagnostic>> {
    let mut diagnostics = Vec::new();
    tracing::info!(
        event = STAGE1_BINDING_SEQUENCE_SEMANTICS_EVENT,
        artifact_kind = ?inputs.artifact_sequence.kind,
        requested_kind = ?inputs.requested_sequence.kind,
        artifact_state_slot_layers = inputs.artifact_sequence.state_slots.len() as u64,
        requested_state_slot_layers = inputs.requested_sequence.state_slots.len() as u64,
        "stage1.binding.sequence_semantics"
    );
    if inputs.artifact_sequence != inputs.requested_sequence {
        diagnostics.push(QuantGraphBindingDiagnostic::new(
            QuantGraphBindingDiagnosticCode::QuantGraphSequenceSemanticsSidecarRebind,
            "sequence_semantics",
        ));
    }
    if inputs.artifact_sequence.kind != SequenceSemanticsKind::Identity
        || inputs.artifact_sequence.has_state_slots()
    {
        diagnostics.push(QuantGraphBindingDiagnostic::new(
            QuantGraphBindingDiagnosticCode::QuantGraphSequenceSemanticsUnsupportedV1,
            "sequence_semantics.state_slots",
        ));
    }

    if diagnostics.is_empty() {
        Ok(inputs.artifact_sequence)
    } else {
        emit_first_hard_binding_diagnostic(
            QuantGraphBindingClass::SequenceSemanticsBinding,
            &diagnostics,
        );
        Err(diagnostics)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClassifyHead {
    pub kind: ClassifyHeadKind,
    pub weight: TensorId,
    pub bias: Option<TensorId>,
    pub logit_format: QuantFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ClassifyHeadKind {
    Tied,
    Untied,
}

#[must_use]
pub const fn classify_logit_format_is_activation_set(format: &QuantFormat) -> bool {
    matches!(
        format,
        QuantFormat::I8 | QuantFormat::I16 | QuantFormat::Q8_8 | QuantFormat::Q4_4
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResidualPlan {
    pub activation_format: QuantFormat,
    pub combine_policy: ResidualCombinePolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ResidualCombinePolicy {
    AddThenClampNamedBoundary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResidualPlanInput {
    pub activation_format: QuantFormat,
    pub combine_policy: ResidualCombinePolicy,
}

pub fn bind_residual_plan(
    input: ResidualPlanInput,
) -> Result<ResidualPlan, Vec<QuantGraphBindingDiagnostic>> {
    if classify_logit_format_is_activation_set(&input.activation_format)
        && input.combine_policy == ResidualCombinePolicy::AddThenClampNamedBoundary
    {
        Ok(ResidualPlan {
            activation_format: input.activation_format,
            combine_policy: input.combine_policy,
        })
    } else {
        Err(vec![QuantGraphBindingDiagnostic::new(
            QuantGraphBindingDiagnosticCode::QuantGraphResidualPlanInvalid,
            "residual_plan",
        )])
    }
}

pub fn bind_classify_head(
    head: ClassifyHead,
    embedding_tensor_id: TensorId,
) -> Result<ClassifyHead, Vec<QuantGraphBindingDiagnostic>> {
    let mut diagnostics = Vec::new();
    if head.kind == ClassifyHeadKind::Tied && head.weight != embedding_tensor_id {
        diagnostics.push(QuantGraphBindingDiagnostic::new(
            QuantGraphBindingDiagnosticCode::QuantGraphClassifyHeadTiedMismatch,
            "classify_head.weight",
        ));
    }
    if !classify_logit_format_is_activation_set(&head.logit_format) {
        diagnostics.push(QuantGraphBindingDiagnostic::new(
            QuantGraphBindingDiagnosticCode::QuantGraphClassifyHeadFormatMismatch,
            "classify_head.logit_format",
        ));
    }

    if diagnostics.is_empty() {
        Ok(head)
    } else {
        Err(diagnostics)
    }
}

pub fn quant_graph_self_hash(quant_graph: &QuantGraph) -> Result<Hash256, QuantGraphProductError> {
    let value = serde_json::to_value(quant_graph)
        .map_err(|err| QuantGraphProductError::CanonicalJson(err.to_string()))?;
    let canonical = canonicalize_value(&value)
        .map_err(|err| QuantGraphProductError::CanonicalJson(err.to_string()))?;
    Ok(domain_hash(
        "QuantGraph",
        QUANT_GRAPH_SCHEMA_ID,
        QUANT_GRAPH_SCHEMA_VERSION,
        &canonical,
    ))
}

pub fn quant_graph_canonical_bytes_hash(
    quant_graph: &QuantGraph,
) -> Result<Hash256, QuantGraphProductError> {
    let value = serde_json::to_value(quant_graph)
        .map_err(|err| QuantGraphProductError::CanonicalJson(err.to_string()))?;
    let canonical = canonicalize_value(&value)
        .map_err(|err| QuantGraphProductError::CanonicalJson(err.to_string()))?;
    Ok(Hash256::from_bytes(Sha256::digest(&canonical).into()))
}

pub fn quant_graph_report_result(
    quant_graph: QuantGraph,
    quant_graph_self_hash: Hash256,
    quant_graph_canonical_bytes_hash: Hash256,
) -> Result<report_schema::QuantGraphResult<QuantGraph>, QuantGraphProductError> {
    Ok(report_schema::QuantGraphResult {
        tensor_count: usize_to_u32(quant_graph.tensors.len(), "result.tensor_count")?,
        norm_plan_count: usize_to_u16(quant_graph.norm_plans.len(), "result.norm_plan_count")?,
        layer_norm_count: usize_to_u16(quant_graph.layer_norms.len(), "result.layer_norm_count")?,
        routing_layers_count: usize_to_u16(
            quant_graph
                .routing_table
                .as_ref()
                .map(|routing| routing.layers.len())
                .unwrap_or_default(),
            "result.routing_layers_count",
        )?,
        expert_section_count: usize_to_u32(
            quant_graph.expert_sections.len(),
            "result.expert_section_count",
        )?,
        classify_head_kind: match quant_graph.classify_head.kind {
            ClassifyHeadKind::Tied => report_schema::ClassifyHeadKind::TiedEmbedding,
            ClassifyHeadKind::Untied => report_schema::ClassifyHeadKind::Untied,
        },
        tensor_summary: tensor_summary(&quant_graph),
        provenance_summary: provenance_summary(&quant_graph),
        decode_spec_summary: report_schema::DecodeSpecSummary {},
        sequence_semantics_summary: report_schema::SequenceSemanticsSummary {},
        classify_head_summary: report_schema::ClassifyHeadSummary {},
        product: quant_graph,
        quant_graph_self_hash,
        quant_graph_canonical_bytes_hash,
    })
}

fn tensor_summary(quant_graph: &QuantGraph) -> Vec<report_schema::TensorSummaryEntry> {
    let mut entries = quant_graph
        .tensors
        .iter()
        .map(|tensor| report_schema::TensorSummaryEntry {
            tensor_id: tensor.tensor_id.get(),
            role: tensor_role_tag(&tensor.role).to_owned(),
        })
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.tensor_id);
    entries
}

fn provenance_summary(quant_graph: &QuantGraph) -> Vec<report_schema::ProvenanceSummaryEntry> {
    quant_graph
        .provenance
        .iter()
        .map(
            |(tensor_id, export_tensor_id)| report_schema::ProvenanceSummaryEntry {
                tensor_id: tensor_id.get(),
                export_tensor_id: export_tensor_id.to_string(),
            },
        )
        .collect()
}

fn tensor_role_tag(role: &QuantTensorRole) -> &'static str {
    match role {
        QuantTensorRole::EmbeddingTable => "EmbeddingTable",
        QuantTensorRole::NormScale { .. } => "NormScale",
        QuantTensorRole::NormBias { .. } => "NormBias",
        QuantTensorRole::RouterWeight { .. } => "RouterWeight",
        QuantTensorRole::RouterBias { .. } => "RouterBias",
        QuantTensorRole::ExpertWeight { .. } => "ExpertWeight",
        QuantTensorRole::ExpertBias { .. } => "ExpertBias",
        QuantTensorRole::ClassifyWeight => "ClassifyWeight",
        QuantTensorRole::ClassifyBias => "ClassifyBias",
    }
}

fn usize_to_u32(value: usize, field: &'static str) -> Result<u32, QuantGraphProductError> {
    u32::try_from(value).map_err(|_| QuantGraphProductError::CountOverflow(field))
}

fn usize_to_u16(value: usize, field: &'static str) -> Result<u16, QuantGraphProductError> {
    u16::try_from(value).map_err(|_| QuantGraphProductError::CountOverflow(field))
}

fn quant_graph_budget_view(
    product: &QuantGraphProduct,
) -> Result<QuantGraphBudgetView, QuantGraphBudgetViewError> {
    let quant_graph = &product.quant_graph;
    let model = &quant_graph.identity.model_spec_summary;
    let mut experts = Vec::new();
    for section in &quant_graph.expert_sections {
        let Some((rows, cols, metadata_bytes, plan)) =
            expert_projection_from_section(section, &quant_graph.tensors)
        else {
            return Err(malformed_budget_view("budget_view.experts"));
        };
        experts.push(ExpertProjection {
            layer: section.layer,
            expert: section.expert,
            rows,
            cols,
            metadata_bytes,
            plan,
        });
    }
    experts.sort_by_key(|expert| (expert.layer, expert.expert));

    let mut reduction_sites = reduction_sites_for_quant_graph(quant_graph);
    reduction_sites.sort_by_key(|site| site.site.clone());

    let view = QuantGraphBudgetView {
        semantic_core_hash: quant_graph.identity.semantic_core_hash,
        quant_graph_hash: product.quant_graph_self_hash,
        layers: (0..model.n_layers).map(LayerId::new).collect(),
        experts,
        shared_kernels: Vec::new(),
        shared_luts: Vec::new(),
        shared_dense_ffn: None,
        reduction_sites,
        sequence_state: SequenceStateProjection::default(),
        routing: RoutingProjection::default(),
    };
    view.validate_semantics()?;
    Ok(view)
}

fn expert_projection_from_section(
    section: &ExpertSection,
    tensors: &[QuantTensorRef],
) -> Option<(u32, u32, u32, TernaryWeightPlan)> {
    let mut rows = None;
    let mut cols = None;
    let mut metadata_bytes = 0_u32;
    let mut plan = None;
    for tensor_id in &section.tensor_refs {
        let tensor = tensors
            .iter()
            .find(|tensor| tensor.tensor_id == *tensor_id)?;
        match tensor.role {
            QuantTensorRole::ExpertWeight { .. } => {
                let dims = tensor.layout.shape.dims();
                if dims.len() == 2 {
                    rows.get_or_insert(dims[0]);
                    cols.get_or_insert(dims[1]);
                }
                plan.get_or_insert_with(|| ternary_plan_from_quant_format(&tensor.quant_format));
                metadata_bytes = metadata_bytes.checked_add(aux_metadata_bytes(tensor)?)?;
            }
            QuantTensorRole::ExpertBias { .. } => {
                metadata_bytes =
                    metadata_bytes.checked_add(tensor.blob.decoded_size_bytes as u32)?;
            }
            _ => return None,
        }
    }
    Some((rows?, cols?, metadata_bytes, plan?))
}

fn aux_metadata_bytes(tensor: &QuantTensorRef) -> Option<u32> {
    tensor.aux_blob_refs.iter().try_fold(0_u32, |sum, aux| {
        sum.checked_add(u32::try_from(aux.blob.decoded_size_bytes).ok()?)
    })
}

fn ternary_plan_from_quant_format(format: &QuantFormat) -> TernaryWeightPlan {
    match format {
        QuantFormat::Binary1 {
            scale_granularity,
            scale_format,
        } => TernaryWeightPlan::new(
            WeightEncoding::Binary1,
            weight_scale_granularity(*scale_granularity),
            weight_scale_format(*scale_format),
            ThresholdPlan::FixedQ8_8,
        ),
        QuantFormat::SparseTernaryBitplanes {
            scale_granularity,
            scale_format,
            ..
        } => TernaryWeightPlan::new(
            WeightEncoding::SparseTernaryBitplanes,
            weight_scale_granularity(*scale_granularity),
            weight_scale_format(*scale_format),
            ThresholdPlan::FixedQ8_8,
        ),
        _ => TernaryWeightPlan::new(
            WeightEncoding::Ternary2,
            WeightScaleGranularity::PerOutputRow,
            WeightScaleFormat::Q8_8,
            ThresholdPlan::FixedQ8_8,
        ),
    }
}

fn weight_scale_granularity(granularity: ScaleGranularity) -> WeightScaleGranularity {
    match granularity {
        ScaleGranularity::Global => WeightScaleGranularity::PerTensor,
        ScaleGranularity::PerOutputRow => WeightScaleGranularity::PerOutputRow,
    }
}

fn weight_scale_format(format: AuxFormat) -> WeightScaleFormat {
    match format {
        AuxFormat::Q4_4 => WeightScaleFormat::Q4_4,
        AuxFormat::Pow2 => WeightScaleFormat::Pow2,
        _ => WeightScaleFormat::Q8_8,
    }
}

fn reduction_sites_for_quant_graph(quant_graph: &QuantGraph) -> Vec<ReductionSiteProjection> {
    let mut sites = Vec::new();
    if let Some(routing_table) = &quant_graph.routing_table {
        sites.extend(
            routing_table
                .layers
                .iter()
                .map(|layer| ReductionSiteProjection {
                    site: reduction_site_id(ReductionSiteKey::Router { layer: layer.layer }),
                    layer: Some(layer.layer),
                    expert: None,
                    term_count: quant_graph.identity.model_spec_summary.d_model,
                    input_max_abs_q: 0,
                    weight_max_abs_q: 0,
                    bias_max_abs_q: None,
                    accumulator_domain: AccumulatorDomain::RawIntegerProducts,
                }),
        );
    }
    for section in &quant_graph.expert_sections {
        for slot in EXPERT_WEIGHT_SLOT_CANONICAL_ORDER {
            sites.push(ReductionSiteProjection {
                site: reduction_site_id(ReductionSiteKey::Expert {
                    layer: section.layer,
                    expert: section.expert,
                    slot,
                }),
                layer: Some(section.layer),
                expert: Some(section.expert),
                term_count: quant_graph.identity.model_spec_summary.d_model,
                input_max_abs_q: 0,
                weight_max_abs_q: 0,
                bias_max_abs_q: None,
                accumulator_domain: AccumulatorDomain::RawIntegerProducts,
            });
        }
    }
    sites.extend(
        quant_graph
            .norm_plans
            .iter()
            .map(|record| ReductionSiteProjection {
                site: reduction_site_id(ReductionSiteKey::Norm {
                    norm_plan_id: record.norm_plan_id,
                }),
                layer: None,
                expert: None,
                term_count: quant_graph.identity.model_spec_summary.d_model,
                input_max_abs_q: 0,
                weight_max_abs_q: 0,
                bias_max_abs_q: None,
                accumulator_domain: AccumulatorDomain::PostScaleQ8_8,
            }),
    );
    sites.push(ReductionSiteProjection {
        site: reduction_site_id(ReductionSiteKey::Classify),
        layer: None,
        expert: None,
        term_count: quant_graph.identity.model_spec_summary.d_model,
        input_max_abs_q: 0,
        weight_max_abs_q: 0,
        bias_max_abs_q: None,
        accumulator_domain: AccumulatorDomain::RawIntegerProducts,
    });
    sites
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReductionSiteKey {
    Router {
        layer: LayerId,
    },
    Expert {
        layer: LayerId,
        expert: ExpertId,
        slot: ExpertWeightSlot,
    },
    Norm {
        norm_plan_id: NormPlanId,
    },
    Classify,
}

#[must_use]
pub fn reduction_site_id(key: ReductionSiteKey) -> ReductionSiteId {
    let value = match key {
        ReductionSiteKey::Router { layer } => format!("router.{}", layer.get()),
        ReductionSiteKey::Expert {
            layer,
            expert,
            slot,
        } => format!(
            "expert.{}.{}.{}",
            layer.get(),
            expert.get(),
            expert_slot_label(slot)
        ),
        ReductionSiteKey::Norm { norm_plan_id } => format!("norm.{}", norm_plan_id.get()),
        ReductionSiteKey::Classify => "classify".to_owned(),
    };
    ReductionSiteId(value)
}

fn expert_slot_label(slot: ExpertWeightSlot) -> &'static str {
    match slot {
        ExpertWeightSlot::FfnGate => "gate",
        ExpertWeightSlot::FfnUp => "up",
        ExpertWeightSlot::FfnDown => "down",
    }
}

fn malformed_budget_view(field: impl Into<String>) -> QuantGraphBudgetViewError {
    QuantGraphBudgetViewError::Malformed {
        field: FieldPath::from(field.into()),
    }
}

pub fn bind_provenance(
    tensors: &[QuantTensorRef],
    tensor_exports: &BTreeMap<TensorId, ExportTensorId>,
) -> Result<TensorProvenanceMap, Vec<QuantGraphBindingDiagnostic>> {
    let mut diagnostics = Vec::new();
    let mut provenance = TensorProvenanceMap::new();

    for tensor in tensors {
        let Some(export_tensor_id) = tensor_exports.get(&tensor.tensor_id) else {
            diagnostics.push(QuantGraphBindingDiagnostic::new(
                QuantGraphBindingDiagnosticCode::QuantGraphExportProvenanceMissing {
                    tensor_id: tensor.tensor_id,
                },
                "provenance",
            ));
            continue;
        };
        provenance.insert(tensor.tensor_id, export_tensor_id.clone());
    }

    diagnostics.extend(provenance_image_diagnostics(
        provenance.iter(),
        tensors
            .iter()
            .flat_map(|tensor| tensor.aux_blob_refs.iter()),
    ));

    if diagnostics.is_empty() {
        Ok(provenance)
    } else {
        Err(diagnostics)
    }
}

fn provenance_image_diagnostics<'a>(
    provenance: impl IntoIterator<Item = (&'a TensorId, &'a ExportTensorId)>,
    aux_refs: impl IntoIterator<Item = &'a QuantAuxBlobRef>,
) -> Vec<QuantGraphBindingDiagnostic> {
    let mut diagnostics = Vec::new();
    if let Err(ProvenanceImageError::NotInjective {
        export_tensor_id, ..
    }) = injective_provenance_image(provenance, aux_refs)
    {
        diagnostics.push(QuantGraphBindingDiagnostic::new(
            QuantGraphBindingDiagnosticCode::QuantGraphProvenanceImageNotInjective {
                export_tensor_id,
            },
            "provenance",
        ));
    }
    diagnostics
}

#[must_use]
pub fn canonicalized_quant_graph(mut graph: QuantGraph) -> QuantGraph {
    canonical_sort_quant_graph(&mut graph);
    graph
}

pub fn canonical_sort_quant_graph(graph: &mut QuantGraph) {
    graph.tensors.sort_by_key(|tensor| tensor.tensor_id);
    graph
        .norm_plans
        .sort_by_key(|record| (record.site, record.norm_plan_id));
    if let Some(routing_table) = &mut graph.routing_table {
        routing_table.layers.sort_by_key(|layer| layer.layer);
    }
    graph
        .expert_sections
        .sort_by_key(|section| (section.layer, section.expert));
}

#[derive(Debug, Clone, Copy)]
pub struct QuantGraphSelfConsistencyContext<'a> {
    pub decode_caps: &'a DecodeCapabilitySet,
    pub blob_index: Option<&'a ResolvedBlobIndex>,
    pub training_residue_absent: bool,
    pub sequence_semantics_tensors_match: bool,
    pub required_features_supported: bool,
    pub reduction_order_policy_enforced: bool,
    pub bit_exact_mid_reduction_saturation_absent: bool,
    pub forbidden_storage_metadata_absent: bool,
    pub decode_requires_rng_matches_spec: bool,
}

#[must_use]
pub fn validate_quant_graph_self_consistency(
    graph: &QuantGraph,
    context: QuantGraphSelfConsistencyContext<'_>,
) -> Vec<QuantGraphBindingDiagnostic> {
    let mut diagnostics = Vec::new();
    let model_spec = &graph.identity.model_spec_summary;
    let tensors_by_id = tensors_by_id(&graph.tensors, &mut diagnostics);

    validate_provenance_totality(graph, &mut diagnostics);
    if !context.training_residue_absent {
        diagnostics.push(QuantGraphBindingDiagnostic::new(
            QuantGraphBindingDiagnosticCode::QuantGraphTrainingResidue,
            "tensors",
        ));
    }
    diagnostics.extend(provenance_image_diagnostics(
        graph.provenance.iter(),
        graph
            .tensors
            .iter()
            .flat_map(|tensor| tensor.aux_blob_refs.iter()),
    ));
    validate_norm_references(graph, &mut diagnostics);
    validate_routing_topology(graph, model_spec, &mut diagnostics);
    validate_expert_sections(graph, &tensors_by_id, &mut diagnostics);
    validate_classify_head(graph, &tensors_by_id, &mut diagnostics);
    if !check_decode_spec_in_capabilities(&graph.decode_spec.spec, context.decode_caps) {
        diagnostics.push(QuantGraphBindingDiagnostic::new(
            QuantGraphBindingDiagnosticCode::QuantGraphDecodeSpecNotInCapabilitySet,
            "decode_spec.spec",
        ));
    }
    validate_layouts(graph, model_spec, &mut diagnostics);
    validate_sequence_semantics(graph, &mut diagnostics);
    if !context.sequence_semantics_tensors_match {
        diagnostics.push(QuantGraphBindingDiagnostic::new(
            QuantGraphBindingDiagnosticCode::QuantGraphSequenceSemanticsTensorMismatch,
            "sequence_semantics",
        ));
    }
    if !context.required_features_supported {
        diagnostics.push(QuantGraphBindingDiagnostic::new(
            QuantGraphBindingDiagnosticCode::QuantGraphRequiredFeatureUnsupported,
            "identity",
        ));
    }
    validate_blob_sizes(graph, context.blob_index, &mut diagnostics);
    validate_embedding_unique(graph, &mut diagnostics);
    validate_model_spec_and_ffn_plans(graph, model_spec, &mut diagnostics);
    validate_layer_norms(graph, model_spec.n_layers, &mut diagnostics);
    validate_norm_sites(graph, model_spec.n_layers, &mut diagnostics);
    validate_aux_blob_refs(graph, &mut diagnostics);
    if !context.decode_requires_rng_matches_spec || !graph.decode_spec.requires_rng_matches_spec() {
        diagnostics.push(QuantGraphBindingDiagnostic::new(
            QuantGraphBindingDiagnosticCode::QuantGraphDecodeRequiresRngMismatch,
            "decode_spec.requires_rng",
        ));
    }
    if bind_residual_plan(ResidualPlanInput {
        activation_format: graph.residual_plan.activation_format.clone(),
        combine_policy: graph.residual_plan.combine_policy,
    })
    .is_err()
    {
        diagnostics.push(QuantGraphBindingDiagnostic::new(
            QuantGraphBindingDiagnosticCode::QuantGraphResidualPlanInvalid,
            "residual_plan",
        ));
    }
    if graph.identity.determinism == DeterminismClass::BitExact
        && !context.reduction_order_policy_enforced
    {
        diagnostics.push(QuantGraphBindingDiagnostic::new(
            QuantGraphBindingDiagnosticCode::QuantGraphDeterminismRequiresEnforcedReductionOrder,
            "identity.determinism",
        ));
    }
    if graph.identity.determinism == DeterminismClass::BitExact
        && !context.bit_exact_mid_reduction_saturation_absent
    {
        diagnostics.push(QuantGraphBindingDiagnostic::new(
            QuantGraphBindingDiagnosticCode::QuantGraphBitExactMidReductionSaturationForbidden,
            "identity.determinism",
        ));
    }
    if !context.forbidden_storage_metadata_absent {
        diagnostics.push(QuantGraphBindingDiagnostic::new(
            QuantGraphBindingDiagnosticCode::QuantGraphForbiddenStorageMetadata,
            "storage_metadata",
        ));
    }

    diagnostics
}

fn tensors_by_id<'a>(
    tensors: &'a [QuantTensorRef],
    diagnostics: &mut Vec<QuantGraphBindingDiagnostic>,
) -> BTreeMap<TensorId, &'a QuantTensorRef> {
    let mut tensors_by_id = BTreeMap::new();
    for tensor in tensors {
        if tensors_by_id.insert(tensor.tensor_id, tensor).is_some() {
            diagnostics.push(QuantGraphBindingDiagnostic::new(
                QuantGraphBindingDiagnosticCode::QuantGraphTensorIdNotUnique {
                    tensor_id: tensor.tensor_id,
                },
                "tensors.tensor_id",
            ));
        }
    }
    tensors_by_id
}

fn validate_provenance_totality(
    graph: &QuantGraph,
    diagnostics: &mut Vec<QuantGraphBindingDiagnostic>,
) {
    for tensor in &graph.tensors {
        if !graph.provenance.contains_key(&tensor.tensor_id) {
            diagnostics.push(QuantGraphBindingDiagnostic::new(
                QuantGraphBindingDiagnosticCode::QuantGraphExportProvenanceMissing {
                    tensor_id: tensor.tensor_id,
                },
                "provenance",
            ));
        }
    }
}

fn validate_norm_references(
    graph: &QuantGraph,
    diagnostics: &mut Vec<QuantGraphBindingDiagnostic>,
) {
    let known_norm_ids = graph
        .norm_plans
        .iter()
        .map(|record| record.norm_plan_id)
        .collect::<BTreeSet<_>>();
    for tensor in &graph.tensors {
        match tensor.role {
            QuantTensorRole::NormScale { norm_plan } | QuantTensorRole::NormBias { norm_plan }
                if !known_norm_ids.contains(&norm_plan) =>
            {
                diagnostics.push(QuantGraphBindingDiagnostic::new(
                    QuantGraphBindingDiagnosticCode::QuantGraphNormPlanReferenceUnresolved,
                    "tensors.role.norm_plan",
                ));
            }
            _ => {}
        }
    }
    for layer_norms in graph.layer_norms.values() {
        for norm_plan_id in [layer_norms.pre_sequence, layer_norms.pre_ffn] {
            if !known_norm_ids.contains(&norm_plan_id) {
                diagnostics.push(QuantGraphBindingDiagnostic::new(
                    QuantGraphBindingDiagnosticCode::QuantGraphNormPlanReferenceUnresolved,
                    "layer_norms",
                ));
            }
        }
    }
}

fn validate_routing_topology(
    graph: &QuantGraph,
    model_spec: &ModelSpecSummary,
    diagnostics: &mut Vec<QuantGraphBindingDiagnostic>,
) {
    let routed_layers = model_spec
        .ffn_kind
        .iter()
        .filter_map(|(layer, kind)| matches!(kind, FfnKindTag::Routed).then_some(*layer))
        .collect::<BTreeSet<_>>();
    let routing_layers = graph
        .routing_table
        .as_ref()
        .map(|routing_table| routing_table.layers.as_slice())
        .unwrap_or(&[]);

    if routed_layers.is_empty() && graph.routing_table.is_some() {
        for router_layer in routing_layers {
            diagnostics.push(QuantGraphBindingDiagnostic::new(
                QuantGraphBindingDiagnosticCode::QuantGraphRoutingPresentForDenseLayer {
                    layer: router_layer.layer,
                },
                "routing_table.layers",
            ));
        }
    }
    if !routed_layers.is_empty() && graph.routing_table.is_none() {
        for layer in routed_layers {
            diagnostics.push(QuantGraphBindingDiagnostic::new(
                QuantGraphBindingDiagnosticCode::QuantGraphRoutingMissingForRoutedLayer { layer },
                "routing_table",
            ));
        }
        return;
    }

    let section_counts = graph
        .expert_sections
        .iter()
        .map(|section| ((section.layer, section.expert), ()))
        .collect::<BTreeMap<_, _>>();
    for router_layer in routing_layers {
        if !routed_layers.contains(&router_layer.layer) {
            diagnostics.push(QuantGraphBindingDiagnostic::new(
                QuantGraphBindingDiagnosticCode::QuantGraphRoutingPresentForDenseLayer {
                    layer: router_layer.layer,
                },
                "routing_table.layers",
            ));
        }
        if model_spec
            .n_experts
            .get(&router_layer.layer)
            .is_none_or(|expected| *expected != router_layer.n_experts)
        {
            diagnostics.push(QuantGraphBindingDiagnostic::new(
                QuantGraphBindingDiagnosticCode::QuantGraphRoutingExpertCoverageMismatch {
                    layer: router_layer.layer,
                },
                "routing_table.layers.n_experts",
            ));
        }
        for expert_index in 0..router_layer.n_experts {
            let expert = ExpertId::new(expert_index);
            if !section_counts.contains_key(&(router_layer.layer, expert)) {
                diagnostics.push(QuantGraphBindingDiagnostic::new(
                    QuantGraphBindingDiagnosticCode::QuantGraphRoutingExpertCoverageGap {
                        layer: router_layer.layer,
                        expert,
                    },
                    "expert_sections",
                ));
            }
        }
    }

    for section in &graph.expert_sections {
        let expected_experts = match model_spec.ffn_kind.get(&section.layer) {
            Some(FfnKindTag::Dense) => 1,
            Some(FfnKindTag::Routed) => *model_spec.n_experts.get(&section.layer).unwrap_or(&0),
            None => 0,
        };
        if section.expert.get() >= expected_experts {
            diagnostics.push(QuantGraphBindingDiagnostic::new(
                QuantGraphBindingDiagnosticCode::QuantGraphRoutingExpertCoverageExtra {
                    layer: section.layer,
                    expert: section.expert,
                },
                "expert_sections",
            ));
        }
    }
}

fn validate_expert_sections(
    graph: &QuantGraph,
    tensors_by_id: &BTreeMap<TensorId, &QuantTensorRef>,
    diagnostics: &mut Vec<QuantGraphBindingDiagnostic>,
) {
    for section in &graph.expert_sections {
        let mut refs = Vec::new();
        for tensor_id in &section.tensor_refs {
            match tensors_by_id.get(tensor_id).map(|tensor| &tensor.role) {
                Some(QuantTensorRole::ExpertWeight {
                    layer,
                    expert,
                    slot,
                }) if *layer == section.layer && *expert == section.expert => {
                    refs.push((slot, false, *tensor_id));
                }
                Some(QuantTensorRole::ExpertBias {
                    layer,
                    expert,
                    slot,
                }) if *layer == section.layer && *expert == section.expert => {
                    refs.push((slot, true, *tensor_id));
                }
                _ => diagnostics.push(QuantGraphBindingDiagnostic::new(
                    QuantGraphBindingDiagnosticCode::QuantGraphExpertSectionWeightMissing {
                        layer: section.layer,
                        expert: section.expert,
                    },
                    "expert_sections.tensor_refs",
                )),
            }
        }

        let has_up = refs
            .iter()
            .any(|(slot, is_bias, _)| **slot == ExpertWeightSlot::FfnUp && !*is_bias);
        let has_down = refs
            .iter()
            .any(|(slot, is_bias, _)| **slot == ExpertWeightSlot::FfnDown && !*is_bias);
        if !has_up || !has_down {
            diagnostics.push(QuantGraphBindingDiagnostic::new(
                QuantGraphBindingDiagnosticCode::QuantGraphExpertSectionWeightMissing {
                    layer: section.layer,
                    expert: section.expert,
                },
                "expert_sections.tensor_refs",
            ));
        }

        if canonical_expert_tensor_refs(&refs) != section.tensor_refs {
            diagnostics.push(QuantGraphBindingDiagnostic::new(
                QuantGraphBindingDiagnosticCode::QuantGraphExpertSectionWeightMissing {
                    layer: section.layer,
                    expert: section.expert,
                },
                "expert_sections.tensor_refs",
            ));
        }

        let has_gate = refs
            .iter()
            .any(|(slot, is_bias, _)| **slot == ExpertWeightSlot::FfnGate && !*is_bias);
        let wants_gate = graph
            .ffn_plans
            .get(&section.layer)
            .is_some_and(|plan| plan.activation_kind == FfnActivationKind::SwiGLU);
        if has_gate != wants_gate {
            diagnostics.push(QuantGraphBindingDiagnostic::new(
                QuantGraphBindingDiagnosticCode::QuantGraphFfnGatePresenceMismatch {
                    layer: section.layer,
                    expert: section.expert,
                },
                "expert_sections.tensor_refs",
            ));
        }
    }
}

fn validate_classify_head(
    graph: &QuantGraph,
    tensors_by_id: &BTreeMap<TensorId, &QuantTensorRef>,
    diagnostics: &mut Vec<QuantGraphBindingDiagnostic>,
) {
    let embedding_id = graph
        .tensors
        .iter()
        .find(|tensor| tensor.role == QuantTensorRole::EmbeddingTable)
        .map(|tensor| tensor.tensor_id);
    match graph.classify_head.kind {
        ClassifyHeadKind::Tied => {
            if embedding_id != Some(graph.classify_head.weight)
                || !matches!(
                    tensors_by_id
                        .get(&graph.classify_head.weight)
                        .map(|tensor| &tensor.role),
                    Some(QuantTensorRole::EmbeddingTable)
                )
            {
                diagnostics.push(QuantGraphBindingDiagnostic::new(
                    QuantGraphBindingDiagnosticCode::QuantGraphClassifyHeadTiedMismatch,
                    "classify_head.weight",
                ));
            }
        }
        ClassifyHeadKind::Untied => {
            if !matches!(
                tensors_by_id
                    .get(&graph.classify_head.weight)
                    .map(|tensor| &tensor.role),
                Some(QuantTensorRole::ClassifyWeight)
            ) {
                diagnostics.push(QuantGraphBindingDiagnostic::new(
                    QuantGraphBindingDiagnosticCode::QuantGraphClassifyHeadTiedMismatch,
                    "classify_head.weight",
                ));
            }
        }
    }
    if !classify_logit_format_is_activation_set(&graph.classify_head.logit_format) {
        diagnostics.push(QuantGraphBindingDiagnostic::new(
            QuantGraphBindingDiagnosticCode::QuantGraphClassifyHeadFormatMismatch,
            "classify_head.logit_format",
        ));
    }
}

fn validate_layouts(
    graph: &QuantGraph,
    model_spec: &ModelSpecSummary,
    diagnostics: &mut Vec<QuantGraphBindingDiagnostic>,
) {
    for tensor in &graph.tensors {
        if expected_role_dims(&tensor.role, model_spec)
            .is_some_and(|expected| tensor.layout.shape.dims() != expected.as_slice())
        {
            diagnostics.push(QuantGraphBindingDiagnostic::new(
                QuantGraphBindingDiagnosticCode::QuantGraphLayoutInconsistentWithModelSpec,
                "tensors.layout",
            ));
        }
    }
}

fn expected_role_dims(role: &QuantTensorRole, model_spec: &ModelSpecSummary) -> Option<Vec<u32>> {
    match role {
        QuantTensorRole::EmbeddingTable | QuantTensorRole::ClassifyWeight => {
            Some(vec![model_spec.vocab_size, model_spec.d_model])
        }
        QuantTensorRole::NormScale { .. } | QuantTensorRole::NormBias { .. } => {
            Some(vec![model_spec.d_model])
        }
        QuantTensorRole::RouterWeight { layer } => model_spec
            .n_experts
            .get(layer)
            .map(|n_experts| vec![u32::from(*n_experts), model_spec.d_model]),
        QuantTensorRole::RouterBias { layer } => model_spec
            .n_experts
            .get(layer)
            .map(|n_experts| vec![u32::from(*n_experts)]),
        QuantTensorRole::ExpertWeight { slot, .. } => match slot {
            ExpertWeightSlot::FfnGate | ExpertWeightSlot::FfnUp => {
                Some(vec![model_spec.d_ff, model_spec.d_model])
            }
            ExpertWeightSlot::FfnDown => Some(vec![model_spec.d_model, model_spec.d_ff]),
        },
        QuantTensorRole::ExpertBias { slot, .. } => match slot {
            ExpertWeightSlot::FfnGate | ExpertWeightSlot::FfnUp => Some(vec![model_spec.d_ff]),
            ExpertWeightSlot::FfnDown => Some(vec![model_spec.d_model]),
        },
        QuantTensorRole::ClassifyBias => Some(vec![model_spec.vocab_size]),
    }
}

fn validate_sequence_semantics(
    graph: &QuantGraph,
    diagnostics: &mut Vec<QuantGraphBindingDiagnostic>,
) {
    if graph.sequence_semantics.kind != SequenceSemanticsKind::Identity
        || graph.sequence_semantics.has_state_slots()
    {
        diagnostics.push(QuantGraphBindingDiagnostic::new(
            QuantGraphBindingDiagnosticCode::QuantGraphSequenceSemanticsTensorMismatch,
            "sequence_semantics",
        ));
    }
}

fn validate_blob_sizes(
    graph: &QuantGraph,
    blob_index: Option<&ResolvedBlobIndex>,
    diagnostics: &mut Vec<QuantGraphBindingDiagnostic>,
) {
    for tensor in &graph.tensors {
        let expected = expected_decoded_tensor_payload_size(
            &tensor.layout,
            &tensor.quant_format,
            &tensor.role,
        );
        if tensor.blob.decoded_size_bytes != expected {
            diagnostics.push(QuantGraphBindingDiagnostic::new(
                QuantGraphBindingDiagnosticCode::QuantGraphBlobRefSizeMismatch {
                    blob_ref: tensor.blob.blob_ref,
                    expected_decoded_size_bytes: expected,
                    observed_decoded_size_bytes: tensor.blob.decoded_size_bytes,
                },
                "tensors.blob.decoded_size_bytes",
            ));
        }
        validate_blob_index_entry(&tensor.blob, blob_index, diagnostics, false);

        for aux_ref in &tensor.aux_blob_refs {
            let expected = expected_decoded_aux_payload_size(&aux_ref.layout, aux_ref.format);
            if aux_ref.blob.decoded_size_bytes != expected {
                diagnostics.push(QuantGraphBindingDiagnostic::new(
                    QuantGraphBindingDiagnosticCode::QuantGraphAuxBlobRefSizeMismatch {
                        blob_ref: aux_ref.blob.blob_ref,
                        expected_decoded_size_bytes: expected,
                        observed_decoded_size_bytes: aux_ref.blob.decoded_size_bytes,
                    },
                    "tensors.aux_blob_refs.blob.decoded_size_bytes",
                ));
            }
            validate_blob_index_entry(&aux_ref.blob, blob_index, diagnostics, true);
        }
    }
}

fn validate_blob_index_entry(
    blob: &ResolvedBlobRef,
    blob_index: Option<&ResolvedBlobIndex>,
    diagnostics: &mut Vec<QuantGraphBindingDiagnostic>,
    is_aux: bool,
) {
    let Some(blob_index) = blob_index else {
        return;
    };
    let Some(metadata) = blob_index.entries.get(&blob.blob_ref) else {
        diagnostics.push(QuantGraphBindingDiagnostic::new(
            QuantGraphBindingDiagnosticCode::QuantGraphBlobRefUnresolvable {
                blob_ref: blob.blob_ref,
            },
            "blob_ref",
        ));
        return;
    };
    if metadata.content_hash != blob.content_hash
        || metadata.codec != blob.codec
        || metadata.encoded_size_bytes != blob.encoded_size_bytes
        || metadata.decoded_size_bytes != blob.decoded_size_bytes
    {
        let code = if is_aux {
            QuantGraphBindingDiagnosticCode::QuantGraphAuxBlobRefSizeMismatch {
                blob_ref: blob.blob_ref,
                expected_decoded_size_bytes: metadata.decoded_size_bytes,
                observed_decoded_size_bytes: blob.decoded_size_bytes,
            }
        } else {
            QuantGraphBindingDiagnosticCode::QuantGraphBlobRefSizeMismatch {
                blob_ref: blob.blob_ref,
                expected_decoded_size_bytes: metadata.decoded_size_bytes,
                observed_decoded_size_bytes: blob.decoded_size_bytes,
            }
        };
        diagnostics.push(QuantGraphBindingDiagnostic::new(code, "blob"));
    }
}

fn validate_embedding_unique(
    graph: &QuantGraph,
    diagnostics: &mut Vec<QuantGraphBindingDiagnostic>,
) {
    let count = graph
        .tensors
        .iter()
        .filter(|tensor| tensor.role == QuantTensorRole::EmbeddingTable)
        .count();
    match count {
        0 => diagnostics.push(QuantGraphBindingDiagnostic::new(
            QuantGraphBindingDiagnosticCode::QuantGraphEmbeddingMissing,
            "tensors.role",
        )),
        1 => {}
        _ => diagnostics.push(QuantGraphBindingDiagnostic::new(
            QuantGraphBindingDiagnosticCode::QuantGraphEmbeddingNotUnique,
            "tensors.role",
        )),
    }
}

fn validate_model_spec_and_ffn_plans(
    graph: &QuantGraph,
    model_spec: &ModelSpecSummary,
    diagnostics: &mut Vec<QuantGraphBindingDiagnostic>,
) {
    let expected_layers = (0..model_spec.n_layers)
        .map(LayerId::new)
        .collect::<BTreeSet<_>>();
    if model_spec.vocab_size == 0
        || model_spec.d_model == 0
        || model_spec.d_ff == 0
        || model_spec.ffn_kind.keys().copied().collect::<BTreeSet<_>>() != expected_layers
        || model_spec
            .n_experts
            .keys()
            .copied()
            .collect::<BTreeSet<_>>()
            != expected_layers
    {
        diagnostics.push(QuantGraphBindingDiagnostic::new(
            QuantGraphBindingDiagnosticCode::QuantGraphLayoutInconsistentWithModelSpec,
            "identity.model_spec_summary",
        ));
    }

    if graph.ffn_plans.keys().copied().collect::<BTreeSet<_>>() != expected_layers {
        diagnostics.push(QuantGraphBindingDiagnostic::new(
            QuantGraphBindingDiagnosticCode::QuantGraphLayoutInconsistentWithModelSpec,
            "ffn_plans",
        ));
    }
    for plan in graph.ffn_plans.values() {
        if !classify_logit_format_is_activation_set(&plan.intermediate_format) {
            diagnostics.push(QuantGraphBindingDiagnostic::new(
                QuantGraphBindingDiagnosticCode::QuantGraphLayoutInconsistentWithModelSpec,
                "ffn_plans.intermediate_format",
            ));
        }
    }
}

fn validate_layer_norms(
    graph: &QuantGraph,
    n_layers: u16,
    diagnostics: &mut Vec<QuantGraphBindingDiagnostic>,
) {
    let expected_layers = (0..n_layers).map(LayerId::new).collect::<BTreeSet<_>>();
    let observed_layers = graph.layer_norms.keys().copied().collect::<BTreeSet<_>>();
    if observed_layers != expected_layers {
        for layer in expected_layers.difference(&observed_layers) {
            diagnostics.push(QuantGraphBindingDiagnostic::new(
                QuantGraphBindingDiagnosticCode::QuantGraphLayerNormsIncomplete { layer: *layer },
                "layer_norms",
            ));
        }
    }
}

fn validate_norm_sites(
    graph: &QuantGraph,
    n_layers: u16,
    diagnostics: &mut Vec<QuantGraphBindingDiagnostic>,
) {
    let mut by_site = BTreeMap::<NormSite, NormPlanId>::new();
    let mut final_count = 0_usize;
    for record in &graph.norm_plans {
        if by_site.insert(record.site, record.norm_plan_id).is_some() {
            diagnostics.push(QuantGraphBindingDiagnostic::new(
                QuantGraphBindingDiagnosticCode::QuantGraphNormSiteDuplicate,
                "norm_plans.site",
            ));
        }
        if record.site == NormSite::Final {
            final_count += 1;
        }
    }
    if final_count != 1 {
        diagnostics.push(QuantGraphBindingDiagnostic::new(
            QuantGraphBindingDiagnosticCode::QuantGraphFinalNormMissing,
            "norm_plans.site",
        ));
    }

    for layer_index in 0..n_layers {
        let layer = LayerId::new(layer_index);
        let Some(layer_norms) = graph.layer_norms.get(&layer) else {
            continue;
        };
        if by_site.get(&NormSite::LayerSequence { layer }) != Some(&layer_norms.pre_sequence)
            || by_site.get(&NormSite::LayerFfn { layer }) != Some(&layer_norms.pre_ffn)
        {
            diagnostics.push(QuantGraphBindingDiagnostic::new(
                QuantGraphBindingDiagnosticCode::QuantGraphNormPlanReferenceUnresolved,
                "layer_norms",
            ));
        }
    }
}

fn validate_aux_blob_refs(graph: &QuantGraph, diagnostics: &mut Vec<QuantGraphBindingDiagnostic>) {
    for tensor in &graph.tensors {
        diagnostics.extend(validate_aux_blob_refs_for_format(
            &tensor.quant_format,
            &tensor.aux_blob_refs,
        ));
    }
}

fn domain_hash(
    type_name: &str,
    schema_id: &str,
    schema_version: &str,
    canonical: &[u8],
) -> Hash256 {
    let mut hasher = Sha256::new();
    hasher.update(format!(
        "gbf:gbf-codegen:{type_name}:{schema_id}:{schema_version}\0"
    ));
    hasher.update(canonical);
    Hash256::from_bytes(hasher.finalize().into())
}

mod resolved_blob_entries {
    use std::collections::BTreeMap;

    use gbf_foundation::BlobRef;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use super::BlobMetadata;

    #[derive(Serialize, Deserialize)]
    struct Entry {
        blob_ref: BlobRef,
        metadata: BlobMetadata,
    }

    pub fn serialize<S>(
        entries: &BTreeMap<BlobRef, BlobMetadata>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        entries
            .iter()
            .map(|(blob_ref, metadata)| Entry {
                blob_ref: *blob_ref,
                metadata: metadata.clone(),
            })
            .collect::<Vec<_>>()
            .serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<BTreeMap<BlobRef, BlobMetadata>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let entries = Vec::<Entry>::deserialize(deserializer)?;
        Ok(entries
            .into_iter()
            .map(|entry| (entry.blob_ref, entry.metadata))
            .collect())
    }
}

mod layer_map_entries {
    use std::collections::BTreeMap;

    use gbf_foundation::LayerId;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    #[derive(Serialize)]
    struct EntryRef<'a, V> {
        layer: LayerId,
        value: &'a V,
    }

    #[derive(Deserialize)]
    struct Entry<V> {
        layer: LayerId,
        value: V,
    }

    pub fn serialize<S, V>(entries: &BTreeMap<LayerId, V>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
        V: Serialize,
    {
        entries
            .iter()
            .map(|(layer, value)| EntryRef {
                layer: *layer,
                value,
            })
            .collect::<Vec<_>>()
            .serialize(serializer)
    }

    pub fn deserialize<'de, D, V>(deserializer: D) -> Result<BTreeMap<LayerId, V>, D::Error>
    where
        D: Deserializer<'de>,
        V: Deserialize<'de>,
    {
        let entries = Vec::<Entry<V>>::deserialize(deserializer)?;
        Ok(entries
            .into_iter()
            .map(|entry| (entry.layer, entry.value))
            .collect())
    }
}

mod tensor_map_entries {
    use std::collections::BTreeMap;

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use super::TensorId;

    #[derive(Serialize)]
    struct EntryRef<'a, V> {
        tensor_id: TensorId,
        value: &'a V,
    }

    #[derive(Deserialize)]
    struct Entry<V> {
        tensor_id: TensorId,
        value: V,
    }

    pub fn serialize<S, V>(
        entries: &BTreeMap<TensorId, V>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
        V: Serialize,
    {
        entries
            .iter()
            .map(|(tensor_id, value)| EntryRef {
                tensor_id: *tensor_id,
                value,
            })
            .collect::<Vec<_>>()
            .serialize(serializer)
    }

    pub fn deserialize<'de, D, V>(deserializer: D) -> Result<BTreeMap<TensorId, V>, D::Error>
    where
        D: Deserializer<'de>,
        V: Deserialize<'de>,
    {
        let entries = Vec::<Entry<V>>::deserialize(deserializer)?;
        Ok(entries
            .into_iter()
            .map(|entry| (entry.tensor_id, entry.value))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    use gbf_artifact::ids::ArtifactPath;
    use gbf_artifact::tensor::{CanonicalTensorShape, TensorElementType};
    use gbf_policy::EvidenceRef;
    use gbf_store::blob::BlobStore;
    use gbf_store::stage_cache::StageCache;
    use gbf_workload::{GoldenVectorRef, WorkloadManifestRef};
    use proptest::prelude::*;
    use serde_json::Value;
    use tracing_subscriber::filter::LevelFilter;
    use tracing_subscriber::prelude::*;

    use crate::validate::{
        ArtifactResolveError, ResolvedBlob, ResolvedEvidence, ResolvedGoldenVector,
        ResolvedSidecar, ResolvedWorkload, SidecarRef,
    };

    use super::*;

    #[test]
    fn quant_graph_struct_top_level_shape_is_closed() {
        let value = serde_json::to_value(quant_graph_fixture()).expect("QuantGraph serializes");
        let keys = object_keys(&value);

        assert_eq!(
            keys,
            vec![
                "classify_head",
                "decode_spec",
                "expert_sections",
                "ffn_plans",
                "identity",
                "layer_norms",
                "norm_plans",
                "provenance",
                "residual_plan",
                "routing_table",
                "sequence_semantics",
                "tensors",
            ]
        );
    }

    #[test]
    fn quant_graph_identity_carries_lowering_manifest_hash() {
        let identity = identity_fixture();

        assert_eq!(identity.lowering_manifest_hash, hash(5));
        assert!(
            serde_json::to_value(identity)
                .expect("identity serializes")
                .get("lowering_manifest_hash")
                .is_some()
        );
    }

    #[test]
    fn quant_graph_identity_does_not_contain_self_hash() {
        let value = serde_json::to_value(identity_fixture()).expect("identity serializes");
        let keys = object_keys(&value);

        assert!(value.get("quant_graph_self_hash").is_none());
        assert!(value.get("quant_graph_canonical_bytes_hash").is_none());
        assert!(!keys.contains(&"self_hash"));
    }

    #[test]
    fn quant_graph_no_storage_metadata_field_compile_check() {
        let value = serde_json::to_value(quant_graph_fixture()).expect("QuantGraph serializes");
        let mut keys = Vec::new();
        collect_json_keys("", &value, &mut keys);
        let forbidden = [
            "residency",
            "storage_class",
            "storage",
            "lifetime",
            "page_id",
            "arena",
            "alias",
            "accumulator_width",
            "tile_size",
        ];

        for key in keys {
            let normalized = key.to_ascii_lowercase();
            assert!(
                forbidden
                    .iter()
                    .all(|forbidden| !normalized.contains(forbidden)),
                "storage-only field leaked into QuantGraph JSON key {key}"
            );
        }
    }

    #[test]
    fn identity_binding_pulls_determinism_from_artifact_not_policy() {
        let inputs = identity_binding_inputs(
            DeterminismClass::BitExact,
            Some(DeterminismClass::Nondeterministic),
        );
        let identity = bind_identity(inputs).expect("identity binding succeeds");

        assert_eq!(identity.determinism, DeterminismClass::BitExact);
    }

    #[test]
    fn identity_binding_records_lowering_manifest_hash() {
        let identity = bind_identity(identity_binding_inputs(
            DeterminismClass::Deterministic,
            None,
        ))
        .expect("identity binding succeeds");

        assert_eq!(identity.lowering_manifest_hash, hash(5));
    }

    #[test]
    fn identity_binding_emits_hash_mismatch_diagnostic_typed() {
        let mut inputs = identity_binding_inputs(DeterminismClass::BitExact, None);
        inputs.semantic_core_hash = hash(0xee);

        let diagnostics = bind_identity(inputs).expect_err("hash mismatch rejects");

        assert!(diagnostics.iter().any(|diagnostic| matches!(
            diagnostic.code,
            QuantGraphBindingDiagnosticCode::QuantGraphIdentityHashMismatch { .. }
        )));
    }

    #[test]
    fn sequence_semantics_binding_runs_before_tensor_binding_compile_check() {
        assert!(
            QuantGraphBindingClass::SequenceSemanticsBinding
                < QuantGraphBindingClass::TensorBinding
        );
    }

    #[test]
    fn sequence_semantics_binding_v1_rejects_non_empty_state_slots() {
        let sequence = SequenceSemanticsSpec {
            kind: SequenceSemanticsKind::Identity,
            state_slots: BTreeMap::from([(
                LayerId::new(0),
                vec![SequenceStateSlot {
                    slot_id: 0,
                    width_bytes: 16,
                }],
            )]),
        };

        let diagnostics = bind_sequence_semantics(SequenceSemanticsBindingInputs {
            artifact_sequence: sequence.clone(),
            requested_sequence: sequence,
        })
        .expect_err("state slots reject in v1");

        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.code
                == QuantGraphBindingDiagnosticCode::QuantGraphSequenceSemanticsUnsupportedV1
        }));
    }

    #[test]
    fn sequence_semantics_binding_rejects_sidecar_rebind() {
        let diagnostics = bind_sequence_semantics(SequenceSemanticsBindingInputs {
            artifact_sequence: SequenceSemanticsSpec::identity(),
            requested_sequence: SequenceSemanticsSpec {
                kind: SequenceSemanticsKind::LinearState,
                state_slots: BTreeMap::new(),
            },
        })
        .expect_err("sidecar sequence rebind rejects");

        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.code
                == QuantGraphBindingDiagnosticCode::QuantGraphSequenceSemanticsSidecarRebind
        }));
    }

    #[test]
    fn norm_plan_id_pre_binding_assigns_ids_in_canonical_order() {
        let binding = bind_norm_plan_ids(2);

        assert_eq!(
            binding.norm_plan_id(NormSite::LayerSequence {
                layer: LayerId::new(0)
            }),
            Some(NormPlanId::new(0))
        );
        assert_eq!(
            binding.norm_plan_id(NormSite::LayerSequence {
                layer: LayerId::new(1)
            }),
            Some(NormPlanId::new(1))
        );
        assert_eq!(
            binding.norm_plan_id(NormSite::LayerFfn {
                layer: LayerId::new(0)
            }),
            Some(NormPlanId::new(2))
        );
        assert_eq!(
            binding.norm_plan_id(NormSite::LayerFfn {
                layer: LayerId::new(1)
            }),
            Some(NormPlanId::new(3))
        );
        assert_eq!(
            binding.norm_plan_id(NormSite::Final),
            Some(NormPlanId::new(4))
        );
    }

    #[test]
    fn norm_plan_id_pre_binding_two_regenerations_match() {
        assert_eq!(bind_norm_plan_ids(3), bind_norm_plan_ids(3));
    }

    #[test]
    fn norm_plan_id_pre_binding_skipping_layer_sequence_is_rejected() {
        let mut binding = bind_norm_plan_ids(2);
        binding.by_site.remove(&NormSite::LayerSequence {
            layer: LayerId::new(1),
        });
        let inputs = norm_plan_inputs_for_layers(2);

        let diagnostics =
            bind_norm_plans(&inputs, &binding).expect_err("skipped LayerSequence rejects");

        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.code
                == QuantGraphBindingDiagnosticCode::QuantGraphNormPlanReferenceUnresolved
                && diagnostic.field.as_str() == "norm_plans.norm_plan_id"
        }));
    }

    #[test]
    fn norm_plan_id_assignment_canonical_under_layer_count_growth() {
        for n_layers in 0..=4 {
            let binding = bind_norm_plan_ids(n_layers);
            let ids = binding
                .by_site
                .iter()
                .map(|(_site, id)| id.get())
                .collect::<Vec<_>>();
            let expected = (0..u32::from(n_layers) * 2 + 1).collect::<Vec<_>>();

            assert_eq!(ids, expected);
        }
    }

    #[test]
    fn model_spec_summary_excludes_per_layer_ffn_plan_fields() {
        let value = serde_json::to_value(model_summary_fixture()).expect("summary serializes");
        let encoded = serde_json::to_string(&value).expect("summary JSON encodes");

        assert!(!encoded.contains("activation_kind"));
        assert!(!encoded.contains("intermediate_format"));
        assert!(value.get("ffn_kind").is_some());
    }

    #[test]
    fn quant_tensor_role_is_closed_enum_no_shared_dense() {
        let roles = [
            QuantTensorRole::EmbeddingTable,
            QuantTensorRole::NormScale {
                norm_plan: NormPlanId::new(1),
            },
            QuantTensorRole::NormBias {
                norm_plan: NormPlanId::new(1),
            },
            QuantTensorRole::RouterWeight {
                layer: LayerId::new(0),
            },
            QuantTensorRole::RouterBias {
                layer: LayerId::new(0),
            },
            QuantTensorRole::ExpertWeight {
                layer: LayerId::new(0),
                expert: ExpertId::new(0),
                slot: ExpertWeightSlot::FfnUp,
            },
            QuantTensorRole::ExpertBias {
                layer: LayerId::new(0),
                expert: ExpertId::new(0),
                slot: ExpertWeightSlot::FfnUp,
            },
            QuantTensorRole::ClassifyWeight,
            QuantTensorRole::ClassifyBias,
        ];
        let encoded = serde_json::to_string(&roles).expect("roles serialize");

        assert!(!encoded.contains("SharedDense"));
    }

    #[test]
    fn tensor_binding_role_format_predicate_table() {
        assert!(role_format_allowed(
            &QuantTensorRole::EmbeddingTable,
            &QuantFormat::I8
        ));
        assert!(!role_format_allowed(
            &QuantTensorRole::EmbeddingTable,
            &ternary_format()
        ));
        assert!(role_format_allowed(
            &QuantTensorRole::NormScale {
                norm_plan: NormPlanId::new(0)
            },
            &QuantFormat::I16
        ));
        assert!(role_format_allowed(
            &QuantTensorRole::ExpertWeight {
                layer: LayerId::new(0),
                expert: ExpertId::new(0),
                slot: ExpertWeightSlot::FfnUp,
            },
            &ternary_format()
        ));
        assert!(!role_format_allowed(
            &QuantTensorRole::ExpertWeight {
                layer: LayerId::new(0),
                expert: ExpertId::new(0),
                slot: ExpertWeightSlot::FfnUp,
            },
            &QuantFormat::I8
        ));
    }

    #[test]
    fn tensor_binding_aux_blob_kind_per_format_consistency() {
        let scale = bound_aux_ref(QuantAuxKind::Scale, AuxFormat::Q8_8, 0x41, 2);
        let threshold = bound_aux_ref(QuantAuxKind::Threshold, AuxFormat::Q8_8, 0x42, 2);
        let sparse = bound_aux_ref(QuantAuxKind::SparseMeta, AuxFormat::I8, 0x43, 1);

        assert!(
            validate_aux_blob_refs_for_format(&ternary_format(), &[scale.clone(), threshold])
                .is_empty()
        );
        assert!(
            validate_aux_blob_refs_for_format(&binary_format(), std::slice::from_ref(&scale))
                .is_empty()
        );
        assert!(
            validate_aux_blob_refs_for_format(&sparse_ternary_format(), &[scale.clone(), sparse])
                .is_empty()
        );
        assert!(!validate_aux_blob_refs_for_format(&QuantFormat::I8, &[scale]).is_empty());
    }

    #[test]
    fn tensor_binding_uses_resolved_blob_ref_decoded_size_not_layout_size() {
        let input = QuantTensorBindingInput {
            tensor_id: TensorId::new(7),
            layout: layout(&[16]),
            quant_format: ternary_format(),
            role: QuantTensorRole::ExpertWeight {
                layer: LayerId::new(0),
                expert: ExpertId::new(0),
                slot: ExpertWeightSlot::FfnUp,
            },
            blob_ref: blob_ref(0x71, 4, BlobCodec::Raw),
            aux_blob_refs: vec![
                aux_binding_input(QuantAuxKind::Scale, AuxFormat::Q8_8, 0x72, "tensor.scale"),
                aux_binding_input(
                    QuantAuxKind::Threshold,
                    AuxFormat::Q8_8,
                    0x73,
                    "tensor.threshold",
                ),
            ],
        };
        let blob_index = ResolvedBlobIndex::new(BTreeMap::from([
            (input.blob_ref, metadata_with_size(0x71, 4)),
            (input.aux_blob_refs[0].blob_ref, metadata_with_size(0x72, 2)),
            (input.aux_blob_refs[1].blob_ref, metadata_with_size(0x73, 2)),
        ]));

        let tensors = bind_quant_tensors(&[input], &blob_index).expect("packed ternary size binds");

        assert_eq!(tensors[0].blob.decoded_size_bytes, 4);
        assert_ne!(tensors[0].blob.decoded_size_bytes, 32);
    }

    #[test]
    fn tensor_binding_rejects_aux_kind_mismatch() {
        let scale = bound_aux_ref(QuantAuxKind::Scale, AuxFormat::Q8_8, 0x41, 2);
        let diagnostics = validate_aux_blob_refs_for_format(&ternary_format(), &[scale]);

        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.code == QuantGraphBindingDiagnosticCode::QuantGraphAuxBlobKindMismatch
        }));
    }

    #[test]
    fn tensor_binding_rejects_blob_decoded_size_mismatch() {
        let input = QuantTensorBindingInput {
            tensor_id: TensorId::new(8),
            layout: layout(&[16]),
            quant_format: QuantFormat::I8,
            role: QuantTensorRole::EmbeddingTable,
            blob_ref: blob_ref(0x81, 16, BlobCodec::Raw),
            aux_blob_refs: Vec::new(),
        };
        let blob_index = ResolvedBlobIndex::new(BTreeMap::from([(
            input.blob_ref,
            metadata_with_size(0x81, 15),
        )]));

        let diagnostics =
            bind_quant_tensors(&[input], &blob_index).expect_err("decoded-size mismatch rejects");

        assert!(diagnostics.iter().any(|diagnostic| matches!(
            diagnostic.code,
            QuantGraphBindingDiagnosticCode::QuantGraphBlobRefSizeMismatch { .. }
        )));
    }

    #[test]
    fn tensor_binding_rejects_kv_slab_as_quant_tensor() {
        let encoded = serde_json::json!({
            "kind": "KvSlab",
            "layer": 0,
            "slot": 0
        });

        assert!(serde_json::from_value::<QuantTensorRole>(encoded).is_err());
    }

    #[test]
    fn tensor_binding_rejects_training_only_residue_structurally() {
        // No v1 QuantTensorRole variant can represent training-only residue,
        // so malformed inputs fail before tensor binding can emit a diagnostic.
        let encoded = serde_json::json!({
            "kind": "TrainingOnlyResidue",
            "tensor_id": 7
        });
        let valid_roles = serde_json::to_string(&[
            QuantTensorRole::EmbeddingTable,
            QuantTensorRole::ClassifyWeight,
            QuantTensorRole::ClassifyBias,
        ])
        .expect("roles serialize");

        assert!(serde_json::from_value::<QuantTensorRole>(encoded).is_err());
        assert!(!valid_roles.contains("TrainingOnly"));
        assert!(!valid_roles.contains("Residue"));
    }

    #[test]
    fn class_1_to_4_collect_diagnostics_no_short_circuit() {
        let mut inputs = quant_graph_inputs_fixture();
        inputs.identity.semantic_core_hash = hash(0xee);
        inputs.sequence_semantics.requested_sequence = SequenceSemanticsSpec {
            kind: SequenceSemanticsKind::LinearState,
            state_slots: BTreeMap::new(),
        };
        inputs.tensor_bindings.push(QuantTensorBindingInput {
            tensor_id: TensorId::new(1),
            layout: layout(&[1]),
            quant_format: ternary_format(),
            role: QuantTensorRole::EmbeddingTable,
            blob_ref: blob_ref(0x99, 1, BlobCodec::Raw),
            aux_blob_refs: Vec::new(),
        });
        inputs.resolved_blob_index.entries.insert(
            blob_ref(0x99, 1, BlobCodec::Raw),
            metadata_with_size(0x99, 1),
        );

        let diagnostics = bind_construction_classes_1_to_4(&inputs).expect_err("class 1-4 rejects");

        assert!(has_diagnostic(&diagnostics, |code| matches!(
            code,
            QuantGraphBindingDiagnosticCode::QuantGraphIdentityHashMismatch { .. }
        )));
        assert!(has_diagnostic(&diagnostics, |code| {
            *code == QuantGraphBindingDiagnosticCode::QuantGraphSequenceSemanticsSidecarRebind
        }));
        assert!(has_diagnostic(&diagnostics, |code| matches!(
            code,
            QuantGraphBindingDiagnosticCode::QuantGraphTensorIdNotUnique { .. }
        )));
        assert!(has_diagnostic(&diagnostics, |code| {
            *code == QuantGraphBindingDiagnosticCode::QuantGraphRoleFormatMismatch
        }));
    }

    #[test]
    fn class_1_to_4_short_circuit_only_when_inputs_invalidated() {
        let mut invalid_class_1_to_4 = quant_graph_inputs_fixture();
        invalid_class_1_to_4.identity.semantic_core_hash = hash(0xee);
        let invalid_capture = TraceCapture::default();
        let invalid_subscriber = tracing_subscriber::registry()
            .with(LevelFilter::TRACE)
            .with(invalid_capture.clone());

        tracing::callsite::rebuild_interest_cache();
        tracing::subscriber::with_default(invalid_subscriber, || {
            build_quant_graph_core(invalid_class_1_to_4)
                .expect_err("class 1 invalidity short-circuits downstream classes");
        });
        tracing::callsite::rebuild_interest_cache();

        assert!(
            !invalid_capture
                .records()
                .iter()
                .any(|record| { record.field_contains("message", "stage1.types.bind_norm_plans") })
        );

        let mut invalid_class_5 = quant_graph_inputs_fixture();
        invalid_class_5
            .norm_plan_bindings
            .retain(|input| input.site != NormSite::Final);
        let downstream_capture = TraceCapture::default();
        let downstream_subscriber = tracing_subscriber::registry()
            .with(LevelFilter::TRACE)
            .with(downstream_capture.clone());

        tracing::callsite::rebuild_interest_cache();
        tracing::subscriber::with_default(downstream_subscriber, || {
            build_quant_graph_core(invalid_class_5)
                .expect_err("class 5 invalidity is reached after valid class 1-4 inputs");
        });
        tracing::callsite::rebuild_interest_cache();

        assert!(
            downstream_capture
                .records()
                .iter()
                .any(|record| { record.field_contains("message", "stage1.types.bind_norm_plans") })
        );
    }

    #[test]
    fn norm_plan_id_serialization_as_integer() {
        let id = NormPlanId::new(42);
        let value = serde_json::to_value(id).expect("serializes to value");
        assert!(
            value.is_number(),
            "NormPlanId must serialize as a bare integer"
        );
        assert_eq!(value.as_u64(), Some(42));
    }

    #[test]
    fn norm_site_is_closed_enum_with_three_variants() {
        let sites = [
            NormSite::LayerSequence {
                layer: LayerId::new(0),
            },
            NormSite::LayerFfn {
                layer: LayerId::new(0),
            },
            NormSite::Final,
        ];
        let encoded = serde_json::to_string(&sites).expect("norm sites serialize");

        assert!(encoded.contains("LayerSequence"));
        assert!(encoded.contains("LayerFfn"));
        assert!(encoded.contains("Final"));
        assert!(!encoded.contains("PostAttention"));
        assert!(!encoded.contains("LayerOutput"));
    }

    #[test]
    fn norm_plan_record_carries_input_and_output_format() {
        let record = norm_plan_record(7, NormSite::Final, QuantFormat::Q8_8, QuantFormat::I8);
        let value = serde_json::to_value(record).expect("norm plan record serializes");

        assert_eq!(value["norm_plan_id"], Value::from(7_u64));
        assert_eq!(value["site"]["kind"], Value::String("Final".to_owned()));
        assert_eq!(
            value["input_format"]["kind"],
            Value::String("Q8_8".to_owned())
        );
        assert_eq!(
            value["output_format"]["kind"],
            Value::String("I8".to_owned())
        );
        assert!(value.get("plan").is_some());
    }

    #[test]
    fn norm_plan_lookup_is_named_not_positional() {
        let mut graph = quant_graph_fixture();
        graph.norm_plans = vec![
            norm_plan_record(30, NormSite::Final, QuantFormat::I8, QuantFormat::I8),
            norm_plan_record(
                10,
                NormSite::LayerSequence {
                    layer: LayerId::new(0),
                },
                QuantFormat::Q8_8,
                QuantFormat::I8,
            ),
            norm_plan_record(
                20,
                NormSite::LayerFfn {
                    layer: LayerId::new(0),
                },
                QuantFormat::I8,
                QuantFormat::I8,
            ),
        ];

        let record = graph
            .norm_plan(NormSite::LayerSequence {
                layer: LayerId::new(0),
            })
            .expect("layer sequence norm resolves by site");

        assert_eq!(record.norm_plan_id, NormPlanId::new(10));
        assert_ne!(graph.norm_plans[0].norm_plan_id, record.norm_plan_id);
        assert!(
            graph
                .norm_plan(NormSite::LayerFfn {
                    layer: LayerId::new(1),
                })
                .is_none()
        );
    }

    #[test]
    fn layer_norms_struct_has_two_required_norm_plan_ids() {
        let layer_norms = LayerNorms {
            pre_sequence: NormPlanId::new(1),
            pre_ffn: NormPlanId::new(2),
        };
        let value = serde_json::to_value(layer_norms).expect("layer norms serialize");
        let keys = object_keys(&value);

        assert_eq!(keys, vec!["pre_ffn", "pre_sequence"]);
        assert_eq!(value["pre_sequence"], Value::from(1_u64));
        assert_eq!(value["pre_ffn"], Value::from(2_u64));
    }

    #[test]
    fn layer_norms_does_not_contain_final_norm() {
        let value = serde_json::to_value(LayerNorms {
            pre_sequence: NormPlanId::new(1),
            pre_ffn: NormPlanId::new(2),
        })
        .expect("layer norms serialize");

        assert!(value.get("final").is_none());
        assert!(value.get("final_norm").is_none());
        assert!(
            !serde_json::to_string(&value)
                .expect("layer norms JSON encodes")
                .contains("Final")
        );
    }

    #[test]
    fn layer_norms_btreemap_canonical_iteration_order() {
        let norm_plan_ids = bind_norm_plan_ids(3);
        let inputs = norm_plan_inputs_for_layers(3);
        let norm_plans = bind_norm_plans(&inputs, &norm_plan_ids).expect("norm plans bind");
        let layer_norms = bind_layer_norms(3, &norm_plans).expect("layer norms bind");

        assert_eq!(
            layer_norms.keys().copied().collect::<Vec<_>>(),
            vec![LayerId::new(0), LayerId::new(1), LayerId::new(2)]
        );
    }

    #[test]
    fn norm_plan_binding_logging_events_are_subscriber_captured() {
        let capture = TraceCapture::default();
        let subscriber = tracing_subscriber::registry()
            .with(LevelFilter::TRACE)
            .with(capture.clone());
        let norm_plan_ids = bind_norm_plan_ids(1);
        let inputs = norm_plan_inputs_for_layers(1);

        tracing::callsite::rebuild_interest_cache();
        tracing::subscriber::with_default(subscriber, || {
            bind_norm_plans(&inputs, &norm_plan_ids).expect("norm plans bind");
        });
        tracing::callsite::rebuild_interest_cache();

        let records = capture.records();
        assert!(records.iter().any(|record| {
            record.level == "INFO"
                && record.field_contains("message", "stage1.types.bind_norm_plans")
                && record.field_equals("n_norm_plans", "3")
                && record.field_equals("has_final_norm", "true")
        }));
        assert!(records.iter().any(|record| {
            record.level == "DEBUG"
                && record.field_contains("message", "stage1.types.norm_site")
                && record.field_equals("norm_plan_id", "0")
                && record.field_contains("site", "LayerSequence")
        }));
    }

    #[test]
    fn construction_class_logging_events_are_subscriber_captured() {
        let capture = TraceCapture::default();
        let subscriber = tracing_subscriber::registry()
            .with(LevelFilter::TRACE)
            .with(capture.clone());
        let inputs = quant_graph_inputs_fixture();

        tracing::callsite::rebuild_interest_cache();
        tracing::subscriber::with_default(subscriber, || {
            bind_construction_classes_1_to_4(&inputs).expect("class 1-4 bindings succeed");
        });
        tracing::callsite::rebuild_interest_cache();

        let records = capture.records();
        assert!(records.iter().any(|record| {
            record.level == "INFO"
                && record.field_equals("event", STAGE1_BINDING_IDENTITY_EVENT)
                && record.field_equals("n_layers", "1")
                && record.field_equals("policy_determinism_present", "false")
        }));
        assert!(records.iter().any(|record| {
            record.level == "INFO"
                && record.field_equals("event", STAGE1_BINDING_SEQUENCE_SEMANTICS_EVENT)
                && record.field_contains("artifact_kind", "Identity")
        }));
        assert!(records.iter().any(|record| {
            record.level == "INFO"
                && record.field_equals("event", STAGE1_BINDING_NORM_PLAN_ID_PRE_EVENT)
                && record.field_equals("n_norm_plan_ids", "3")
        }));
        assert!(records.iter().any(|record| {
            record.level == "INFO"
                && record.field_equals("event", STAGE1_BINDING_TENSOR_EVENT)
                && record.field_equals("n_tensors", "3")
                && record.field_equals("n_diagnostics", "0")
        }));
        assert!(records.iter().any(|record| {
            record.level == "DEBUG"
                && record.field_equals("event", STAGE1_BINDING_TENSOR_ONE_EVENT)
                && record.field_equals("tensor_id", "1")
        }));
    }

    #[test]
    fn construction_class_first_hard_logging_is_subscriber_captured() {
        let capture = TraceCapture::default();
        let subscriber = tracing_subscriber::registry()
            .with(LevelFilter::TRACE)
            .with(capture.clone());
        let mut inputs = quant_graph_inputs_fixture();
        inputs.identity.semantic_core_hash = hash(0xee);

        tracing::callsite::rebuild_interest_cache();
        tracing::subscriber::with_default(subscriber, || {
            bind_construction_classes_1_to_4(&inputs).expect_err("identity mismatch rejects");
        });
        tracing::callsite::rebuild_interest_cache();

        let records = capture.records();
        assert!(records.iter().any(|record| {
            record.level == "ERROR"
                && record.field_equals("event", STAGE1_BINDING_FIRST_HARD_EVENT)
                && record.field_contains("class", "IdentityBinding")
                && record.field_contains("field", "identity.semantic_core_hash")
        }));
    }

    #[test]
    fn quant_graph_product_implements_budget_source() {
        let product = quant_graph_product_fixture();
        let view = product.to_budget_view().expect("budget view projects");

        assert_eq!(product.quant_graph_hash(), product.quant_graph_self_hash);
        assert_eq!(
            product.semantic_core_hash(),
            product.quant_graph.identity.semantic_core_hash
        );
        assert_eq!(view.quant_graph_hash, product.quant_graph_self_hash);
        assert_eq!(
            view.semantic_core_hash,
            product.quant_graph.identity.semantic_core_hash
        );
        assert_eq!(view.layers, vec![LayerId::new(0)]);
        assert_eq!(view.experts.len(), 1);
        assert!(view.shared_dense_ffn.is_none());
    }

    #[test]
    fn quant_graph_to_budget_view_always_emits_shared_dense_ffn_none_in_v1() {
        let product = quant_graph_product_fixture();

        assert!(
            product
                .to_budget_view()
                .expect("budget view projects")
                .shared_dense_ffn
                .is_none()
        );
    }

    #[test]
    fn quant_graph_reduction_site_id_canonical_scheme_table() {
        let cases = [
            (
                ReductionSiteKey::Router {
                    layer: LayerId::new(2),
                },
                "router.2",
            ),
            (
                ReductionSiteKey::Expert {
                    layer: LayerId::new(3),
                    expert: ExpertId::new(5),
                    slot: ExpertWeightSlot::FfnGate,
                },
                "expert.3.5.gate",
            ),
            (
                ReductionSiteKey::Expert {
                    layer: LayerId::new(3),
                    expert: ExpertId::new(5),
                    slot: ExpertWeightSlot::FfnUp,
                },
                "expert.3.5.up",
            ),
            (
                ReductionSiteKey::Expert {
                    layer: LayerId::new(3),
                    expert: ExpertId::new(5),
                    slot: ExpertWeightSlot::FfnDown,
                },
                "expert.3.5.down",
            ),
            (
                ReductionSiteKey::Norm {
                    norm_plan_id: NormPlanId::new(7),
                },
                "norm.7",
            ),
            (ReductionSiteKey::Classify, "classify"),
        ];

        for (key, expected) in cases {
            assert_eq!(reduction_site_id(key).0, expected);
        }
    }

    #[test]
    fn quant_graph_self_hash_uses_domain_hash() {
        let graph = self_consistent_graph_fixture();
        let value = serde_json::to_value(&graph).expect("quant graph serializes");
        let canonical = canonicalize_value(&value).expect("quant graph canonicalizes");
        let expected = domain_hash(
            "QuantGraph",
            QUANT_GRAPH_SCHEMA_ID,
            QUANT_GRAPH_SCHEMA_VERSION,
            &canonical,
        );

        assert_eq!(
            quant_graph_self_hash(&graph).expect("self hash computes"),
            expected
        );
    }

    #[test]
    fn quant_graph_product_report_embeds_real_quant_graph() {
        let product = quant_graph_product_fixture();
        let result = product
            .report
            .body
            .result
            .as_ref()
            .expect("passed report has result");

        assert_eq!(result.product, product.quant_graph);
        assert_eq!(result.quant_graph_self_hash, product.quant_graph_self_hash);
        assert_eq!(
            result.quant_graph_canonical_bytes_hash,
            product.quant_graph_canonical_bytes_hash
        );
    }

    #[test]
    fn norm_plan_binding_norm_site_unique_across_records() {
        let norm_plan_ids = bind_norm_plan_ids(1);
        let inputs = vec![
            norm_binding_input(NormSite::Final, QuantFormat::Q8_8, QuantFormat::Q8_8),
            norm_binding_input(NormSite::Final, QuantFormat::I8, QuantFormat::I8),
        ];

        let diagnostics =
            bind_norm_plans(&inputs, &norm_plan_ids).expect_err("duplicate site rejects");

        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.code == QuantGraphBindingDiagnosticCode::QuantGraphNormSiteDuplicate
        }));
    }

    #[test]
    fn norm_plan_binding_input_output_format_per_plan_table() {
        let norm_plan_ids = bind_norm_plan_ids(0);
        let inputs = [norm_binding_input(
            NormSite::Final,
            QuantFormat::Q8_8,
            QuantFormat::I16,
        )];

        let records = bind_norm_plans(&inputs, &norm_plan_ids).expect("final norm binds");

        assert_eq!(records[0].site, NormSite::Final);
        assert_eq!(records[0].input_format, QuantFormat::Q8_8);
        assert_eq!(records[0].output_format, QuantFormat::I16);
    }

    #[test]
    fn norm_plan_binding_unresolved_reference_diagnostic() {
        let norm_plan_ids = bind_norm_plan_ids(0);
        let inputs = [norm_binding_input(
            NormSite::LayerSequence {
                layer: LayerId::new(3),
            },
            QuantFormat::Q8_8,
            QuantFormat::Q8_8,
        )];

        let diagnostics =
            bind_norm_plans(&inputs, &norm_plan_ids).expect_err("unknown site rejects");

        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.code
                == QuantGraphBindingDiagnosticCode::QuantGraphNormPlanReferenceUnresolved
        }));
    }

    #[test]
    fn layer_norms_binding_covers_every_layer() {
        let norm_plans = vec![
            norm_record(
                0,
                NormSite::LayerSequence {
                    layer: LayerId::new(0),
                },
            ),
            norm_record(
                1,
                NormSite::LayerFfn {
                    layer: LayerId::new(0),
                },
            ),
            norm_record(2, NormSite::Final),
        ];

        let layer_norms = bind_layer_norms(1, &norm_plans).expect("layer norms bind");

        assert_eq!(
            layer_norms.get(&LayerId::new(0)),
            Some(&LayerNorms {
                pre_sequence: NormPlanId::new(0),
                pre_ffn: NormPlanId::new(1),
            })
        );
    }

    #[test]
    fn layer_norms_binding_excludes_final_norm() {
        let norm_plans = vec![
            norm_record(
                0,
                NormSite::LayerSequence {
                    layer: LayerId::new(0),
                },
            ),
            norm_record(
                1,
                NormSite::LayerFfn {
                    layer: LayerId::new(0),
                },
            ),
            norm_record(2, NormSite::Final),
        ];
        let layer_norms = bind_layer_norms(1, &norm_plans).expect("layer norms bind");

        assert_eq!(layer_norms.len(), 1);
        assert!(
            !serde_json::to_string(&layer_norms)
                .expect("layer norms JSON encodes")
                .contains("Final")
        );
    }

    #[test]
    fn layer_norms_binding_emits_incomplete_diagnostic() {
        let norm_plans = vec![norm_record(
            0,
            NormSite::LayerSequence {
                layer: LayerId::new(0),
            },
        )];

        let diagnostics =
            bind_layer_norms(1, &norm_plans).expect_err("incomplete layer norm rejects");

        assert!(diagnostics.iter().any(|diagnostic| matches!(
            diagnostic.code,
            QuantGraphBindingDiagnosticCode::QuantGraphLayerNormsIncomplete { .. }
        )));
    }

    #[test]
    fn router_semantics_only_top1_hard_in_v1() {
        let semantics = RouterSemantics::Top1Hard {
            gate_weight: RouterGateWeightSemantics::SelectedScore,
            tie_break: RouterTieBreak::LowestExpertId,
        };
        let value = serde_json::to_value(semantics).expect("router semantics serializes");
        let encoded = serde_json::to_string(&value).expect("router semantics JSON encodes");

        assert_eq!(value["kind"], Value::String("Top1Hard".to_owned()));
        assert!(encoded.contains("SelectedScore"));
        assert!(encoded.contains("LowestExpertId"));
        assert!(!encoded.contains("SoftTop1"));
        assert!(!encoded.contains("TopK"));
    }

    #[test]
    fn router_gate_weight_semantics_carries_one_or_selected_score() {
        let variants = [
            RouterGateWeightSemantics::One,
            RouterGateWeightSemantics::SelectedScore,
        ];
        let encoded = serde_json::to_string(&variants).expect("gate weight semantics serialize");

        assert!(encoded.contains("One"));
        assert!(encoded.contains("SelectedScore"));
        assert!(!encoded.contains("Discarded"));
        assert!(!encoded.contains("Implicit"));
    }

    #[test]
    fn router_tie_break_lowest_expert_id_only_in_v1() {
        let value = serde_json::to_value(RouterTieBreak::LowestExpertId)
            .expect("router tie break serializes");

        assert_eq!(value["kind"], Value::String("LowestExpertId".to_owned()));
        assert!(
            !serde_json::to_string(&value)
                .expect("router tie break JSON encodes")
                .contains("HighestScoreFirst")
        );
    }

    #[test]
    fn routing_table_and_router_layer_carry_typed_semantics() {
        let routing = RoutingTable {
            layers: vec![RouterLayer {
                layer: LayerId::new(1),
                n_experts: 4,
                router_weight: TensorId::new(10),
                router_bias: Some(TensorId::new(11)),
                semantics: RouterSemantics::Top1Hard {
                    gate_weight: RouterGateWeightSemantics::One,
                    tie_break: RouterTieBreak::LowestExpertId,
                },
            }],
        };
        let value = serde_json::to_value(routing).expect("routing table serializes");
        let layer = &value["layers"][0];

        assert_eq!(layer["layer"], Value::from(1_u64));
        assert_eq!(layer["n_experts"], Value::from(4_u64));
        assert_eq!(layer["router_weight"], Value::from(10_u64));
        assert_eq!(layer["router_bias"], Value::from(11_u64));
        assert_eq!(
            layer["semantics"]["kind"],
            Value::String("Top1Hard".to_owned())
        );
    }

    #[test]
    fn expert_section_struct_no_residency_field() {
        let section = ExpertSection {
            layer: LayerId::new(2),
            expert: ExpertId::new(0),
            tensor_refs: vec![TensorId::new(1), TensorId::new(2)],
        };
        let value = serde_json::to_value(section).expect("expert section serializes");
        let keys = object_keys(&value);

        assert_eq!(keys, vec!["expert", "layer", "tensor_refs"]);
        assert!(value.get("residency_hint").is_none());
        assert!(value.get("storage").is_none());
    }

    #[test]
    fn expert_weight_slot_three_variants_closed_and_ordered() {
        let encoded = serde_json::to_string(&EXPERT_WEIGHT_SLOT_CANONICAL_ORDER)
            .expect("expert slots serialize");

        assert_eq!(
            EXPERT_WEIGHT_SLOT_CANONICAL_ORDER,
            [
                ExpertWeightSlot::FfnGate,
                ExpertWeightSlot::FfnUp,
                ExpertWeightSlot::FfnDown,
            ]
        );
        assert!(encoded.contains("FfnGate"));
        assert!(encoded.contains("FfnUp"));
        assert!(encoded.contains("FfnDown"));
        assert!(!encoded.contains("SharedDense"));
    }

    #[test]
    fn ffn_activation_kind_four_variants_closed() {
        let variants = [
            FfnActivationKind::Relu,
            FfnActivationKind::Gelu,
            FfnActivationKind::SiLU,
            FfnActivationKind::SwiGLU,
        ];
        let encoded = serde_json::to_string(&variants).expect("activation kinds serialize");

        assert!(encoded.contains("Relu"));
        assert!(encoded.contains("Gelu"));
        assert!(encoded.contains("SiLU"));
        assert!(encoded.contains("SwiGLU"));
        assert!(!encoded.contains("LeakyRelu"));
    }

    #[test]
    fn ffn_plan_carries_activation_kind_and_intermediate_format() {
        let plan = FfnPlan {
            layer: LayerId::new(3),
            activation_kind: FfnActivationKind::SwiGLU,
            intermediate_format: QuantFormat::Q4_4,
        };
        let value = serde_json::to_value(plan).expect("ffn plan serializes");

        assert_eq!(value["layer"], Value::from(3_u64));
        assert_eq!(
            value["activation_kind"]["kind"],
            Value::String("SwiGLU".to_owned())
        );
        assert_eq!(
            value["intermediate_format"]["kind"],
            Value::String("Q4_4".to_owned())
        );
    }

    #[test]
    fn ffn_topology_kind_tag_three_variants_only() {
        let variants = [
            FfnTopologyKindTag::Dense,
            FfnTopologyKindTag::Routed,
            FfnTopologyKindTag::Mixed,
        ];
        let encoded = serde_json::to_string(&variants).expect("topology tags serialize");

        assert!(encoded.contains("Dense"));
        assert!(encoded.contains("Routed"));
        assert!(encoded.contains("Mixed"));
        assert!(!encoded.contains("SharedDense"));
        assert!(!encoded.contains("RoutedWithSharedDense"));
    }

    #[test]
    fn compute_ffn_topology_kind_predicate_table() {
        assert_eq!(
            compute_ffn_topology_kind(&model_summary_with_ffn_kinds([
                (0, FfnKindTag::Dense),
                (1, FfnKindTag::Dense),
            ])),
            FfnTopologyKindTag::Dense
        );
        assert_eq!(
            compute_ffn_topology_kind(&model_summary_with_ffn_kinds([
                (0, FfnKindTag::Routed),
                (1, FfnKindTag::Routed),
            ])),
            FfnTopologyKindTag::Routed
        );
        assert_eq!(
            compute_ffn_topology_kind(&model_summary_with_ffn_kinds([
                (0, FfnKindTag::Dense),
                (1, FfnKindTag::Routed),
            ])),
            FfnTopologyKindTag::Mixed
        );
    }

    #[test]
    fn routing_binding_dense_layer_no_routing_table_entry() {
        let model = model_summary_with_ffn_kinds([(0, FfnKindTag::Dense)]);

        let routing = bind_routing_table(&model, Vec::new()).expect("dense layer has no routing");

        assert!(routing.is_none());
    }

    #[test]
    fn routing_binding_routed_layer_n_experts_match() {
        let model = model_summary_with_experts([(0, FfnKindTag::Routed, 2)]);

        let routing = bind_routing_table(&model, vec![router_layer(0, 2)])
            .expect("routed layer binds with matching expert count")
            .expect("routed layer yields routing table");

        assert_eq!(routing.layers.len(), 1);
        assert_eq!(routing.layers[0].layer, LayerId::new(0));
        assert_eq!(routing.layers[0].n_experts, 2);
    }

    #[test]
    fn routing_binding_routing_present_for_dense_diagnostic() {
        let model = model_summary_with_ffn_kinds([(0, FfnKindTag::Dense)]);

        let diagnostics =
            bind_routing_table(&model, vec![router_layer(0, 1)]).expect_err("dense router rejects");

        assert!(diagnostics.iter().any(|diagnostic| matches!(
            diagnostic.code,
            QuantGraphBindingDiagnosticCode::QuantGraphRoutingPresentForDenseLayer { .. }
        )));
    }

    #[test]
    fn routing_binding_routing_missing_for_routed_diagnostic() {
        let model = model_summary_with_experts([(0, FfnKindTag::Routed, 2)]);

        let diagnostics =
            bind_routing_table(&model, Vec::new()).expect_err("missing routed layer rejects");

        assert!(diagnostics.iter().any(|diagnostic| matches!(
            diagnostic.code,
            QuantGraphBindingDiagnosticCode::QuantGraphRoutingMissingForRoutedLayer { .. }
        )));
    }

    #[test]
    fn routing_binding_expert_coverage_gap_diagnostic() {
        let model = model_summary_with_experts([(0, FfnKindTag::Routed, 2)]);

        let diagnostics = bind_routing_table(&model, vec![router_layer(0, 1)])
            .expect_err("expert count mismatch rejects");

        assert!(diagnostics.iter().any(|diagnostic| matches!(
            diagnostic.code,
            QuantGraphBindingDiagnosticCode::QuantGraphRoutingExpertCoverageMismatch { .. }
        )));
    }

    #[test]
    fn expert_binding_routed_layer_full_expert_coverage() {
        let model = model_summary_with_experts([(0, FfnKindTag::Routed, 2)]);
        let ffn_plans = ffn_plans([(0, FfnActivationKind::Relu)]);
        let tensors = vec![
            expert_weight_tensor(10, 0, 0, ExpertWeightSlot::FfnUp),
            expert_weight_tensor(11, 0, 0, ExpertWeightSlot::FfnDown),
            expert_weight_tensor(12, 0, 1, ExpertWeightSlot::FfnUp),
            expert_weight_tensor(13, 0, 1, ExpertWeightSlot::FfnDown),
        ];

        let sections =
            bind_expert_sections(&model, &ffn_plans, &tensors).expect("all experts bind");

        assert_eq!(
            sections
                .iter()
                .map(|section| (section.layer, section.expert))
                .collect::<Vec<_>>(),
            vec![
                (LayerId::new(0), ExpertId::new(0)),
                (LayerId::new(0), ExpertId::new(1)),
            ]
        );
    }

    #[test]
    fn expert_binding_ffn_gate_iff_swiglu_per_layer() {
        let model = model_summary_with_experts([(0, FfnKindTag::Routed, 1)]);
        let ffn_plans = ffn_plans([(0, FfnActivationKind::SwiGLU)]);
        let tensors = vec![
            expert_weight_tensor(10, 0, 0, ExpertWeightSlot::FfnGate),
            expert_weight_tensor(11, 0, 0, ExpertWeightSlot::FfnUp),
            expert_weight_tensor(12, 0, 0, ExpertWeightSlot::FfnDown),
        ];

        let sections =
            bind_expert_sections(&model, &ffn_plans, &tensors).expect("swiglu gate binds");

        assert_eq!(sections.len(), 1);
        assert_eq!(
            sections[0].tensor_refs,
            vec![TensorId::new(10), TensorId::new(11), TensorId::new(12)]
        );
    }

    #[test]
    fn expert_binding_tensor_refs_canonical_order() {
        let model = model_summary_with_experts([(0, FfnKindTag::Routed, 1)]);
        let ffn_plans = ffn_plans([(0, FfnActivationKind::SwiGLU)]);
        let tensors = vec![
            expert_bias_tensor(16, 0, 0, ExpertWeightSlot::FfnDown),
            expert_weight_tensor(14, 0, 0, ExpertWeightSlot::FfnUp),
            expert_bias_tensor(13, 0, 0, ExpertWeightSlot::FfnGate),
            expert_weight_tensor(15, 0, 0, ExpertWeightSlot::FfnDown),
            expert_weight_tensor(12, 0, 0, ExpertWeightSlot::FfnGate),
            expert_bias_tensor(17, 0, 0, ExpertWeightSlot::FfnUp),
        ];

        let sections =
            bind_expert_sections(&model, &ffn_plans, &tensors).expect("expert section binds");

        assert_eq!(
            sections[0].tensor_refs,
            vec![
                TensorId::new(12),
                TensorId::new(13),
                TensorId::new(14),
                TensorId::new(17),
                TensorId::new(15),
                TensorId::new(16),
            ]
        );
    }

    #[test]
    fn expert_binding_weight_missing_diagnostic() {
        let model = model_summary_with_ffn_kinds([(0, FfnKindTag::Dense)]);
        let ffn_plans = ffn_plans([(0, FfnActivationKind::Relu)]);
        let tensors = vec![expert_weight_tensor(10, 0, 0, ExpertWeightSlot::FfnUp)];

        let diagnostics =
            bind_expert_sections(&model, &ffn_plans, &tensors).expect_err("missing down rejects");

        assert!(diagnostics.iter().any(|diagnostic| matches!(
            diagnostic.code,
            QuantGraphBindingDiagnosticCode::QuantGraphExpertSectionWeightMissing { .. }
        )));
    }

    #[test]
    fn expert_binding_ffn_gate_mismatch_diagnostic() {
        let model = model_summary_with_ffn_kinds([(0, FfnKindTag::Dense)]);
        let ffn_plans = ffn_plans([(0, FfnActivationKind::Relu)]);
        let tensors = vec![
            expert_weight_tensor(10, 0, 0, ExpertWeightSlot::FfnGate),
            expert_weight_tensor(11, 0, 0, ExpertWeightSlot::FfnUp),
            expert_weight_tensor(12, 0, 0, ExpertWeightSlot::FfnDown),
        ];

        let diagnostics =
            bind_expert_sections(&model, &ffn_plans, &tensors).expect_err("relu gate rejects");

        assert!(diagnostics.iter().any(|diagnostic| matches!(
            diagnostic.code,
            QuantGraphBindingDiagnosticCode::QuantGraphFfnGatePresenceMismatch { .. }
        )));
    }

    #[test]
    fn routing_binding_expert_coverage_extra_diagnostic() {
        let model = model_summary_with_ffn_kinds([(0, FfnKindTag::Dense)]);
        let ffn_plans = ffn_plans([(0, FfnActivationKind::Relu)]);
        let tensors = vec![
            expert_weight_tensor(10, 0, 0, ExpertWeightSlot::FfnUp),
            expert_weight_tensor(11, 0, 0, ExpertWeightSlot::FfnDown),
            expert_weight_tensor(12, 0, 1, ExpertWeightSlot::FfnUp),
            expert_weight_tensor(13, 0, 1, ExpertWeightSlot::FfnDown),
        ];

        let diagnostics = bind_expert_sections(&model, &ffn_plans, &tensors)
            .expect_err("extra dense expert rejects");

        assert!(diagnostics.iter().any(|diagnostic| matches!(
            diagnostic.code,
            QuantGraphBindingDiagnosticCode::QuantGraphRoutingExpertCoverageExtra { .. }
        )));
    }

    #[test]
    fn residual_plan_binding_combine_policy_add_then_clamp() {
        let residual_plan = bind_residual_plan(ResidualPlanInput {
            activation_format: QuantFormat::Q8_8,
            combine_policy: ResidualCombinePolicy::AddThenClampNamedBoundary,
        })
        .expect("named residual policy binds");

        assert_eq!(
            residual_plan.combine_policy,
            ResidualCombinePolicy::AddThenClampNamedBoundary
        );
    }

    #[test]
    fn residual_plan_binding_activation_format_in_set() {
        for activation_format in [
            QuantFormat::I8,
            QuantFormat::I16,
            QuantFormat::Q8_8,
            QuantFormat::Q4_4,
        ] {
            assert!(
                bind_residual_plan(ResidualPlanInput {
                    activation_format,
                    combine_policy: ResidualCombinePolicy::AddThenClampNamedBoundary,
                })
                .is_ok(),
                "activation boundary is accepted"
            );
        }
    }

    #[test]
    fn residual_plan_binding_invalid_diagnostic() {
        let diagnostics = bind_residual_plan(ResidualPlanInput {
            activation_format: ternary_format(),
            combine_policy: ResidualCombinePolicy::AddThenClampNamedBoundary,
        })
        .expect_err("non-activation residual format rejects");

        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.code == QuantGraphBindingDiagnosticCode::QuantGraphResidualPlanInvalid
        }));
    }

    #[test]
    fn classify_head_kind_tied_or_untied_only() {
        let variants = [ClassifyHeadKind::Tied, ClassifyHeadKind::Untied];
        let encoded = serde_json::to_string(&variants).expect("classify kinds serialize");

        assert!(encoded.contains("Tied"));
        assert!(encoded.contains("Untied"));
        assert!(!encoded.contains("Inferred"));
        assert!(!encoded.contains("Default"));
    }

    #[test]
    fn classify_head_logit_format_only_in_activation_set() {
        for format in [
            QuantFormat::I8,
            QuantFormat::I16,
            QuantFormat::Q8_8,
            QuantFormat::Q4_4,
        ] {
            assert!(
                classify_logit_format_is_activation_set(&format),
                "{format:?} is an admissible classify logit boundary"
            );
        }

        assert!(!classify_logit_format_is_activation_set(
            &QuantFormat::Ternary2 {
                scale_granularity: ScaleGranularity::PerOutputRow,
                scale_format: AuxFormat::Q8_8,
                threshold_granularity: ThresholdGranularity::PerOutputRow,
            }
        ));
    }

    #[test]
    fn classify_head_bias_independent_of_kind() {
        let tied_with_bias = ClassifyHead {
            kind: ClassifyHeadKind::Tied,
            weight: TensorId::new(1),
            bias: Some(TensorId::new(2)),
            logit_format: QuantFormat::Q8_8,
        };
        let untied_without_bias = ClassifyHead {
            kind: ClassifyHeadKind::Untied,
            weight: TensorId::new(3),
            bias: None,
            logit_format: QuantFormat::I8,
        };

        let tied = serde_json::to_value(tied_with_bias).expect("tied head serializes");
        let untied = serde_json::to_value(untied_without_bias).expect("untied head serializes");

        assert_eq!(tied["kind"]["kind"], Value::String("Tied".to_owned()));
        assert_eq!(tied["bias"], Value::from(2_u64));
        assert_eq!(untied["kind"]["kind"], Value::String("Untied".to_owned()));
        assert!(untied["bias"].is_null());
    }

    #[test]
    fn classify_head_logit_format_not_coupled_to_tied_weight_format() {
        let head = ClassifyHead {
            kind: ClassifyHeadKind::Tied,
            weight: TensorId::new(1),
            bias: None,
            logit_format: QuantFormat::I16,
        };
        let value = serde_json::to_value(head).expect("classify head serializes");

        assert_eq!(
            value["logit_format"]["kind"],
            Value::String("I16".to_owned())
        );
        assert_eq!(value["weight"], Value::from(1_u64));
    }

    #[test]
    fn decode_spec_record_requires_rng_matches_spec() {
        let argmax = DecodeSpecRecord {
            decode_plan_id: DecodePlanId::new(1),
            spec: DecodeSpec::Argmax,
            requires_rng: false,
        };
        let sampled = DecodeSpecRecord {
            decode_plan_id: DecodePlanId::new(2),
            spec: DecodeSpec::TopKTemperature {
                k: 8,
                temperature_q8_8: 256,
            },
            requires_rng: true,
        };

        assert!(argmax.requires_rng_matches_spec());
        assert!(sampled.requires_rng_matches_spec());
        assert!(
            !DecodeSpecRecord {
                requires_rng: false,
                ..sampled
            }
            .requires_rng_matches_spec()
        );
    }

    #[test]
    fn decode_spec_record_in_capability_set_check() {
        let argmax_caps = DecodeCapabilitySet {
            supported: BTreeSet::from([DecodeMode::Argmax]),
        };
        let sampled = DecodeSpec::TopKTemperature {
            k: 4,
            temperature_q8_8: 384,
        };

        assert!(check_decode_spec_in_capabilities(
            &DecodeSpec::Argmax,
            &argmax_caps
        ));
        assert!(!check_decode_spec_in_capabilities(&sampled, &argmax_caps));
    }

    #[test]
    fn decode_spec_record_carries_explicit_plan_id_and_no_silent_default() {
        let record = DecodeSpecRecord {
            decode_plan_id: DecodePlanId::new(9),
            spec: DecodeSpec::Argmax,
            requires_rng: false,
        };
        let value = serde_json::to_value(record).expect("decode spec record serializes");
        let keys = object_keys(&value);

        assert_eq!(keys, vec!["decode_plan_id", "requires_rng", "spec"]);
        assert_eq!(value["decode_plan_id"], Value::from(9_u64));
        assert_eq!(value["spec"]["kind"], Value::String("Argmax".to_owned()));
    }

    #[test]
    fn decode_binding_no_silent_default() {
        let diagnostics = bind_decode_spec(
            DecodePlanId::new(1),
            DecodeBindingSource::UnboundDefault,
            &decode_caps([DecodeMode::Argmax]),
        )
        .expect_err("unbound default rejects");

        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.code == QuantGraphBindingDiagnosticCode::QuantGraphDecodeSpecUnboundDefault
        }));
    }

    #[test]
    fn decode_binding_policy_narrows_capabilities() {
        let record = bind_decode_spec(
            DecodePlanId::new(1),
            DecodeBindingSource::Explicit {
                spec: DecodeSpec::Argmax,
            },
            &decode_caps([DecodeMode::Argmax]),
        )
        .expect("explicit policy in capabilities binds");

        assert_eq!(record.spec, DecodeSpec::Argmax);
        assert!(!record.requires_rng);
    }

    #[test]
    fn decode_binding_default_must_be_hash_bound() {
        let source = DecodeBindingSource::ArtifactDefault {
            spec: DecodeSpec::Argmax,
            hash_bound: false,
        };

        let diagnostics = bind_decode_spec(
            DecodePlanId::new(1),
            source,
            &decode_caps([DecodeMode::Argmax]),
        )
        .expect_err("unbound artifact default rejects");

        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.code == QuantGraphBindingDiagnosticCode::QuantGraphDecodeSpecUnboundDefault
        }));

        assert!(
            bind_decode_spec(
                DecodePlanId::new(1),
                DecodeBindingSource::ArtifactDefault {
                    spec: DecodeSpec::Argmax,
                    hash_bound: true,
                },
                &decode_caps([DecodeMode::Argmax]),
            )
            .is_ok()
        );
    }

    #[test]
    fn decode_binding_spec_not_in_capability_set_diagnostic() {
        let diagnostics = bind_decode_spec(
            DecodePlanId::new(1),
            DecodeBindingSource::Explicit {
                spec: DecodeSpec::TopKTemperature {
                    k: 8,
                    temperature_q8_8: 256,
                },
            },
            &decode_caps([DecodeMode::Argmax]),
        )
        .expect_err("unsupported decode mode rejects");

        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.code
                == QuantGraphBindingDiagnosticCode::QuantGraphDecodeSpecNotInCapabilitySet
        }));
    }

    #[test]
    fn classify_head_binding_tied_weight_equals_embedding() {
        let head = ClassifyHead {
            kind: ClassifyHeadKind::Tied,
            weight: TensorId::new(1),
            bias: None,
            logit_format: QuantFormat::Q8_8,
        };

        let bound = bind_classify_head(head, TensorId::new(1)).expect("tied head binds");

        assert_eq!(bound.weight, TensorId::new(1));
    }

    #[test]
    fn classify_head_binding_logit_format_independent_of_tied() {
        let head = ClassifyHead {
            kind: ClassifyHeadKind::Tied,
            weight: TensorId::new(1),
            bias: None,
            logit_format: QuantFormat::I16,
        };

        let bound = bind_classify_head(head, TensorId::new(1))
            .expect("tied head can use distinct logit boundary");

        assert_eq!(bound.logit_format, QuantFormat::I16);
    }

    #[test]
    fn classify_head_binding_bias_independent_of_kind() {
        let tied = ClassifyHead {
            kind: ClassifyHeadKind::Tied,
            weight: TensorId::new(1),
            bias: Some(TensorId::new(2)),
            logit_format: QuantFormat::Q8_8,
        };
        let untied = ClassifyHead {
            kind: ClassifyHeadKind::Untied,
            weight: TensorId::new(3),
            bias: None,
            logit_format: QuantFormat::Q8_8,
        };

        assert!(bind_classify_head(tied, TensorId::new(1)).is_ok());
        assert!(bind_classify_head(untied, TensorId::new(1)).is_ok());
    }

    #[test]
    fn classify_head_binding_tied_mismatch_diagnostic() {
        let head = ClassifyHead {
            kind: ClassifyHeadKind::Tied,
            weight: TensorId::new(2),
            bias: None,
            logit_format: QuantFormat::Q8_8,
        };

        let diagnostics =
            bind_classify_head(head, TensorId::new(1)).expect_err("wrong tied weight rejects");

        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.code == QuantGraphBindingDiagnosticCode::QuantGraphClassifyHeadTiedMismatch
        }));
    }

    #[test]
    fn classify_head_binding_format_mismatch_diagnostic() {
        let head = ClassifyHead {
            kind: ClassifyHeadKind::Untied,
            weight: TensorId::new(2),
            bias: None,
            logit_format: ternary_format(),
        };

        let diagnostics =
            bind_classify_head(head, TensorId::new(1)).expect_err("bad logit format rejects");

        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.code == QuantGraphBindingDiagnosticCode::QuantGraphClassifyHeadFormatMismatch
        }));
    }

    #[test]
    fn provenance_binding_total_image_typed_diagnostic() {
        let graph = self_consistent_graph_fixture();
        let mut exports = graph.provenance.clone();
        exports.remove(&TensorId::new(2));

        let diagnostics =
            bind_provenance(&graph.tensors, &exports).expect_err("missing tensor export rejects");

        assert!(has_diagnostic(&diagnostics, |code| matches!(
            code,
            QuantGraphBindingDiagnosticCode::QuantGraphExportProvenanceMissing {
                tensor_id
            } if *tensor_id == TensorId::new(2)
        )));
    }

    #[test]
    fn provenance_binding_injective_across_main_and_aux() {
        let mut graph = self_consistent_graph_fixture();
        graph.tensors[1].aux_blob_refs[0].export_tensor_id = export_tensor_id("tensor.embedding");

        let diagnostics = bind_provenance(&graph.tensors, &graph.provenance)
            .expect_err("duplicate main/aux provenance rejects");

        assert!(has_diagnostic(&diagnostics, |code| matches!(
            code,
            QuantGraphBindingDiagnosticCode::QuantGraphProvenanceImageNotInjective { .. }
        )));
    }

    #[test]
    fn canonical_sort_two_regenerations_byte_identical() {
        let mut first = self_consistent_graph_fixture();
        first.tensors.reverse();
        first.expert_sections.reverse();
        first.norm_plans.reverse();
        let mut second = self_consistent_graph_fixture();

        canonical_sort_quant_graph(&mut first);
        canonical_sort_quant_graph(&mut second);
        let first_bytes =
            canonicalize_value(&serde_json::to_value(&first).expect("first graph serializes"))
                .expect("first graph canonicalizes");
        let second_bytes =
            canonicalize_value(&serde_json::to_value(&second).expect("second graph serializes"))
                .expect("second graph canonicalizes");

        assert_eq!(first_bytes, second_bytes);
    }

    #[test]
    fn canonical_sort_expert_sections_by_layer_then_expert() {
        let mut graph = self_consistent_graph_fixture();
        graph.expert_sections = vec![
            ExpertSection {
                layer: LayerId::new(1),
                expert: ExpertId::new(0),
                tensor_refs: Vec::new(),
            },
            ExpertSection {
                layer: LayerId::new(0),
                expert: ExpertId::new(1),
                tensor_refs: Vec::new(),
            },
            ExpertSection {
                layer: LayerId::new(0),
                expert: ExpertId::new(0),
                tensor_refs: Vec::new(),
            },
        ];

        canonical_sort_quant_graph(&mut graph);

        assert_eq!(
            graph
                .expert_sections
                .iter()
                .map(|section| (section.layer, section.expert))
                .collect::<Vec<_>>(),
            vec![
                (LayerId::new(0), ExpertId::new(0)),
                (LayerId::new(0), ExpertId::new(1)),
                (LayerId::new(1), ExpertId::new(0)),
            ]
        );
    }

    #[test]
    fn canonical_sort_provenance_by_tensor_id() {
        let mut provenance = TensorProvenanceMap::new();
        provenance.insert(TensorId::new(3), export_tensor_id("tensor.three"));
        provenance.insert(TensorId::new(1), export_tensor_id("tensor.one"));

        assert_eq!(
            provenance.keys().copied().collect::<Vec<_>>(),
            vec![TensorId::new(1), TensorId::new(3)]
        );
    }

    #[test]
    fn canonical_sort_norm_plans_by_site_then_id() {
        let mut graph = self_consistent_graph_fixture();
        graph.norm_plans.reverse();

        canonical_sort_quant_graph(&mut graph);

        assert_eq!(
            graph
                .norm_plans
                .iter()
                .map(|record| record.site)
                .collect::<Vec<_>>(),
            vec![
                NormSite::LayerSequence {
                    layer: LayerId::new(0)
                },
                NormSite::LayerFfn {
                    layer: LayerId::new(0)
                },
                NormSite::Final,
            ]
        );
    }

    #[test]
    fn canonical_sort_idempotent_under_repeated_application() {
        let mut graph = self_consistent_graph_fixture();
        graph.tensors.reverse();

        canonical_sort_quant_graph(&mut graph);
        let once = graph.clone();
        canonical_sort_quant_graph(&mut graph);

        assert_eq!(graph, once);
    }

    #[test]
    fn router_semantics_v1_accepts_top1_hard_one_lowest_expert_id() {
        let semantics = bind_router_semantics_v1(RouterSemanticsBindingInput::Top1Hard {
            gate_weight: RouterGateWeightSemanticsBindingInput::One,
            tie_break: RouterTieBreakBindingInput::LowestExpertId,
        })
        .expect("one-weight top1 hard binds");

        assert_eq!(
            semantics,
            RouterSemantics::Top1Hard {
                gate_weight: RouterGateWeightSemantics::One,
                tie_break: RouterTieBreak::LowestExpertId,
            }
        );
    }

    #[test]
    fn router_semantics_v1_accepts_top1_hard_selected_score() {
        let semantics = bind_router_semantics_v1(RouterSemanticsBindingInput::Top1Hard {
            gate_weight: RouterGateWeightSemanticsBindingInput::SelectedScore,
            tie_break: RouterTieBreakBindingInput::LowestExpertId,
        })
        .expect("selected-score top1 hard binds");

        assert_eq!(
            semantics,
            RouterSemantics::Top1Hard {
                gate_weight: RouterGateWeightSemantics::SelectedScore,
                tie_break: RouterTieBreak::LowestExpertId,
            }
        );
    }

    #[test]
    fn router_semantics_v1_rejects_non_top1_hard() {
        let diagnostics = bind_router_semantics_v1(RouterSemanticsBindingInput::UnsupportedV1 {
            tag: "SoftTop1".to_owned(),
        })
        .expect_err("non-top1 hard rejects");

        assert!(has_diagnostic(&diagnostics, |code| {
            code == &QuantGraphBindingDiagnosticCode::QuantGraphRouterGateWeightSemanticsUnsupported
        }));
    }

    #[test]
    fn router_semantics_v1_rejects_non_lowest_expert_tie_break() {
        let diagnostics = bind_router_semantics_v1(RouterSemanticsBindingInput::Top1Hard {
            gate_weight: RouterGateWeightSemanticsBindingInput::One,
            tie_break: RouterTieBreakBindingInput::UnsupportedV1,
        })
        .expect_err("unsupported tie break rejects");

        assert!(has_diagnostic(&diagnostics, |code| {
            code == &QuantGraphBindingDiagnosticCode::QuantGraphRouterTieBreakUnsupported
        }));
    }

    #[test]
    fn router_semantics_v1_rejects_unsupported_gate_weight() {
        let diagnostics = bind_router_semantics_v1(RouterSemanticsBindingInput::Top1Hard {
            gate_weight: RouterGateWeightSemanticsBindingInput::UnsupportedV1,
            tie_break: RouterTieBreakBindingInput::LowestExpertId,
        })
        .expect_err("unsupported gate weight rejects");

        assert!(has_diagnostic(&diagnostics, |code| {
            code == &QuantGraphBindingDiagnosticCode::QuantGraphRouterGateWeightSemanticsUnsupported
        }));
    }

    #[test]
    fn self_consistency_valid_fixture_has_no_diagnostics() {
        let graph = self_consistent_graph_fixture();

        assert!(self_consistency_diagnostics(&graph).is_empty());
    }

    #[test]
    fn self_consistency_sc1_tensor_id_not_unique() {
        let mut graph = self_consistent_graph_fixture();
        graph.tensors[1].tensor_id = graph.tensors[0].tensor_id;

        let diagnostics = self_consistency_diagnostics(&graph);

        assert!(has_diagnostic(&diagnostics, |code| matches!(
            code,
            QuantGraphBindingDiagnosticCode::QuantGraphTensorIdNotUnique { .. }
        )));
    }

    #[test]
    fn self_consistency_sc2_provenance_image_not_injective() {
        let mut graph = self_consistent_graph_fixture();
        graph
            .provenance
            .insert(TensorId::new(2), export_tensor_id("tensor.embedding"));

        let diagnostics = self_consistency_diagnostics(&graph);

        assert!(has_diagnostic(&diagnostics, |code| matches!(
            code,
            QuantGraphBindingDiagnosticCode::QuantGraphProvenanceImageNotInjective { .. }
        )));
    }

    #[test]
    fn self_consistency_sc3_norm_plan_id_unresolved() {
        let mut graph = self_consistent_graph_fixture();
        graph.layer_norms.get_mut(&LayerId::new(0)).unwrap().pre_ffn = NormPlanId::new(99);

        let diagnostics = self_consistency_diagnostics(&graph);

        assert!(has_diagnostic(&diagnostics, |code| {
            code == &QuantGraphBindingDiagnosticCode::QuantGraphNormPlanReferenceUnresolved
        }));
    }

    #[test]
    fn self_consistency_sc4_routing_topology_gap_and_extra() {
        let mut graph = self_consistent_graph_fixture();
        graph
            .identity
            .model_spec_summary
            .ffn_kind
            .insert(LayerId::new(0), FfnKindTag::Routed);
        graph
            .identity
            .model_spec_summary
            .n_experts
            .insert(LayerId::new(0), 2);
        graph.routing_table = Some(RoutingTable {
            layers: vec![router_layer(0, 2)],
        });
        graph.expert_sections.push(ExpertSection {
            layer: LayerId::new(0),
            expert: ExpertId::new(3),
            tensor_refs: Vec::new(),
        });

        let diagnostics = self_consistency_diagnostics(&graph);

        assert!(has_diagnostic(&diagnostics, |code| matches!(
            code,
            QuantGraphBindingDiagnosticCode::QuantGraphRoutingExpertCoverageGap { .. }
        )));
        assert!(has_diagnostic(&diagnostics, |code| matches!(
            code,
            QuantGraphBindingDiagnosticCode::QuantGraphRoutingExpertCoverageExtra { .. }
        )));
    }

    #[test]
    fn self_consistency_sc5_expert_section_weight_missing() {
        let mut graph = self_consistent_graph_fixture();
        graph.expert_sections[0].tensor_refs = vec![TensorId::new(99)];

        let diagnostics = self_consistency_diagnostics(&graph);

        assert!(has_diagnostic(&diagnostics, |code| matches!(
            code,
            QuantGraphBindingDiagnosticCode::QuantGraphExpertSectionWeightMissing { .. }
        )));
    }

    #[test]
    fn self_consistency_sc6_expert_section_tensor_refs_order() {
        let mut graph = self_consistent_graph_fixture();
        graph.expert_sections[0].tensor_refs = vec![TensorId::new(3), TensorId::new(2)];

        let diagnostics = self_consistency_diagnostics(&graph);

        assert!(has_diagnostic(&diagnostics, |code| matches!(
            code,
            QuantGraphBindingDiagnosticCode::QuantGraphExpertSectionWeightMissing { .. }
        )));
    }

    #[test]
    fn self_consistency_sc7_classify_head_weight_role_mismatch() {
        let mut graph = self_consistent_graph_fixture();
        graph.classify_head.weight = TensorId::new(2);

        let diagnostics = self_consistency_diagnostics(&graph);

        assert!(has_diagnostic(&diagnostics, |code| {
            code == &QuantGraphBindingDiagnosticCode::QuantGraphClassifyHeadTiedMismatch
        }));
    }

    #[test]
    fn self_consistency_sc8_decode_spec_not_in_caps() {
        let graph = self_consistent_graph_fixture();
        let decode_caps = decode_caps([DecodeMode::TopKTemperature]);

        let diagnostics =
            validate_quant_graph_self_consistency(&graph, self_consistency_context(&decode_caps));

        assert!(has_diagnostic(&diagnostics, |code| {
            code == &QuantGraphBindingDiagnosticCode::QuantGraphDecodeSpecNotInCapabilitySet
        }));
    }

    #[test]
    fn self_consistency_sc9_layout_inconsistent_with_model_spec() {
        let mut graph = self_consistent_graph_fixture();
        graph.tensors[0].layout = layout(&[1, 8]);

        let diagnostics = self_consistency_diagnostics(&graph);

        assert!(has_diagnostic(&diagnostics, |code| {
            code == &QuantGraphBindingDiagnosticCode::QuantGraphLayoutInconsistentWithModelSpec
        }));
    }

    #[test]
    fn self_consistency_sc11_sequence_semantics_tensor_mismatch() {
        let mut graph = self_consistent_graph_fixture();
        graph.sequence_semantics = SequenceSemanticsSpec {
            kind: SequenceSemanticsKind::LinearState,
            state_slots: BTreeMap::new(),
        };

        let diagnostics = self_consistency_diagnostics(&graph);

        assert!(has_diagnostic(&diagnostics, |code| {
            code == &QuantGraphBindingDiagnosticCode::QuantGraphSequenceSemanticsTensorMismatch
        }));
    }

    #[test]
    fn self_consistency_sc12_required_features_unsupported() {
        let graph = self_consistent_graph_fixture();
        let decode_caps = decode_caps([DecodeMode::Argmax]);
        let mut context = self_consistency_context(&decode_caps);
        context.required_features_supported = false;

        let diagnostics = validate_quant_graph_self_consistency(&graph, context);

        assert!(has_diagnostic(&diagnostics, |code| {
            code == &QuantGraphBindingDiagnosticCode::QuantGraphRequiredFeatureUnsupported
        }));
    }

    #[test]
    fn self_consistency_rejects_training_residue() {
        let graph = self_consistent_graph_fixture();
        let decode_caps = decode_caps([DecodeMode::Argmax]);
        let mut context = self_consistency_context(&decode_caps);
        context.training_residue_absent = false;

        let diagnostics = validate_quant_graph_self_consistency(&graph, context);

        assert!(has_diagnostic(&diagnostics, |code| {
            code == &QuantGraphBindingDiagnosticCode::QuantGraphTrainingResidue
        }));
    }

    #[test]
    fn self_consistency_rejects_unenforced_reduction_order_for_bit_exact() {
        let graph = self_consistent_graph_fixture();
        let decode_caps = decode_caps([DecodeMode::Argmax]);
        let mut context = self_consistency_context(&decode_caps);
        context.reduction_order_policy_enforced = false;

        let diagnostics = validate_quant_graph_self_consistency(&graph, context);

        assert!(has_diagnostic(&diagnostics, |code| {
            code == &QuantGraphBindingDiagnosticCode::QuantGraphDeterminismRequiresEnforcedReductionOrder
        }));
    }

    #[test]
    fn self_consistency_rejects_forbidden_storage_metadata() {
        let graph = self_consistent_graph_fixture();
        let decode_caps = decode_caps([DecodeMode::Argmax]);
        let mut context = self_consistency_context(&decode_caps);
        context.forbidden_storage_metadata_absent = false;

        let diagnostics = validate_quant_graph_self_consistency(&graph, context);

        assert!(has_diagnostic(&diagnostics, |code| {
            code == &QuantGraphBindingDiagnosticCode::QuantGraphForbiddenStorageMetadata
        }));
    }

    #[test]
    fn self_consistency_sc13_blob_decoded_size() {
        let mut graph = self_consistent_graph_fixture();
        graph.tensors[0].blob.decoded_size_bytes = 1;

        let diagnostics = self_consistency_diagnostics(&graph);

        assert!(has_diagnostic(&diagnostics, |code| matches!(
            code,
            QuantGraphBindingDiagnosticCode::QuantGraphBlobRefSizeMismatch { .. }
        )));
    }

    #[test]
    fn self_consistency_sc14_embedding_unique() {
        let mut graph = self_consistent_graph_fixture();
        graph.tensors.push(tensor_with_layout_role(
            4,
            QuantTensorRole::EmbeddingTable,
            QuantFormat::I8,
            &[80, 8],
            640,
            Vec::new(),
        ));
        graph.provenance.insert(
            TensorId::new(4),
            export_tensor_id("tensor.embedding.duplicate"),
        );

        let diagnostics = self_consistency_diagnostics(&graph);

        assert!(has_diagnostic(&diagnostics, |code| {
            code == &QuantGraphBindingDiagnosticCode::QuantGraphEmbeddingNotUnique
        }));
    }

    #[test]
    fn self_consistency_sc17_ffn_gate_iff_swiglu() {
        let mut graph = self_consistent_graph_fixture();
        graph
            .ffn_plans
            .get_mut(&LayerId::new(0))
            .unwrap()
            .activation_kind = FfnActivationKind::SwiGLU;

        let diagnostics = self_consistency_diagnostics(&graph);

        assert!(has_diagnostic(&diagnostics, |code| matches!(
            code,
            QuantGraphBindingDiagnosticCode::QuantGraphFfnGatePresenceMismatch { .. }
        )));
    }

    #[test]
    fn self_consistency_sc18_layer_norms_complete() {
        let mut graph = self_consistent_graph_fixture();
        graph.layer_norms.clear();

        let diagnostics = self_consistency_diagnostics(&graph);

        assert!(has_diagnostic(&diagnostics, |code| matches!(
            code,
            QuantGraphBindingDiagnosticCode::QuantGraphLayerNormsIncomplete { .. }
        )));
    }

    #[test]
    fn self_consistency_sc20_norm_site_uniqueness() {
        let mut graph = self_consistent_graph_fixture();
        graph.norm_plans.pop();

        let diagnostics = self_consistency_diagnostics(&graph);

        assert!(has_diagnostic(&diagnostics, |code| {
            code == &QuantGraphBindingDiagnosticCode::QuantGraphFinalNormMissing
        }));
    }

    #[test]
    fn self_consistency_sc21_aux_blob_kind_per_format() {
        let mut graph = self_consistent_graph_fixture();
        graph.tensors[1].aux_blob_refs.clear();

        let diagnostics = self_consistency_diagnostics(&graph);

        assert!(has_diagnostic(&diagnostics, |code| {
            code == &QuantGraphBindingDiagnosticCode::QuantGraphAuxBlobKindMismatch
        }));
    }

    #[test]
    fn self_consistency_sc22_decode_requires_rng() {
        let mut graph = self_consistent_graph_fixture();
        graph.decode_spec.requires_rng = true;

        let diagnostics = self_consistency_diagnostics(&graph);

        assert!(has_diagnostic(&diagnostics, |code| {
            code == &QuantGraphBindingDiagnosticCode::QuantGraphDecodeRequiresRngMismatch
        }));
    }

    #[test]
    fn self_consistency_sc23_residual_plan_invalid() {
        let mut graph = self_consistent_graph_fixture();
        graph.residual_plan.activation_format = ternary_format();

        let diagnostics = self_consistency_diagnostics(&graph);

        assert!(has_diagnostic(&diagnostics, |code| {
            code == &QuantGraphBindingDiagnosticCode::QuantGraphResidualPlanInvalid
        }));
    }

    #[test]
    fn self_consistency_bit_exact_mid_reduction_saturation_forbidden() {
        let graph = self_consistent_graph_fixture();
        let decode_caps = decode_caps([DecodeMode::Argmax]);
        let mut context = self_consistency_context(&decode_caps);
        context.bit_exact_mid_reduction_saturation_absent = false;

        let diagnostics = validate_quant_graph_self_consistency(&graph, context);

        assert!(has_diagnostic(&diagnostics, |code| {
            code == &QuantGraphBindingDiagnosticCode::QuantGraphBitExactMidReductionSaturationForbidden
        }));
    }

    #[test]
    fn self_consistency_collects_all_diagnostics_no_short_circuit() {
        let mut graph = self_consistent_graph_fixture();
        graph.tensors[1].tensor_id = graph.tensors[0].tensor_id;
        graph.provenance.clear();
        graph.decode_spec.requires_rng = true;
        graph.residual_plan.activation_format = ternary_format();

        let diagnostics = self_consistency_diagnostics(&graph);
        let distinct_codes = diagnostics
            .iter()
            .map(|diagnostic| format!("{:?}", diagnostic.code))
            .collect::<BTreeSet<_>>();

        assert!(distinct_codes.len() >= 4);
    }

    #[test]
    fn self_consistency_diagnostic_severity_always_hard() {
        let graph = {
            let mut graph = self_consistent_graph_fixture();
            graph.decode_spec.requires_rng = true;
            graph
        };

        let diagnostics = self_consistency_diagnostics(&graph);

        assert!(!diagnostics.is_empty());
        assert!(diagnostics.iter().all(|diagnostic| {
            diagnostic.code.severity() == QuantGraphDiagnosticSeverity::Hard
        }));
    }

    #[test]
    fn quant_aux_blob_ref_carries_export_tensor_id() {
        let aux = aux_ref("tensor.weight.scale", hash(0xa1));
        let value = serde_json::to_value(aux).expect("aux ref serializes");

        assert_eq!(
            value["export_tensor_id"],
            Value::String("tensor.weight.scale".to_owned())
        );
    }

    #[test]
    fn resolved_blob_ref_carries_codec_and_decoded_size() {
        let resolved = resolved_blob_ref(0x20, 17, 33, BlobCodec::Zstd);
        let value = serde_json::to_value(resolved).expect("resolved blob serializes");

        assert_eq!(value["codec"], Value::String("zstd".to_owned()));
        assert_eq!(value["decoded_size_bytes"], Value::from(33_u64));
        assert_eq!(value["encoded_size_bytes"], Value::from(17_u64));
    }

    #[test]
    fn resolved_blob_index_self_hash_is_canonical_iteration_order() {
        let mut first = BTreeMap::new();
        first.insert(blob_ref(0x02, 20, BlobCodec::Raw), metadata(0xb2));
        first.insert(blob_ref(0x01, 10, BlobCodec::Zstd), metadata(0xb1));

        let mut second = BTreeMap::new();
        second.insert(blob_ref(0x01, 10, BlobCodec::Zstd), metadata(0xb1));
        second.insert(blob_ref(0x02, 20, BlobCodec::Raw), metadata(0xb2));

        assert_eq!(
            resolved_blob_index_self_hash(&first),
            resolved_blob_index_self_hash(&second)
        );
        assert_eq!(
            ResolvedBlobIndex::new(first.clone()).self_hash,
            resolved_blob_index_self_hash(&first)
        );
    }

    #[test]
    fn resolved_blob_index_two_regenerations_byte_identical() {
        let mut first = BTreeMap::new();
        first.insert(blob_ref(0x03, 30, BlobCodec::Raw), metadata(0xb3));
        first.insert(blob_ref(0x01, 10, BlobCodec::Zstd), metadata(0xb1));
        first.insert(blob_ref(0x02, 20, BlobCodec::Raw), metadata(0xb2));

        let mut second = BTreeMap::new();
        second.insert(blob_ref(0x02, 20, BlobCodec::Raw), metadata(0xb2));
        second.insert(blob_ref(0x03, 30, BlobCodec::Raw), metadata(0xb3));
        second.insert(blob_ref(0x01, 10, BlobCodec::Zstd), metadata(0xb1));

        let first_index = ResolvedBlobIndex::new(first);
        let second_index = ResolvedBlobIndex::new(second);
        let first_value = serde_json::to_value(&first_index).expect("index serializes");
        let second_value = serde_json::to_value(&second_index).expect("index serializes");

        assert_eq!(
            canonicalize_value(&first_value).expect("first index canonicalizes"),
            canonicalize_value(&second_value).expect("second index canonicalizes")
        );
        assert_eq!(first_index.self_hash, second_index.self_hash);
    }

    #[test]
    fn resolved_blob_index_round_trips_via_domain_hash() {
        let mut entries = BTreeMap::new();
        entries.insert(blob_ref(0x02, 20, BlobCodec::Raw), metadata(0xb2));
        entries.insert(blob_ref(0x01, 10, BlobCodec::Zstd), metadata(0xb1));
        let index = ResolvedBlobIndex::new(entries);
        let value = serde_json::to_value(&index).expect("blob index serializes");
        let decoded: ResolvedBlobIndex =
            serde_json::from_value(value.clone()).expect("blob index deserializes");

        assert_eq!(decoded, index);
        assert_eq!(
            decoded.self_hash,
            resolved_blob_index_self_hash(&decoded.entries)
        );
        assert_eq!(object_keys(&value), vec!["entries", "self_hash"]);
        assert!(value["entries"].is_array());
    }

    #[test]
    fn tensor_provenance_map_is_btreemap_canonical_order() {
        let mut provenance = TensorProvenanceMap::new();
        provenance.insert(TensorId::new(2), export_tensor_id("tensor.two"));
        provenance.insert(TensorId::new(1), export_tensor_id("tensor.one"));

        assert_eq!(
            provenance.keys().copied().collect::<Vec<_>>(),
            vec![TensorId::new(1), TensorId::new(2)]
        );
    }

    #[test]
    fn provenance_image_injective_across_main_and_aux() {
        let mut provenance = TensorProvenanceMap::new();
        provenance.insert(TensorId::new(1), export_tensor_id("tensor.weight"));
        let aux = aux_ref("tensor.weight.scale", hash(0xa1));

        assert!(injective_provenance_image(provenance.iter(), [&aux]).is_ok());

        let duplicate_aux = aux_ref("tensor.weight", hash(0xa2));
        let error = injective_provenance_image(provenance.iter(), [&duplicate_aux])
            .expect_err("duplicate main/aux export ids reject");

        assert!(matches!(
            error,
            ProvenanceImageError::NotInjective { field, .. }
                if field.as_str() == "aux_blob_refs.export_tensor_id"
        ));
    }

    proptest! {
        #[test]
        fn tensor_provenance_image_injectivity_property_test(
            export_ids in prop::collection::vec(0_u8..16, 1..24)
        ) {
            let provenance = export_ids
                .iter()
                .enumerate()
                .map(|(index, export_id)| {
                    (
                        TensorId::new(index as u32),
                        export_tensor_id(&format!("tensor.generated.{export_id}")),
                    )
                })
                .collect::<BTreeMap<_, _>>();
            let mut unique = BTreeSet::new();
            let expected_ok = export_ids.iter().all(|export_id| unique.insert(*export_id));

            prop_assert_eq!(
                injective_provenance_image(provenance.iter(), std::iter::empty()).is_ok(),
                expected_ok
            );
        }

        #[test]
        fn quant_graph_serde_round_trip_property(hash_byte in any::<u8>()) {
            let mut graph = self_consistent_graph_fixture();
            graph.identity.lowering_manifest_hash = hash(hash_byte);

            let encoded = serde_json::to_vec(&graph).expect("QuantGraph serializes");
            let decoded: QuantGraph =
                serde_json::from_slice(&encoded).expect("QuantGraph deserializes");

            prop_assert_eq!(&decoded, &graph);
            prop_assert_eq!(
                canonicalize_value(&serde_json::to_value(&decoded).expect("decoded graph to JSON"))
                    .expect("decoded graph canonicalizes"),
                canonicalize_value(&serde_json::to_value(&graph).expect("graph to JSON"))
                    .expect("graph canonicalizes")
            );
        }

        #[test]
        fn tensor_binding_round_trip_property_test(hash_byte in 1_u8..=200, elements in 1_u32..=64) {
            let input = QuantTensorBindingInput {
                tensor_id: TensorId::new(u32::from(hash_byte)),
                layout: layout(&[elements]),
                quant_format: QuantFormat::I8,
                role: QuantTensorRole::EmbeddingTable,
                blob_ref: blob_ref(hash_byte, elements, BlobCodec::Raw),
                aux_blob_refs: Vec::new(),
            };
            let blob_index = ResolvedBlobIndex::new(BTreeMap::from([(
                input.blob_ref,
                BlobMetadata {
                    content_hash: hash(hash_byte),
                    encoded_size_bytes: u64::from(elements),
                    decoded_size_bytes: u64::from(elements),
                    codec: BlobCodec::Raw,
                },
            )]));

            let tensors = bind_quant_tensors(std::slice::from_ref(&input), &blob_index)
                .expect("generated tensor binds");
            let encoded = serde_json::to_vec(&tensors[0]).expect("tensor serializes");
            let decoded: QuantTensorRef =
                serde_json::from_slice(&encoded).expect("tensor deserializes");

            prop_assert_eq!(decoded, tensors[0].clone());
        }
    }

    fn quant_graph_fixture() -> QuantGraph {
        let tensor = tensor_ref();
        let mut provenance = TensorProvenanceMap::new();
        provenance.insert(tensor.tensor_id, export_tensor_id("tensor.embedding"));

        QuantGraph {
            identity: identity_fixture(),
            tensors: vec![tensor],
            norm_plans: Vec::new(),
            layer_norms: BTreeMap::new(),
            routing_table: None,
            expert_sections: Vec::new(),
            ffn_plans: BTreeMap::new(),
            decode_spec: decode_spec_record_fixture(),
            sequence_semantics: SequenceSemanticsSpec::identity(),
            provenance,
            classify_head: classify_head_fixture(),
            residual_plan: residual_plan_fixture(),
        }
    }

    fn self_consistent_graph_fixture() -> QuantGraph {
        let embedding = tensor_with_layout_role(
            1,
            QuantTensorRole::EmbeddingTable,
            QuantFormat::I8,
            &[80, 8],
            640,
            Vec::new(),
        );
        let up = tensor_with_layout_role(
            2,
            QuantTensorRole::ExpertWeight {
                layer: LayerId::new(0),
                expert: ExpertId::new(0),
                slot: ExpertWeightSlot::FfnUp,
            },
            binary_format(),
            &[16, 8],
            16,
            vec![bound_aux_ref(QuantAuxKind::Scale, AuxFormat::Q8_8, 0x62, 2)],
        );
        let down = tensor_with_layout_role(
            3,
            QuantTensorRole::ExpertWeight {
                layer: LayerId::new(0),
                expert: ExpertId::new(0),
                slot: ExpertWeightSlot::FfnDown,
            },
            binary_format(),
            &[8, 16],
            16,
            vec![bound_aux_ref(QuantAuxKind::Scale, AuxFormat::Q8_8, 0x63, 2)],
        );

        QuantGraph {
            identity: identity_fixture(),
            tensors: vec![embedding, up, down],
            norm_plans: vec![
                norm_record(
                    0,
                    NormSite::LayerSequence {
                        layer: LayerId::new(0),
                    },
                ),
                norm_record(
                    1,
                    NormSite::LayerFfn {
                        layer: LayerId::new(0),
                    },
                ),
                norm_record(2, NormSite::Final),
            ],
            layer_norms: BTreeMap::from([(
                LayerId::new(0),
                LayerNorms {
                    pre_sequence: NormPlanId::new(0),
                    pre_ffn: NormPlanId::new(1),
                },
            )]),
            routing_table: None,
            expert_sections: vec![ExpertSection {
                layer: LayerId::new(0),
                expert: ExpertId::new(0),
                tensor_refs: vec![TensorId::new(2), TensorId::new(3)],
            }],
            ffn_plans: ffn_plans([(0, FfnActivationKind::Relu)]),
            decode_spec: decode_spec_record_fixture(),
            sequence_semantics: SequenceSemanticsSpec::identity(),
            provenance: BTreeMap::from([
                (TensorId::new(1), export_tensor_id("tensor.embedding")),
                (TensorId::new(2), export_tensor_id("tensor.ffn.up")),
                (TensorId::new(3), export_tensor_id("tensor.ffn.down")),
            ]),
            classify_head: classify_head_fixture(),
            residual_plan: residual_plan_fixture(),
        }
    }

    fn quant_graph_product_fixture() -> QuantGraphProduct {
        let graph = self_consistent_graph_fixture();
        let input_identity = quant_graph_input_identity(&graph, hash(0x77));
        QuantGraphProduct::new(graph, input_identity).expect("quant graph product builds")
    }

    fn self_consistency_diagnostics(graph: &QuantGraph) -> Vec<QuantGraphBindingDiagnostic> {
        let decode_caps = decode_caps([DecodeMode::Argmax]);
        validate_quant_graph_self_consistency(graph, self_consistency_context(&decode_caps))
    }

    fn self_consistency_context<'a>(
        decode_caps: &'a DecodeCapabilitySet,
    ) -> QuantGraphSelfConsistencyContext<'a> {
        QuantGraphSelfConsistencyContext {
            decode_caps,
            blob_index: None,
            training_residue_absent: true,
            sequence_semantics_tensors_match: true,
            required_features_supported: true,
            reduction_order_policy_enforced: true,
            bit_exact_mid_reduction_saturation_absent: true,
            forbidden_storage_metadata_absent: true,
            decode_requires_rng_matches_spec: true,
        }
    }

    fn has_diagnostic(
        diagnostics: &[QuantGraphBindingDiagnostic],
        predicate: impl Fn(&QuantGraphBindingDiagnosticCode) -> bool,
    ) -> bool {
        diagnostics
            .iter()
            .any(|diagnostic| predicate(&diagnostic.code))
    }

    fn identity_fixture() -> QuantGraphIdentity {
        QuantGraphIdentity {
            artifact_core_hash: hash(1),
            policy_resolution_self_hash: hash(2),
            artifact_validation_self_hash: hash(3),
            semantic_core_hash: hash(4),
            lowering_manifest_hash: hash(5),
            determinism: DeterminismClass::BitExact,
            model_spec_summary: model_summary_fixture(),
        }
    }

    fn identity_binding_inputs(
        artifact_determinism: DeterminismClass,
        policy_determinism: Option<DeterminismClass>,
    ) -> IdentityBindingInputs {
        IdentityBindingInputs {
            artifact_core_hash: hash(1),
            policy_resolution_self_hash: hash(2),
            artifact_validation_self_hash: hash(3),
            semantic_core_hash: hash(4),
            validated_artifact_effective_core_hash: hash(4),
            lowering_manifest_hash: hash(5),
            artifact_determinism,
            policy_determinism,
            model_spec_summary: model_summary_fixture(),
        }
    }

    fn model_summary_fixture() -> ModelSpecSummary {
        ModelSpecSummary {
            n_layers: 1,
            n_experts: BTreeMap::from([(LayerId::new(0), 1)]),
            d_model: 8,
            d_ff: 16,
            vocab_size: 80,
            ffn_kind: BTreeMap::from([(LayerId::new(0), FfnKindTag::Dense)]),
        }
    }

    fn model_summary_with_ffn_kinds(
        kinds: impl IntoIterator<Item = (u16, FfnKindTag)>,
    ) -> ModelSpecSummary {
        model_summary_with_experts(kinds.into_iter().map(|(layer, kind)| (layer, kind, 1)))
    }

    fn model_summary_with_experts(
        kinds: impl IntoIterator<Item = (u16, FfnKindTag, u16)>,
    ) -> ModelSpecSummary {
        let entries = kinds
            .into_iter()
            .map(|(layer, kind, n_experts)| (LayerId::new(layer), kind, n_experts))
            .collect::<Vec<_>>();
        let ffn_kind = entries
            .iter()
            .map(|(layer, kind, _)| (*layer, *kind))
            .collect::<BTreeMap<_, _>>();
        let n_experts = entries
            .iter()
            .map(|(layer, _, n_experts)| (*layer, *n_experts))
            .collect::<BTreeMap<_, _>>();

        ModelSpecSummary {
            n_layers: ffn_kind.len() as u16,
            n_experts,
            d_model: 8,
            d_ff: 16,
            vocab_size: 80,
            ffn_kind,
        }
    }

    fn router_layer(layer: u16, n_experts: u16) -> RouterLayer {
        RouterLayer {
            layer: LayerId::new(layer),
            n_experts,
            router_weight: TensorId::new(100 + u32::from(layer)),
            router_bias: Some(TensorId::new(200 + u32::from(layer))),
            semantics: RouterSemantics::Top1Hard {
                gate_weight: RouterGateWeightSemantics::SelectedScore,
                tie_break: RouterTieBreak::LowestExpertId,
            },
        }
    }

    fn ffn_plans(
        entries: impl IntoIterator<Item = (u16, FfnActivationKind)>,
    ) -> BTreeMap<LayerId, FfnPlan> {
        entries
            .into_iter()
            .map(|(layer, activation_kind)| {
                let layer = LayerId::new(layer);
                (
                    layer,
                    FfnPlan {
                        layer,
                        activation_kind,
                        intermediate_format: QuantFormat::Q8_8,
                    },
                )
            })
            .collect()
    }

    fn expert_weight_tensor(
        tensor_id: u32,
        layer: u16,
        expert: u16,
        slot: ExpertWeightSlot,
    ) -> QuantTensorRef {
        tensor_with_role(
            tensor_id,
            QuantTensorRole::ExpertWeight {
                layer: LayerId::new(layer),
                expert: ExpertId::new(expert),
                slot,
            },
            ternary_format(),
        )
    }

    fn expert_bias_tensor(
        tensor_id: u32,
        layer: u16,
        expert: u16,
        slot: ExpertWeightSlot,
    ) -> QuantTensorRef {
        tensor_with_role(
            tensor_id,
            QuantTensorRole::ExpertBias {
                layer: LayerId::new(layer),
                expert: ExpertId::new(expert),
                slot,
            },
            QuantFormat::I16,
        )
    }

    fn tensor_with_role(
        tensor_id: u32,
        role: QuantTensorRole,
        quant_format: QuantFormat,
    ) -> QuantTensorRef {
        QuantTensorRef {
            tensor_id: TensorId::new(tensor_id),
            layout: layout(&[2, 2]),
            quant_format,
            role,
            blob: resolved_blob_ref(tensor_id as u8, 1, 1, BlobCodec::Raw),
            aux_blob_refs: Vec::new(),
        }
    }

    fn tensor_with_layout_role(
        tensor_id: u32,
        role: QuantTensorRole,
        quant_format: QuantFormat,
        dims: &[u32],
        decoded_size_bytes: u64,
        aux_blob_refs: Vec<QuantAuxBlobRef>,
    ) -> QuantTensorRef {
        QuantTensorRef {
            tensor_id: TensorId::new(tensor_id),
            layout: layout(dims),
            quant_format,
            role,
            blob: resolved_blob_ref(
                tensor_id as u8,
                decoded_size_bytes,
                decoded_size_bytes,
                BlobCodec::Raw,
            ),
            aux_blob_refs,
        }
    }

    fn tensor_ref() -> QuantTensorRef {
        QuantTensorRef {
            tensor_id: TensorId::new(1),
            layout: layout(&[80, 8]),
            quant_format: QuantFormat::I8,
            role: QuantTensorRole::EmbeddingTable,
            blob: resolved_blob_ref(0x10, 640, 640, BlobCodec::Raw),
            aux_blob_refs: Vec::new(),
        }
    }

    fn decode_caps(modes: impl IntoIterator<Item = DecodeMode>) -> DecodeCapabilitySet {
        DecodeCapabilitySet {
            supported: modes.into_iter().collect(),
        }
    }

    fn decode_spec_record_fixture() -> DecodeSpecRecord {
        DecodeSpecRecord {
            decode_plan_id: DecodePlanId::new(0),
            spec: DecodeSpec::Argmax,
            requires_rng: false,
        }
    }

    fn classify_head_fixture() -> ClassifyHead {
        ClassifyHead {
            kind: ClassifyHeadKind::Tied,
            weight: TensorId::new(1),
            bias: None,
            logit_format: QuantFormat::Q8_8,
        }
    }

    fn residual_plan_fixture() -> ResidualPlan {
        ResidualPlan {
            activation_format: QuantFormat::Q8_8,
            combine_policy: ResidualCombinePolicy::AddThenClampNamedBoundary,
        }
    }

    fn norm_plan_record(
        id: u32,
        site: NormSite,
        input_format: QuantFormat,
        output_format: QuantFormat,
    ) -> NormPlanRecord {
        NormPlanRecord {
            norm_plan_id: NormPlanId::new(id),
            site,
            plan: norm_plan_fixture(),
            input_format,
            output_format,
        }
    }

    fn norm_record(id: u32, site: NormSite) -> NormPlanRecord {
        norm_plan_record(id, site, QuantFormat::Q8_8, QuantFormat::Q8_8)
    }

    fn norm_binding_input(
        site: NormSite,
        input_format: QuantFormat,
        output_format: QuantFormat,
    ) -> NormPlanBindingInput {
        NormPlanBindingInput {
            site,
            plan: norm_plan_fixture(),
            input_format,
            output_format,
        }
    }

    fn norm_plan_inputs_for_layers(n_layers: u16) -> Vec<NormPlanBindingInput> {
        let mut inputs = Vec::new();
        for layer in 0..n_layers {
            inputs.push(norm_binding_input(
                NormSite::LayerSequence {
                    layer: LayerId::new(layer),
                },
                QuantFormat::Q8_8,
                QuantFormat::I8,
            ));
        }
        for layer in 0..n_layers {
            inputs.push(norm_binding_input(
                NormSite::LayerFfn {
                    layer: LayerId::new(layer),
                },
                QuantFormat::I8,
                QuantFormat::I8,
            ));
        }
        inputs.push(norm_binding_input(
            NormSite::Final,
            QuantFormat::Q8_8,
            QuantFormat::I8,
        ));
        inputs
    }

    fn norm_plan_fixture() -> NormPlan {
        NormPlan::tile_rms_then_affine_clip(
            gbf_artifact::norm_plan::NormTileRmsSpec {
                tile_width: 8,
                epsilon: 1.0e-5,
            },
            gbf_artifact::norm_plan::NormAffineParams {
                scale: 1.0,
                bias: 0.0,
            },
            gbf_artifact::norm_plan::NormClipBounds { lo: -2.0, hi: 2.0 },
        )
    }

    fn aux_ref(export_tensor_path: &str, content_hash: Hash256) -> QuantAuxBlobRef {
        QuantAuxBlobRef {
            kind: QuantAuxKind::Scale,
            layout: layout(&[1]),
            format: AuxFormat::Q8_8,
            blob: ResolvedBlobRef {
                blob_ref: blob_ref(0x30, 2, BlobCodec::Raw),
                content_hash,
                encoded_size_bytes: 2,
                decoded_size_bytes: 2,
                codec: BlobCodec::Raw,
            },
            export_tensor_id: export_tensor_id(export_tensor_path),
        }
    }

    fn bound_aux_ref(
        kind: QuantAuxKind,
        format: AuxFormat,
        hash_byte: u8,
        decoded_size_bytes: u64,
    ) -> QuantAuxBlobRef {
        QuantAuxBlobRef {
            kind,
            layout: layout(&[1]),
            format,
            blob: resolved_blob_ref(
                hash_byte,
                decoded_size_bytes,
                decoded_size_bytes,
                BlobCodec::Raw,
            ),
            export_tensor_id: export_tensor_id(&format!("tensor.aux.{hash_byte}")),
        }
    }

    fn aux_binding_input(
        kind: QuantAuxKind,
        format: AuxFormat,
        hash_byte: u8,
        export_tensor_path: &str,
    ) -> QuantAuxBlobBindingInput {
        QuantAuxBlobBindingInput {
            kind,
            layout: layout(&[1]),
            format,
            blob_ref: blob_ref(hash_byte, 2, BlobCodec::Raw),
            export_tensor_id: export_tensor_id(export_tensor_path),
        }
    }

    fn ternary_format() -> QuantFormat {
        QuantFormat::Ternary2 {
            scale_granularity: ScaleGranularity::PerOutputRow,
            scale_format: AuxFormat::Q8_8,
            threshold_granularity: ThresholdGranularity::PerOutputRow,
        }
    }

    fn binary_format() -> QuantFormat {
        QuantFormat::Binary1 {
            scale_granularity: ScaleGranularity::PerOutputRow,
            scale_format: AuxFormat::Q8_8,
        }
    }

    fn sparse_ternary_format() -> QuantFormat {
        QuantFormat::SparseTernaryBitplanes {
            scale_granularity: ScaleGranularity::PerOutputRow,
            scale_format: AuxFormat::Q8_8,
            sparse_meta_kind: SparseMetaKind::RowOffsets,
        }
    }

    fn resolved_blob_ref(
        hash_byte: u8,
        encoded_size_bytes: u64,
        decoded_size_bytes: u64,
        codec: BlobCodec,
    ) -> ResolvedBlobRef {
        ResolvedBlobRef {
            blob_ref: blob_ref(hash_byte, encoded_size_bytes as u32, codec),
            content_hash: hash(hash_byte),
            encoded_size_bytes,
            decoded_size_bytes,
            codec,
        }
    }

    fn metadata(hash_byte: u8) -> BlobMetadata {
        BlobMetadata {
            content_hash: hash(hash_byte),
            encoded_size_bytes: u64::from(hash_byte),
            decoded_size_bytes: u64::from(hash_byte) * 2,
            codec: BlobCodec::Raw,
        }
    }

    fn metadata_with_size(hash_byte: u8, decoded_size_bytes: u64) -> BlobMetadata {
        BlobMetadata {
            content_hash: hash(hash_byte),
            encoded_size_bytes: decoded_size_bytes,
            decoded_size_bytes,
            codec: BlobCodec::Raw,
        }
    }

    fn blob_ref(hash_byte: u8, len: u32, codec: BlobCodec) -> BlobRef {
        BlobRef {
            hash: hash(hash_byte),
            len,
            codec,
        }
    }

    fn layout(dims: &[u32]) -> CanonicalTensorLayout {
        CanonicalTensorLayout::new(
            CanonicalTensorShape::new(dims.to_vec()).expect("shape is valid"),
            TensorElementType::Q8_8,
        )
    }

    fn export_tensor_id(value: &str) -> ExportTensorId {
        ArtifactPath::new(value).expect("export tensor id is valid")
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

    fn object_keys(value: &Value) -> Vec<&str> {
        let mut keys = value
            .as_object()
            .expect("value is object")
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>();
        keys.sort_unstable();
        keys
    }

    fn collect_json_keys(prefix: &str, value: &Value, keys: &mut Vec<String>) {
        match value {
            Value::Object(map) => {
                for (key, nested) in map {
                    let path = if prefix.is_empty() {
                        key.clone()
                    } else {
                        format!("{prefix}.{key}")
                    };
                    keys.push(path.clone());
                    collect_json_keys(&path, nested, keys);
                }
            }
            Value::Array(values) => {
                for nested in values {
                    collect_json_keys(prefix, nested, keys);
                }
            }
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
        }
    }

    #[test]
    fn build_quant_graph_core_signature_takes_resolved_blob_index_not_resolver() {
        fn assert_signature(
            _: fn(QuantGraphInputs) -> Result<QuantGraphProduct, QuantGraphStageFailure>,
        ) {
        }

        assert_signature(build_quant_graph_core);
        let inputs = quant_graph_inputs_fixture();
        assert!(!inputs.resolved_blob_index.entries.is_empty());
    }

    #[test]
    fn build_quant_graph_core_uses_prebuilt_blob_index_without_resolver_io() {
        let inputs = quant_graph_inputs_fixture();
        let mut missing_resolver = Resolver::from_inputs(&inputs);
        missing_resolver.blobs.clear();

        let product = build_quant_graph_core(inputs.clone())
            .expect("pure core succeeds from prebuilt blob index");
        let driver_resolution = build_stage1_resolved_blob_index(&inputs, &missing_resolver)
            .expect_err("driver IO would fail without resolver bytes");

        assert_eq!(product.report.outcome, ReportOutcome::Passed);
        assert_eq!(
            product.report.body.input_identity.resolved_blob_index_hash,
            inputs.resolved_blob_index.self_hash
        );
        assert_eq!(
            driver_resolution.kind,
            QuantGraphStageFailureKind::BlobResolution
        );
    }

    #[test]
    fn build_quant_graph_core_idempotent_under_repeated_invocation() {
        let inputs = quant_graph_inputs_fixture();

        let first = build_quant_graph_core(inputs.clone()).expect("first core run succeeds");
        let second = build_quant_graph_core(inputs).expect("second core run succeeds");

        assert_eq!(first, second);
        assert_eq!(
            canonicalize_report(&first.report).expect("first report canonicalizes"),
            canonicalize_report(&second.report).expect("second report canonicalizes")
        );
    }

    #[test]
    fn fixture_quant_graph_dense_toy0_passes_self_hash_round_trip() {
        let fixture = passing_fixture("dense_toy0");
        let product = fixture_quant_graph_product(fixture);
        assert_passing_fixture_metadata(fixture, &product);
        assert_quant_graph_report_self_hash_round_trips(&product.report);
    }

    #[test]
    fn fixture_quant_graph_dense_toy1_passes_self_consistency() {
        for name in ["dense_toy1_tied", "dense_toy1_untied"] {
            let fixture = passing_fixture(name);
            let inputs = fixture.inputs();
            let product = build_quant_graph_core(inputs).expect("dense Toy1 fixture passes");

            assert_passing_fixture_metadata(fixture, &product);
            assert_eq!(product.report.outcome, ReportOutcome::Passed);
            assert!(product.quant_graph.routing_table.is_none());
        }
    }

    #[test]
    fn fixture_quant_graph_routed_basic_passes_self_consistency() {
        for name in ["routed_basic_one", "routed_basic_selected_score"] {
            let fixture = passing_fixture(name);
            let inputs = fixture.inputs();
            let product = build_quant_graph_core(inputs).expect("routed fixture passes");

            assert_passing_fixture_metadata(fixture, &product);
            assert_eq!(product.report.outcome, ReportOutcome::Passed);
            assert_eq!(
                product
                    .quant_graph
                    .routing_table
                    .as_ref()
                    .expect("routed fixture has routing table")
                    .layers
                    .len(),
                1
            );
        }
    }

    #[test]
    fn fixture_quant_graph_mixed_topology_passes_self_consistency() {
        let fixture = passing_fixture("mixed_topology");
        let product = fixture_quant_graph_product(fixture);

        assert_passing_fixture_metadata(fixture, &product);
        assert_eq!(
            compute_ffn_topology_kind(&product.quant_graph.identity.model_spec_summary),
            FfnTopologyKindTag::Mixed
        );
    }

    #[test]
    fn fixture_quant_graph_routed_gate_weight_one_variant() {
        let product = fixture_quant_graph_product(passing_fixture("routed_basic_one"));
        let layer = &product
            .quant_graph
            .routing_table
            .as_ref()
            .expect("routed fixture has routing")
            .layers[0];

        assert!(matches!(
            layer.semantics,
            RouterSemantics::Top1Hard {
                gate_weight: RouterGateWeightSemantics::One,
                tie_break: RouterTieBreak::LowestExpertId,
            }
        ));
    }

    #[test]
    fn fixture_quant_graph_routed_gate_weight_selected_score_variant() {
        let product = fixture_quant_graph_product(passing_fixture("routed_basic_selected_score"));
        let layer = &product
            .quant_graph
            .routing_table
            .as_ref()
            .expect("routed fixture has routing")
            .layers[0];

        assert!(matches!(
            layer.semantics,
            RouterSemantics::Top1Hard {
                gate_weight: RouterGateWeightSemantics::SelectedScore,
                tie_break: RouterTieBreak::LowestExpertId,
            }
        ));
    }

    #[test]
    fn fixture_quant_graph_every_reject_class_has_typed_diagnostic() {
        let cases = reject_fixture_cases();
        assert_eq!(cases.len(), 36);
        let expected_dirs = cases
            .iter()
            .map(|case| case.dir.to_owned())
            .collect::<BTreeSet<_>>();
        assert_eq!(reject_fixture_dirs(), expected_dirs);

        for case in cases {
            let expected = read_fixture_file(&["reject", case.dir, "expected.toml"]);
            let counterexample = read_fixture_file(&["reject", case.dir, "inputs.toml"]);
            let readme = read_fixture_file(&["reject", case.dir, "README.md"]);
            assert!(expected.contains(&format!("qg_reject = {}", case.qg_reject)));
            assert!(expected.contains(&format!("diagnostic_code = \"{}\"", case.expected_code)));
            assert!(expected.contains("severity = \"Hard\""));
            assert!(counterexample.contains(&format!("qg_reject = {}", case.qg_reject)));
            assert!(
                counterexample.contains(&format!("counterexample = \"{}\"", case.counterexample))
            );
            assert!(readme.contains(case.expected_code));

            let failure = build_quant_graph_core(case.inputs())
                .expect_err("reject fixture must fail through Stage 1 core");
            assert_eq!(failure.kind, QuantGraphStageFailureKind::Rejected);
            let matching = failure
                .binding_diagnostics
                .iter()
                .find(|diagnostic| diagnostic_code_kind(&diagnostic.code) == case.expected_code)
                .unwrap_or_else(|| {
                    panic!(
                        "reject fixture {} did not emit {}; diagnostics: {:?}",
                        case.dir, case.expected_code, failure.binding_diagnostics
                    )
                });
            assert_eq!(matching.code.severity(), QuantGraphDiagnosticSeverity::Hard);
        }
    }

    #[test]
    fn fixture_quant_graph_cache_hit_byte_identical_product() {
        for fixture in passing_fixture_cases() {
            assert_stage1_cache_hit_byte_identical(fixture);
        }
    }

    fn assert_stage1_cache_hit_byte_identical(fixture: PassingQuantGraphFixture) {
        let inputs = fixture.inputs();
        let resolver = Resolver::from_inputs(&inputs);
        let report_dir = tempfile::tempdir().expect("report tempdir");
        let (_store_dir, store) = store();
        let cache = StageCache::new(&store);
        let env = PassEnvironment::new(&resolver)
            .with_report_dir(report_dir.path())
            .with_stage_cache(&cache);

        let first = run_stage1(inputs.clone(), env).expect("first fixture run succeeds");
        let first_report =
            std::fs::read(report_dir.path().join("quant_graph.json")).expect("first report");
        let second = run_stage1(inputs, env).expect("second fixture run succeeds");
        let second_report =
            std::fs::read(report_dir.path().join("quant_graph.json")).expect("second report");

        assert_eq!(first, second);
        assert_eq!(first_report, second_report);
    }

    #[test]
    fn fixture_quant_graph_two_clean_regenerations_byte_identical() {
        for fixture in passing_fixture_cases() {
            let first = fixture_quant_graph_product(fixture);
            let second = fixture_quant_graph_product(fixture);
            let first_bytes = canonicalize_report(&first.report).expect("first canonicalizes");
            let second_bytes = canonicalize_report(&second.report).expect("second canonicalizes");

            assert_eq!(first, second);
            assert_eq!(first_bytes, second_bytes);
        }
    }

    #[test]
    fn fixture_quant_graph_no_real_exported_models_in_fixtures() {
        for path in quant_graph_fixture_files() {
            let metadata = std::fs::metadata(&path).expect("fixture file metadata");
            assert!(
                metadata.len() <= 64 * 1024,
                "fixture file exceeds tiny-fixture limit: {}",
                path.display()
            );
        }
    }

    #[test]
    fn fixture_quant_graph_print_golden_hashes_when_requested() {
        if std::env::var_os("GBF_PRINT_QUANT_GRAPH_FIXTURE_GOLDENS").is_none() {
            return;
        }
        for fixture in passing_fixture_cases() {
            let product = fixture_quant_graph_product(fixture);
            println!(
                "{} quant_graph_self_hash={} report_self_hash={} quant_graph_canonical_bytes_hash={}",
                fixture.dir,
                product.quant_graph_self_hash,
                product.report.report_self_hash,
                product.quant_graph_canonical_bytes_hash
            );
        }
    }

    #[test]
    fn run_stage1_emits_report_and_writes_cache() {
        let inputs = quant_graph_inputs_fixture();
        let resolver = Resolver::from_inputs(&inputs);
        let report_dir = tempfile::tempdir().expect("report tempdir");
        let (_store_dir, store) = store();
        let cache = StageCache::new(&store);

        let product = run_stage1(
            inputs.clone(),
            PassEnvironment::new(&resolver)
                .with_report_dir(report_dir.path())
                .with_stage_cache(&cache),
        )
        .expect("driver succeeds");

        let report_bytes =
            std::fs::read(report_dir.path().join("quant_graph.json")).expect("report emitted");
        let decoded: ReportEnvelope<report_schema::QuantGraphReportBody<QuantGraph>> =
            serde_json::from_slice(&report_bytes).expect("report decodes");
        assert_eq!(decoded.outcome, ReportOutcome::Passed);
        assert_eq!(decoded.report_self_hash, product.report.report_self_hash);

        let cell = get_stage1_success(&cache, &stage1_cache_key_material(&inputs))
            .expect("cache lookup succeeds")
            .expect("success cell exists");
        assert!(matches!(cell, Stage1CacheCell::QuantGraphSuccess { .. }));
    }

    #[test]
    fn run_stage1_writes_stage_cache_failure_memo_on_failed() {
        let mut inputs = quant_graph_inputs_fixture();
        inputs.decode_source = DecodeBindingSource::UnboundDefault;
        let resolver = Resolver::from_inputs(&inputs);
        let report_dir = tempfile::tempdir().expect("report tempdir");
        let (_store_dir, store) = store();
        let cache = StageCache::new(&store);

        let failure = run_stage1(
            inputs.clone(),
            PassEnvironment::new(&resolver)
                .with_report_dir(report_dir.path())
                .with_stage_cache(&cache),
        )
        .expect_err("driver rejects invalid decode binding");

        assert_eq!(failure.kind, QuantGraphStageFailureKind::Rejected);
        assert!(failure.report.is_some());
        assert!(
            get_stage1_success(&cache, &stage1_cache_key_material(&inputs))
                .expect("success lookup")
                .is_none()
        );
        let cell = get_stage1_failure_memo(&cache, &stage1_cache_key_material(&inputs))
            .expect("failure lookup")
            .expect("failure memo exists");
        assert!(matches!(cell, Stage1CacheCell::FailureMemo { .. }));
    }

    #[test]
    fn run_stage1_failure_memo_never_used_as_success() {
        let mut inputs = quant_graph_inputs_fixture();
        inputs.decode_source = DecodeBindingSource::UnboundDefault;
        let resolver = Resolver::from_inputs(&inputs);
        let report_dir = tempfile::tempdir().expect("report tempdir");
        let (_store_dir, store) = store();
        let cache = StageCache::new(&store);
        let env = PassEnvironment::new(&resolver)
            .with_report_dir(report_dir.path())
            .with_stage_cache(&cache);
        let material = stage1_cache_key_material(&inputs);

        let first = run_stage1(inputs.clone(), env).expect_err("first run writes failure memo");
        let second = run_stage1(inputs.clone(), env).expect_err("second run replays failure memo");

        assert_eq!(first.kind, QuantGraphStageFailureKind::Rejected);
        assert_eq!(second.kind, QuantGraphStageFailureKind::CacheHitFailureMemo);
        assert!(
            get_stage1_success(&cache, &material)
                .expect("success lookup")
                .is_none()
        );
        assert!(
            get_stage1_failure_memo(&cache, &material)
                .expect("failure lookup")
                .is_some()
        );
    }

    #[test]
    fn run_stage1_handles_missing_blob_ref_with_typed_diagnostic() {
        let inputs = quant_graph_inputs_fixture();
        let missing_blob_ref = inputs
            .tensor_bindings
            .first()
            .expect("fixture has tensor bindings")
            .blob_ref;
        let mut resolver = Resolver::from_inputs(&inputs);
        resolver.blobs.remove(&missing_blob_ref);
        let report_dir = tempfile::tempdir().expect("report tempdir");

        let failure = run_stage1(
            inputs,
            PassEnvironment::new(&resolver).with_report_dir(report_dir.path()),
        )
        .expect_err("missing blob rejects before core binding");

        assert_eq!(failure.kind, QuantGraphStageFailureKind::BlobResolution);
        assert!(has_diagnostic(
            &failure.binding_diagnostics,
            |code| matches!(
                code,
                QuantGraphBindingDiagnosticCode::QuantGraphBlobRefUnresolvable { blob_ref }
                    if *blob_ref == missing_blob_ref
            )
        ));
        assert!(failure.report.is_some());
        let report_bytes =
            std::fs::read(report_dir.path().join("quant_graph.json")).expect("failure report");
        let decoded: ReportEnvelope<report_schema::QuantGraphReportBody<QuantGraph>> =
            serde_json::from_slice(&report_bytes).expect("failure report decodes");
        assert_eq!(decoded.outcome, ReportOutcome::Failed);
    }

    #[test]
    fn run_stage1_two_clean_runs_byte_identical_quant_graph_json() {
        let inputs = quant_graph_inputs_fixture();
        let resolver = Resolver::from_inputs(&inputs);
        let first_report_dir = tempfile::tempdir().expect("first report tempdir");
        let second_report_dir = tempfile::tempdir().expect("second report tempdir");

        let first = run_stage1(
            inputs.clone(),
            PassEnvironment::new(&resolver).with_report_dir(first_report_dir.path()),
        )
        .expect("first clean driver run succeeds");
        let second = run_stage1(
            inputs,
            PassEnvironment::new(&resolver).with_report_dir(second_report_dir.path()),
        )
        .expect("second clean driver run succeeds");
        let first_report = std::fs::read(first_report_dir.path().join("quant_graph.json"))
            .expect("first report emitted");
        let second_report = std::fs::read(second_report_dir.path().join("quant_graph.json"))
            .expect("second report emitted");

        assert_eq!(first, second);
        assert_eq!(first_report, second_report);
    }

    #[test]
    fn run_stage1_cache_hit_replays_byte_identical_product() {
        let inputs = quant_graph_inputs_fixture();
        let resolver = Resolver::from_inputs(&inputs);
        let report_dir = tempfile::tempdir().expect("report tempdir");
        let (_store_dir, store) = store();
        let cache = StageCache::new(&store);
        let env = PassEnvironment::new(&resolver)
            .with_report_dir(report_dir.path())
            .with_stage_cache(&cache);

        let first = run_stage1(inputs.clone(), env).expect("first driver run succeeds");
        let first_report =
            std::fs::read(report_dir.path().join("quant_graph.json")).expect("first report");
        let second = run_stage1(inputs.clone(), env).expect("second driver run succeeds");
        let second_report =
            std::fs::read(report_dir.path().join("quant_graph.json")).expect("second report");

        assert_eq!(first, second);
        assert_eq!(first_report, second_report);
        assert!(
            get_stage1_success(&cache, &stage1_cache_key_material(&inputs))
                .expect("success lookup")
                .is_some()
        );
    }

    #[derive(Clone, Copy)]
    struct PassingQuantGraphFixture {
        dir: &'static str,
        description: &'static str,
        inputs: fn() -> QuantGraphInputs,
    }

    impl PassingQuantGraphFixture {
        fn inputs(self) -> QuantGraphInputs {
            (self.inputs)()
        }
    }

    #[derive(Clone, Copy)]
    struct RejectQuantGraphFixture {
        qg_reject: u8,
        dir: &'static str,
        expected_code: &'static str,
        counterexample: &'static str,
        inputs: fn() -> QuantGraphInputs,
    }

    impl RejectQuantGraphFixture {
        fn inputs(self) -> QuantGraphInputs {
            (self.inputs)()
        }
    }

    fn passing_fixture(name: &str) -> PassingQuantGraphFixture {
        passing_fixture_cases()
            .into_iter()
            .find(|fixture| fixture.dir == name)
            .expect("passing fixture is registered")
    }

    fn passing_fixture_cases() -> [PassingQuantGraphFixture; 6] {
        [
            PassingQuantGraphFixture {
                dir: "dense_toy0",
                description: "minimal single-layer dense FFN",
                inputs: dense_toy0_inputs,
            },
            PassingQuantGraphFixture {
                dir: "dense_toy1_tied",
                description: "two-layer dense FFN with tied classify head",
                inputs: dense_toy1_tied_inputs,
            },
            PassingQuantGraphFixture {
                dir: "dense_toy1_untied",
                description: "two-layer dense FFN with untied classify head",
                inputs: dense_toy1_untied_inputs,
            },
            PassingQuantGraphFixture {
                dir: "routed_basic_one",
                description: "single routed layer with unit router gate weight",
                inputs: routed_basic_one_inputs,
            },
            PassingQuantGraphFixture {
                dir: "routed_basic_selected_score",
                description: "single routed layer with selected-score router gate weight",
                inputs: routed_basic_selected_score_inputs,
            },
            PassingQuantGraphFixture {
                dir: "mixed_topology",
                description: "one dense layer and one routed layer",
                inputs: mixed_topology_inputs,
            },
        ]
    }

    fn assert_passing_fixture_metadata(
        fixture: PassingQuantGraphFixture,
        product: &QuantGraphProduct,
    ) {
        let manifest = read_fixture_file(&[fixture.dir, "fixture.toml"]);
        let self_hash = read_fixture_file(&[fixture.dir, "quant_graph_self_hash"]);
        let canonical_hash = read_fixture_file(&[fixture.dir, "quant_graph_canonical_bytes_hash"]);
        let report_hash = read_fixture_file(&[fixture.dir, "report_self_hash"]);

        assert!(manifest.contains(&format!("name = \"{}\"", fixture.dir)));
        assert!(manifest.contains(&format!("description = \"{}\"", fixture.description)));
        assert_eq!(self_hash.trim(), product.quant_graph_self_hash.to_string());
        assert_eq!(
            canonical_hash.trim(),
            product.quant_graph_canonical_bytes_hash.to_string()
        );
        assert_eq!(
            report_hash.trim(),
            product.report.report_self_hash.to_string()
        );
    }

    fn fixture_quant_graph_product(fixture: PassingQuantGraphFixture) -> QuantGraphProduct {
        build_quant_graph_core(fixture.inputs()).expect("fixture QuantGraph builds")
    }

    fn assert_quant_graph_report_self_hash_round_trips(
        report: &ReportEnvelope<report_schema::QuantGraphReportBody<QuantGraph>>,
    ) {
        let canonical = canonicalize_report(report).expect("report canonicalizes");
        let decoded: ReportEnvelope<report_schema::QuantGraphReportBody<QuantGraph>> =
            serde_json::from_slice(&canonical).expect("canonical report decodes");
        let rehashed = decoded
            .clone()
            .with_computed_self_hash()
            .expect("decoded report self-hash recomputes");

        assert_eq!(decoded.report_self_hash, report.report_self_hash);
        assert_eq!(rehashed.report_self_hash, report.report_self_hash);
    }

    fn dense_toy0_inputs() -> QuantGraphInputs {
        quant_graph_inputs_from_graph(self_consistent_graph_fixture())
    }

    fn dense_toy1_tied_inputs() -> QuantGraphInputs {
        quant_graph_inputs_from_graph(dense_toy1_graph(ClassifyHeadKind::Tied))
    }

    fn dense_toy1_untied_inputs() -> QuantGraphInputs {
        quant_graph_inputs_from_graph(dense_toy1_graph(ClassifyHeadKind::Untied))
    }

    fn routed_basic_one_inputs() -> QuantGraphInputs {
        quant_graph_inputs_from_graph(routed_basic_graph(RouterGateWeightSemantics::One))
    }

    fn routed_basic_selected_score_inputs() -> QuantGraphInputs {
        quant_graph_inputs_from_graph(routed_basic_graph(RouterGateWeightSemantics::SelectedScore))
    }

    fn mixed_topology_inputs() -> QuantGraphInputs {
        quant_graph_inputs_from_graph(mixed_topology_graph())
    }

    fn dense_toy1_graph(classify_kind: ClassifyHeadKind) -> QuantGraph {
        let mut graph = self_consistent_graph_fixture();
        graph.identity.model_spec_summary =
            model_summary_with_ffn_kinds([(0, FfnKindTag::Dense), (1, FfnKindTag::Dense)]);
        graph.norm_plans = norm_records_for_layers(2);
        graph.layer_norms = layer_norms_for_layers(2);
        graph.ffn_plans = ffn_plans([(0, FfnActivationKind::Relu), (1, FfnActivationKind::Gelu)]);
        push_expert_pair(&mut graph, 1, 0, 4, 5);
        match classify_kind {
            ClassifyHeadKind::Tied => {}
            ClassifyHeadKind::Untied => {
                graph.tensors.push(tensor_with_layout_role(
                    6,
                    QuantTensorRole::ClassifyWeight,
                    QuantFormat::I8,
                    &[80, 8],
                    640,
                    Vec::new(),
                ));
                graph
                    .provenance
                    .insert(TensorId::new(6), export_tensor_id("tensor.classify.weight"));
                graph.classify_head = ClassifyHead {
                    kind: ClassifyHeadKind::Untied,
                    weight: TensorId::new(6),
                    bias: None,
                    logit_format: QuantFormat::Q8_8,
                };
            }
        }
        graph
    }

    fn routed_basic_graph(gate_weight: RouterGateWeightSemantics) -> QuantGraph {
        let mut graph = self_consistent_graph_fixture();
        graph.identity.model_spec_summary =
            model_summary_with_experts([(0, FfnKindTag::Routed, 4)]);
        graph.tensors.truncate(1);
        graph
            .provenance
            .retain(|tensor_id, _| *tensor_id == TensorId::new(1));
        graph.expert_sections.clear();
        graph.ffn_plans = ffn_plans([(0, FfnActivationKind::Relu)]);
        for expert in 0_u16..4 {
            let up_id = 10 + u32::from(expert) * 2;
            push_expert_pair(&mut graph, 0, expert, up_id, up_id + 1);
        }
        push_router_tensors(&mut graph, 0, 4, 30, 31);
        graph.routing_table = Some(RoutingTable {
            layers: vec![RouterLayer {
                layer: LayerId::new(0),
                n_experts: 4,
                router_weight: TensorId::new(30),
                router_bias: Some(TensorId::new(31)),
                semantics: RouterSemantics::Top1Hard {
                    gate_weight,
                    tie_break: RouterTieBreak::LowestExpertId,
                },
            }],
        });
        graph
    }

    fn mixed_topology_graph() -> QuantGraph {
        let mut graph = self_consistent_graph_fixture();
        graph.identity.model_spec_summary =
            model_summary_with_experts([(0, FfnKindTag::Dense, 1), (1, FfnKindTag::Routed, 2)]);
        graph.norm_plans = norm_records_for_layers(2);
        graph.layer_norms = layer_norms_for_layers(2);
        graph.ffn_plans = ffn_plans([(0, FfnActivationKind::Relu), (1, FfnActivationKind::SiLU)]);
        for expert in 0_u16..2 {
            let up_id = 40 + u32::from(expert) * 2;
            push_expert_pair(&mut graph, 1, expert, up_id, up_id + 1);
        }
        push_router_tensors(&mut graph, 1, 2, 50, 51);
        graph.routing_table = Some(RoutingTable {
            layers: vec![RouterLayer {
                layer: LayerId::new(1),
                n_experts: 2,
                router_weight: TensorId::new(50),
                router_bias: Some(TensorId::new(51)),
                semantics: RouterSemantics::Top1Hard {
                    gate_weight: RouterGateWeightSemantics::SelectedScore,
                    tie_break: RouterTieBreak::LowestExpertId,
                },
            }],
        });
        graph
    }

    fn push_expert_pair(
        graph: &mut QuantGraph,
        layer: u16,
        expert: u16,
        up_tensor_id: u32,
        down_tensor_id: u32,
    ) {
        let layer_id = LayerId::new(layer);
        let expert_id = ExpertId::new(expert);
        graph.tensors.push(tensor_with_layout_role(
            up_tensor_id,
            QuantTensorRole::ExpertWeight {
                layer: layer_id,
                expert: expert_id,
                slot: ExpertWeightSlot::FfnUp,
            },
            binary_format(),
            &[16, 8],
            16,
            vec![bound_aux_ref(
                QuantAuxKind::Scale,
                AuxFormat::Q8_8,
                0x80 + up_tensor_id as u8,
                2,
            )],
        ));
        graph.tensors.push(tensor_with_layout_role(
            down_tensor_id,
            QuantTensorRole::ExpertWeight {
                layer: layer_id,
                expert: expert_id,
                slot: ExpertWeightSlot::FfnDown,
            },
            binary_format(),
            &[8, 16],
            16,
            vec![bound_aux_ref(
                QuantAuxKind::Scale,
                AuxFormat::Q8_8,
                0x80 + down_tensor_id as u8,
                2,
            )],
        ));
        graph.provenance.insert(
            TensorId::new(up_tensor_id),
            export_tensor_id(&format!("tensor.layer{layer}.expert{expert}.up")),
        );
        graph.provenance.insert(
            TensorId::new(down_tensor_id),
            export_tensor_id(&format!("tensor.layer{layer}.expert{expert}.down")),
        );
        graph.expert_sections.push(ExpertSection {
            layer: layer_id,
            expert: expert_id,
            tensor_refs: vec![TensorId::new(up_tensor_id), TensorId::new(down_tensor_id)],
        });
    }

    fn push_router_tensors(
        graph: &mut QuantGraph,
        layer: u16,
        n_experts: u16,
        weight_tensor_id: u32,
        bias_tensor_id: u32,
    ) {
        graph.tensors.push(tensor_with_layout_role(
            weight_tensor_id,
            QuantTensorRole::RouterWeight {
                layer: LayerId::new(layer),
            },
            QuantFormat::I8,
            &[u32::from(n_experts), 8],
            u64::from(n_experts) * 8,
            Vec::new(),
        ));
        graph.tensors.push(tensor_with_layout_role(
            bias_tensor_id,
            QuantTensorRole::RouterBias {
                layer: LayerId::new(layer),
            },
            QuantFormat::I16,
            &[u32::from(n_experts)],
            u64::from(n_experts) * 2,
            Vec::new(),
        ));
        graph.provenance.insert(
            TensorId::new(weight_tensor_id),
            export_tensor_id(&format!("tensor.layer{layer}.router.weight")),
        );
        graph.provenance.insert(
            TensorId::new(bias_tensor_id),
            export_tensor_id(&format!("tensor.layer{layer}.router.bias")),
        );
    }

    fn norm_records_for_layers(n_layers: u16) -> Vec<NormPlanRecord> {
        let mut records = Vec::new();
        let mut id = 0_u32;
        for layer in 0..n_layers {
            records.push(norm_record(
                id,
                NormSite::LayerSequence {
                    layer: LayerId::new(layer),
                },
            ));
            id += 1;
        }
        for layer in 0..n_layers {
            records.push(norm_record(
                id,
                NormSite::LayerFfn {
                    layer: LayerId::new(layer),
                },
            ));
            id += 1;
        }
        records.push(norm_record(id, NormSite::Final));
        records
    }

    fn layer_norms_for_layers(n_layers: u16) -> BTreeMap<LayerId, LayerNorms> {
        let mut layer_norms = BTreeMap::new();
        for layer in 0..n_layers {
            layer_norms.insert(
                LayerId::new(layer),
                LayerNorms {
                    pre_sequence: NormPlanId::new(u32::from(layer)),
                    pre_ffn: NormPlanId::new(u32::from(n_layers + layer)),
                },
            );
        }
        layer_norms
    }

    fn reject_fixture_cases() -> [RejectQuantGraphFixture; 36] {
        [
            reject_fixture(
                1,
                "qg_reject_01_training_residue",
                "QuantGraphTrainingResidue",
                "gbf-codegen::s1::quant_graph::tests::reject_training_residue_inputs",
                reject_training_residue_inputs,
            ),
            reject_fixture(
                2,
                "qg_reject_02_role_format_mismatch",
                "QuantGraphRoleFormatMismatch",
                "gbf-codegen::s1::quant_graph::tests::reject_role_format_mismatch_inputs",
                reject_role_format_mismatch_inputs,
            ),
            reject_fixture(
                3,
                "qg_reject_03_routing_missing_for_routed_layer",
                "QuantGraphRoutingMissingForRoutedLayer",
                "gbf-codegen::s1::quant_graph::tests::reject_routing_missing_for_routed_layer_inputs",
                reject_routing_missing_for_routed_layer_inputs,
            ),
            reject_fixture(
                4,
                "qg_reject_04_routing_present_for_dense_layer",
                "QuantGraphRoutingPresentForDenseLayer",
                "gbf-codegen::s1::quant_graph::tests::reject_routing_present_for_dense_layer_inputs",
                reject_routing_present_for_dense_layer_inputs,
            ),
            reject_fixture(
                5,
                "qg_reject_05_routing_expert_coverage_mismatch",
                "QuantGraphRoutingExpertCoverageMismatch",
                "gbf-codegen::s1::quant_graph::tests::reject_routing_expert_coverage_mismatch_inputs",
                reject_routing_expert_coverage_mismatch_inputs,
            ),
            reject_fixture(
                6,
                "qg_reject_06_tensor_id_not_unique",
                "QuantGraphTensorIdNotUnique",
                "gbf-codegen::s1::quant_graph::tests::reject_tensor_id_not_unique_inputs",
                reject_tensor_id_not_unique_inputs,
            ),
            reject_fixture(
                7,
                "qg_reject_07_identity_hash_mismatch",
                "QuantGraphIdentityHashMismatch",
                "gbf-codegen::s1::quant_graph::tests::reject_identity_hash_mismatch_inputs",
                reject_identity_hash_mismatch_inputs,
            ),
            reject_fixture(
                8,
                "qg_reject_08_export_provenance_missing",
                "QuantGraphExportProvenanceMissing",
                "gbf-codegen::s1::quant_graph::tests::reject_export_provenance_missing_inputs",
                reject_export_provenance_missing_inputs,
            ),
            reject_fixture(
                9,
                "qg_reject_09_provenance_image_not_injective",
                "QuantGraphProvenanceImageNotInjective",
                "gbf-codegen::s1::quant_graph::tests::reject_provenance_image_not_injective_inputs",
                reject_provenance_image_not_injective_inputs,
            ),
            reject_fixture(
                10,
                "qg_reject_10_norm_plan_reference_unresolved",
                "QuantGraphNormPlanReferenceUnresolved",
                "gbf-codegen::s1::quant_graph::tests::reject_norm_plan_reference_unresolved_inputs",
                reject_norm_plan_reference_unresolved_inputs,
            ),
            reject_fixture(
                11,
                "qg_reject_11_expert_section_weight_missing",
                "QuantGraphExpertSectionWeightMissing",
                "gbf-codegen::s1::quant_graph::tests::reject_expert_section_weight_missing_inputs",
                reject_expert_section_weight_missing_inputs,
            ),
            reject_fixture(
                12,
                "qg_reject_12_classify_head_tied_mismatch",
                "QuantGraphClassifyHeadTiedMismatch",
                "gbf-codegen::s1::quant_graph::tests::reject_classify_head_tied_mismatch_inputs",
                reject_classify_head_tied_mismatch_inputs,
            ),
            reject_fixture(
                13,
                "qg_reject_13_classify_head_format_mismatch",
                "QuantGraphClassifyHeadFormatMismatch",
                "gbf-codegen::s1::quant_graph::tests::reject_classify_head_format_mismatch_inputs",
                reject_classify_head_format_mismatch_inputs,
            ),
            reject_fixture(
                14,
                "qg_reject_14_decode_spec_not_in_capability_set",
                "QuantGraphDecodeSpecNotInCapabilitySet",
                "gbf-codegen::s1::quant_graph::tests::reject_decode_spec_not_in_capability_set_inputs",
                reject_decode_spec_not_in_capability_set_inputs,
            ),
            reject_fixture(
                15,
                "qg_reject_15_sequence_semantics_tensor_mismatch",
                "QuantGraphSequenceSemanticsTensorMismatch",
                "gbf-codegen::s1::quant_graph::tests::reject_sequence_semantics_tensor_mismatch_inputs",
                reject_sequence_semantics_tensor_mismatch_inputs,
            ),
            reject_fixture(
                16,
                "qg_reject_16_layout_inconsistent_with_model_spec",
                "QuantGraphLayoutInconsistentWithModelSpec",
                "gbf-codegen::s1::quant_graph::tests::reject_layout_inconsistent_with_model_spec_inputs",
                reject_layout_inconsistent_with_model_spec_inputs,
            ),
            reject_fixture(
                17,
                "qg_reject_17_blob_ref_unresolvable",
                "QuantGraphBlobRefUnresolvable",
                "gbf-codegen::s1::quant_graph::tests::reject_blob_ref_unresolvable_inputs",
                reject_blob_ref_unresolvable_inputs,
            ),
            reject_fixture(
                18,
                "qg_reject_18_blob_ref_size_mismatch",
                "QuantGraphBlobRefSizeMismatch",
                "gbf-codegen::s1::quant_graph::tests::reject_blob_ref_size_mismatch_inputs",
                reject_blob_ref_size_mismatch_inputs,
            ),
            reject_fixture(
                19,
                "qg_reject_19_aux_blob_ref_size_mismatch",
                "QuantGraphAuxBlobRefSizeMismatch",
                "gbf-codegen::s1::quant_graph::tests::reject_aux_blob_ref_size_mismatch_inputs",
                reject_aux_blob_ref_size_mismatch_inputs,
            ),
            reject_fixture(
                20,
                "qg_reject_20_determinism_requires_enforced_reduction_order",
                "QuantGraphDeterminismRequiresEnforcedReductionOrder",
                "gbf-codegen::s1::quant_graph::tests::reject_determinism_requires_enforced_reduction_order_inputs",
                reject_determinism_requires_enforced_reduction_order_inputs,
            ),
            reject_fixture(
                21,
                "qg_reject_21_required_feature_unsupported",
                "QuantGraphRequiredFeatureUnsupported",
                "gbf-codegen::s1::quant_graph::tests::reject_required_feature_unsupported_inputs",
                reject_required_feature_unsupported_inputs,
            ),
            reject_fixture(
                22,
                "qg_reject_22_forbidden_storage_metadata",
                "QuantGraphForbiddenStorageMetadata",
                "gbf-codegen::s1::quant_graph::tests::reject_forbidden_storage_metadata_inputs",
                reject_forbidden_storage_metadata_inputs,
            ),
            reject_fixture(
                23,
                "qg_reject_23_embedding_missing",
                "QuantGraphEmbeddingMissing",
                "gbf-codegen::s1::quant_graph::tests::reject_embedding_missing_inputs",
                reject_embedding_missing_inputs,
            ),
            reject_fixture(
                24,
                "qg_reject_24_embedding_not_unique",
                "QuantGraphEmbeddingNotUnique",
                "gbf-codegen::s1::quant_graph::tests::reject_embedding_not_unique_inputs",
                reject_embedding_not_unique_inputs,
            ),
            reject_fixture(
                25,
                "qg_reject_25_ffn_gate_presence_mismatch",
                "QuantGraphFfnGatePresenceMismatch",
                "gbf-codegen::s1::quant_graph::tests::reject_ffn_gate_presence_mismatch_inputs",
                reject_ffn_gate_presence_mismatch_inputs,
            ),
            reject_fixture(
                26,
                "qg_reject_26_layer_norms_incomplete",
                "QuantGraphLayerNormsIncomplete",
                "gbf-codegen::s1::quant_graph::tests::reject_layer_norms_incomplete_inputs",
                reject_layer_norms_incomplete_inputs,
            ),
            reject_fixture(
                27,
                "qg_reject_27_final_norm_missing",
                "QuantGraphFinalNormMissing",
                "gbf-codegen::s1::quant_graph::tests::reject_final_norm_missing_inputs",
                reject_final_norm_missing_inputs,
            ),
            reject_fixture(
                28,
                "qg_reject_28_norm_site_duplicate",
                "QuantGraphNormSiteDuplicate",
                "gbf-codegen::s1::quant_graph::tests::reject_norm_site_duplicate_inputs",
                reject_norm_site_duplicate_inputs,
            ),
            reject_fixture(
                29,
                "qg_reject_29_aux_blob_kind_mismatch",
                "QuantGraphAuxBlobKindMismatch",
                "gbf-codegen::s1::quant_graph::tests::reject_aux_blob_kind_mismatch_inputs",
                reject_aux_blob_kind_mismatch_inputs,
            ),
            reject_fixture(
                30,
                "qg_reject_30_decode_requires_rng_mismatch",
                "QuantGraphDecodeRequiresRngMismatch",
                "gbf-codegen::s1::quant_graph::tests::reject_decode_requires_rng_mismatch_inputs",
                reject_decode_requires_rng_mismatch_inputs,
            ),
            reject_fixture(
                31,
                "qg_reject_31_router_gate_weight_semantics_unsupported",
                "QuantGraphRouterGateWeightSemanticsUnsupported",
                "gbf-codegen::s1::quant_graph::tests::reject_router_gate_weight_semantics_unsupported_inputs",
                reject_router_gate_weight_semantics_unsupported_inputs,
            ),
            reject_fixture(
                32,
                "qg_reject_32_residual_plan_invalid",
                "QuantGraphResidualPlanInvalid",
                "gbf-codegen::s1::quant_graph::tests::reject_residual_plan_invalid_inputs",
                reject_residual_plan_invalid_inputs,
            ),
            reject_fixture(
                33,
                "qg_reject_33_routing_expert_coverage_gap",
                "QuantGraphRoutingExpertCoverageGap",
                "gbf-codegen::s1::quant_graph::tests::reject_routing_expert_coverage_gap_inputs",
                reject_routing_expert_coverage_gap_inputs,
            ),
            reject_fixture(
                34,
                "qg_reject_34_routing_expert_coverage_extra",
                "QuantGraphRoutingExpertCoverageExtra",
                "gbf-codegen::s1::quant_graph::tests::reject_routing_expert_coverage_extra_inputs",
                reject_routing_expert_coverage_extra_inputs,
            ),
            reject_fixture(
                35,
                "qg_reject_35_bit_exact_mid_reduction_saturation_forbidden",
                "QuantGraphBitExactMidReductionSaturationForbidden",
                "gbf-codegen::s1::quant_graph::tests::reject_bit_exact_mid_reduction_saturation_forbidden_inputs",
                reject_bit_exact_mid_reduction_saturation_forbidden_inputs,
            ),
            reject_fixture(
                36,
                "qg_reject_36_router_tie_break_unsupported",
                "QuantGraphRouterTieBreakUnsupported",
                "gbf-codegen::s1::quant_graph::tests::reject_router_tie_break_unsupported_inputs",
                reject_router_tie_break_unsupported_inputs,
            ),
        ]
    }

    fn reject_fixture(
        qg_reject: u8,
        dir: &'static str,
        expected_code: &'static str,
        counterexample: &'static str,
        inputs: fn() -> QuantGraphInputs,
    ) -> RejectQuantGraphFixture {
        RejectQuantGraphFixture {
            qg_reject,
            dir,
            expected_code,
            counterexample,
            inputs,
        }
    }

    fn diagnostic_code_kind(code: &QuantGraphBindingDiagnosticCode) -> String {
        serde_json::to_value(code).expect("diagnostic serializes")["kind"]
            .as_str()
            .expect("diagnostic has kind")
            .to_owned()
    }

    fn reject_training_residue_inputs() -> QuantGraphInputs {
        let mut inputs = dense_toy0_inputs();
        inputs.training_residue_absent = false;
        inputs
    }

    fn reject_role_format_mismatch_inputs() -> QuantGraphInputs {
        let mut inputs = dense_toy0_inputs();
        inputs.tensor_bindings[0].quant_format = binary_format();
        inputs
    }

    fn reject_routing_missing_for_routed_layer_inputs() -> QuantGraphInputs {
        let mut inputs = routed_basic_one_inputs();
        inputs.router_layers.clear();
        inputs
    }

    fn reject_routing_present_for_dense_layer_inputs() -> QuantGraphInputs {
        let mut inputs = dense_toy0_inputs();
        inputs.router_layers.push(RouterLayerBindingInput {
            layer: LayerId::new(0),
            n_experts: 1,
            router_weight: TensorId::new(100),
            router_bias: None,
            semantics: RouterSemanticsBindingInput::Top1Hard {
                gate_weight: RouterGateWeightSemanticsBindingInput::One,
                tie_break: RouterTieBreakBindingInput::LowestExpertId,
            },
        });
        inputs
    }

    fn reject_routing_expert_coverage_mismatch_inputs() -> QuantGraphInputs {
        let mut inputs = routed_basic_one_inputs();
        inputs.router_layers[0].n_experts = 3;
        inputs
    }

    fn reject_tensor_id_not_unique_inputs() -> QuantGraphInputs {
        let mut inputs = dense_toy0_inputs();
        inputs.tensor_bindings[1].tensor_id = inputs.tensor_bindings[0].tensor_id;
        inputs
    }

    fn reject_identity_hash_mismatch_inputs() -> QuantGraphInputs {
        let mut inputs = dense_toy0_inputs();
        inputs.identity.semantic_core_hash = hash(0xee);
        inputs
    }

    fn reject_export_provenance_missing_inputs() -> QuantGraphInputs {
        let mut inputs = dense_toy0_inputs();
        inputs
            .tensor_exports
            .remove(&inputs.tensor_bindings[0].tensor_id);
        inputs
    }

    fn reject_provenance_image_not_injective_inputs() -> QuantGraphInputs {
        let mut inputs = dense_toy0_inputs();
        let duplicate = inputs
            .tensor_exports
            .get(&inputs.tensor_bindings[0].tensor_id)
            .expect("fixture has first export")
            .clone();
        inputs
            .tensor_exports
            .insert(inputs.tensor_bindings[1].tensor_id, duplicate);
        inputs
    }

    fn reject_norm_plan_reference_unresolved_inputs() -> QuantGraphInputs {
        let mut inputs = dense_toy0_inputs();
        let tensor = tensor_with_layout_role(
            40,
            QuantTensorRole::NormScale {
                norm_plan: NormPlanId::new(99),
            },
            QuantFormat::Q8_8,
            &[8],
            16,
            Vec::new(),
        );
        push_tensor_input(&mut inputs, tensor, "tensor.bad_norm_scale");
        inputs
    }

    fn reject_expert_section_weight_missing_inputs() -> QuantGraphInputs {
        let mut inputs = dense_toy0_inputs();
        inputs
            .tensor_bindings
            .retain(|binding| binding.tensor_id != TensorId::new(3));
        inputs.tensor_exports.remove(&TensorId::new(3));
        inputs
    }

    fn reject_classify_head_tied_mismatch_inputs() -> QuantGraphInputs {
        let mut inputs = dense_toy0_inputs();
        inputs.classify_head.weight = TensorId::new(2);
        inputs
    }

    fn reject_classify_head_format_mismatch_inputs() -> QuantGraphInputs {
        let mut inputs = dense_toy0_inputs();
        inputs.classify_head.logit_format = ternary_format();
        inputs
    }

    fn reject_decode_spec_not_in_capability_set_inputs() -> QuantGraphInputs {
        let mut inputs = dense_toy0_inputs();
        inputs.decode_caps = decode_caps([DecodeMode::TopKTemperature]);
        inputs
    }

    fn reject_sequence_semantics_tensor_mismatch_inputs() -> QuantGraphInputs {
        let mut inputs = dense_toy0_inputs();
        inputs.sequence_semantics_tensors_match = false;
        inputs
    }

    fn reject_layout_inconsistent_with_model_spec_inputs() -> QuantGraphInputs {
        let mut inputs = dense_toy0_inputs();
        inputs.tensor_bindings[0].layout = layout(&[1, 8]);
        replace_tensor_blob(&mut inputs, 0, 0x41, 8);
        inputs
    }

    fn reject_blob_ref_unresolvable_inputs() -> QuantGraphInputs {
        let mut inputs = dense_toy0_inputs();
        let blob_ref = inputs.tensor_bindings[0].blob_ref;
        let mut entries = inputs.resolved_blob_index.entries.clone();
        entries.remove(&blob_ref);
        inputs.resolved_blob_index = ResolvedBlobIndex::new(entries);
        inputs
    }

    fn reject_blob_ref_size_mismatch_inputs() -> QuantGraphInputs {
        let mut inputs = dense_toy0_inputs();
        let blob_ref = inputs.tensor_bindings[0].blob_ref;
        replace_blob_metadata(&mut inputs, blob_ref, 1);
        inputs
    }

    fn reject_aux_blob_ref_size_mismatch_inputs() -> QuantGraphInputs {
        let mut inputs = dense_toy0_inputs();
        let blob_ref = inputs.tensor_bindings[1].aux_blob_refs[0].blob_ref;
        replace_blob_metadata(&mut inputs, blob_ref, 1);
        inputs
    }

    fn reject_determinism_requires_enforced_reduction_order_inputs() -> QuantGraphInputs {
        let mut inputs = dense_toy0_inputs();
        inputs.reduction_order_policy_enforced = false;
        inputs
    }

    fn reject_required_feature_unsupported_inputs() -> QuantGraphInputs {
        let mut inputs = dense_toy0_inputs();
        inputs.required_features_supported = false;
        inputs
    }

    fn reject_forbidden_storage_metadata_inputs() -> QuantGraphInputs {
        let mut inputs = dense_toy0_inputs();
        inputs.forbidden_storage_metadata_absent = false;
        inputs
    }

    fn reject_embedding_missing_inputs() -> QuantGraphInputs {
        let mut inputs = dense_toy0_inputs();
        inputs.tensor_bindings[0].role = QuantTensorRole::ClassifyWeight;
        inputs.classify_head = ClassifyHead {
            kind: ClassifyHeadKind::Untied,
            weight: inputs.tensor_bindings[0].tensor_id,
            bias: None,
            logit_format: QuantFormat::Q8_8,
        };
        inputs
    }

    fn reject_embedding_not_unique_inputs() -> QuantGraphInputs {
        let mut inputs = dense_toy0_inputs();
        let tensor = tensor_with_layout_role(
            40,
            QuantTensorRole::EmbeddingTable,
            QuantFormat::I8,
            &[80, 8],
            640,
            Vec::new(),
        );
        push_tensor_input(&mut inputs, tensor, "tensor.embedding.duplicate");
        inputs
    }

    fn reject_ffn_gate_presence_mismatch_inputs() -> QuantGraphInputs {
        let mut inputs = dense_toy0_inputs();
        inputs
            .ffn_plans
            .get_mut(&LayerId::new(0))
            .expect("fixture has layer 0 ffn plan")
            .activation_kind = FfnActivationKind::SwiGLU;
        inputs
    }

    fn reject_layer_norms_incomplete_inputs() -> QuantGraphInputs {
        let mut inputs = dense_toy0_inputs();
        inputs
            .norm_plan_bindings
            .retain(|binding| !matches!(binding.site, NormSite::LayerFfn { .. }));
        inputs
    }

    fn reject_final_norm_missing_inputs() -> QuantGraphInputs {
        let mut inputs = dense_toy0_inputs();
        inputs
            .norm_plan_bindings
            .retain(|binding| binding.site != NormSite::Final);
        inputs
    }

    fn reject_norm_site_duplicate_inputs() -> QuantGraphInputs {
        let mut inputs = dense_toy0_inputs();
        let duplicate = inputs.norm_plan_bindings[0].clone();
        inputs.norm_plan_bindings.push(duplicate);
        inputs
    }

    fn reject_aux_blob_kind_mismatch_inputs() -> QuantGraphInputs {
        let mut inputs = dense_toy0_inputs();
        inputs.tensor_bindings[1].aux_blob_refs.clear();
        inputs
    }

    fn reject_decode_requires_rng_mismatch_inputs() -> QuantGraphInputs {
        let mut inputs = dense_toy0_inputs();
        inputs.decode_requires_rng_matches_spec = false;
        inputs
    }

    fn reject_router_gate_weight_semantics_unsupported_inputs() -> QuantGraphInputs {
        let mut inputs = routed_basic_one_inputs();
        inputs.router_layers[0].semantics = RouterSemanticsBindingInput::Top1Hard {
            gate_weight: RouterGateWeightSemanticsBindingInput::UnsupportedV1,
            tie_break: RouterTieBreakBindingInput::LowestExpertId,
        };
        inputs
    }

    fn reject_residual_plan_invalid_inputs() -> QuantGraphInputs {
        let mut inputs = dense_toy0_inputs();
        inputs.residual_plan.activation_format = ternary_format();
        inputs
    }

    fn reject_routing_expert_coverage_gap_inputs() -> QuantGraphInputs {
        let mut inputs = routed_basic_one_inputs();
        inputs.tensor_bindings.retain(|binding| {
            !matches!(
                binding.role,
                QuantTensorRole::ExpertWeight {
                    layer,
                    expert,
                    ..
                } if layer == LayerId::new(0) && expert == ExpertId::new(3)
            )
        });
        inputs.tensor_exports.retain(|tensor_id, _| {
            inputs
                .tensor_bindings
                .iter()
                .any(|binding| binding.tensor_id == *tensor_id)
        });
        inputs
    }

    fn reject_routing_expert_coverage_extra_inputs() -> QuantGraphInputs {
        let mut graph = self_consistent_graph_fixture();
        push_expert_pair(&mut graph, 0, 1, 40, 41);
        quant_graph_inputs_from_graph(graph)
    }

    fn reject_bit_exact_mid_reduction_saturation_forbidden_inputs() -> QuantGraphInputs {
        let mut inputs = dense_toy0_inputs();
        inputs.bit_exact_mid_reduction_saturation_absent = false;
        inputs
    }

    fn reject_router_tie_break_unsupported_inputs() -> QuantGraphInputs {
        let mut inputs = routed_basic_one_inputs();
        inputs.router_layers[0].semantics = RouterSemanticsBindingInput::Top1Hard {
            gate_weight: RouterGateWeightSemanticsBindingInput::One,
            tie_break: RouterTieBreakBindingInput::UnsupportedV1,
        };
        inputs
    }

    fn replace_blob_metadata(
        inputs: &mut QuantGraphInputs,
        blob_ref: BlobRef,
        decoded_size_bytes: u64,
    ) {
        let mut entries = inputs.resolved_blob_index.entries.clone();
        let metadata = entries
            .get_mut(&blob_ref)
            .expect("fixture blob index contains blob");
        metadata.decoded_size_bytes = decoded_size_bytes;
        inputs.resolved_blob_index = ResolvedBlobIndex::new(entries);
    }

    fn replace_tensor_blob(
        inputs: &mut QuantGraphInputs,
        tensor_index: usize,
        hash_byte: u8,
        decoded_size_bytes: u64,
    ) {
        let blob_ref = blob_ref(hash_byte, decoded_size_bytes as u32, BlobCodec::Raw);
        inputs.tensor_bindings[tensor_index].blob_ref = blob_ref;
        let mut entries = inputs.resolved_blob_index.entries.clone();
        entries.insert(
            blob_ref,
            BlobMetadata {
                content_hash: hash(hash_byte),
                encoded_size_bytes: decoded_size_bytes,
                decoded_size_bytes,
                codec: BlobCodec::Raw,
            },
        );
        inputs.resolved_blob_index = ResolvedBlobIndex::new(entries);
    }

    fn push_tensor_input(
        inputs: &mut QuantGraphInputs,
        tensor: QuantTensorRef,
        export_tensor_path: &str,
    ) {
        let mut entries = inputs.resolved_blob_index.entries.clone();
        entries.insert(tensor.blob.blob_ref, metadata_from_resolved(&tensor.blob));
        for aux in &tensor.aux_blob_refs {
            entries.insert(aux.blob.blob_ref, metadata_from_resolved(&aux.blob));
        }
        inputs.resolved_blob_index = ResolvedBlobIndex::new(entries);
        inputs
            .tensor_exports
            .insert(tensor.tensor_id, export_tensor_id(export_tensor_path));
        inputs.tensor_bindings.push(QuantTensorBindingInput {
            tensor_id: tensor.tensor_id,
            layout: tensor.layout,
            quant_format: tensor.quant_format,
            role: tensor.role,
            blob_ref: tensor.blob.blob_ref,
            aux_blob_refs: tensor
                .aux_blob_refs
                .into_iter()
                .map(|aux| QuantAuxBlobBindingInput {
                    kind: aux.kind,
                    layout: aux.layout,
                    format: aux.format,
                    blob_ref: aux.blob.blob_ref,
                    export_tensor_id: aux.export_tensor_id,
                })
                .collect(),
        });
    }

    fn fixture_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/quant_graph")
    }

    fn fixture_path(parts: &[&str]) -> PathBuf {
        let mut path = fixture_root();
        for part in parts {
            path.push(part);
        }
        path
    }

    fn read_fixture_file(parts: &[&str]) -> String {
        let path = fixture_path(parts);
        std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("failed to read fixture {}: {err}", path.display()))
    }

    fn quant_graph_fixture_files() -> Vec<PathBuf> {
        fn collect(path: PathBuf, files: &mut Vec<PathBuf>) {
            for entry in std::fs::read_dir(&path).unwrap_or_else(|err| {
                panic!("failed to read fixture dir {}: {err}", path.display())
            }) {
                let entry = entry.expect("fixture dir entry");
                let path = entry.path();
                if path.is_dir() {
                    collect(path, files);
                } else {
                    files.push(path);
                }
            }
        }

        let mut files = Vec::new();
        collect(fixture_root(), &mut files);
        files
    }

    fn reject_fixture_dirs() -> BTreeSet<String> {
        std::fs::read_dir(fixture_path(&["reject"]))
            .expect("reject fixture dir reads")
            .map(|entry| entry.expect("reject dir entry").path())
            .filter(|path| path.is_dir())
            .map(|path| {
                path.file_name()
                    .expect("reject dir has name")
                    .to_str()
                    .expect("reject dir is utf-8")
                    .to_owned()
            })
            .collect()
    }

    fn quant_graph_inputs_fixture() -> QuantGraphInputs {
        quant_graph_inputs_from_graph(self_consistent_graph_fixture())
    }

    fn quant_graph_inputs_from_graph(graph: QuantGraph) -> QuantGraphInputs {
        let tensor_bindings = graph
            .tensors
            .iter()
            .map(|tensor| QuantTensorBindingInput {
                tensor_id: tensor.tensor_id,
                layout: tensor.layout.clone(),
                quant_format: tensor.quant_format.clone(),
                role: tensor.role.clone(),
                blob_ref: tensor.blob.blob_ref,
                aux_blob_refs: tensor
                    .aux_blob_refs
                    .iter()
                    .map(|aux| QuantAuxBlobBindingInput {
                        kind: aux.kind,
                        layout: aux.layout.clone(),
                        format: aux.format,
                        blob_ref: aux.blob.blob_ref,
                        export_tensor_id: aux.export_tensor_id.clone(),
                    })
                    .collect(),
            })
            .collect::<Vec<_>>();
        let norm_plan_bindings = graph
            .norm_plans
            .iter()
            .map(|record| NormPlanBindingInput {
                site: record.site,
                plan: record.plan.clone(),
                input_format: record.input_format.clone(),
                output_format: record.output_format.clone(),
            })
            .collect::<Vec<_>>();
        let mut blob_entries = BTreeMap::new();
        for tensor in &graph.tensors {
            blob_entries.insert(tensor.blob.blob_ref, metadata_from_resolved(&tensor.blob));
            for aux in &tensor.aux_blob_refs {
                blob_entries.insert(aux.blob.blob_ref, metadata_from_resolved(&aux.blob));
            }
        }
        let sequence_semantics = graph.sequence_semantics.clone();

        QuantGraphInputs {
            identity: IdentityBindingInputs {
                artifact_core_hash: graph.identity.artifact_core_hash,
                policy_resolution_self_hash: graph.identity.policy_resolution_self_hash,
                artifact_validation_self_hash: graph.identity.artifact_validation_self_hash,
                semantic_core_hash: graph.identity.semantic_core_hash,
                validated_artifact_effective_core_hash: graph.identity.semantic_core_hash,
                lowering_manifest_hash: graph.identity.lowering_manifest_hash,
                artifact_determinism: graph.identity.determinism,
                policy_determinism: None,
                model_spec_summary: graph.identity.model_spec_summary.clone(),
            },
            resolved_blob_index: ResolvedBlobIndex::new(blob_entries),
            tensor_bindings,
            norm_plan_bindings,
            router_layers: graph
                .routing_table
                .as_ref()
                .map(|routing| {
                    routing
                        .layers
                        .iter()
                        .map(|layer| RouterLayerBindingInput {
                            layer: layer.layer,
                            n_experts: layer.n_experts,
                            router_weight: layer.router_weight,
                            router_bias: layer.router_bias,
                            semantics: match layer.semantics {
                                RouterSemantics::Top1Hard {
                                    gate_weight,
                                    tie_break: RouterTieBreak::LowestExpertId,
                                } => RouterSemanticsBindingInput::Top1Hard {
                                    gate_weight: match gate_weight {
                                        RouterGateWeightSemantics::One => {
                                            RouterGateWeightSemanticsBindingInput::One
                                        }
                                        RouterGateWeightSemantics::SelectedScore => {
                                            RouterGateWeightSemanticsBindingInput::SelectedScore
                                        }
                                    },
                                    tie_break: RouterTieBreakBindingInput::LowestExpertId,
                                },
                            },
                        })
                        .collect()
                })
                .unwrap_or_default(),
            ffn_plans: graph.ffn_plans.clone(),
            decode_plan_id: graph.decode_spec.decode_plan_id,
            decode_source: DecodeBindingSource::Explicit {
                spec: graph.decode_spec.spec.clone(),
            },
            decode_caps: decode_caps([DecodeMode::Argmax]),
            sequence_semantics: SequenceSemanticsBindingInputs {
                artifact_sequence: sequence_semantics.clone(),
                requested_sequence: sequence_semantics,
            },
            residual_plan: ResidualPlanInput {
                activation_format: graph.residual_plan.activation_format.clone(),
                combine_policy: graph.residual_plan.combine_policy,
            },
            classify_head: graph.classify_head.clone(),
            tensor_exports: graph.provenance.clone(),
            training_residue_absent: true,
            sequence_semantics_tensors_match: true,
            required_features_supported: true,
            reduction_order_policy_enforced: true,
            bit_exact_mid_reduction_saturation_absent: true,
            forbidden_storage_metadata_absent: true,
            decode_requires_rng_matches_spec: true,
        }
    }

    fn metadata_from_resolved(blob: &ResolvedBlobRef) -> BlobMetadata {
        BlobMetadata {
            content_hash: blob.content_hash,
            encoded_size_bytes: blob.encoded_size_bytes,
            decoded_size_bytes: blob.decoded_size_bytes,
            codec: blob.codec,
        }
    }

    struct Resolver {
        blobs: BTreeMap<BlobRef, Vec<u8>>,
    }

    impl Resolver {
        fn from_inputs(inputs: &QuantGraphInputs) -> Self {
            let blobs = quant_graph_blob_refs(inputs)
                .into_iter()
                .map(|blob_ref| (blob_ref, vec![0_u8; blob_ref.len as usize]))
                .collect();
            Self { blobs }
        }
    }

    impl ArtifactResolver for Resolver {
        fn resolve_blob(&self, blob: &BlobRef) -> Result<ResolvedBlob, ArtifactResolveError> {
            let Some(bytes) = self.blobs.get(blob) else {
                return Err(ArtifactResolveError::not_found(blob.hash.to_string()));
            };
            Ok(ResolvedBlob {
                bytes: bytes.clone(),
                content_hash: blob.hash,
            })
        }

        fn resolve_sidecar(
            &self,
            _sidecar: &SidecarRef,
        ) -> Result<ResolvedSidecar, ArtifactResolveError> {
            Err(ArtifactResolveError::unsupported(
                "Stage 1 tests do not resolve sidecars",
            ))
        }

        fn resolve_evidence(
            &self,
            _evidence: &EvidenceRef,
        ) -> Result<ResolvedEvidence, ArtifactResolveError> {
            Err(ArtifactResolveError::unsupported(
                "Stage 1 tests do not resolve evidence",
            ))
        }

        fn resolve_workload(
            &self,
            _workload: &WorkloadManifestRef,
        ) -> Result<ResolvedWorkload, ArtifactResolveError> {
            Err(ArtifactResolveError::unsupported(
                "Stage 1 tests do not resolve workloads",
            ))
        }

        fn resolve_golden_vector(
            &self,
            _vector: &GoldenVectorRef,
        ) -> Result<ResolvedGoldenVector, ArtifactResolveError> {
            Err(ArtifactResolveError::unsupported(
                "Stage 1 tests do not resolve golden vectors",
            ))
        }
    }

    fn store() -> (tempfile::TempDir, BlobStore) {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = BlobStore::open(dir.path().to_path_buf()).expect("blob store");
        (dir, store)
    }
}
