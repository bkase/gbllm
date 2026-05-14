//! Stage 3 `GbInferIR` public type surface.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::io;
use std::path::Path;
use std::time::Instant;

use gbf_artifact::norm_plan::NormPlan;
use gbf_foundation::{ExpertId, FieldPath, Hash256, LayerId};
use gbf_policy::{
    ReductionSiteId, RuntimeMode, ValidationCode, ValidationDetail, ValidationDiagnostic,
    ValidationOrigin,
};
use gbf_report::canonical_json::canonicalize_value;
use gbf_report::report_schemas::{
    infer_ir_v1 as report_schema, quant_graph_v1 as qg_report_schema,
};
use gbf_report::{ReportEnvelope, ReportOutcome, canonicalize as canonicalize_report};
use gbf_store::stage_cache::compose_key;
use serde::de;
use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};

use crate::budget::StaticBudgetReport;
use crate::policy::ResolvedPolicyProduct;
use crate::s1::quant_graph::{
    DecodePlanId, DeterminismClass, ExpertWeightSlot, FfnActivationKind, FfnKindTag, NormPlanId,
    NormSite, PassEnvironment, QuantFormat, QuantGraph, QuantTensorRole, TensorId,
};
use crate::stage_cache::{
    CachedReportBytes, Stage3CacheCell, Stage3CacheKeyMaterial, Stage3CellKind,
    get_stage3_failure_memo, get_stage3_success, materialize_stage3_cached_report,
    put_stage3_failure_memo, put_stage3_success, rewrap_stage3_cached_report_audit_parents,
    stage3_infer_ir_store_key,
};

pub const INFER_IR_SCHEMA_ID: &str = "infer_ir.v1";
pub const INFER_IR_SCHEMA_VERSION: &str = "1.0.0";
pub const PASS_VERSION_INFER_IR: &str = "1.0.0";
pub const STAGE3_SEMANTIC_EQUIVALENCE_RUN_EVENT: &str = "stage3.semantic_equivalence.run";
pub const STAGE3_SEMANTIC_EQUIVALENCE_FAILED_EVENT: &str = "stage3.semantic_equivalence.failed";

#[cfg(any(feature = "semantic_equivalence_check", test))]
const SEMANTIC_EQUIVALENCE_NUMERIC_BOUNDARIES: &[&str] = &[
    "residual_plan.activation_format",
    "norm_plan.input_format",
    "norm_plan.output_format",
    "decode_spec.requires_rng",
];

/// Stage 3's policy-facing projection. Audit parent hashes stay outside this
/// structure so K3 and `InferIrIdentity` move only when IR-shaping policy moves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InferIrPolicyProjection {
    pub requested_runtime_modes: BTreeSet<RuntimeMode>,
    pub allowed_ingress_modes: BTreeSet<TokenIngressMode>,
}

impl InferIrPolicyProjection {
    #[must_use]
    pub fn from_resolved_policy(product: &ResolvedPolicyProduct) -> Self {
        Self {
            requested_runtime_modes: product
                .policy
                .effective_constraints
                .requested_runtime_modes
                .clone(),
            allowed_ingress_modes: BTreeSet::from([
                TokenIngressMode::AutoRegressive,
                TokenIngressMode::Prompt,
            ]),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InferIrAuditParents {
    pub policy_resolution_self_hash: Hash256,
    pub compile_request_hash: Hash256,
}

pub trait StaticBudgetReportSelfHash {
    fn report_self_hash(&self) -> Hash256;
}

impl StaticBudgetReportSelfHash for StaticBudgetReport {
    fn report_self_hash(&self) -> Hash256 {
        self.report.report_self_hash
    }
}

pub trait StaticBudgetReductionSites {
    fn reduction_site_ids(&self) -> Vec<ReductionSiteId>;
}

impl StaticBudgetReductionSites for StaticBudgetReport {
    fn reduction_site_ids(&self) -> Vec<ReductionSiteId> {
        self.report
            .body
            .projections
            .accumulator_maxima
            .iter()
            .map(|bound| bound.site.clone())
            .collect()
    }
}

pub struct GbInferIRInputs<'a, B: StaticBudgetReportSelfHash + ?Sized = StaticBudgetReport> {
    pub quant_graph: &'a QuantGraph,
    pub quant_graph_self_hash: Hash256,
    pub policy_projection: InferIrPolicyProjection,
    pub audit_parents: InferIrAuditParents,
    pub static_budget: &'a B,
    pub static_budget_self_hash: Hash256,
}

impl<'a, B: StaticBudgetReportSelfHash + ?Sized> Clone for GbInferIRInputs<'a, B> {
    fn clone(&self) -> Self {
        Self {
            quant_graph: self.quant_graph,
            quant_graph_self_hash: self.quant_graph_self_hash,
            policy_projection: self.policy_projection.clone(),
            audit_parents: self.audit_parents,
            static_budget: self.static_budget,
            static_budget_self_hash: self.static_budget_self_hash,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct GbInferIRStageFailure {
    pub kind: GbInferIRStageFailureKind,
    pub message: String,
    pub report: Option<Box<ReportEnvelope<report_schema::InferIrReportBody<GbInferIR>>>>,
    pub diagnostics: Vec<ValidationDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GbInferIRStageFailureKind {
    Rejected,
    CacheHitFailureMemo,
    Product,
    ReportIo,
    StageCache,
}

impl fmt::Display for GbInferIRStageFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for GbInferIRStageFailure {}

impl GbInferIRStageFailure {
    fn rejected<B: StaticBudgetReportSelfHash + ?Sized>(
        inputs: &GbInferIRInputs<'_, B>,
        diagnostics: Vec<ValidationDiagnostic>,
    ) -> Self {
        let report = infer_ir_failure_report(inputs, diagnostics.clone())
            .ok()
            .map(Box::new);
        Self {
            kind: GbInferIRStageFailureKind::Rejected,
            message: "Stage 3 InferIR binding rejected the inputs".to_owned(),
            report,
            diagnostics,
        }
    }

    fn cache_hit_failure_memo(
        report: ReportEnvelope<report_schema::InferIrReportBody<GbInferIR>>,
        diagnostics: Vec<ValidationDiagnostic>,
    ) -> Self {
        Self {
            kind: GbInferIRStageFailureKind::CacheHitFailureMemo,
            message: "Stage 3 InferIR failure memo replayed from cache".to_owned(),
            report: Some(Box::new(report)),
            diagnostics,
        }
    }

    fn product<B: StaticBudgetReportSelfHash + ?Sized>(
        inputs: &GbInferIRInputs<'_, B>,
        err: GbInferIrProductError,
    ) -> Self {
        let diagnostics = vec![hard_semantic_diagnostic(
            "infer_ir.report",
            ValidationDetail::Field {
                field: FieldPath::from("infer_ir.report"),
            },
        )];
        let report = infer_ir_failure_report(inputs, diagnostics.clone())
            .ok()
            .map(Box::new);
        Self {
            kind: GbInferIRStageFailureKind::Product,
            message: err.to_string(),
            report,
            diagnostics,
        }
    }

    fn report_io(message: impl Into<String>) -> Self {
        Self {
            kind: GbInferIRStageFailureKind::ReportIo,
            message: message.into(),
            report: None,
            diagnostics: Vec::new(),
        }
    }

    fn stage_cache(err: crate::stage_cache::CodegenStageCacheError) -> Self {
        Self {
            kind: GbInferIRStageFailureKind::StageCache,
            message: err.to_string(),
            report: None,
            diagnostics: Vec::new(),
        }
    }
}

#[allow(clippy::result_large_err)]
pub fn build_infer_ir_core<B>(
    inputs: GbInferIRInputs<'_, B>,
) -> Result<GbInferIRProduct, GbInferIRStageFailure>
where
    B: StaticBudgetReportSelfHash + StaticBudgetReductionSites + ?Sized,
{
    let wave4 = bind_wave4_classes_1_to_5(&inputs)
        .map_err(|diagnostics| GbInferIRStageFailure::rejected(&inputs, diagnostics))?;
    let infer_ir = bind_wave4_classes_6_to_10(&inputs, wave4)
        .map_err(|diagnostics| GbInferIRStageFailure::rejected(&inputs, diagnostics))?;
    let fixture_equivalence =
        fixture_semantic_equivalence(&infer_ir, inputs.quant_graph, &stage3_fixture_inputs())
            .map_err(|diagnostics| GbInferIRStageFailure::rejected(&inputs, diagnostics))?;
    GbInferIRProduct::new(
        infer_ir,
        inputs.audit_parents,
        inputs.policy_projection.requested_runtime_modes.clone(),
        report_schema::FixtureEquivalenceTag::from(fixture_equivalence),
    )
    .map_err(|err| GbInferIRStageFailure::product(&inputs, err))
}

#[allow(clippy::result_large_err)]
pub fn run_stage3<B>(
    inputs: GbInferIRInputs<'_, B>,
    env: PassEnvironment<'_>,
) -> Result<GbInferIRProduct, GbInferIRStageFailure>
where
    B: StaticBudgetReportSelfHash + StaticBudgetReductionSites + ?Sized,
{
    let started = Instant::now();
    let material = stage3_cache_key_material(&inputs)
        .map_err(|diagnostics| GbInferIRStageFailure::rejected(&inputs, diagnostics))?;

    if let Some(cache) = env.stage_cache {
        let success_key = compose_key(&stage3_infer_ir_store_key(
            &material,
            Stage3CellKind::Success,
        ));
        tracing::debug!(
            k3_hash = ?success_key,
            state = "lookup_success",
            "stage3.driver.cache_lookup"
        );
        if let Some(cell) =
            get_stage3_success(cache, &material).map_err(GbInferIRStageFailure::stage_cache)?
        {
            let cached_report = materialize_stage3_cached_report(&cell);
            let refreshed_report = decode_and_rewrap_cached_stage3_report(
                &cached_report.canonical_bytes,
                inputs.audit_parents,
            )?;
            let report_bytes = canonicalize_infer_ir_report(&refreshed_report)?;
            emit_infer_ir_report_bytes(env.report_dir, &report_bytes)?;
            if let Stage3CacheCell::InferIrSuccess { product, .. } = cell {
                let mut product = *product;
                product.report = refreshed_report;
                tracing::info!(
                    infer_ir_self_hash = %product.infer_ir_self_hash,
                    cache_state = "hit_success",
                    audit_rewrap = true,
                    total_ms = started.elapsed().as_millis() as u64,
                    "stage3.driver.run"
                );
                return Ok(product);
            }
        }

        let failure_key = compose_key(&stage3_infer_ir_store_key(
            &material,
            Stage3CellKind::FailureMemo,
        ));
        tracing::debug!(
            k3_hash = ?failure_key,
            state = "lookup_failure_memo",
            "stage3.driver.cache_lookup"
        );
        if let Some(cell) =
            get_stage3_failure_memo(cache, &material).map_err(GbInferIRStageFailure::stage_cache)?
        {
            let cached_report = materialize_stage3_cached_report(&cell);
            let refreshed_report = decode_and_rewrap_cached_stage3_report(
                &cached_report.canonical_bytes,
                inputs.audit_parents,
            )?;
            let report_bytes = canonicalize_infer_ir_report(&refreshed_report)?;
            emit_infer_ir_report_bytes(env.report_dir, &report_bytes)?;
            if let Stage3CacheCell::FailureMemo { diagnostics, .. } = cell {
                tracing::info!(
                    cache_state = "hit_failure_memo",
                    audit_rewrap = true,
                    total_ms = started.elapsed().as_millis() as u64,
                    "stage3.driver.run"
                );
                return Err(GbInferIRStageFailure::cache_hit_failure_memo(
                    refreshed_report,
                    diagnostics,
                ));
            }
        }
    }

    match build_infer_ir_core(inputs.clone()) {
        Ok(product) => {
            let report_bytes = canonicalize_infer_ir_report(&product.report)?;
            emit_infer_ir_report_bytes(env.report_dir, &report_bytes)?;
            if let Some(cache) = env.stage_cache {
                put_stage3_success(cache, &material, &product, report_bytes)
                    .map_err(GbInferIRStageFailure::stage_cache)?;
            }
            tracing::info!(
                infer_ir_self_hash = %product.infer_ir_self_hash,
                cache_state = "miss_success",
                audit_rewrap = false,
                total_ms = started.elapsed().as_millis() as u64,
                "stage3.driver.run"
            );
            Ok(product)
        }
        Err(failure) => {
            if let Some(report) = failure.report.as_deref() {
                let report_bytes = canonicalize_infer_ir_report(report)?;
                emit_infer_ir_report_bytes(env.report_dir, &report_bytes)?;
                if let Some(cache) = env.stage_cache {
                    put_stage3_failure_memo(
                        cache,
                        &material,
                        CachedReportBytes {
                            report_self_hash: report.report_self_hash,
                            canonical_bytes: report_bytes,
                        },
                        failure.diagnostics.clone(),
                    )
                    .map_err(GbInferIRStageFailure::stage_cache)?;
                }
            }
            tracing::info!(
                cache_state = "miss_failure",
                audit_rewrap = false,
                total_ms = started.elapsed().as_millis() as u64,
                "stage3.driver.run"
            );
            Err(failure)
        }
    }
}

pub fn stage3_cache_key_material<B: StaticBudgetReportSelfHash + ?Sized>(
    inputs: &GbInferIRInputs<'_, B>,
) -> Result<Stage3CacheKeyMaterial, Vec<ValidationDiagnostic>> {
    let mut diagnostics = Vec::new();
    let Some(infer_ir_policy_projection_hash) =
        hash_infer_ir_policy_projection(&inputs.policy_projection, &mut diagnostics)
    else {
        return Err(diagnostics);
    };
    Ok(Stage3CacheKeyMaterial::new(
        inputs.quant_graph_self_hash,
        infer_ir_policy_projection_hash,
        inputs.static_budget_self_hash,
    ))
}

fn stage3_fixture_inputs() -> FixtureInputSet {
    FixtureInputSet::fixture(vec![stage3_fixture_input(0), stage3_fixture_input(7)])
}

fn stage3_fixture_input(token_id: u32) -> FixtureInput {
    FixtureInput {
        token_id,
        sequence_state_seed: Hash256::from_bytes([0x61; 32]),
        rng_seed: Hash256::from_bytes([0x62; 32]),
    }
}

fn infer_ir_failure_report<B: StaticBudgetReportSelfHash + ?Sized>(
    inputs: &GbInferIRInputs<'_, B>,
    diagnostics: Vec<ValidationDiagnostic>,
) -> Result<ReportEnvelope<report_schema::InferIrReportBody<GbInferIR>>, GbInferIrProductError> {
    ReportEnvelope::new(
        ReportOutcome::Failed,
        report_schema::InferIrReportBody::<GbInferIR>::new(
            infer_ir_input_identity_from_inputs(inputs)?,
            None,
            diagnostics,
        ),
    )
    .map_err(|err| GbInferIrProductError::ReportEnvelope(err.to_string()))?
    .with_computed_self_hash()
    .map_err(|err| GbInferIrProductError::ReportSelfHash(err.to_string()))
}

fn infer_ir_input_identity_from_inputs<B: StaticBudgetReportSelfHash + ?Sized>(
    inputs: &GbInferIRInputs<'_, B>,
) -> Result<report_schema::InferIrInputIdentity, GbInferIrProductError> {
    let mut diagnostics = Vec::new();
    let Some(requested_runtime_modes_hash) = hash_requested_runtime_modes(
        &inputs.policy_projection.requested_runtime_modes,
        &mut diagnostics,
    ) else {
        return Err(GbInferIrProductError::CanonicalJson(format!(
            "failed to hash requested runtime modes for failed infer_ir report: {diagnostics:?}"
        )));
    };
    Ok(report_schema::InferIrInputIdentity {
        quant_graph_self_hash: inputs.quant_graph_self_hash,
        policy_resolution_self_hash: inputs.audit_parents.policy_resolution_self_hash,
        compile_request_hash: inputs.audit_parents.compile_request_hash,
        static_budget_self_hash: inputs.static_budget_self_hash,
        requested_runtime_modes_hash,
        determinism: report_determinism_class(inputs.quant_graph.identity.determinism),
        requested_runtime_modes: inputs.policy_projection.requested_runtime_modes.clone(),
    })
}

fn decode_and_rewrap_cached_stage3_report(
    report_bytes: &[u8],
    audit_parents: InferIrAuditParents,
) -> Result<ReportEnvelope<report_schema::InferIrReportBody<GbInferIR>>, GbInferIRStageFailure> {
    let report = serde_json::from_slice::<
        ReportEnvelope<report_schema::InferIrReportBody<GbInferIR>>,
    >(report_bytes)
    .map_err(|err| {
        GbInferIRStageFailure::report_io(format!(
            "cached Stage 3 infer_ir report did not decode: {err}"
        ))
    })?;
    rewrap_stage3_cached_report_audit_parents(&report, audit_parents).map_err(|err| {
        GbInferIRStageFailure::report_io(format!(
            "cached Stage 3 infer_ir report audit rewrap failed: {err}"
        ))
    })
}

fn canonicalize_infer_ir_report(
    report: &ReportEnvelope<report_schema::InferIrReportBody<GbInferIR>>,
) -> Result<Vec<u8>, GbInferIRStageFailure> {
    canonicalize_report(report).map_err(|err| {
        GbInferIRStageFailure::report_io(format!(
            "Stage 3 infer_ir report did not canonicalize: {err}"
        ))
    })
}

fn emit_infer_ir_report_bytes(
    report_dir: Option<&Path>,
    bytes: &[u8],
) -> Result<(), GbInferIRStageFailure> {
    let Some(report_dir) = report_dir else {
        return Ok(());
    };
    fs::create_dir_all(report_dir).map_err(|err| {
        report_io_error(format!(
            "failed to create Stage 3 report directory {}: {err}",
            report_dir.display()
        ))
    })?;
    let path = report_dir.join("infer_ir.json");
    fs::write(&path, bytes).map_err(|err| {
        report_io_error(format!(
            "failed to write Stage 3 report {}: {err}",
            path.display()
        ))
    })?;
    tracing::debug!(
        canonical_bytes_len = bytes.len() as u64,
        report_path = %path.display(),
        "stage3.driver.report_emit"
    );
    Ok(())
}

fn report_io_error(message: String) -> GbInferIRStageFailure {
    GbInferIRStageFailure::report_io(io::Error::other(message).to_string())
}

pub fn bind_identity<B: StaticBudgetReportSelfHash + ?Sized>(
    inputs: &GbInferIRInputs<'_, B>,
) -> Result<InferIrIdentity, Vec<ValidationDiagnostic>> {
    let mut diagnostics = Vec::new();
    if inputs.static_budget_self_hash != inputs.static_budget.report_self_hash() {
        diagnostics.push(hard_semantic_diagnostic(
            "static_budget_self_hash",
            ValidationDetail::HashMismatch {
                expected: inputs.static_budget.report_self_hash(),
                observed: inputs.static_budget_self_hash,
            },
        ));
    }

    let infer_ir_policy_projection_hash =
        hash_infer_ir_policy_projection(&inputs.policy_projection, &mut diagnostics);
    let requested_runtime_modes_hash = hash_requested_runtime_modes(
        &inputs.policy_projection.requested_runtime_modes,
        &mut diagnostics,
    );

    tracing::info!(
        has_static_budget_match =
            inputs.static_budget_self_hash == inputs.static_budget.report_self_hash(),
        "stage3.binding.identity"
    );

    match (
        diagnostics.is_empty(),
        infer_ir_policy_projection_hash,
        requested_runtime_modes_hash,
    ) {
        (true, Some(infer_ir_policy_projection_hash), Some(requested_runtime_modes_hash)) => {
            Ok(InferIrIdentity {
                quant_graph_self_hash: inputs.quant_graph_self_hash,
                infer_ir_policy_projection_hash,
                static_budget_self_hash: inputs.static_budget_self_hash,
                requested_runtime_modes_hash,
                determinism: inputs.quant_graph.identity.determinism,
                // T-B5.13/T-B5.15 replace this after canonical sort binds the real order.
                topological_order_hash: Hash256::ZERO,
            })
        }
        _ => Err(diagnostics),
    }
}

pub fn bind_token_input(
    policy_projection: &InferIrPolicyProjection,
) -> Result<TokenInput, GbInferIrTypeError> {
    tracing::info!(
        n_ingress_modes = policy_projection.allowed_ingress_modes.len() as u64,
        "stage3.binding.token_input"
    );
    TokenInput::new(
        TokenInputId::new(0),
        ValueId::new(0),
        policy_projection.allowed_ingress_modes.clone(),
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GbInferIR {
    pub identity: InferIrIdentity,
    pub token_inputs: Vec<TokenInput>,
    pub nodes: Vec<GbNode>,
    pub values: Vec<ValueDecl>,
    pub effects: Vec<EffectDecl>,
    pub provenance: InferIrProvenance,
    pub anchors: NodeAnchorMap,
}

impl GbInferIR {
    pub fn new(
        identity: InferIrIdentity,
        token_inputs: Vec<TokenInput>,
        nodes: Vec<GbNode>,
        values: Vec<ValueDecl>,
        effects: Vec<EffectDecl>,
        provenance: InferIrProvenance,
        anchors: NodeAnchorMap,
    ) -> Result<Self, GbInferIrTypeError> {
        if token_inputs.len() != 1 {
            return Err(GbInferIrTypeError::TokenInputsV1Count {
                observed: token_inputs.len(),
            });
        }

        let effect_summary = EffectClassSummary::new(&effects)?;
        tracing::info!(
            n_effect_classes = effect_summary.n_effect_classes as u64,
            has_rng_chain = effect_summary.has_rng_chain,
            has_state_chains = effect_summary.has_state_chains,
            "stage3.types.bind_effects"
        );
        provenance.validate_totality(&nodes, &values, &effects)?;
        validate_anchor_totality(&anchors, &nodes)?;
        tracing::info!(
            n_node_provenance = provenance.nodes.len() as u64,
            n_value_provenance = provenance.values.len() as u64,
            n_effect_provenance = provenance.effects.len() as u64,
            "stage3.types.bind_provenance"
        );

        for token_input in &token_inputs {
            tracing::info!(
                n_ingress_modes = token_input.allowed_ingress_modes.len() as u64,
                "stage3.types.bind_token_input"
            );
        }

        reject_fault_boundary_provenance(&provenance)?;
        reject_fault_boundary_node_edges(&nodes, &effects)?;

        for node in &nodes {
            tracing::debug!(
                op_tag = ?node.op.tag(),
                "stage3.types.infer_op_variant"
            );
        }

        tracing::info!(
            n_layers = infer_layers_count(&nodes) as u64,
            n_nodes = nodes.len() as u64,
            n_values = values.len() as u64,
            n_effects = effects.len() as u64,
            "stage3.types.bind_gb_infer_ir"
        );

        Ok(Self {
            identity,
            token_inputs,
            nodes,
            values,
            effects,
            provenance,
            anchors,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GbInferIRProduct {
    pub infer_ir: GbInferIR,
    pub report: ReportEnvelope<report_schema::InferIrReportBody<GbInferIR>>,
    pub infer_ir_self_hash: Hash256,
    pub infer_ir_canonical_bytes_hash: Hash256,
}

impl GbInferIRProduct {
    pub fn new(
        infer_ir: GbInferIR,
        audit_parents: InferIrAuditParents,
        requested_runtime_modes: BTreeSet<RuntimeMode>,
        fixture_equivalence: report_schema::FixtureEquivalenceTag,
    ) -> Result<Self, GbInferIrProductError> {
        let infer_ir_self_hash = infer_ir_self_hash(&infer_ir)?;
        let infer_ir_canonical_bytes_hash = infer_ir_canonical_bytes_hash(&infer_ir)?;
        let input_identity =
            infer_ir_input_identity(&infer_ir, audit_parents, requested_runtime_modes);
        let result = infer_ir_report_result(
            infer_ir.clone(),
            fixture_equivalence,
            infer_ir_self_hash,
            infer_ir_canonical_bytes_hash,
        )?;
        let report = ReportEnvelope::new(
            ReportOutcome::Passed,
            report_schema::InferIrReportBody::new(input_identity, Some(result), Vec::new()),
        )
        .map_err(|err| GbInferIrProductError::ReportEnvelope(err.to_string()))?
        .with_computed_self_hash()
        .map_err(|err| GbInferIrProductError::ReportSelfHash(err.to_string()))?;

        Ok(Self {
            infer_ir,
            report,
            infer_ir_self_hash,
            infer_ir_canonical_bytes_hash,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GbInferIrProductError {
    CanonicalJson(String),
    ReportEnvelope(String),
    ReportSelfHash(String),
    CountOverflow(&'static str),
}

impl fmt::Display for GbInferIrProductError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CanonicalJson(message) => {
                write!(f, "infer_ir canonical JSON error: {message}")
            }
            Self::ReportEnvelope(message) => {
                write!(f, "infer_ir report envelope error: {message}")
            }
            Self::ReportSelfHash(message) => {
                write!(f, "infer_ir report self-hash error: {message}")
            }
            Self::CountOverflow(field) => write!(f, "infer_ir report count overflows {field}"),
        }
    }
}

impl std::error::Error for GbInferIrProductError {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FixtureInputSet {
    pub fixture_build: bool,
    pub inputs: Vec<FixtureInput>,
}

impl FixtureInputSet {
    #[must_use]
    pub fn fixture(inputs: Vec<FixtureInput>) -> Self {
        Self {
            fixture_build: true,
            inputs,
        }
    }

    #[must_use]
    pub fn non_fixture(inputs: Vec<FixtureInput>) -> Self {
        Self {
            fixture_build: false,
            inputs,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FixtureInput {
    pub token_id: u32,
    pub sequence_state_seed: Hash256,
    pub rng_seed: Hash256,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum FixtureEquivalenceResult {
    VerifiedFixtureBitExact,
    Skipped {
        reason: report_schema::FixtureEquivalenceSkippedReason,
    },
}

impl From<FixtureEquivalenceResult> for report_schema::FixtureEquivalenceTag {
    fn from(value: FixtureEquivalenceResult) -> Self {
        match value {
            FixtureEquivalenceResult::VerifiedFixtureBitExact => Self::VerifiedFixtureBitExact,
            FixtureEquivalenceResult::Skipped { reason } => Self::Skipped { reason },
        }
    }
}

pub fn fixture_semantic_equivalence(
    infer_ir: &GbInferIR,
    quant_graph: &QuantGraph,
    fixture: &FixtureInputSet,
) -> Result<FixtureEquivalenceResult, Vec<ValidationDiagnostic>> {
    fixture_semantic_equivalence_check(infer_ir, quant_graph, fixture)
}

pub fn fixture_semantic_equivalence_check(
    infer_ir: &GbInferIR,
    quant_graph: &QuantGraph,
    fixture: &FixtureInputSet,
) -> Result<FixtureEquivalenceResult, Vec<ValidationDiagnostic>> {
    if quant_graph.identity.determinism != DeterminismClass::BitExact {
        return Ok(FixtureEquivalenceResult::Skipped {
            reason: report_schema::FixtureEquivalenceSkippedReason::NonBitExactDeterminism,
        });
    }
    if !fixture.fixture_build {
        return Ok(FixtureEquivalenceResult::Skipped {
            reason: report_schema::FixtureEquivalenceSkippedReason::NonFixtureBuild,
        });
    }

    #[cfg(not(feature = "semantic_equivalence_check"))]
    {
        tracing::info!(
            event = STAGE3_SEMANTIC_EQUIVALENCE_RUN_EVENT,
            fixture_count = fixture.inputs.len() as u64,
            bit_exact_match_count = 0_u64,
            skipped_count = fixture.inputs.len() as u64,
            "stage3.semantic_equivalence.run"
        );
        let _ = infer_ir;
        Ok(FixtureEquivalenceResult::Skipped {
            reason: report_schema::FixtureEquivalenceSkippedReason::FeatureFlagDisabled,
        })
    }

    #[cfg(feature = "semantic_equivalence_check")]
    {
        let mut diagnostics = Vec::new();
        let mut bit_exact_match_count = 0_u64;
        for (sample_index, input) in fixture.inputs.iter().enumerate() {
            let ir_output = canonical::reference::eval_canonical_ir(infer_ir, input);
            let qg_output = canonical::reference::eval_canonical_qg(quant_graph, input);
            if ir_output == qg_output {
                bit_exact_match_count += 1;
            } else {
                diagnostics.push(infer_ir_semantic_equivalence_failed_diagnostic(
                    sample_index,
                ));
            }
        }
        tracing::info!(
            event = STAGE3_SEMANTIC_EQUIVALENCE_RUN_EVENT,
            fixture_count = fixture.inputs.len() as u64,
            bit_exact_match_count,
            skipped_count = 0_u64,
            "stage3.semantic_equivalence.run"
        );
        if diagnostics.is_empty() {
            Ok(FixtureEquivalenceResult::VerifiedFixtureBitExact)
        } else {
            tracing::error!(
                event = STAGE3_SEMANTIC_EQUIVALENCE_FAILED_EVENT,
                n_failures = diagnostics.len() as u64,
                "stage3.semantic_equivalence.failed"
            );
            Err(diagnostics)
        }
    }
}

fn infer_layers_count(nodes: &[GbNode]) -> usize {
    nodes
        .iter()
        .filter_map(|node| node.op.layer())
        .collect::<BTreeSet<_>>()
        .len()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GbInferIrTypeError {
    TokenInputsV1Count { observed: usize },
    TokenInputIngressModesEmpty,
    FaultBoundaryReserved { effect_id: Option<EffectId> },
    NodeProvenanceMissing { node_id: NodeId },
    ValueProvenanceMissing { value_id: ValueId },
    EffectProvenanceMissing { effect_id: EffectId },
    SemanticAnchorMissing { node_id: NodeId },
    SemanticAnchorCanonicalize { message: String },
}

impl fmt::Display for GbInferIrTypeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TokenInputsV1Count { observed } => {
                write!(
                    f,
                    "infer_ir.v1 requires exactly one token input, observed {observed}"
                )
            }
            Self::TokenInputIngressModesEmpty => {
                write!(f, "infer_ir.v1 token input ingress modes must be non-empty")
            }
            Self::FaultBoundaryReserved { effect_id } => {
                write!(
                    f,
                    "infer_ir.v1 reserves FaultBoundary effects and must not emit them"
                )?;
                if let Some(effect_id) = effect_id {
                    write!(f, " (effect_id={})", effect_id.get())?;
                }
                Ok(())
            }
            Self::NodeProvenanceMissing { node_id } => {
                write!(
                    f,
                    "infer_ir.v1 missing provenance for node_id={}",
                    node_id.get()
                )
            }
            Self::ValueProvenanceMissing { value_id } => {
                write!(
                    f,
                    "infer_ir.v1 missing provenance for value_id={}",
                    value_id.get()
                )
            }
            Self::EffectProvenanceMissing { effect_id } => {
                write!(
                    f,
                    "infer_ir.v1 missing provenance for effect_id={}",
                    effect_id.get()
                )
            }
            Self::SemanticAnchorMissing { node_id } => {
                write!(
                    f,
                    "infer_ir.v1 missing semantic anchor for node_id={}",
                    node_id.get()
                )
            }
            Self::SemanticAnchorCanonicalize { message } => {
                write!(
                    f,
                    "infer_ir.v1 semantic anchor canonicalization failed: {message}"
                )
            }
        }
    }
}

impl std::error::Error for GbInferIrTypeError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InferIrIdentity {
    pub quant_graph_self_hash: Hash256,
    pub infer_ir_policy_projection_hash: Hash256,
    pub static_budget_self_hash: Hash256,
    pub requested_runtime_modes_hash: Hash256,
    pub determinism: DeterminismClass,
    pub topological_order_hash: Hash256,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenInput {
    pub token_input_id: TokenInputId,
    pub value_id: ValueId,
    pub allowed_ingress_modes: NonEmptySet<TokenIngressMode>,
}

impl TokenInput {
    pub fn new(
        token_input_id: TokenInputId,
        value_id: ValueId,
        allowed_ingress_modes: BTreeSet<TokenIngressMode>,
    ) -> Result<Self, GbInferIrTypeError> {
        let allowed_ingress_modes = NonEmptySet::new(allowed_ingress_modes)
            .map_err(|NonEmptySetError| GbInferIrTypeError::TokenInputIngressModesEmpty)?;
        Ok(Self {
            token_input_id,
            value_id,
            allowed_ingress_modes,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct TokenInputId(u8);

impl TokenInputId {
    #[must_use]
    pub const fn new(value: u8) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }
}

impl From<u8> for TokenInputId {
    fn from(value: u8) -> Self {
        Self::new(value)
    }
}

impl<'de> Deserialize<'de> for TokenInputId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_u8_id(deserializer, "TokenInputId").map(Self::new)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct NonEmptySet<T> {
    values: BTreeSet<T>,
}

impl<T: Ord> NonEmptySet<T> {
    pub fn new(values: BTreeSet<T>) -> Result<Self, NonEmptySetError> {
        if values.is_empty() {
            return Err(NonEmptySetError);
        }
        Ok(Self { values })
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.values.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        false
    }

    #[must_use]
    pub fn contains(&self, value: &T) -> bool {
        self.values.contains(value)
    }

    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.values.iter()
    }
}

impl<T> IntoIterator for NonEmptySet<T> {
    type Item = T;
    type IntoIter = std::collections::btree_set::IntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        self.values.into_iter()
    }
}

impl<'de, T> Deserialize<'de> for NonEmptySet<T>
where
    T: Deserialize<'de> + Ord,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let values = BTreeSet::<T>::deserialize(deserializer)?;
        Self::new(values)
            .map_err(|NonEmptySetError| de::Error::custom("infer_ir.v1 requires a non-empty set"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NonEmptySetError;

fn deserialize_u8_id<'de, D>(deserializer: D, type_name: &'static str) -> Result<u8, D::Error>
where
    D: Deserializer<'de>,
{
    let value = deserialize_u32_id(deserializer, type_name)?;
    u8::try_from(value).map_err(|_| de::Error::custom(format!("{type_name} out of u8 range")))
}

fn deserialize_u32_id<'de, D>(deserializer: D, type_name: &'static str) -> Result<u32, D::Error>
where
    D: Deserializer<'de>,
{
    struct IdVisitor {
        type_name: &'static str,
    }

    impl<'de> de::Visitor<'de> for IdVisitor {
        type Value = u32;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(formatter, "{} as a u32 or string key", self.type_name)
        }

        fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            u32::try_from(value)
                .map_err(|_| E::custom(format!("{} out of u32 range", self.type_name)))
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            value
                .parse::<u32>()
                .map_err(|_| E::custom(format!("invalid {} string key", self.type_name)))
        }
    }

    deserializer.deserialize_any(IdVisitor { type_name })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum TokenIngressMode {
    Prompt,
    AutoRegressive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GbNode {
    pub node_id: NodeId,
    pub op: InferOp,
    pub inputs: Vec<ValueId>,
    pub effects_in: Vec<EffectId>,
    pub outputs: Vec<ValueId>,
    pub effects_out: Vec<EffectId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reduction_site: Option<ReductionSiteId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct NodeId(u32);

impl NodeId {
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl From<u32> for NodeId {
    fn from(value: u32) -> Self {
        Self::new(value)
    }
}

impl<'de> Deserialize<'de> for NodeId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_u32_id(deserializer, "NodeId").map(Self::new)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct ValueId(u32);

impl ValueId {
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl From<u32> for ValueId {
    fn from(value: u32) -> Self {
        Self::new(value)
    }
}

impl<'de> Deserialize<'de> for ValueId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_u32_id(deserializer, "ValueId").map(Self::new)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct EffectId(u32);

impl EffectId {
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl From<u32> for EffectId {
    fn from(value: u32) -> Self {
        Self::new(value)
    }
}

impl<'de> Deserialize<'de> for EffectId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_u32_id(deserializer, "EffectId").map(Self::new)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum InferOp {
    Embedding {
        token_input: TokenInputId,
    },
    SequenceRead {
        slot: StateSlotId,
    },
    SequenceStep {
        layer: LayerId,
    },
    SequenceWrite {
        slot: StateSlotId,
    },
    RouterMatVec {
        layer: LayerId,
    },
    RouteTop1 {
        layer: LayerId,
    },
    SelectExpertTop1 {
        layer: LayerId,
    },
    ExpertMatVec {
        layer: LayerId,
        expert: ExpertId,
        slot: ExpertWeightSlot,
    },
    FfnActivation {
        layer: LayerId,
        expert: ExpertId,
    },
    CombineResidual {
        layer: Option<LayerId>,
        site: ResidualSite,
    },
    Norm {
        plan: NormPlanId,
    },
    Classify,
    DecodeToken {
        plan: DecodePlanId,
    },
}

impl InferOp {
    #[must_use]
    pub const fn tag(self) -> InferOpTag {
        match self {
            Self::Classify => InferOpTag::Classify,
            Self::CombineResidual { .. } => InferOpTag::CombineResidual,
            Self::DecodeToken { .. } => InferOpTag::DecodeToken,
            Self::Embedding { .. } => InferOpTag::Embedding,
            Self::ExpertMatVec { .. } => InferOpTag::ExpertMatVec,
            Self::FfnActivation { .. } => InferOpTag::FfnActivation,
            Self::Norm { .. } => InferOpTag::Norm,
            Self::RouteTop1 { .. } => InferOpTag::RouteTop1,
            Self::RouterMatVec { .. } => InferOpTag::RouterMatVec,
            Self::SelectExpertTop1 { .. } => InferOpTag::SelectExpertTop1,
            Self::SequenceRead { .. } => InferOpTag::SequenceRead,
            Self::SequenceStep { .. } => InferOpTag::SequenceStep,
            Self::SequenceWrite { .. } => InferOpTag::SequenceWrite,
        }
    }

    #[must_use]
    pub const fn layer(self) -> Option<LayerId> {
        match self {
            Self::SequenceStep { layer }
            | Self::RouterMatVec { layer }
            | Self::RouteTop1 { layer }
            | Self::SelectExpertTop1 { layer }
            | Self::ExpertMatVec { layer, .. }
            | Self::FfnActivation { layer, .. } => Some(layer),
            Self::CombineResidual { layer, .. } => layer,
            Self::Embedding { .. }
            | Self::SequenceRead { .. }
            | Self::SequenceWrite { .. }
            | Self::Norm { .. }
            | Self::Classify
            | Self::DecodeToken { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum InferOpTag {
    Classify,
    CombineResidual,
    DecodeToken,
    Embedding,
    ExpertMatVec,
    FfnActivation,
    Norm,
    RouteTop1,
    RouterMatVec,
    SelectExpertTop1,
    SequenceRead,
    SequenceStep,
    SequenceWrite,
}

pub const INFER_OP_TAG_CANONICAL_ORDER: [InferOpTag; 13] = [
    InferOpTag::Classify,
    InferOpTag::CombineResidual,
    InferOpTag::DecodeToken,
    InferOpTag::Embedding,
    InferOpTag::ExpertMatVec,
    InferOpTag::FfnActivation,
    InferOpTag::Norm,
    InferOpTag::RouteTop1,
    InferOpTag::RouterMatVec,
    InferOpTag::SelectExpertTop1,
    InferOpTag::SequenceRead,
    InferOpTag::SequenceStep,
    InferOpTag::SequenceWrite,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ResidualSite {
    PostFfn,
    PostSequence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValueDecl {
    pub value_id: ValueId,
    pub kind: ValueKind,
    pub format: ValueFormat,
    pub layout: ValueLayout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ValueKind {
    Activation,
    DecodedToken,
    EmbeddingOutput,
    ExpertCandidate,
    ExpertIntermediate,
    ExpertOutput,
    GateWeight,
    InputToken,
    LogitVector,
    NormalizedActivation,
    RouterDecision,
    RouterScore,
    SequenceBlockOutput,
    SequenceStateNext,
    SequenceStateRead,
}

pub const VALUE_KIND_CANONICAL_ORDER: [ValueKind; 15] = [
    ValueKind::Activation,
    ValueKind::DecodedToken,
    ValueKind::EmbeddingOutput,
    ValueKind::ExpertCandidate,
    ValueKind::ExpertIntermediate,
    ValueKind::ExpertOutput,
    ValueKind::GateWeight,
    ValueKind::InputToken,
    ValueKind::LogitVector,
    ValueKind::NormalizedActivation,
    ValueKind::RouterDecision,
    ValueKind::RouterScore,
    ValueKind::SequenceBlockOutput,
    ValueKind::SequenceStateNext,
    ValueKind::SequenceStateRead,
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ValueFormat {
    Quant { format: QuantFormat },
    ExactAccumulator,
    TokenIdDomain { vocab_size: u32 },
    ExpertIdDomain { n_experts: u16 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValueLayout {
    pub shape: Vec<ValueAxis>,
}

impl ValueLayout {
    #[must_use]
    pub fn scalar() -> Self {
        Self { shape: Vec::new() }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ValueAxis {
    Token,
    Model,
    Expert,
    Vocabulary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectDecl {
    pub effect_id: EffectId,
    pub class: EffectClass,
}

impl EffectDecl {
    #[must_use]
    pub const fn new(effect_id: EffectId, class: EffectClass) -> Self {
        Self { effect_id, class }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum EffectClass {
    SequenceState { slot: StateSlotId },
    Rng { slot: RngSlot },
    FaultBoundary,
}

impl EffectClass {
    #[must_use]
    pub const fn tag(self) -> EffectClassTag {
        match self {
            Self::FaultBoundary => EffectClassTag::FaultBoundary,
            Self::Rng { .. } => EffectClassTag::Rng,
            Self::SequenceState { .. } => EffectClassTag::SequenceState,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum EffectClassTag {
    FaultBoundary,
    Rng,
    SequenceState,
}

pub const EFFECT_CLASS_TAG_CANONICAL_ORDER: [EffectClassTag; 3] = [
    EffectClassTag::FaultBoundary,
    EffectClassTag::Rng,
    EffectClassTag::SequenceState,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct StateSlotId(u32);

impl StateSlotId {
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl<'de> Deserialize<'de> for StateSlotId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_u32_id(deserializer, "StateSlotId").map(Self::new)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum RngSlot {
    Decode,
}

pub const RNG_SLOT_CANONICAL_ORDER: [RngSlot; 1] = [RngSlot::Decode];

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InferIrProvenance {
    pub nodes: BTreeMap<NodeId, QuantGraphEntityRef>,
    pub values: BTreeMap<ValueId, ValueProducerRef>,
    pub effects: BTreeMap<EffectId, EffectProvenance>,
}

impl InferIrProvenance {
    pub fn validate_totality(
        &self,
        nodes: &[GbNode],
        values: &[ValueDecl],
        effects: &[EffectDecl],
    ) -> Result<(), GbInferIrTypeError> {
        for node in nodes {
            if !self.nodes.contains_key(&node.node_id) {
                return Err(GbInferIrTypeError::NodeProvenanceMissing {
                    node_id: node.node_id,
                });
            }
        }

        for value_id in values
            .iter()
            .map(|value| value.value_id)
            .chain(nodes.iter().flat_map(|node| {
                node.inputs
                    .iter()
                    .copied()
                    .chain(node.outputs.iter().copied())
            }))
        {
            if !self.values.contains_key(&value_id) {
                return Err(GbInferIrTypeError::ValueProvenanceMissing { value_id });
            }
        }

        for effect_id in effects
            .iter()
            .map(|effect| effect.effect_id)
            .chain(nodes.iter().flat_map(|node| {
                node.effects_in
                    .iter()
                    .copied()
                    .chain(node.effects_out.iter().copied())
            }))
        {
            if !self.effects.contains_key(&effect_id) {
                return Err(GbInferIrTypeError::EffectProvenanceMissing { effect_id });
            }
        }

        Ok(())
    }
}

pub type NodeAnchorMap = BTreeMap<NodeId, SemanticAnchor>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticAnchor {
    pub anchor_id: Hash256,
}

impl SemanticAnchor {
    #[must_use]
    pub const fn new(anchor_id: Hash256) -> Self {
        Self { anchor_id }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalProvenanceTuple {
    pub op_tag: InferOpTag,
    pub layer: Option<LayerId>,
    pub expert: Option<ExpertId>,
    pub expert_weight_slot: Option<ExpertWeightSlot>,
    pub norm_site: Option<NormSite>,
    pub state_slot: Option<StateSlotId>,
    pub residual_site: Option<ResidualSite>,
    pub occurrence_index: u32,
}

impl CanonicalProvenanceTuple {
    #[must_use]
    pub const fn new(op_tag: InferOpTag, occurrence_index: u32) -> Self {
        Self {
            op_tag,
            layer: None,
            expert: None,
            expert_weight_slot: None,
            norm_site: None,
            state_slot: None,
            residual_site: None,
            occurrence_index,
        }
    }
}

pub fn compute_semantic_anchor(
    quant_graph_self_hash: Hash256,
    node_id: NodeId,
    op_tag: InferOpTag,
    canonical_provenance_tuple: &CanonicalProvenanceTuple,
) -> Result<SemanticAnchor, GbInferIrTypeError> {
    let material = serde_json::json!({
        "quant_graph_self_hash": quant_graph_self_hash,
        "node_id": node_id,
        "op_tag": op_tag,
        "canonical_provenance_tuple": canonical_provenance_tuple,
    });
    let canonical = canonicalize_value(&material).map_err(|error| {
        GbInferIrTypeError::SemanticAnchorCanonicalize {
            message: error.to_string(),
        }
    })?;
    let anchor = SemanticAnchor::new(domain_hash("SemanticAnchor", "v1", &canonical));

    tracing::debug!(
        node_id = node_id.get() as u64,
        op_tag = ?op_tag,
        anchor = %anchor.anchor_id,
        "stage3.types.anchor_compute"
    );

    Ok(anchor)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum QuantGraphEntityRef {
    Embedding,
    NormPlan {
        plan: NormPlanId,
    },
    NormSite {
        site: NormSite,
    },
    RouterLayer {
        layer: LayerId,
    },
    RouterTensor {
        layer: LayerId,
        tensor: TensorId,
    },
    RouterSelection {
        layer: LayerId,
    },
    ExpertSection {
        layer: LayerId,
        expert: ExpertId,
    },
    ExpertTensor {
        layer: LayerId,
        expert: ExpertId,
        slot: ExpertWeightSlot,
        tensor: TensorId,
    },
    FfnActivationSite {
        layer: LayerId,
        expert: ExpertId,
    },
    ResidualSiteRef {
        layer: Option<LayerId>,
        site: ResidualSite,
    },
    DecodePlan {
        plan: DecodePlanId,
    },
    ClassifyHead,
    SequenceSlot {
        slot: StateSlotId,
    },
    SequenceStep {
        layer: LayerId,
    },
    TokenInput {
        token_input: TokenInputId,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ValueProducerRef {
    Node { node: NodeId },
    External { token_input: TokenInputId },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum EffectProvenance {
    ExternalRoot { class: EffectClass },
    NodeOutput { node: NodeId, class: EffectClass },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValueAllocation {
    pub values: Vec<ValueDecl>,
    by_key: BTreeMap<ValueAllocationKey, ValueId>,
}

impl ValueAllocation {
    #[must_use]
    pub fn id(&self, key: ValueAllocationKey) -> Option<ValueId> {
        self.by_key.get(&key).copied()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ValueAllocationKey {
    InputToken,
    EmbeddingOutput,
    NormOutput {
        site: NormSite,
    },
    PostSequenceResidual {
        layer: LayerId,
    },
    RouterScore {
        layer: LayerId,
    },
    RouterDecision {
        layer: LayerId,
    },
    GateWeight {
        layer: LayerId,
    },
    ExpertIntermediate {
        layer: LayerId,
        expert: ExpertId,
        stage: ExpertIntermediateStage,
    },
    ExpertCandidate {
        layer: LayerId,
        expert: ExpertId,
    },
    ExpertOutput {
        layer: LayerId,
    },
    PostFfnResidual {
        layer: LayerId,
    },
    FinalNorm,
    Logits,
    DecodedToken,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ExpertIntermediateStage {
    FfnGate,
    FfnUp,
    Activation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectAllocation {
    pub effects: Vec<EffectDecl>,
    pub decode_rng: Option<EffectEdgePair>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectEdgePair {
    pub input: EffectId,
    pub output: EffectId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InferIrWave4 {
    pub identity: InferIrIdentity,
    pub token_input: TokenInput,
    pub values: ValueAllocation,
    pub effects: EffectAllocation,
    pub nodes: Vec<GbNode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReductionSiteKey {
    RouterMatVec {
        layer: LayerId,
    },
    ExpertMatVec {
        layer: LayerId,
        expert: ExpertId,
        slot: ExpertWeightSlot,
    },
    Norm {
        norm_plan: NormPlanId,
    },
    Classify {
        classify_weight: TensorId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
struct TopologicalOrderEntry {
    node_id: NodeId,
    op_tag: InferOpTag,
    canonical_provenance_tuple: CanonicalProvenanceTuple,
}

pub fn bind_wave4_classes_6_to_10<B>(
    inputs: &GbInferIRInputs<'_, B>,
    wave4: InferIrWave4,
) -> Result<GbInferIR, Vec<ValidationDiagnostic>>
where
    B: StaticBudgetReportSelfHash + StaticBudgetReductionSites + ?Sized,
{
    let nodes = bind_reduction_sites(wave4.nodes, inputs.quant_graph, inputs.static_budget)?;
    let (mut identity, nodes) = bind_canonical_sort(wave4.identity, nodes, inputs.quant_graph)?;
    let topological_order_hash = compute_topological_order_hash(&nodes, inputs.quant_graph)?;
    identity.topological_order_hash = topological_order_hash;
    let provenance = bind_infer_ir_provenance(
        &nodes,
        &wave4.values.values,
        &wave4.effects.effects,
        std::slice::from_ref(&wave4.token_input),
        inputs.quant_graph,
    )?;
    let anchors = bind_node_anchors(inputs.quant_graph_self_hash, &nodes, inputs.quant_graph)?;
    let ir = GbInferIR::new(
        identity,
        vec![wave4.token_input],
        nodes,
        wave4.values.values,
        wave4.effects.effects,
        provenance,
        anchors,
    )
    .map_err(|error| {
        vec![hard_semantic_diagnostic(
            "infer_ir.product",
            ValidationDetail::Field {
                field: FieldPath::from(error.to_string()),
            },
        )]
    })?;
    let diagnostics = validate_infer_ir_self_consistency(&ir, inputs.quant_graph);
    if diagnostics.is_empty() {
        Ok(ir)
    } else {
        Err(diagnostics)
    }
}

pub fn bind_reduction_sites<B: StaticBudgetReductionSites + ?Sized>(
    nodes: Vec<GbNode>,
    quant_graph: &QuantGraph,
    static_budget: &B,
) -> Result<Vec<GbNode>, Vec<ValidationDiagnostic>> {
    let site_ids = static_budget.reduction_site_ids();
    let mut counts = BTreeMap::<ReductionSiteId, usize>::new();
    for site_id in site_ids {
        *counts.entry(site_id).or_default() += 1;
    }

    let mut diagnostics = Vec::new();
    let mut bound = Vec::with_capacity(nodes.len());
    for mut node in nodes {
        if let Some(key) = reduction_site_key_for_op(node.op, quant_graph) {
            let site_id = canonical_reduction_site_id(key);
            match counts.get(&site_id).copied().unwrap_or(0) {
                1 => node.reduction_site = Some(site_id),
                _ => {
                    diagnostics.push(infer_ir_diagnostic(
                        "InferIrReductionSiteMissing",
                        "reduction_site",
                    ));
                    node.reduction_site = None;
                }
            }
        } else {
            node.reduction_site = None;
        }
        bound.push(node);
    }

    tracing::info!(
        n_nodes = bound.len() as u64,
        n_missing = diagnostics.len() as u64,
        "stage3.binding.reduction_site"
    );

    if diagnostics.is_empty() {
        Ok(bound)
    } else {
        Err(diagnostics)
    }
}

pub fn bind_infer_ir_provenance(
    nodes: &[GbNode],
    values: &[ValueDecl],
    effects: &[EffectDecl],
    token_inputs: &[TokenInput],
    quant_graph: &QuantGraph,
) -> Result<InferIrProvenance, Vec<ValidationDiagnostic>> {
    let node_output_effects = nodes
        .iter()
        .flat_map(|node| {
            node.effects_out
                .iter()
                .copied()
                .map(move |effect_id| (effect_id, node.node_id))
        })
        .collect::<BTreeMap<_, _>>();
    let effect_classes = effects
        .iter()
        .map(|effect| (effect.effect_id, effect.class))
        .collect::<BTreeMap<_, _>>();

    let node_provenance = nodes
        .iter()
        .map(|node| {
            (
                node.node_id,
                quant_graph_entity_ref_for_op(node.op, quant_graph),
            )
        })
        .collect();
    let mut value_provenance = BTreeMap::new();
    let mut diagnostics = Vec::new();
    for value in values {
        let producer = token_inputs
            .iter()
            .find(|token_input| token_input.value_id == value.value_id)
            .map(|token_input| ValueProducerRef::External {
                token_input: token_input.token_input_id,
            })
            .or_else(|| {
                nodes
                    .iter()
                    .find(|node| node.outputs.contains(&value.value_id))
                    .map(|node| ValueProducerRef::Node { node: node.node_id })
            });
        match producer {
            Some(producer) => {
                value_provenance.insert(value.value_id, producer);
            }
            None => diagnostics.push(infer_ir_value_producer_missing_diagnostic(value.value_id)),
        }
    }
    let effect_provenance = effects
        .iter()
        .map(|effect| {
            let class = effect_classes
                .get(&effect.effect_id)
                .copied()
                .unwrap_or(effect.class);
            let source = node_output_effects
                .get(&effect.effect_id)
                .copied()
                .map(|node| EffectProvenance::NodeOutput { node, class })
                .unwrap_or(EffectProvenance::ExternalRoot { class });
            (effect.effect_id, source)
        })
        .collect();
    let provenance = InferIrProvenance {
        nodes: node_provenance,
        values: value_provenance,
        effects: effect_provenance,
    };

    tracing::info!(
        n_node_provenance = provenance.nodes.len() as u64,
        n_value_provenance = provenance.values.len() as u64,
        n_effect_provenance = provenance.effects.len() as u64,
        "stage3.binding.provenance"
    );
    if diagnostics.is_empty() {
        Ok(provenance)
    } else {
        Err(diagnostics)
    }
}

pub fn bind_node_anchors(
    quant_graph_self_hash: Hash256,
    nodes: &[GbNode],
    quant_graph: &QuantGraph,
) -> Result<NodeAnchorMap, Vec<ValidationDiagnostic>> {
    let tuples = canonical_provenance_tuples(nodes, quant_graph);
    let mut anchors = NodeAnchorMap::new();
    for node in nodes {
        let tuple = tuples
            .get(&node.node_id)
            .expect("tuple computed for every node");
        let anchor =
            compute_semantic_anchor(quant_graph_self_hash, node.node_id, node.op.tag(), tuple)
                .map_err(|error| {
                    vec![hard_semantic_diagnostic(
                        "anchors",
                        ValidationDetail::Field {
                            field: FieldPath::from(error.to_string()),
                        },
                    )]
                })?;
        anchors.insert(node.node_id, anchor);
    }

    tracing::info!(n_anchors = anchors.len() as u64, "stage3.binding.anchor");
    Ok(anchors)
}

pub fn bind_canonical_sort(
    identity: InferIrIdentity,
    mut nodes: Vec<GbNode>,
    quant_graph: &QuantGraph,
) -> Result<(InferIrIdentity, Vec<GbNode>), Vec<ValidationDiagnostic>> {
    nodes.sort_by_key(|node| canonical_node_sort_key(node, quant_graph));
    for (index, node) in nodes.iter_mut().enumerate() {
        node.node_id = NodeId::new(index as u32);
    }
    tracing::info!(
        n_nodes = nodes.len() as u64,
        "stage3.binding.canonical_sort"
    );
    Ok((identity, nodes))
}

pub fn compute_topological_order_hash(
    nodes: &[GbNode],
    quant_graph: &QuantGraph,
) -> Result<Hash256, Vec<ValidationDiagnostic>> {
    let tuples = canonical_provenance_tuples(nodes, quant_graph);
    let entries = nodes
        .iter()
        .map(|node| TopologicalOrderEntry {
            node_id: node.node_id,
            op_tag: node.op.tag(),
            canonical_provenance_tuple: tuples
                .get(&node.node_id)
                .expect("tuple computed for every node")
                .clone(),
        })
        .collect::<Vec<_>>();
    let value = serde_json::to_value(&entries).map_err(|error| {
        vec![hard_semantic_diagnostic(
            "topological_order_hash",
            ValidationDetail::Field {
                field: FieldPath::from(error.to_string()),
            },
        )]
    })?;
    let canonical = canonicalize_value(&value).map_err(|error| {
        vec![hard_semantic_diagnostic(
            "topological_order_hash",
            ValidationDetail::Field {
                field: FieldPath::from(error.to_string()),
            },
        )]
    })?;
    Ok(domain_hash(
        "InferIrTopologicalOrder",
        INFER_IR_SCHEMA_ID,
        &canonical,
    ))
}

pub fn bind_wave4_classes_1_to_5<B: StaticBudgetReportSelfHash + ?Sized>(
    inputs: &GbInferIRInputs<'_, B>,
) -> Result<InferIrWave4, Vec<ValidationDiagnostic>> {
    let mut diagnostics = Vec::new();

    let identity = match bind_identity(inputs) {
        Ok(identity) => Some(identity),
        Err(mut errors) => {
            diagnostics.append(&mut errors);
            None
        }
    };
    let token_input = match bind_token_input(&inputs.policy_projection) {
        Ok(token_input) => Some(token_input),
        Err(error) => {
            diagnostics.push(hard_semantic_diagnostic(
                "token_inputs.allowed_ingress_modes",
                ValidationDetail::Field {
                    field: FieldPath::from(error.to_string()),
                },
            ));
            None
        }
    };
    let values = match bind_value_allocation(inputs.quant_graph) {
        Ok(values) => Some(values),
        Err(mut errors) => {
            diagnostics.append(&mut errors);
            None
        }
    };
    let effects = match bind_effect_allocation(inputs.quant_graph) {
        Ok(effects) => Some(effects),
        Err(mut errors) => {
            diagnostics.append(&mut errors);
            None
        }
    };

    match (
        identity,
        token_input,
        values,
        effects,
        diagnostics.is_empty(),
    ) {
        (Some(identity), Some(token_input), Some(values), Some(effects), true) => {
            let nodes = bind_node_building(inputs.quant_graph, &values, &effects)?;
            Ok(InferIrWave4 {
                identity,
                token_input,
                values,
                effects,
                nodes,
            })
        }
        _ => Err(diagnostics),
    }
}

pub fn bind_value_allocation(
    quant_graph: &QuantGraph,
) -> Result<ValueAllocation, Vec<ValidationDiagnostic>> {
    let mut diagnostics = Vec::new();
    let Some(embedding_format) = tensor_format_by_role(quant_graph, |role| {
        matches!(role, QuantTensorRole::EmbeddingTable)
    }) else {
        diagnostics.push(hard_semantic_diagnostic(
            "tensors.embedding",
            ValidationDetail::Field {
                field: FieldPath::from("tensors.embedding"),
            },
        ));
        return Err(diagnostics);
    };

    let mut allocation = ValueAllocationBuilder::default();
    let model = &quant_graph.identity.model_spec_summary;
    allocation.push(
        ValueAllocationKey::InputToken,
        ValueKind::InputToken,
        ValueFormat::TokenIdDomain {
            vocab_size: model.vocab_size,
        },
        ValueLayout::scalar(),
    );
    allocation.push(
        ValueAllocationKey::EmbeddingOutput,
        ValueKind::EmbeddingOutput,
        ValueFormat::Quant {
            format: embedding_format,
        },
        vector_layout(ValueAxis::Model),
    );

    for layer_index in 0..model.n_layers {
        let layer = LayerId::new(layer_index);
        let Some(layer_norms) = quant_graph.layer_norms.get(&layer) else {
            diagnostics.push(hard_semantic_diagnostic(
                "layer_norms",
                ValidationDetail::Field {
                    field: FieldPath::from("layer_norms"),
                },
            ));
            continue;
        };
        let Some(pre_sequence) = norm_plan_by_id(quant_graph, layer_norms.pre_sequence) else {
            diagnostics.push(hard_semantic_diagnostic(
                "layer_norms.pre_sequence",
                ValidationDetail::Field {
                    field: FieldPath::from("layer_norms.pre_sequence"),
                },
            ));
            continue;
        };
        let Some(pre_ffn) = norm_plan_by_id(quant_graph, layer_norms.pre_ffn) else {
            diagnostics.push(hard_semantic_diagnostic(
                "layer_norms.pre_ffn",
                ValidationDetail::Field {
                    field: FieldPath::from("layer_norms.pre_ffn"),
                },
            ));
            continue;
        };

        allocation.push(
            ValueAllocationKey::NormOutput {
                site: pre_sequence.site,
            },
            ValueKind::NormalizedActivation,
            ValueFormat::Quant {
                format: pre_sequence.output_format.clone(),
            },
            vector_layout(ValueAxis::Model),
        );
        allocation.push(
            ValueAllocationKey::PostSequenceResidual { layer },
            ValueKind::Activation,
            ValueFormat::Quant {
                format: quant_graph.residual_plan.activation_format.clone(),
            },
            vector_layout(ValueAxis::Model),
        );
        allocation.push(
            ValueAllocationKey::NormOutput { site: pre_ffn.site },
            ValueKind::NormalizedActivation,
            ValueFormat::Quant {
                format: pre_ffn.output_format.clone(),
            },
            vector_layout(ValueAxis::Model),
        );

        if layer_kind(quant_graph, layer) == Some(FfnKindTag::Routed) {
            allocation.push(
                ValueAllocationKey::RouterScore { layer },
                ValueKind::RouterScore,
                ValueFormat::ExactAccumulator,
                vector_layout(ValueAxis::Expert),
            );
            allocation.push(
                ValueAllocationKey::RouterDecision { layer },
                ValueKind::RouterDecision,
                ValueFormat::ExpertIdDomain {
                    n_experts: n_experts_for_layer(quant_graph, layer),
                },
                ValueLayout::scalar(),
            );
            allocation.push(
                ValueAllocationKey::GateWeight { layer },
                ValueKind::GateWeight,
                ValueFormat::Quant {
                    format: QuantFormat::Q8_8,
                },
                ValueLayout::scalar(),
            );
        }

        for expert in expected_experts_for_layer(quant_graph, layer) {
            if ffn_activation_kind(quant_graph, layer) == Some(FfnActivationKind::SwiGLU) {
                allocation.push(
                    ValueAllocationKey::ExpertIntermediate {
                        layer,
                        expert,
                        stage: ExpertIntermediateStage::FfnGate,
                    },
                    ValueKind::ExpertIntermediate,
                    ValueFormat::ExactAccumulator,
                    vector_layout(ValueAxis::Model),
                );
            }
            allocation.push(
                ValueAllocationKey::ExpertIntermediate {
                    layer,
                    expert,
                    stage: ExpertIntermediateStage::FfnUp,
                },
                ValueKind::ExpertIntermediate,
                ValueFormat::ExactAccumulator,
                vector_layout(ValueAxis::Model),
            );
            allocation.push(
                ValueAllocationKey::ExpertIntermediate {
                    layer,
                    expert,
                    stage: ExpertIntermediateStage::Activation,
                },
                ValueKind::ExpertIntermediate,
                ValueFormat::Quant {
                    format: quant_graph
                        .ffn_plans
                        .get(&layer)
                        .map(|plan| plan.intermediate_format.clone())
                        .unwrap_or(QuantFormat::Q8_8),
                },
                vector_layout(ValueAxis::Model),
            );
            allocation.push(
                ValueAllocationKey::ExpertCandidate { layer, expert },
                ValueKind::ExpertCandidate,
                ValueFormat::ExactAccumulator,
                vector_layout(ValueAxis::Model),
            );
        }

        if layer_kind(quant_graph, layer) == Some(FfnKindTag::Routed) {
            allocation.push(
                ValueAllocationKey::ExpertOutput { layer },
                ValueKind::ExpertOutput,
                ValueFormat::ExactAccumulator,
                vector_layout(ValueAxis::Model),
            );
        }
        allocation.push(
            ValueAllocationKey::PostFfnResidual { layer },
            ValueKind::Activation,
            ValueFormat::Quant {
                format: quant_graph.residual_plan.activation_format.clone(),
            },
            vector_layout(ValueAxis::Model),
        );
    }

    let Some(final_norm) = quant_graph.norm_plan(NormSite::Final) else {
        diagnostics.push(hard_semantic_diagnostic(
            "norm_plans.final",
            ValidationDetail::Field {
                field: FieldPath::from("norm_plans.final"),
            },
        ));
        return Err(diagnostics);
    };
    allocation.push(
        ValueAllocationKey::FinalNorm,
        ValueKind::NormalizedActivation,
        ValueFormat::Quant {
            format: final_norm.output_format.clone(),
        },
        vector_layout(ValueAxis::Model),
    );
    allocation.push(
        ValueAllocationKey::Logits,
        ValueKind::LogitVector,
        ValueFormat::ExactAccumulator,
        vector_layout(ValueAxis::Vocabulary),
    );
    allocation.push(
        ValueAllocationKey::DecodedToken,
        ValueKind::DecodedToken,
        ValueFormat::TokenIdDomain {
            vocab_size: model.vocab_size,
        },
        ValueLayout::scalar(),
    );

    tracing::info!(
        n_values = allocation.values.len() as u64,
        "stage3.binding.value_alloc"
    );

    if diagnostics.is_empty() {
        Ok(allocation.finish())
    } else {
        Err(diagnostics)
    }
}

pub fn bind_effect_allocation(
    quant_graph: &QuantGraph,
) -> Result<EffectAllocation, Vec<ValidationDiagnostic>> {
    let mut diagnostics = Vec::new();
    if quant_graph.sequence_semantics.has_state_slots() {
        diagnostics.push(infer_ir_sequence_semantics_unsupported_diagnostic());
    }

    let mut effects = Vec::new();
    let decode_rng = if quant_graph.decode_spec.requires_rng {
        let class = EffectClass::Rng {
            slot: RngSlot::Decode,
        };
        let pair = EffectEdgePair {
            input: EffectId::new(0),
            output: EffectId::new(1),
        };
        effects.push(EffectDecl::new(pair.input, class));
        effects.push(EffectDecl::new(pair.output, class));
        Some(pair)
    } else {
        None
    };

    tracing::info!(
        n_effects = effects.len() as u64,
        has_rng_chain = decode_rng.is_some(),
        "stage3.binding.effect_alloc"
    );

    if diagnostics.is_empty() {
        Ok(EffectAllocation {
            effects,
            decode_rng,
        })
    } else {
        Err(diagnostics)
    }
}

pub fn bind_node_building(
    quant_graph: &QuantGraph,
    values: &ValueAllocation,
    effects: &EffectAllocation,
) -> Result<Vec<GbNode>, Vec<ValidationDiagnostic>> {
    let mut diagnostics = Vec::new();
    validate_routing_shape_for_node_building(quant_graph, &mut diagnostics);
    if !diagnostics.is_empty() {
        return Err(diagnostics);
    }

    let mut builder = NodeBuilder::default();
    let token_input_id = TokenInputId::new(0);
    builder.push(
        InferOp::Embedding {
            token_input: token_input_id,
        },
        vec![required_value(values, ValueAllocationKey::InputToken)],
        Vec::new(),
        vec![required_value(values, ValueAllocationKey::EmbeddingOutput)],
        Vec::new(),
    );

    let model = &quant_graph.identity.model_spec_summary;
    let mut residual_stream = required_value(values, ValueAllocationKey::EmbeddingOutput);
    for layer_index in 0..model.n_layers {
        let layer = LayerId::new(layer_index);
        let layer_norms = quant_graph
            .layer_norms
            .get(&layer)
            .expect("value allocation requires layer norms");
        let pre_sequence_site = norm_plan_by_id(quant_graph, layer_norms.pre_sequence)
            .expect("value allocation requires pre-sequence norm")
            .site;
        let pre_sequence_output = required_value(
            values,
            ValueAllocationKey::NormOutput {
                site: pre_sequence_site,
            },
        );
        builder.push(
            InferOp::Norm {
                plan: layer_norms.pre_sequence,
            },
            vec![residual_stream],
            Vec::new(),
            vec![pre_sequence_output],
            Vec::new(),
        );

        let post_sequence =
            required_value(values, ValueAllocationKey::PostSequenceResidual { layer });
        builder.push(
            InferOp::CombineResidual {
                layer: Some(layer),
                site: ResidualSite::PostSequence,
            },
            vec![pre_sequence_output],
            Vec::new(),
            vec![post_sequence],
            Vec::new(),
        );

        let pre_ffn_site = norm_plan_by_id(quant_graph, layer_norms.pre_ffn)
            .expect("value allocation requires pre-ffn norm")
            .site;
        let pre_ffn_output = required_value(
            values,
            ValueAllocationKey::NormOutput { site: pre_ffn_site },
        );
        builder.push(
            InferOp::Norm {
                plan: layer_norms.pre_ffn,
            },
            vec![post_sequence],
            Vec::new(),
            vec![pre_ffn_output],
            Vec::new(),
        );

        if layer_kind(quant_graph, layer) == Some(FfnKindTag::Routed) {
            let router_score = required_value(values, ValueAllocationKey::RouterScore { layer });
            builder.push(
                InferOp::RouterMatVec { layer },
                vec![pre_ffn_output],
                Vec::new(),
                vec![router_score],
                Vec::new(),
            );
            builder.push(
                InferOp::RouteTop1 { layer },
                vec![router_score],
                Vec::new(),
                vec![
                    required_value(values, ValueAllocationKey::RouterDecision { layer }),
                    required_value(values, ValueAllocationKey::GateWeight { layer }),
                ],
                Vec::new(),
            );
        }

        for expert in expected_experts_for_layer(quant_graph, layer) {
            if ffn_activation_kind(quant_graph, layer) == Some(FfnActivationKind::SwiGLU) {
                builder.push(
                    InferOp::ExpertMatVec {
                        layer,
                        expert,
                        slot: ExpertWeightSlot::FfnGate,
                    },
                    vec![pre_ffn_output],
                    Vec::new(),
                    vec![required_value(
                        values,
                        ValueAllocationKey::ExpertIntermediate {
                            layer,
                            expert,
                            stage: ExpertIntermediateStage::FfnGate,
                        },
                    )],
                    Vec::new(),
                );
            }
            let ffn_up = required_value(
                values,
                ValueAllocationKey::ExpertIntermediate {
                    layer,
                    expert,
                    stage: ExpertIntermediateStage::FfnUp,
                },
            );
            builder.push(
                InferOp::ExpertMatVec {
                    layer,
                    expert,
                    slot: ExpertWeightSlot::FfnUp,
                },
                vec![pre_ffn_output],
                Vec::new(),
                vec![ffn_up],
                Vec::new(),
            );

            let mut activation_inputs = vec![ffn_up];
            if ffn_activation_kind(quant_graph, layer) == Some(FfnActivationKind::SwiGLU) {
                activation_inputs.push(required_value(
                    values,
                    ValueAllocationKey::ExpertIntermediate {
                        layer,
                        expert,
                        stage: ExpertIntermediateStage::FfnGate,
                    },
                ));
            }
            let activation = required_value(
                values,
                ValueAllocationKey::ExpertIntermediate {
                    layer,
                    expert,
                    stage: ExpertIntermediateStage::Activation,
                },
            );
            builder.push(
                InferOp::FfnActivation { layer, expert },
                activation_inputs,
                Vec::new(),
                vec![activation],
                Vec::new(),
            );

            builder.push(
                InferOp::ExpertMatVec {
                    layer,
                    expert,
                    slot: ExpertWeightSlot::FfnDown,
                },
                vec![activation],
                Vec::new(),
                vec![required_value(
                    values,
                    ValueAllocationKey::ExpertCandidate { layer, expert },
                )],
                Vec::new(),
            );
        }

        let expert_result = if layer_kind(quant_graph, layer) == Some(FfnKindTag::Routed) {
            let mut select_inputs = vec![
                required_value(values, ValueAllocationKey::RouterDecision { layer }),
                required_value(values, ValueAllocationKey::GateWeight { layer }),
            ];
            select_inputs.extend(
                expected_experts_for_layer(quant_graph, layer)
                    .into_iter()
                    .map(|expert| {
                        required_value(
                            values,
                            ValueAllocationKey::ExpertCandidate { layer, expert },
                        )
                    }),
            );
            let expert_output = required_value(values, ValueAllocationKey::ExpertOutput { layer });
            builder.push(
                InferOp::SelectExpertTop1 { layer },
                select_inputs,
                Vec::new(),
                vec![expert_output],
                Vec::new(),
            );
            expert_output
        } else {
            required_value(
                values,
                ValueAllocationKey::ExpertCandidate {
                    layer,
                    expert: ExpertId::new(0),
                },
            )
        };

        let post_ffn = required_value(values, ValueAllocationKey::PostFfnResidual { layer });
        builder.push(
            InferOp::CombineResidual {
                layer: Some(layer),
                site: ResidualSite::PostFfn,
            },
            vec![post_sequence, expert_result],
            Vec::new(),
            vec![post_ffn],
            Vec::new(),
        );
        residual_stream = post_ffn;
    }

    let final_norm = required_value(values, ValueAllocationKey::FinalNorm);
    builder.push(
        InferOp::Norm {
            plan: quant_graph
                .norm_plan(NormSite::Final)
                .expect("value allocation requires final norm")
                .norm_plan_id,
        },
        vec![residual_stream],
        Vec::new(),
        vec![final_norm],
        Vec::new(),
    );
    let logits = required_value(values, ValueAllocationKey::Logits);
    builder.push(
        InferOp::Classify,
        vec![final_norm],
        Vec::new(),
        vec![logits],
        Vec::new(),
    );

    let (effects_in, effects_out) = effects
        .decode_rng
        .map(|pair| (vec![pair.input], vec![pair.output]))
        .unwrap_or_default();
    builder.push(
        InferOp::DecodeToken {
            plan: quant_graph.decode_spec.decode_plan_id,
        },
        vec![logits],
        effects_in,
        vec![required_value(values, ValueAllocationKey::DecodedToken)],
        effects_out,
    );

    tracing::info!(
        n_nodes = builder.nodes.len() as u64,
        "stage3.binding.node_building"
    );
    Ok(builder.nodes)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReductionSiteRequirement {
    Forbidden,
    Required,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpSignature {
    pub input_kinds: Vec<ValueKind>,
    pub output_kinds: Vec<ValueKind>,
    pub effects_in: Vec<EffectClass>,
    pub effects_out: Vec<EffectClass>,
    pub reduction_site: ReductionSiteRequirement,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpSignatureMismatch {
    pub node_id: NodeId,
    pub op_tag: InferOpTag,
    pub reason: String,
}

pub fn op_signature(
    op: InferOp,
    quant_graph: &QuantGraph,
) -> Result<OpSignature, Vec<ValidationDiagnostic>> {
    let signature = match op {
        InferOp::Embedding { .. } => OpSignature {
            input_kinds: vec![ValueKind::InputToken],
            output_kinds: vec![ValueKind::EmbeddingOutput],
            effects_in: Vec::new(),
            effects_out: Vec::new(),
            reduction_site: ReductionSiteRequirement::Forbidden,
        },
        InferOp::SequenceRead { slot } => {
            let class = EffectClass::SequenceState { slot };
            OpSignature {
                input_kinds: Vec::new(),
                output_kinds: vec![ValueKind::SequenceStateRead],
                effects_in: vec![class],
                effects_out: vec![class],
                reduction_site: ReductionSiteRequirement::Forbidden,
            }
        }
        InferOp::SequenceStep { layer } => {
            let slots = state_slots_for_layer(quant_graph, layer);
            let mut input_kinds = vec![ValueKind::NormalizedActivation];
            input_kinds.extend(std::iter::repeat_n(
                ValueKind::SequenceStateRead,
                slots.len(),
            ));
            let mut output_kinds = vec![ValueKind::SequenceBlockOutput];
            output_kinds.extend(std::iter::repeat_n(
                ValueKind::SequenceStateNext,
                slots.len(),
            ));
            OpSignature {
                input_kinds,
                output_kinds,
                effects_in: Vec::new(),
                effects_out: Vec::new(),
                reduction_site: ReductionSiteRequirement::Forbidden,
            }
        }
        InferOp::SequenceWrite { slot } => {
            let class = EffectClass::SequenceState { slot };
            OpSignature {
                input_kinds: vec![ValueKind::SequenceStateNext],
                output_kinds: Vec::new(),
                effects_in: vec![class],
                effects_out: vec![class],
                reduction_site: ReductionSiteRequirement::Forbidden,
            }
        }
        InferOp::RouterMatVec { .. } => OpSignature {
            input_kinds: vec![ValueKind::NormalizedActivation],
            output_kinds: vec![ValueKind::RouterScore],
            effects_in: Vec::new(),
            effects_out: Vec::new(),
            reduction_site: ReductionSiteRequirement::Required,
        },
        InferOp::RouteTop1 { .. } => OpSignature {
            input_kinds: vec![ValueKind::RouterScore],
            output_kinds: vec![ValueKind::RouterDecision, ValueKind::GateWeight],
            effects_in: Vec::new(),
            effects_out: Vec::new(),
            reduction_site: ReductionSiteRequirement::Forbidden,
        },
        InferOp::SelectExpertTop1 { layer } => {
            let mut input_kinds = vec![ValueKind::RouterDecision, ValueKind::GateWeight];
            input_kinds.extend(std::iter::repeat_n(
                ValueKind::ExpertCandidate,
                usize::from(n_experts_for_layer(quant_graph, layer)),
            ));
            OpSignature {
                input_kinds,
                output_kinds: vec![ValueKind::ExpertOutput],
                effects_in: Vec::new(),
                effects_out: Vec::new(),
                reduction_site: ReductionSiteRequirement::Forbidden,
            }
        }
        InferOp::ExpertMatVec { slot, .. } => {
            let (input_kinds, output_kinds) = match slot {
                ExpertWeightSlot::FfnGate | ExpertWeightSlot::FfnUp => (
                    vec![ValueKind::NormalizedActivation],
                    vec![ValueKind::ExpertIntermediate],
                ),
                ExpertWeightSlot::FfnDown => (
                    vec![ValueKind::ExpertIntermediate],
                    vec![ValueKind::ExpertCandidate],
                ),
            };
            OpSignature {
                input_kinds,
                output_kinds,
                effects_in: Vec::new(),
                effects_out: Vec::new(),
                reduction_site: ReductionSiteRequirement::Required,
            }
        }
        InferOp::FfnActivation { layer, .. } => {
            let mut input_kinds = vec![ValueKind::ExpertIntermediate];
            if ffn_activation_kind(quant_graph, layer) == Some(FfnActivationKind::SwiGLU) {
                input_kinds.push(ValueKind::ExpertIntermediate);
            }
            OpSignature {
                input_kinds,
                output_kinds: vec![ValueKind::ExpertIntermediate],
                effects_in: Vec::new(),
                effects_out: Vec::new(),
                reduction_site: ReductionSiteRequirement::Forbidden,
            }
        }
        InferOp::CombineResidual { layer, site } => {
            let input_kinds = match site {
                ResidualSite::PostSequence => vec![ValueKind::NormalizedActivation],
                ResidualSite::PostFfn => {
                    match layer.and_then(|layer| layer_kind(quant_graph, layer)) {
                        Some(FfnKindTag::Routed) => {
                            vec![ValueKind::Activation, ValueKind::ExpertOutput]
                        }
                        _ => vec![ValueKind::Activation, ValueKind::ExpertCandidate],
                    }
                }
            };
            OpSignature {
                input_kinds,
                output_kinds: vec![ValueKind::Activation],
                effects_in: Vec::new(),
                effects_out: Vec::new(),
                reduction_site: ReductionSiteRequirement::Forbidden,
            }
        }
        InferOp::Norm { plan } => {
            let Some(record) = norm_plan_by_id(quant_graph, plan) else {
                return Err(vec![hard_semantic_diagnostic(
                    "norm_plans.norm_plan_id",
                    ValidationDetail::Field {
                        field: FieldPath::from("norm_plans.norm_plan_id"),
                    },
                )]);
            };
            OpSignature {
                input_kinds: vec![ValueKind::Activation],
                output_kinds: vec![ValueKind::NormalizedActivation],
                effects_in: Vec::new(),
                effects_out: Vec::new(),
                reduction_site: if matches!(record.plan, NormPlan::TileRmsThenAffineClip(_)) {
                    ReductionSiteRequirement::Required
                } else {
                    ReductionSiteRequirement::Forbidden
                },
            }
        }
        InferOp::Classify => OpSignature {
            input_kinds: vec![ValueKind::NormalizedActivation],
            output_kinds: vec![ValueKind::LogitVector],
            effects_in: Vec::new(),
            effects_out: Vec::new(),
            reduction_site: ReductionSiteRequirement::Required,
        },
        InferOp::DecodeToken { .. } => {
            let rng = quant_graph
                .decode_spec
                .requires_rng
                .then_some(EffectClass::Rng {
                    slot: RngSlot::Decode,
                });
            OpSignature {
                input_kinds: vec![ValueKind::LogitVector],
                output_kinds: vec![ValueKind::DecodedToken],
                effects_in: rng.into_iter().collect(),
                effects_out: rng.into_iter().collect(),
                reduction_site: ReductionSiteRequirement::Forbidden,
            }
        }
    };
    Ok(signature)
}

#[must_use]
pub fn reduction_site_bearing(op: InferOp, quant_graph: &QuantGraph) -> bool {
    op_signature(op, quant_graph)
        .is_ok_and(|signature| signature.reduction_site == ReductionSiteRequirement::Required)
}

pub fn check_op_signature(
    node: &GbNode,
    values: &[ValueDecl],
    effects: &[EffectDecl],
    quant_graph: &QuantGraph,
) -> Result<(), OpSignatureMismatch> {
    let signature =
        op_signature(node.op, quant_graph).map_err(|diagnostics| OpSignatureMismatch {
            node_id: node.node_id,
            op_tag: node.op.tag(),
            reason: format!("signature unavailable: {diagnostics:?}"),
        })?;
    let values_by_id = values
        .iter()
        .map(|value| (value.value_id, value.kind))
        .collect::<BTreeMap<_, _>>();
    let effects_by_id = effects
        .iter()
        .map(|effect| (effect.effect_id, effect.class))
        .collect::<BTreeMap<_, _>>();

    let input_kinds =
        value_kinds(&node.inputs, &values_by_id).map_err(|reason| OpSignatureMismatch {
            node_id: node.node_id,
            op_tag: node.op.tag(),
            reason,
        })?;
    let output_kinds =
        value_kinds(&node.outputs, &values_by_id).map_err(|reason| OpSignatureMismatch {
            node_id: node.node_id,
            op_tag: node.op.tag(),
            reason,
        })?;
    let effects_in =
        effect_classes(&node.effects_in, &effects_by_id).map_err(|reason| OpSignatureMismatch {
            node_id: node.node_id,
            op_tag: node.op.tag(),
            reason,
        })?;
    let effects_out = effect_classes(&node.effects_out, &effects_by_id).map_err(|reason| {
        OpSignatureMismatch {
            node_id: node.node_id,
            op_tag: node.op.tag(),
            reason,
        }
    })?;

    let accepted = input_kinds_match(node.op, &input_kinds, &signature.input_kinds)
        && output_kinds == signature.output_kinds
        && effects_in == signature.effects_in
        && effects_out == signature.effects_out
        && match signature.reduction_site {
            ReductionSiteRequirement::Forbidden => node.reduction_site.is_none(),
            ReductionSiteRequirement::Required => node.reduction_site.is_some(),
        };
    tracing::debug!(
        node_id_placeholder = node.node_id.get() as u64,
        op_tag = ?node.op.tag(),
        accepted,
        "stage3.op_signature.check"
    );

    if accepted {
        Ok(())
    } else {
        Err(OpSignatureMismatch {
            node_id: node.node_id,
            op_tag: node.op.tag(),
            reason: "node does not match op signature".to_owned(),
        })
    }
}

#[derive(Default)]
struct ValueAllocationBuilder {
    values: Vec<ValueDecl>,
    by_key: BTreeMap<ValueAllocationKey, ValueId>,
    next_value_id: u32,
}

impl ValueAllocationBuilder {
    fn push(
        &mut self,
        key: ValueAllocationKey,
        kind: ValueKind,
        format: ValueFormat,
        layout: ValueLayout,
    ) -> ValueId {
        let value_id = ValueId::new(self.next_value_id);
        self.next_value_id += 1;
        self.values.push(ValueDecl {
            value_id,
            kind,
            format,
            layout,
        });
        let previous = self.by_key.insert(key, value_id);
        debug_assert!(previous.is_none(), "duplicate value allocation key");
        value_id
    }

    fn finish(self) -> ValueAllocation {
        ValueAllocation {
            values: self.values,
            by_key: self.by_key,
        }
    }
}

#[derive(Default)]
struct NodeBuilder {
    nodes: Vec<GbNode>,
}

impl NodeBuilder {
    // These NodeIds are provisional handles for the class-5 intermediate.
    // Class 9 canonical sort overwrites them after ordering is finalized.
    fn push(
        &mut self,
        op: InferOp,
        inputs: Vec<ValueId>,
        effects_in: Vec<EffectId>,
        outputs: Vec<ValueId>,
        effects_out: Vec<EffectId>,
    ) {
        let node_id = NodeId::new(self.nodes.len() as u32);
        tracing::debug!(
            layer = op.layer().map(|layer| layer.get()),
            op_tag = ?op.tag(),
            "stage3.binding.node"
        );
        self.nodes.push(GbNode {
            node_id,
            op,
            inputs,
            effects_in,
            outputs,
            effects_out,
            reduction_site: None,
        });
    }
}

fn hard_semantic_diagnostic(
    field: impl Into<FieldPath>,
    detail: ValidationDetail,
) -> ValidationDiagnostic {
    let field = field.into();
    ValidationDiagnostic::hard(
        ValidationOrigin::SemanticCore,
        ValidationCode::ReportSemanticInvariantViolated {
            field: field.clone(),
        },
        detail,
        Vec::new(),
    )
}

fn hash_infer_ir_policy_projection(
    projection: &InferIrPolicyProjection,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) -> Option<Hash256> {
    canonical_domain_hash(
        "gbf-codegen",
        "InferIrPolicyProjection",
        INFER_IR_SCHEMA_ID,
        projection,
        "policy_projection",
        diagnostics,
    )
}

fn hash_requested_runtime_modes(
    requested_runtime_modes: &BTreeSet<RuntimeMode>,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) -> Option<Hash256> {
    canonical_domain_hash(
        "gbf-policy",
        "RuntimeModeSet",
        "v1",
        requested_runtime_modes,
        "policy_projection.requested_runtime_modes",
        diagnostics,
    )
}

fn canonical_domain_hash<T: Serialize>(
    namespace: &str,
    type_name: &str,
    schema_version: &str,
    value: &T,
    field: &'static str,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) -> Option<Hash256> {
    let value = match serde_json::to_value(value) {
        Ok(value) => value,
        Err(error) => {
            diagnostics.push(hard_semantic_diagnostic(
                field,
                ValidationDetail::Field {
                    field: FieldPath::from(error.to_string()),
                },
            ));
            return None;
        }
    };
    let canonical = match canonicalize_value(&value) {
        Ok(canonical) => canonical,
        Err(error) => {
            diagnostics.push(hard_semantic_diagnostic(
                field,
                ValidationDetail::Field {
                    field: FieldPath::from(error.to_string()),
                },
            ));
            return None;
        }
    };

    Some(domain_hash_in_namespace(
        namespace,
        type_name,
        schema_version,
        &canonical,
    ))
}

fn report_determinism_class(class: DeterminismClass) -> qg_report_schema::DeterminismClassTag {
    match class {
        DeterminismClass::BitExact => qg_report_schema::DeterminismClassTag::BitExact,
        DeterminismClass::Deterministic => qg_report_schema::DeterminismClassTag::Deterministic,
        DeterminismClass::Nondeterministic => {
            qg_report_schema::DeterminismClassTag::Nondeterministic
        }
    }
}

fn report_infer_op_tag(tag: InferOpTag) -> report_schema::InferOpTag {
    match tag {
        InferOpTag::Classify => report_schema::InferOpTag::Classify,
        InferOpTag::CombineResidual => report_schema::InferOpTag::CombineResidual,
        InferOpTag::DecodeToken => report_schema::InferOpTag::DecodeToken,
        InferOpTag::Embedding => report_schema::InferOpTag::Embedding,
        InferOpTag::ExpertMatVec => report_schema::InferOpTag::ExpertMatVec,
        InferOpTag::FfnActivation => report_schema::InferOpTag::FfnActivation,
        InferOpTag::Norm => report_schema::InferOpTag::Norm,
        InferOpTag::RouteTop1 => report_schema::InferOpTag::RouteTop1,
        InferOpTag::RouterMatVec => report_schema::InferOpTag::RouterMatVec,
        InferOpTag::SelectExpertTop1 => report_schema::InferOpTag::SelectExpertTop1,
        InferOpTag::SequenceRead => report_schema::InferOpTag::SequenceRead,
        InferOpTag::SequenceStep => report_schema::InferOpTag::SequenceStep,
        InferOpTag::SequenceWrite => report_schema::InferOpTag::SequenceWrite,
    }
}

fn report_effect_class_tag(tag: EffectClassTag) -> report_schema::EffectClassTag {
    match tag {
        EffectClassTag::FaultBoundary => report_schema::EffectClassTag::FaultBoundary,
        EffectClassTag::Rng => report_schema::EffectClassTag::Rng,
        EffectClassTag::SequenceState => report_schema::EffectClassTag::SequenceState,
    }
}

fn report_value_kind_tag(tag: ValueKind) -> report_schema::ValueKindTag {
    match tag {
        ValueKind::Activation => report_schema::ValueKindTag::Activation,
        ValueKind::DecodedToken => report_schema::ValueKindTag::DecodedToken,
        ValueKind::EmbeddingOutput => report_schema::ValueKindTag::EmbeddingOutput,
        ValueKind::ExpertCandidate => report_schema::ValueKindTag::ExpertCandidate,
        ValueKind::ExpertIntermediate => report_schema::ValueKindTag::ExpertIntermediate,
        ValueKind::ExpertOutput => report_schema::ValueKindTag::ExpertOutput,
        ValueKind::GateWeight => report_schema::ValueKindTag::GateWeight,
        ValueKind::InputToken => report_schema::ValueKindTag::InputToken,
        ValueKind::LogitVector => report_schema::ValueKindTag::LogitVector,
        ValueKind::NormalizedActivation => report_schema::ValueKindTag::NormalizedActivation,
        ValueKind::RouterDecision => report_schema::ValueKindTag::RouterDecision,
        ValueKind::RouterScore => report_schema::ValueKindTag::RouterScore,
        ValueKind::SequenceBlockOutput => report_schema::ValueKindTag::SequenceBlockOutput,
        ValueKind::SequenceStateNext => report_schema::ValueKindTag::SequenceStateNext,
        ValueKind::SequenceStateRead => report_schema::ValueKindTag::SequenceStateRead,
    }
}

fn usize_to_u32(value: usize, field: &'static str) -> Result<u32, GbInferIrProductError> {
    u32::try_from(value).map_err(|_| GbInferIrProductError::CountOverflow(field))
}

fn usize_to_u16(value: usize, field: &'static str) -> Result<u16, GbInferIrProductError> {
    u16::try_from(value).map_err(|_| GbInferIrProductError::CountOverflow(field))
}

fn usize_to_u8(value: usize, field: &'static str) -> Result<u8, GbInferIrProductError> {
    u8::try_from(value).map_err(|_| GbInferIrProductError::CountOverflow(field))
}

fn tensor_format_by_role(
    quant_graph: &QuantGraph,
    predicate: impl Fn(&QuantTensorRole) -> bool,
) -> Option<QuantFormat> {
    quant_graph
        .tensors
        .iter()
        .find(|tensor| predicate(&tensor.role))
        .map(|tensor| tensor.quant_format.clone())
}

fn vector_layout(axis: ValueAxis) -> ValueLayout {
    ValueLayout { shape: vec![axis] }
}

fn norm_plan_by_id(
    quant_graph: &QuantGraph,
    norm_plan_id: NormPlanId,
) -> Option<&crate::s1::quant_graph::NormPlanRecord> {
    quant_graph
        .norm_plans
        .iter()
        .find(|record| record.norm_plan_id == norm_plan_id)
}

fn layer_kind(quant_graph: &QuantGraph, layer: LayerId) -> Option<FfnKindTag> {
    quant_graph
        .identity
        .model_spec_summary
        .ffn_kind
        .get(&layer)
        .copied()
}

fn ffn_activation_kind(quant_graph: &QuantGraph, layer: LayerId) -> Option<FfnActivationKind> {
    quant_graph
        .ffn_plans
        .get(&layer)
        .map(|plan| plan.activation_kind)
}

fn n_experts_for_layer(quant_graph: &QuantGraph, layer: LayerId) -> u16 {
    match layer_kind(quant_graph, layer) {
        Some(FfnKindTag::Dense) => 1,
        Some(FfnKindTag::Routed) => quant_graph
            .identity
            .model_spec_summary
            .n_experts
            .get(&layer)
            .copied()
            .unwrap_or(0),
        None => 0,
    }
}

fn expected_experts_for_layer(quant_graph: &QuantGraph, layer: LayerId) -> Vec<ExpertId> {
    (0..n_experts_for_layer(quant_graph, layer))
        .map(ExpertId::new)
        .collect()
}

fn state_slots_for_layer(quant_graph: &QuantGraph, layer: LayerId) -> Vec<StateSlotId> {
    let mut slots = quant_graph
        .sequence_semantics
        .state_slots
        .get(&layer)
        .into_iter()
        .flat_map(|slots| slots.iter())
        .map(|slot| StateSlotId::new(u32::from(slot.slot_id)))
        .collect::<Vec<_>>();
    slots.sort();
    slots
}

#[cfg(any(feature = "semantic_equivalence_check", test))]
fn semantic_op_tags_from_quant_graph(quant_graph: &QuantGraph) -> Vec<InferOpTag> {
    let model = &quant_graph.identity.model_spec_summary;
    let mut tags = vec![InferOpTag::Embedding];
    for layer_index in 0..model.n_layers {
        let layer = LayerId::new(layer_index);
        tags.extend([
            InferOpTag::Norm,
            InferOpTag::CombineResidual,
            InferOpTag::Norm,
        ]);
        if layer_kind(quant_graph, layer) == Some(FfnKindTag::Routed) {
            tags.extend([InferOpTag::RouterMatVec, InferOpTag::RouteTop1]);
        }
        let experts = expected_experts_for_layer(quant_graph, layer);
        if ffn_activation_kind(quant_graph, layer) == Some(FfnActivationKind::SwiGLU) {
            for _expert in &experts {
                tags.push(InferOpTag::ExpertMatVec);
            }
        }
        for _expert in &experts {
            tags.push(InferOpTag::ExpertMatVec);
        }
        for _expert in &experts {
            tags.push(InferOpTag::FfnActivation);
        }
        for _expert in &experts {
            tags.push(InferOpTag::ExpertMatVec);
        }
        if layer_kind(quant_graph, layer) == Some(FfnKindTag::Routed) {
            tags.push(InferOpTag::SelectExpertTop1);
        }
        tags.push(InferOpTag::CombineResidual);
    }
    tags.extend([
        InferOpTag::Norm,
        InferOpTag::Classify,
        InferOpTag::DecodeToken,
    ]);
    tags
}

#[cfg(any(feature = "semantic_equivalence_check", test))]
mod canonical {
    pub(super) mod reference {
        use serde::Serialize;

        use super::super::*;

        #[derive(Debug, Clone, PartialEq, Eq)]
        pub(in crate::s3::infer_ir) struct CanonicalReferenceOutput {
            pub(in crate::s3::infer_ir) trace_hash: Hash256,
        }

        pub(in crate::s3::infer_ir) fn eval_canonical_ir(
            infer_ir: &GbInferIR,
            input: &FixtureInput,
        ) -> CanonicalReferenceOutput {
            let op_tags = infer_ir
                .nodes
                .iter()
                .map(|node| node.op.tag())
                .collect::<Vec<_>>();
            let decode_plan = infer_ir.nodes.iter().find_map(|node| match node.op {
                InferOp::DecodeToken { plan } => Some(plan.get()),
                _ => None,
            });
            let requires_rng = infer_ir
                .effects
                .iter()
                .any(|effect| matches!(effect.class, EffectClass::Rng { .. }));
            reference_output(ReferenceMaterial {
                evaluator: "ir",
                op_tags,
                decode_plan,
                requires_rng,
                token_id: input.token_id,
                sequence_state_seed: input.sequence_state_seed,
                rng_seed: input.rng_seed,
                numeric_boundaries: SEMANTIC_EQUIVALENCE_NUMERIC_BOUNDARIES,
            })
        }

        pub(in crate::s3::infer_ir) fn eval_canonical_qg(
            quant_graph: &QuantGraph,
            input: &FixtureInput,
        ) -> CanonicalReferenceOutput {
            reference_output(ReferenceMaterial {
                evaluator: "ir",
                op_tags: semantic_op_tags_from_quant_graph(quant_graph),
                decode_plan: Some(quant_graph.decode_spec.decode_plan_id.get()),
                requires_rng: quant_graph.decode_spec.requires_rng,
                token_id: input.token_id,
                sequence_state_seed: input.sequence_state_seed,
                rng_seed: input.rng_seed,
                numeric_boundaries: SEMANTIC_EQUIVALENCE_NUMERIC_BOUNDARIES,
            })
        }

        #[derive(Serialize)]
        struct ReferenceMaterial<'a> {
            evaluator: &'a str,
            op_tags: Vec<InferOpTag>,
            decode_plan: Option<u32>,
            requires_rng: bool,
            token_id: u32,
            sequence_state_seed: Hash256,
            rng_seed: Hash256,
            numeric_boundaries: &'static [&'static str],
        }

        fn reference_output(material: ReferenceMaterial<'_>) -> CanonicalReferenceOutput {
            let value = serde_json::to_value(&material).expect("reference material serializes");
            let canonical = canonicalize_value(&value).expect("reference material canonicalizes");
            CanonicalReferenceOutput {
                trace_hash: domain_hash(
                    "FixtureSemanticEquivalence",
                    INFER_IR_SCHEMA_ID,
                    &canonical,
                ),
            }
        }
    }
}

fn validate_routing_shape_for_node_building(
    quant_graph: &QuantGraph,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    for layer_index in 0..quant_graph.identity.model_spec_summary.n_layers {
        let layer = LayerId::new(layer_index);
        let has_router = router_layer(quant_graph, layer).is_some();
        match layer_kind(quant_graph, layer) {
            Some(FfnKindTag::Dense) if has_router => {
                diagnostics.push(infer_ir_router_present_for_dense_diagnostic(layer))
            }
            Some(FfnKindTag::Routed) if !has_router => {
                diagnostics.push(infer_ir_routed_missing_router_diagnostic(layer))
            }
            _ => {}
        }
    }
}

fn router_layer(
    quant_graph: &QuantGraph,
    layer: LayerId,
) -> Option<&crate::s1::quant_graph::RouterLayer> {
    quant_graph
        .routing_table
        .as_ref()
        .and_then(|routing| routing.layers.iter().find(|entry| entry.layer == layer))
}

fn required_value(values: &ValueAllocation, key: ValueAllocationKey) -> ValueId {
    values
        .id(key)
        .expect("value allocation key must exist after successful allocation")
}

fn value_kinds(
    ids: &[ValueId],
    values_by_id: &BTreeMap<ValueId, ValueKind>,
) -> Result<Vec<ValueKind>, String> {
    ids.iter()
        .map(|value_id| {
            values_by_id
                .get(value_id)
                .copied()
                .ok_or_else(|| format!("missing value_id={}", value_id.get()))
        })
        .collect()
}

fn effect_classes(
    ids: &[EffectId],
    effects_by_id: &BTreeMap<EffectId, EffectClass>,
) -> Result<Vec<EffectClass>, String> {
    ids.iter()
        .map(|effect_id| {
            effects_by_id
                .get(effect_id)
                .copied()
                .ok_or_else(|| format!("missing effect_id={}", effect_id.get()))
        })
        .collect()
}

fn input_kinds_match(op: InferOp, observed: &[ValueKind], expected: &[ValueKind]) -> bool {
    if observed == expected {
        return true;
    }
    matches!(op, InferOp::Norm { .. })
        && observed == [ValueKind::EmbeddingOutput].as_slice()
        && expected == [ValueKind::Activation].as_slice()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExpectedValueSpec {
    kind: ValueKind,
    format: ValueFormat,
}

fn expected_output_specs(op: InferOp, quant_graph: &QuantGraph) -> Option<Vec<ExpectedValueSpec>> {
    let specs = match op {
        InferOp::Embedding { .. } => vec![ExpectedValueSpec {
            kind: ValueKind::EmbeddingOutput,
            format: ValueFormat::Quant {
                format: tensor_format_by_role(quant_graph, |role| {
                    matches!(role, QuantTensorRole::EmbeddingTable)
                })?,
            },
        }],
        InferOp::SequenceRead { .. } => vec![ExpectedValueSpec {
            kind: ValueKind::SequenceStateRead,
            format: ValueFormat::ExactAccumulator,
        }],
        InferOp::SequenceStep { layer } => {
            let mut specs = vec![ExpectedValueSpec {
                kind: ValueKind::SequenceBlockOutput,
                format: ValueFormat::Quant {
                    format: quant_graph.residual_plan.activation_format.clone(),
                },
            }];
            specs.extend(
                state_slots_for_layer(quant_graph, layer)
                    .into_iter()
                    .map(|_| ExpectedValueSpec {
                        kind: ValueKind::SequenceStateNext,
                        format: ValueFormat::ExactAccumulator,
                    }),
            );
            specs
        }
        InferOp::SequenceWrite { .. } => Vec::new(),
        InferOp::RouterMatVec { .. } => vec![ExpectedValueSpec {
            kind: ValueKind::RouterScore,
            format: ValueFormat::ExactAccumulator,
        }],
        InferOp::RouteTop1 { layer } => vec![
            ExpectedValueSpec {
                kind: ValueKind::RouterDecision,
                format: ValueFormat::ExpertIdDomain {
                    n_experts: n_experts_for_layer(quant_graph, layer),
                },
            },
            ExpectedValueSpec {
                kind: ValueKind::GateWeight,
                format: ValueFormat::Quant {
                    format: QuantFormat::Q8_8,
                },
            },
        ],
        InferOp::SelectExpertTop1 { .. } => vec![ExpectedValueSpec {
            kind: ValueKind::ExpertOutput,
            format: ValueFormat::ExactAccumulator,
        }],
        InferOp::ExpertMatVec {
            slot: ExpertWeightSlot::FfnDown,
            ..
        } => vec![ExpectedValueSpec {
            kind: ValueKind::ExpertCandidate,
            format: ValueFormat::ExactAccumulator,
        }],
        InferOp::ExpertMatVec { .. } => vec![ExpectedValueSpec {
            kind: ValueKind::ExpertIntermediate,
            format: ValueFormat::ExactAccumulator,
        }],
        InferOp::FfnActivation { layer, .. } => vec![ExpectedValueSpec {
            kind: ValueKind::ExpertIntermediate,
            format: ValueFormat::Quant {
                format: quant_graph
                    .ffn_plans
                    .get(&layer)?
                    .intermediate_format
                    .clone(),
            },
        }],
        InferOp::CombineResidual { .. } => vec![ExpectedValueSpec {
            kind: ValueKind::Activation,
            format: ValueFormat::Quant {
                format: quant_graph.residual_plan.activation_format.clone(),
            },
        }],
        InferOp::Norm { plan } => vec![ExpectedValueSpec {
            kind: ValueKind::NormalizedActivation,
            format: ValueFormat::Quant {
                format: norm_plan_by_id(quant_graph, plan)?.output_format.clone(),
            },
        }],
        InferOp::Classify => vec![ExpectedValueSpec {
            kind: ValueKind::LogitVector,
            format: ValueFormat::ExactAccumulator,
        }],
        InferOp::DecodeToken { .. } => vec![ExpectedValueSpec {
            kind: ValueKind::DecodedToken,
            format: ValueFormat::TokenIdDomain {
                vocab_size: quant_graph.identity.model_spec_summary.vocab_size,
            },
        }],
    };
    Some(specs)
}

fn values_by_id(values: &[ValueDecl]) -> BTreeMap<ValueId, &ValueDecl> {
    values.iter().map(|value| (value.value_id, value)).collect()
}

fn reduction_site_key_for_op(op: InferOp, quant_graph: &QuantGraph) -> Option<ReductionSiteKey> {
    match op {
        InferOp::RouterMatVec { layer } => Some(ReductionSiteKey::RouterMatVec { layer }),
        InferOp::ExpertMatVec {
            layer,
            expert,
            slot,
        } => Some(ReductionSiteKey::ExpertMatVec {
            layer,
            expert,
            slot,
        }),
        InferOp::Norm { plan } => norm_plan_by_id(quant_graph, plan)
            .is_some_and(|record| matches!(record.plan, NormPlan::TileRmsThenAffineClip(_)))
            .then_some(ReductionSiteKey::Norm { norm_plan: plan }),
        InferOp::Classify => Some(ReductionSiteKey::Classify {
            classify_weight: quant_graph.classify_head.weight,
        }),
        _ => None,
    }
}

fn canonical_reduction_site_id(key: ReductionSiteKey) -> ReductionSiteId {
    let id = match key {
        ReductionSiteKey::RouterMatVec { layer } => format!("router.{}", layer.get()),
        ReductionSiteKey::ExpertMatVec {
            layer,
            expert,
            slot,
        } => format!(
            "expert.{}.{}.{}",
            layer.get(),
            expert.get(),
            expert_slot_label(slot)
        ),
        ReductionSiteKey::Norm { norm_plan } => format!("norm.{}", norm_plan.get()),
        ReductionSiteKey::Classify { .. } => "classify".to_owned(),
    };
    ReductionSiteId(id)
}

fn expert_slot_label(slot: ExpertWeightSlot) -> &'static str {
    match slot {
        ExpertWeightSlot::FfnGate => "gate",
        ExpertWeightSlot::FfnUp => "up",
        ExpertWeightSlot::FfnDown => "down",
    }
}

fn expert_slot_stage(slot: ExpertWeightSlot) -> u8 {
    match slot {
        ExpertWeightSlot::FfnGate => 8,
        ExpertWeightSlot::FfnUp => 9,
        ExpertWeightSlot::FfnDown => 11,
    }
}

fn canonical_node_sort_key(node: &GbNode, quant_graph: &QuantGraph) -> (i32, u8, u32, String) {
    let n_layers = i32::from(quant_graph.identity.model_spec_summary.n_layers);
    let (primary, secondary, tertiary) = match node.op {
        InferOp::Embedding { .. } => (-1, 0, 0),
        InferOp::Norm { plan } => {
            match norm_plan_by_id(quant_graph, plan).map(|record| record.site) {
                Some(NormSite::LayerSequence { layer }) => (i32::from(layer.get()), 0, 0),
                Some(NormSite::LayerFfn { layer }) => (i32::from(layer.get()), 5, 0),
                Some(NormSite::Final) | None => (n_layers, 0, 0),
            }
        }
        InferOp::SequenceRead { slot } => (0, 1, slot.get()),
        InferOp::SequenceStep { layer } => (i32::from(layer.get()), 2, 0),
        InferOp::SequenceWrite { slot } => (0, 3, slot.get()),
        InferOp::CombineResidual {
            layer,
            site: ResidualSite::PostSequence,
        } => (layer.map_or(0, |layer| i32::from(layer.get())), 4, 0),
        InferOp::RouterMatVec { layer } => (i32::from(layer.get()), 6, 0),
        InferOp::RouteTop1 { layer } => (i32::from(layer.get()), 7, 0),
        InferOp::ExpertMatVec {
            layer,
            expert,
            slot,
        } => (
            i32::from(layer.get()),
            expert_slot_stage(slot),
            u32::from(expert.get()),
        ),
        InferOp::FfnActivation { layer, expert } => {
            (i32::from(layer.get()), 10, u32::from(expert.get()))
        }
        InferOp::SelectExpertTop1 { layer } => (i32::from(layer.get()), 12, 0),
        InferOp::CombineResidual {
            layer,
            site: ResidualSite::PostFfn,
        } => (layer.map_or(0, |layer| i32::from(layer.get())), 13, 0),
        InferOp::Classify => (n_layers + 1, 0, 0),
        InferOp::DecodeToken { .. } => (n_layers + 2, 0, 0),
    };
    let tuple = canonical_provenance_tuple_base(node.op, quant_graph);
    let quaternary =
        serde_json::to_string(&tuple).expect("canonical provenance tuple serializes for sort key");
    (primary, secondary, tertiary, quaternary)
}

fn canonical_provenance_tuples(
    nodes: &[GbNode],
    quant_graph: &QuantGraph,
) -> BTreeMap<NodeId, CanonicalProvenanceTuple> {
    let mut counts = BTreeMap::<String, u32>::new();
    let mut tuples = BTreeMap::new();
    for node in nodes {
        let mut tuple = canonical_provenance_tuple_base(node.op, quant_graph);
        let key = serde_json::to_string(&tuple)
            .expect("canonical provenance tuple serializes for occurrence key");
        let occurrence = counts.entry(key).or_default();
        tuple.occurrence_index = *occurrence;
        *occurrence += 1;
        tuples.insert(node.node_id, tuple);
    }
    tuples
}

fn canonical_provenance_tuple_base(
    op: InferOp,
    quant_graph: &QuantGraph,
) -> CanonicalProvenanceTuple {
    let mut tuple = CanonicalProvenanceTuple::new(op.tag(), 0);
    match op {
        InferOp::Embedding { .. } => {}
        InferOp::SequenceRead { slot } | InferOp::SequenceWrite { slot } => {
            tuple.state_slot = Some(slot);
        }
        InferOp::SequenceStep { layer }
        | InferOp::RouterMatVec { layer }
        | InferOp::RouteTop1 { layer }
        | InferOp::SelectExpertTop1 { layer } => {
            tuple.layer = Some(layer);
        }
        InferOp::ExpertMatVec {
            layer,
            expert,
            slot,
        } => {
            tuple.layer = Some(layer);
            tuple.expert = Some(expert);
            tuple.expert_weight_slot = Some(slot);
        }
        InferOp::FfnActivation { layer, expert } => {
            tuple.layer = Some(layer);
            tuple.expert = Some(expert);
        }
        InferOp::CombineResidual { layer, site } => {
            tuple.layer = layer;
            tuple.residual_site = Some(site);
        }
        InferOp::Norm { plan } => {
            tuple.norm_site = norm_plan_by_id(quant_graph, plan).map(|record| record.site);
        }
        InferOp::Classify => {}
        InferOp::DecodeToken { .. } => {}
    }
    tuple
}

fn quant_graph_entity_ref_for_op(op: InferOp, quant_graph: &QuantGraph) -> QuantGraphEntityRef {
    match op {
        InferOp::Embedding { token_input } => QuantGraphEntityRef::TokenInput { token_input },
        InferOp::Norm { plan } => QuantGraphEntityRef::NormPlan { plan },
        InferOp::SequenceRead { slot } | InferOp::SequenceWrite { slot } => {
            QuantGraphEntityRef::SequenceSlot { slot }
        }
        InferOp::SequenceStep { layer } => QuantGraphEntityRef::SequenceStep { layer },
        InferOp::RouterMatVec { layer } => QuantGraphEntityRef::RouterLayer { layer },
        InferOp::RouteTop1 { layer } | InferOp::SelectExpertTop1 { layer } => {
            QuantGraphEntityRef::RouterSelection { layer }
        }
        InferOp::ExpertMatVec {
            layer,
            expert,
            slot,
        } => expert_tensor_id(quant_graph, layer, expert, slot)
            .map(|tensor| QuantGraphEntityRef::ExpertTensor {
                layer,
                expert,
                slot,
                tensor,
            })
            .unwrap_or(QuantGraphEntityRef::ExpertSection { layer, expert }),
        InferOp::FfnActivation { layer, expert } => {
            QuantGraphEntityRef::FfnActivationSite { layer, expert }
        }
        InferOp::CombineResidual { layer, site } => {
            QuantGraphEntityRef::ResidualSiteRef { layer, site }
        }
        InferOp::Classify => QuantGraphEntityRef::ClassifyHead,
        InferOp::DecodeToken { plan } => QuantGraphEntityRef::DecodePlan { plan },
    }
}

fn expert_tensor_id(
    quant_graph: &QuantGraph,
    layer: LayerId,
    expert: ExpertId,
    slot: ExpertWeightSlot,
) -> Option<TensorId> {
    quant_graph
        .tensors
        .iter()
        .find_map(|tensor| match tensor.role {
            QuantTensorRole::ExpertWeight {
                layer: tensor_layer,
                expert: tensor_expert,
                slot: tensor_slot,
            } if tensor_layer == layer && tensor_expert == expert && tensor_slot == slot => {
                Some(tensor.tensor_id)
            }
            _ => None,
        })
}

pub fn validate_infer_ir_self_consistency(
    ir: &GbInferIR,
    quant_graph: &QuantGraph,
) -> Vec<ValidationDiagnostic> {
    let mut diagnostics = Vec::new();
    check_unique_ids(ir, &mut diagnostics);
    check_value_inputs_and_cycles(ir, &mut diagnostics);
    check_effect_inputs(ir, &mut diagnostics);
    check_value_format_consistency(ir, quant_graph, &mut diagnostics);
    check_norm_format_chain(ir, quant_graph, &mut diagnostics);
    check_residual_boundaries(ir, quant_graph, &mut diagnostics);
    check_expert_section_roles(ir, quant_graph, &mut diagnostics);
    check_route_top1_semantics(ir, quant_graph, &mut diagnostics);
    check_topological_order(ir, quant_graph, &mut diagnostics);
    check_reachability(ir, &mut diagnostics);
    check_token_input_rules(ir, &mut diagnostics);
    check_router_consumers(ir, &mut diagnostics);
    check_decode_rng(ir, quant_graph, &mut diagnostics);
    check_sequence_slot_coverage(ir, quant_graph, &mut diagnostics);
    check_dense_routed_shape(ir, quant_graph, &mut diagnostics);
    check_fault_boundary_absent(ir, &mut diagnostics);
    check_op_signatures(ir, quant_graph, &mut diagnostics);
    check_op_histogram_total(ir, &mut diagnostics);
    tracing::info!(
        n_diagnostics = diagnostics.len() as u64,
        "stage3.binding.self_consistency"
    );
    diagnostics
}

fn check_unique_ids(ir: &GbInferIR, diagnostics: &mut Vec<ValidationDiagnostic>) {
    let ok = all_unique(ir.nodes.iter().map(|node| node.node_id))
        && all_unique(ir.values.iter().map(|value| value.value_id))
        && all_unique(ir.effects.iter().map(|effect| effect.effect_id));
    log_sc_rule("SC-1", ok, diagnostics.len());
    if !ok {
        diagnostics.push(infer_ir_diagnostic("InferIrIdNotUnique", "ids"));
    }
}

fn check_value_inputs_and_cycles(ir: &GbInferIR, diagnostics: &mut Vec<ValidationDiagnostic>) {
    let token_value = ir.token_inputs.first().map(|token| token.value_id);
    let mut produced_at = BTreeMap::new();
    for (index, node) in ir.nodes.iter().enumerate() {
        for output in &node.outputs {
            produced_at.insert(*output, index);
        }
    }
    let mut ok = true;
    for (index, node) in ir.nodes.iter().enumerate() {
        for input in &node.inputs {
            if Some(*input) == token_value {
                continue;
            }
            match produced_at.get(input).copied() {
                Some(producer) if producer < index => {}
                Some(_) => {
                    ok = false;
                    diagnostics.push(infer_ir_diagnostic("InferIrCycleDetected", "nodes"));
                }
                None => {
                    ok = false;
                    diagnostics.push(infer_ir_diagnostic(
                        "InferIrInputValueIdMissing",
                        "nodes.inputs",
                    ));
                }
            }
        }
    }
    log_sc_rule("SC-2/4", ok, diagnostics.len());
}

fn check_effect_inputs(ir: &GbInferIR, diagnostics: &mut Vec<ValidationDiagnostic>) {
    let provenance = &ir.provenance.effects;
    let node_index = ir
        .nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (node.node_id, index))
        .collect::<BTreeMap<_, _>>();
    let mut produced_at = BTreeMap::new();
    for (index, node) in ir.nodes.iter().enumerate() {
        for output in &node.effects_out {
            produced_at.insert(*output, index);
        }
    }
    let mut ok = true;
    for (index, node) in ir.nodes.iter().enumerate() {
        for effect_in in &node.effects_in {
            match provenance.get(effect_in) {
                Some(EffectProvenance::ExternalRoot { .. }) => {}
                Some(EffectProvenance::NodeOutput { node, .. }) => {
                    if node_index
                        .get(node)
                        .copied()
                        .is_none_or(|producer| producer >= index)
                    {
                        ok = false;
                    }
                }
                None => ok = false,
            }
            if let Some(producer) = produced_at.get(effect_in).copied() {
                ok &= producer < index;
            }
        }
    }
    log_sc_rule("SC-3", ok, diagnostics.len());
    if !ok {
        diagnostics.push(infer_ir_diagnostic("InferIrEffectChainMismatch", "effects"));
    }
}

fn check_value_format_consistency(
    ir: &GbInferIR,
    quant_graph: &QuantGraph,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    let values_by_id = values_by_id(&ir.values);
    let mut ok = true;
    for node in &ir.nodes {
        let Some(expected) = expected_output_specs(node.op, quant_graph) else {
            ok = false;
            diagnostics.push(infer_ir_value_format_mismatch_diagnostic("nodes.outputs"));
            continue;
        };
        if node.outputs.len() != expected.len() {
            ok = false;
            diagnostics.push(infer_ir_value_format_mismatch_for_op(node.op));
            continue;
        }
        for (value_id, expected) in node.outputs.iter().zip(expected.iter()) {
            match values_by_id.get(value_id) {
                Some(value) if value.kind == expected.kind && value.format == expected.format => {}
                _ => {
                    ok = false;
                    diagnostics.push(infer_ir_value_format_mismatch_for_op(node.op));
                }
            }
        }
    }
    log_sc_rule("SC-5", ok, diagnostics.len());
}

fn check_norm_format_chain(
    ir: &GbInferIR,
    quant_graph: &QuantGraph,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    let values_by_id = values_by_id(&ir.values);
    let mut ok = true;
    for node in &ir.nodes {
        let InferOp::Norm { plan } = node.op else {
            continue;
        };
        let records = quant_graph
            .norm_plans
            .iter()
            .filter(|record| record.norm_plan_id == plan)
            .collect::<Vec<_>>();
        if records.len() != 1 || node.inputs.len() != 1 || node.outputs.len() != 1 {
            ok = false;
            diagnostics.push(infer_ir_norm_format_mismatch_diagnostic());
            continue;
        }
        let record = records[0];
        let input_format = values_by_id.get(&node.inputs[0]).map(|value| &value.format);
        let output_format = values_by_id
            .get(&node.outputs[0])
            .map(|value| &value.format);
        if input_format
            != Some(&ValueFormat::Quant {
                format: record.input_format.clone(),
            })
            || output_format
                != Some(&ValueFormat::Quant {
                    format: record.output_format.clone(),
                })
        {
            ok = false;
            diagnostics.push(infer_ir_norm_format_mismatch_diagnostic());
        }
    }
    log_sc_rule("SC-6", ok, diagnostics.len());
}

fn check_residual_boundaries(
    ir: &GbInferIR,
    quant_graph: &QuantGraph,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    let values_by_id = values_by_id(&ir.values);
    let expected = ValueFormat::Quant {
        format: quant_graph.residual_plan.activation_format.clone(),
    };
    let mut ok = true;
    for node in &ir.nodes {
        if !matches!(node.op, InferOp::CombineResidual { .. }) {
            continue;
        }
        if node.outputs.len() != 1
            || values_by_id
                .get(&node.outputs[0])
                .map(|value| &value.format)
                != Some(&expected)
        {
            ok = false;
            diagnostics.push(infer_ir_residual_boundary_mismatch_diagnostic());
        }
    }
    log_sc_rule("SC-residual-boundary", ok, diagnostics.len());
}

fn check_expert_section_roles(
    ir: &GbInferIR,
    quant_graph: &QuantGraph,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    let tensor_roles = quant_graph
        .tensors
        .iter()
        .map(|tensor| (tensor.tensor_id, &tensor.role))
        .collect::<BTreeMap<_, _>>();
    let mut ok = true;
    for node in &ir.nodes {
        let InferOp::ExpertMatVec {
            layer,
            expert,
            slot,
        } = node.op
        else {
            continue;
        };
        let sections = quant_graph
            .expert_sections
            .iter()
            .filter(|section| section.layer == layer && section.expert == expert)
            .collect::<Vec<_>>();
        let role_matches = sections.len() == 1
            && sections[0].tensor_refs.iter().any(|tensor_id| {
                tensor_roles.get(tensor_id).is_some_and(|role| {
                    matches!(
                        **role,
                        QuantTensorRole::ExpertWeight {
                            layer: tensor_layer,
                            expert: tensor_expert,
                            slot: tensor_slot,
                        } if tensor_layer == layer && tensor_expert == expert && tensor_slot == slot
                    )
                })
            });
        if !role_matches {
            ok = false;
            diagnostics.push(infer_ir_expert_section_role_mismatch_diagnostic(
                layer, expert,
            ));
        }
    }
    log_sc_rule("SC-7", ok, diagnostics.len());
}

fn check_route_top1_semantics(
    ir: &GbInferIR,
    quant_graph: &QuantGraph,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    let mut ok = true;
    for node in &ir.nodes {
        let InferOp::RouteTop1 { layer } = node.op else {
            continue;
        };
        let layers = quant_graph
            .routing_table
            .as_ref()
            .map(|routing| {
                routing
                    .layers
                    .iter()
                    .filter(|entry| entry.layer == layer)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let semantics_ok = layers.len() == 1
            && matches!(
                layers[0].semantics,
                crate::s1::quant_graph::RouterSemantics::Top1Hard { .. }
            );
        if !semantics_ok {
            ok = false;
            diagnostics.push(infer_ir_non_v1_router_semantics_diagnostic(layer));
        }
    }
    log_sc_rule("SC-8", ok, diagnostics.len());
}

fn check_topological_order(
    ir: &GbInferIR,
    quant_graph: &QuantGraph,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    let sorted = bind_canonical_sort(ir.identity, ir.nodes.clone(), quant_graph)
        .map(|(_, nodes)| nodes)
        .unwrap_or_default();
    let observed = ir
        .nodes
        .iter()
        .map(|node| (node.node_id, node.op))
        .collect::<Vec<_>>();
    let expected = sorted
        .iter()
        .map(|node| (node.node_id, node.op))
        .collect::<Vec<_>>();
    let hash_ok = compute_topological_order_hash(&ir.nodes, quant_graph)
        .is_ok_and(|hash| hash == ir.identity.topological_order_hash);
    let ok = observed == expected && hash_ok;
    log_sc_rule("SC-12", ok, diagnostics.len());
    if !ok {
        diagnostics.push(infer_ir_diagnostic(
            "InferIrTopologicalOrderMismatch",
            "nodes",
        ));
    }
}

fn check_reachability(ir: &GbInferIR, diagnostics: &mut Vec<ValidationDiagnostic>) {
    let consumed = ir
        .nodes
        .iter()
        .flat_map(|node| node.inputs.iter().copied())
        .fold(BTreeMap::<ValueId, usize>::new(), |mut counts, value| {
            *counts.entry(value).or_default() += 1;
            counts
        });
    let mut ok = true;
    for value in &ir.values {
        if value.kind != ValueKind::DecodedToken
            && consumed.get(&value.value_id).copied().unwrap_or(0) == 0
        {
            ok = false;
            let diagnostic = match value.kind {
                ValueKind::RouterScore => "InferIrRouterScoreOrphaned",
                ValueKind::SequenceStateNext => "InferIrSequenceStateNextOrphaned",
                _ => "InferIrUnreachableNode",
            };
            diagnostics.push(infer_ir_diagnostic(diagnostic, "values"));
        }
    }
    log_sc_rule("SC-13", ok, diagnostics.len());
}

fn check_token_input_rules(ir: &GbInferIR, diagnostics: &mut Vec<ValidationDiagnostic>) {
    let Some(token_input) = ir.token_inputs.first() else {
        diagnostics.push(infer_ir_token_ingress_ambiguous_diagnostic());
        log_sc_rule("SC-14", false, diagnostics.len());
        return;
    };
    let embeddings = ir
        .nodes
        .iter()
        .filter_map(|node| match node.op {
            InferOp::Embedding { token_input } => Some((token_input, node)),
            _ => None,
        })
        .collect::<Vec<_>>();
    let ok = ir.token_inputs.len() == 1
        && embeddings.len() == 1
        && embeddings[0].0 == token_input.token_input_id
        && embeddings[0].1.inputs.len() == 1
        && embeddings[0].1.inputs.as_slice() == [token_input.value_id].as_slice();
    log_sc_rule("SC-14/14a", ok, diagnostics.len());
    if !ok {
        diagnostics.push(infer_ir_token_ingress_ambiguous_diagnostic());
    }
}

fn check_router_consumers(ir: &GbInferIR, diagnostics: &mut Vec<ValidationDiagnostic>) {
    let select_inputs = ir
        .nodes
        .iter()
        .filter(|node| matches!(node.op, InferOp::SelectExpertTop1 { .. }))
        .flat_map(|node| node.inputs.iter().copied())
        .fold(BTreeMap::<ValueId, usize>::new(), |mut counts, value| {
            *counts.entry(value).or_default() += 1;
            counts
        });
    let mut ok = true;
    for value in &ir.values {
        if matches!(
            value.kind,
            ValueKind::GateWeight | ValueKind::RouterDecision
        ) && select_inputs.get(&value.value_id).copied().unwrap_or(0) != 1
        {
            ok = false;
            diagnostics.push(infer_ir_diagnostic(
                if value.kind == ValueKind::GateWeight {
                    "InferIrGateWeightNotConsumed"
                } else {
                    "InferIrRouterDecisionNotConsumed"
                },
                "values",
            ));
        }
    }
    log_sc_rule("SC-15/16", ok, diagnostics.len());
}

fn check_decode_rng(
    ir: &GbInferIR,
    quant_graph: &QuantGraph,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    let decode_nodes = ir
        .nodes
        .iter()
        .filter(|node| matches!(node.op, InferOp::DecodeToken { .. }))
        .collect::<Vec<_>>();
    let effects_by_id = ir
        .effects
        .iter()
        .map(|effect| (effect.effect_id, effect.class))
        .collect::<BTreeMap<_, _>>();
    let mut plan_ok = true;
    let mut rng_ok = decode_nodes.len() == 1;
    let mut pure_rng_ok = true;

    for node in &decode_nodes {
        if !matches!(
            node.op,
            InferOp::DecodeToken { plan } if plan == quant_graph.decode_spec.decode_plan_id
        ) {
            plan_ok = false;
        }
        let rng_in = node
            .effects_in
            .iter()
            .filter(|effect_id| is_rng_decode_effect(**effect_id, &effects_by_id))
            .copied()
            .collect::<Vec<_>>();
        let rng_out = node
            .effects_out
            .iter()
            .filter(|effect_id| is_rng_decode_effect(**effect_id, &effects_by_id))
            .copied()
            .collect::<Vec<_>>();
        if quant_graph.decode_spec.requires_rng {
            rng_ok &= rng_in.len() == 1 && rng_out.len() == 1 && rng_in[0] != rng_out[0];
        } else {
            rng_ok &= rng_in.is_empty() && rng_out.is_empty();
        }
    }

    for node in ir
        .nodes
        .iter()
        .filter(|node| !matches!(node.op, InferOp::DecodeToken { .. }))
    {
        let touches_rng = node
            .effects_in
            .iter()
            .chain(node.effects_out.iter())
            .any(|effect_id| is_rng_decode_effect(*effect_id, &effects_by_id));
        if touches_rng {
            pure_rng_ok = false;
            diagnostics.push(infer_ir_unexpected_rng_effect_on_pure_op_diagnostic(
                node.op.tag(),
            ));
        }
    }

    log_sc_rule("SC-10", plan_ok && rng_ok && pure_rng_ok, diagnostics.len());
    if !plan_ok {
        diagnostics.push(infer_ir_decode_plan_mismatch_diagnostic());
    }
    if !rng_ok {
        diagnostics.push(infer_ir_decode_rng_binding_mismatch_diagnostic());
    }
}

fn check_sequence_slot_coverage(
    ir: &GbInferIR,
    quant_graph: &QuantGraph,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    let expected_by_layer = (0..quant_graph.identity.model_spec_summary.n_layers)
        .map(LayerId::new)
        .map(|layer| {
            (
                layer,
                state_slots_for_layer(quant_graph, layer)
                    .into_iter()
                    .collect::<BTreeSet<_>>(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let observed_reads = ir
        .nodes
        .iter()
        .filter_map(|node| match node.op {
            InferOp::SequenceRead { slot } => Some(slot),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    let observed_writes = ir
        .nodes
        .iter()
        .filter_map(|node| match node.op {
            InferOp::SequenceWrite { slot } => Some(slot),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    let step_layers = ir
        .nodes
        .iter()
        .filter_map(|node| match node.op {
            InferOp::SequenceStep { layer } => Some(layer),
            _ => None,
        })
        .collect::<BTreeSet<_>>();

    let mut ok = true;
    for (layer, expected) in expected_by_layer {
        let layer_ok = if expected.is_empty() {
            !step_layers.contains(&layer) && observed_reads.is_empty() && observed_writes.is_empty()
        } else {
            step_layers.contains(&layer)
                && observed_reads == expected
                && observed_writes == expected
        };
        if !layer_ok {
            ok = false;
            diagnostics.push(infer_ir_sequence_slot_coverage_mismatch_diagnostic(layer));
        }
    }
    log_sc_rule("SC-10a", ok, diagnostics.len());
}

fn check_dense_routed_shape(
    ir: &GbInferIR,
    quant_graph: &QuantGraph,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    let mut ok = true;
    for layer_index in 0..quant_graph.identity.model_spec_summary.n_layers {
        let layer = LayerId::new(layer_index);
        let router_count = count_nodes(
            ir,
            |op| matches!(op, InferOp::RouterMatVec { layer: node_layer } if node_layer == layer),
        );
        let route_count = count_nodes(
            ir,
            |op| matches!(op, InferOp::RouteTop1 { layer: node_layer } if node_layer == layer),
        );
        let select_count = count_nodes(
            ir,
            |op| matches!(op, InferOp::SelectExpertTop1 { layer: node_layer } if node_layer == layer),
        );
        let expected_gate =
            ffn_activation_kind(quant_graph, layer) == Some(FfnActivationKind::SwiGLU);
        let expected_experts = expected_experts_for_layer(quant_graph, layer);
        let mut layer_ok = true;

        match layer_kind(quant_graph, layer) {
            Some(FfnKindTag::Dense) => {
                if router_count != 0 || route_count != 0 || select_count != 0 {
                    layer_ok = false;
                    diagnostics.push(infer_ir_router_present_for_dense_diagnostic(layer));
                }
                layer_ok &= expected_experts == [ExpertId::new(0)];
            }
            Some(FfnKindTag::Routed)
                if router_count != 1 || route_count != 1 || select_count != 1 =>
            {
                layer_ok = false;
                diagnostics.push(infer_ir_routed_missing_router_diagnostic(layer));
            }
            Some(FfnKindTag::Routed) => {}
            _ => {}
        }

        for expert in expected_experts.iter().copied() {
            let up_count = count_nodes(ir, |op| {
                matches!(
                    op,
                    InferOp::ExpertMatVec {
                        layer: node_layer,
                        expert: node_expert,
                        slot: ExpertWeightSlot::FfnUp,
                    } if node_layer == layer && node_expert == expert
                )
            });
            let down_count = count_nodes(ir, |op| {
                matches!(
                    op,
                    InferOp::ExpertMatVec {
                        layer: node_layer,
                        expert: node_expert,
                        slot: ExpertWeightSlot::FfnDown,
                    } if node_layer == layer && node_expert == expert
                )
            });
            let gate_count = count_nodes(ir, |op| {
                matches!(
                    op,
                    InferOp::ExpertMatVec {
                        layer: node_layer,
                        expert: node_expert,
                        slot: ExpertWeightSlot::FfnGate,
                    } if node_layer == layer && node_expert == expert
                )
            });
            let activation_count = count_nodes(ir, |op| {
                matches!(
                    op,
                    InferOp::FfnActivation {
                        layer: node_layer,
                        expert: node_expert,
                    } if node_layer == layer && node_expert == expert
                )
            });
            layer_ok &= up_count == 1
                && down_count == 1
                && activation_count == 1
                && gate_count == if expected_gate { 1 } else { 0 };
        }

        let actual_experts = ir
            .nodes
            .iter()
            .filter_map(|node| match node.op {
                InferOp::ExpertMatVec {
                    layer: node_layer,
                    expert,
                    ..
                }
                | InferOp::FfnActivation {
                    layer: node_layer,
                    expert,
                } if node_layer == layer => Some(expert),
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        layer_ok &= actual_experts == expected_experts.iter().copied().collect::<BTreeSet<_>>();

        if !layer_ok {
            ok = false;
            diagnostics.push(infer_ir_dense_routed_shape_mismatch_diagnostic(layer));
        }
    }
    log_sc_rule("SC-9", ok, diagnostics.len());
}

fn check_fault_boundary_absent(ir: &GbInferIR, diagnostics: &mut Vec<ValidationDiagnostic>) {
    let ok = ir
        .effects
        .iter()
        .all(|effect| !matches!(effect.class, EffectClass::FaultBoundary));
    log_sc_rule("SC-11", ok, diagnostics.len());
    if !ok {
        diagnostics.push(infer_ir_fault_boundary_emitted_diagnostic());
    }
}

fn check_op_signatures(
    ir: &GbInferIR,
    quant_graph: &QuantGraph,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    let mut ok = true;
    for node in &ir.nodes {
        if check_op_signature(node, &ir.values, &ir.effects, quant_graph).is_err() {
            ok = false;
            diagnostics.push(infer_ir_diagnostic("InferIrOpSignatureMismatch", "nodes"));
        }
    }
    log_sc_rule("SC-18", ok, diagnostics.len());
}

fn check_op_histogram_total(ir: &GbInferIR, diagnostics: &mut Vec<ValidationDiagnostic>) {
    let histogram = infer_op_histogram(&ir.nodes);
    let total = histogram.values().copied().sum::<u32>();
    let ok = total == ir.nodes.len() as u32
        && histogram.len() == INFER_OP_TAG_CANONICAL_ORDER.len()
        && INFER_OP_TAG_CANONICAL_ORDER
            .iter()
            .all(|tag| histogram.contains_key(tag));
    log_sc_rule("SC-17", ok, diagnostics.len());
    if !ok {
        diagnostics.push(infer_ir_op_histogram_total_mismatch_diagnostic());
    }
}

#[must_use]
pub fn infer_op_histogram(nodes: &[GbNode]) -> BTreeMap<InferOpTag, u32> {
    let mut histogram = INFER_OP_TAG_CANONICAL_ORDER
        .into_iter()
        .map(|tag| (tag, 0_u32))
        .collect::<BTreeMap<_, _>>();
    for node in nodes {
        *histogram.entry(node.op.tag()).or_default() += 1;
    }
    histogram
}

#[must_use]
pub fn infer_op_histogram_total_matches_node_count(
    histogram: &BTreeMap<InferOpTag, u32>,
    node_count: u32,
) -> bool {
    histogram.len() == INFER_OP_TAG_CANONICAL_ORDER.len()
        && INFER_OP_TAG_CANONICAL_ORDER
            .iter()
            .all(|tag| histogram.contains_key(tag))
        && histogram.values().copied().sum::<u32>() == node_count
}

pub fn infer_ir_self_hash(infer_ir: &GbInferIR) -> Result<Hash256, GbInferIrProductError> {
    let value = serde_json::to_value(infer_ir)
        .map_err(|err| GbInferIrProductError::CanonicalJson(err.to_string()))?;
    let canonical = canonicalize_value(&value)
        .map_err(|err| GbInferIrProductError::CanonicalJson(err.to_string()))?;
    Ok(domain_hash("GbInferIR", INFER_IR_SCHEMA_ID, &canonical))
}

pub fn infer_ir_canonical_bytes_hash(
    infer_ir: &GbInferIR,
) -> Result<Hash256, GbInferIrProductError> {
    let value = serde_json::to_value(infer_ir)
        .map_err(|err| GbInferIrProductError::CanonicalJson(err.to_string()))?;
    let canonical = canonicalize_value(&value)
        .map_err(|err| GbInferIrProductError::CanonicalJson(err.to_string()))?;
    Ok(Hash256::from_bytes(Sha256::digest(&canonical).into()))
}

#[must_use]
pub fn infer_ir_input_identity(
    infer_ir: &GbInferIR,
    audit_parents: InferIrAuditParents,
    requested_runtime_modes: BTreeSet<RuntimeMode>,
) -> report_schema::InferIrInputIdentity {
    report_schema::InferIrInputIdentity {
        quant_graph_self_hash: infer_ir.identity.quant_graph_self_hash,
        policy_resolution_self_hash: audit_parents.policy_resolution_self_hash,
        compile_request_hash: audit_parents.compile_request_hash,
        static_budget_self_hash: infer_ir.identity.static_budget_self_hash,
        requested_runtime_modes_hash: infer_ir.identity.requested_runtime_modes_hash,
        determinism: report_determinism_class(infer_ir.identity.determinism),
        requested_runtime_modes,
    }
}

pub fn infer_ir_report_result(
    infer_ir: GbInferIR,
    fixture_equivalence: report_schema::FixtureEquivalenceTag,
    infer_ir_self_hash: Hash256,
    infer_ir_canonical_bytes_hash: Hash256,
) -> Result<report_schema::InferIrResult<GbInferIR>, GbInferIrProductError> {
    Ok(report_schema::InferIrResult {
        node_count: usize_to_u32(infer_ir.nodes.len(), "result.node_count")?,
        value_count: usize_to_u32(infer_ir.values.len(), "result.value_count")?,
        effect_count: usize_to_u16(infer_ir.effects.len(), "result.effect_count")?,
        token_input_count: usize_to_u8(infer_ir.token_inputs.len(), "result.token_input_count")?,
        topological_order_hash: infer_ir.identity.topological_order_hash,
        op_histogram: report_infer_op_histogram(&infer_ir.nodes),
        effect_class_histogram: report_effect_class_histogram(&infer_ir.effects),
        value_kind_histogram: report_value_kind_histogram(&infer_ir.values),
        anchor_count: usize_to_u32(infer_ir.anchors.len(), "result.anchor_count")?,
        fixture_equivalence,
        product: infer_ir,
        infer_ir_self_hash,
        infer_ir_canonical_bytes_hash,
    })
}

#[must_use]
pub fn report_infer_op_histogram(nodes: &[GbNode]) -> BTreeMap<report_schema::InferOpTag, u32> {
    let mut histogram = report_schema::INFER_OP_TAG_CANONICAL_ORDER
        .into_iter()
        .map(|tag| (tag, 0_u32))
        .collect::<BTreeMap<_, _>>();
    for node in nodes {
        *histogram
            .entry(report_infer_op_tag(node.op.tag()))
            .or_default() += 1;
    }
    histogram
}

#[must_use]
pub fn report_effect_class_histogram(
    effects: &[EffectDecl],
) -> BTreeMap<report_schema::EffectClassTag, u16> {
    let mut histogram = report_schema::EFFECT_CLASS_TAG_CANONICAL_ORDER
        .into_iter()
        .map(|tag| (tag, 0_u16))
        .collect::<BTreeMap<_, _>>();
    for effect in effects {
        *histogram
            .entry(report_effect_class_tag(effect.class.tag()))
            .or_default() += 1;
    }
    histogram
}

#[must_use]
pub fn report_value_kind_histogram(
    values: &[ValueDecl],
) -> BTreeMap<report_schema::ValueKindTag, u32> {
    let mut histogram = report_schema::VALUE_KIND_TAG_CANONICAL_ORDER
        .into_iter()
        .map(|tag| (tag, 0_u32))
        .collect::<BTreeMap<_, _>>();
    for value in values {
        *histogram
            .entry(report_value_kind_tag(value.kind))
            .or_default() += 1;
    }
    histogram
}

fn count_nodes(ir: &GbInferIR, predicate: impl Fn(InferOp) -> bool) -> usize {
    ir.nodes.iter().filter(|node| predicate(node.op)).count()
}

fn is_rng_decode_effect(
    effect_id: EffectId,
    effects_by_id: &BTreeMap<EffectId, EffectClass>,
) -> bool {
    matches!(
        effects_by_id.get(&effect_id),
        Some(EffectClass::Rng {
            slot: RngSlot::Decode
        })
    )
}

fn all_unique<T: Ord>(values: impl IntoIterator<Item = T>) -> bool {
    let mut seen = BTreeSet::new();
    values.into_iter().all(|value| seen.insert(value))
}

fn infer_ir_diagnostic(code: &'static str, field: &'static str) -> ValidationDiagnostic {
    hard_semantic_diagnostic(
        field,
        ValidationDetail::Field {
            field: FieldPath::from(code),
        },
    )
}

fn infer_ir_router_present_for_dense_diagnostic(layer: LayerId) -> ValidationDiagnostic {
    ValidationDiagnostic::hard(
        ValidationOrigin::SemanticCore,
        ValidationCode::InferIrRouterPresentForDenseLayer { layer },
        ValidationDetail::Field {
            field: FieldPath::from("routing_table.layers"),
        },
        Vec::new(),
    )
}

fn infer_ir_routed_missing_router_diagnostic(layer: LayerId) -> ValidationDiagnostic {
    ValidationDiagnostic::hard(
        ValidationOrigin::SemanticCore,
        ValidationCode::InferIrRouterMatVecMissingForRoutedLayer { layer },
        ValidationDetail::Field {
            field: FieldPath::from("routing_table.layers"),
        },
        Vec::new(),
    )
}

fn infer_ir_sequence_semantics_unsupported_diagnostic() -> ValidationDiagnostic {
    let field = FieldPath::from("sequence_semantics.state_slots");
    ValidationDiagnostic::hard(
        ValidationOrigin::SemanticCore,
        ValidationCode::InferIrSequenceSemanticsUnsupportedV1 {
            field: field.clone(),
        },
        ValidationDetail::Field { field },
        Vec::new(),
    )
}

fn infer_ir_value_producer_missing_diagnostic(value_id: ValueId) -> ValidationDiagnostic {
    ValidationDiagnostic::hard(
        ValidationOrigin::SemanticCore,
        ValidationCode::InferIrValueProducerMissing {
            value_id: value_id.get(),
        },
        ValidationDetail::Field {
            field: FieldPath::from("provenance.values"),
        },
        Vec::new(),
    )
}

fn infer_ir_value_format_mismatch_for_op(op: InferOp) -> ValidationDiagnostic {
    infer_ir_value_format_mismatch_diagnostic(format!("nodes.{:?}.outputs", op.tag()))
}

fn infer_ir_value_format_mismatch_diagnostic(field: impl Into<FieldPath>) -> ValidationDiagnostic {
    let field = field.into();
    ValidationDiagnostic::hard(
        ValidationOrigin::SemanticCore,
        ValidationCode::InferIrValueFormatMismatch {
            field: field.clone(),
        },
        ValidationDetail::Field { field },
        Vec::new(),
    )
}

fn infer_ir_norm_format_mismatch_diagnostic() -> ValidationDiagnostic {
    let field = FieldPath::from("nodes.norm.format");
    ValidationDiagnostic::hard(
        ValidationOrigin::SemanticCore,
        ValidationCode::InferIrNormFormatMismatch {
            field: field.clone(),
        },
        ValidationDetail::Field { field },
        Vec::new(),
    )
}

fn infer_ir_residual_boundary_mismatch_diagnostic() -> ValidationDiagnostic {
    let field = FieldPath::from("nodes.combine_residual.output_format");
    ValidationDiagnostic::hard(
        ValidationOrigin::SemanticCore,
        ValidationCode::InferIrResidualBoundaryMismatch {
            field: field.clone(),
        },
        ValidationDetail::Field { field },
        Vec::new(),
    )
}

fn infer_ir_expert_section_role_mismatch_diagnostic(
    layer: LayerId,
    expert: ExpertId,
) -> ValidationDiagnostic {
    ValidationDiagnostic::hard(
        ValidationOrigin::SemanticCore,
        ValidationCode::InferIrExpertSectionRoleMismatch { layer, expert },
        ValidationDetail::Field {
            field: FieldPath::from("expert_sections.tensor_refs"),
        },
        Vec::new(),
    )
}

fn infer_ir_non_v1_router_semantics_diagnostic(layer: LayerId) -> ValidationDiagnostic {
    ValidationDiagnostic::hard(
        ValidationOrigin::SemanticCore,
        ValidationCode::InferIrNonV1RouterSemantics { layer },
        ValidationDetail::Field {
            field: FieldPath::from("routing_table.layers.semantics"),
        },
        Vec::new(),
    )
}

fn infer_ir_token_ingress_ambiguous_diagnostic() -> ValidationDiagnostic {
    let field = FieldPath::from("token_inputs");
    ValidationDiagnostic::hard(
        ValidationOrigin::SemanticCore,
        ValidationCode::InferIrTokenIngressAmbiguous {
            field: field.clone(),
        },
        ValidationDetail::Field { field },
        Vec::new(),
    )
}

fn infer_ir_decode_plan_mismatch_diagnostic() -> ValidationDiagnostic {
    let field = FieldPath::from("nodes.decode.plan");
    ValidationDiagnostic::hard(
        ValidationOrigin::SemanticCore,
        ValidationCode::InferIrDecodePlanMismatch {
            field: field.clone(),
        },
        ValidationDetail::Field { field },
        Vec::new(),
    )
}

fn infer_ir_decode_rng_binding_mismatch_diagnostic() -> ValidationDiagnostic {
    let field = FieldPath::from("nodes.decode.effects");
    ValidationDiagnostic::hard(
        ValidationOrigin::SemanticCore,
        ValidationCode::InferIrDecodeRngBindingMismatch {
            field: field.clone(),
        },
        ValidationDetail::Field { field },
        Vec::new(),
    )
}

fn infer_ir_unexpected_rng_effect_on_pure_op_diagnostic(
    op_tag: InferOpTag,
) -> ValidationDiagnostic {
    let field = FieldPath::from(format!("nodes.{op_tag:?}.effects"));
    ValidationDiagnostic::hard(
        ValidationOrigin::SemanticCore,
        ValidationCode::InferIrUnexpectedRngEffectOnPureOp {
            field: field.clone(),
        },
        ValidationDetail::Field { field },
        Vec::new(),
    )
}

fn infer_ir_sequence_slot_coverage_mismatch_diagnostic(layer: LayerId) -> ValidationDiagnostic {
    ValidationDiagnostic::hard(
        ValidationOrigin::SemanticCore,
        ValidationCode::InferIrSequenceSlotCoverageMismatch { layer },
        ValidationDetail::Field {
            field: FieldPath::from("sequence_semantics.state_slots"),
        },
        Vec::new(),
    )
}

fn infer_ir_dense_routed_shape_mismatch_diagnostic(layer: LayerId) -> ValidationDiagnostic {
    ValidationDiagnostic::hard(
        ValidationOrigin::SemanticCore,
        ValidationCode::InferIrDenseRoutedShapeMismatch { layer },
        ValidationDetail::Field {
            field: FieldPath::from("nodes.ffn_shape"),
        },
        Vec::new(),
    )
}

fn infer_ir_fault_boundary_emitted_diagnostic() -> ValidationDiagnostic {
    let field = FieldPath::from("effects");
    ValidationDiagnostic::hard(
        ValidationOrigin::SemanticCore,
        ValidationCode::InferIrFaultBoundaryEmittedV1Forbidden {
            field: field.clone(),
        },
        ValidationDetail::Field { field },
        Vec::new(),
    )
}

fn infer_ir_op_histogram_total_mismatch_diagnostic() -> ValidationDiagnostic {
    let field = FieldPath::from("result.op_histogram");
    ValidationDiagnostic::hard(
        ValidationOrigin::SemanticCore,
        ValidationCode::InferIrOpHistogramTotalMismatch {
            field: field.clone(),
        },
        ValidationDetail::Field { field },
        Vec::new(),
    )
}

#[cfg(any(feature = "semantic_equivalence_check", test))]
fn infer_ir_semantic_equivalence_failed_diagnostic(sample_index: usize) -> ValidationDiagnostic {
    let field = FieldPath::from(format!("semantic_equivalence.fixture[{sample_index}]"));
    ValidationDiagnostic::hard(
        ValidationOrigin::SemanticCore,
        ValidationCode::InferIrSemanticEquivalenceFailed {
            field: field.clone(),
        },
        ValidationDetail::Field { field },
        Vec::new(),
    )
}

fn log_sc_rule(rule: &'static str, outcome: bool, n_diagnostics: usize) {
    tracing::debug!(
        rule,
        outcome,
        n_diagnostics = n_diagnostics as u64,
        "stage3.self_consistency.rule"
    );
}

struct EffectClassSummary {
    n_effect_classes: usize,
    has_rng_chain: bool,
    has_state_chains: bool,
}

impl EffectClassSummary {
    fn new(effects: &[EffectDecl]) -> Result<Self, GbInferIrTypeError> {
        let mut classes = BTreeSet::new();
        for effect in effects {
            if matches!(effect.class, EffectClass::FaultBoundary) {
                return Err(GbInferIrTypeError::FaultBoundaryReserved {
                    effect_id: Some(effect.effect_id),
                });
            }
            classes.insert(effect.class);
        }

        Ok(Self {
            n_effect_classes: classes.len(),
            has_rng_chain: classes
                .iter()
                .any(|class| matches!(class, EffectClass::Rng { .. })),
            has_state_chains: classes
                .iter()
                .any(|class| matches!(class, EffectClass::SequenceState { .. })),
        })
    }
}

fn reject_fault_boundary_provenance(
    provenance: &InferIrProvenance,
) -> Result<(), GbInferIrTypeError> {
    for effect in provenance.effects.values() {
        let class = match effect {
            EffectProvenance::ExternalRoot { class }
            | EffectProvenance::NodeOutput { class, .. } => *class,
        };
        if matches!(class, EffectClass::FaultBoundary) {
            return Err(GbInferIrTypeError::FaultBoundaryReserved { effect_id: None });
        }
    }
    Ok(())
}

fn reject_fault_boundary_node_edges(
    nodes: &[GbNode],
    effects: &[EffectDecl],
) -> Result<(), GbInferIrTypeError> {
    let effect_classes = effects
        .iter()
        .map(|effect| (effect.effect_id, effect.class))
        .collect::<BTreeMap<_, _>>();

    for node in nodes {
        for effect_id in node.effects_in.iter().chain(node.effects_out.iter()) {
            if matches!(
                effect_classes.get(effect_id),
                Some(EffectClass::FaultBoundary)
            ) {
                return Err(GbInferIrTypeError::FaultBoundaryReserved {
                    effect_id: Some(*effect_id),
                });
            }
        }
    }

    Ok(())
}

fn validate_anchor_totality(
    anchors: &NodeAnchorMap,
    nodes: &[GbNode],
) -> Result<(), GbInferIrTypeError> {
    for node in nodes {
        if !anchors.contains_key(&node.node_id) {
            return Err(GbInferIrTypeError::SemanticAnchorMissing {
                node_id: node.node_id,
            });
        }
    }
    Ok(())
}

fn domain_hash(type_name: &str, schema_version: &str, canonical: &[u8]) -> Hash256 {
    domain_hash_in_namespace("gbf-codegen", type_name, schema_version, canonical)
}

fn domain_hash_in_namespace(
    namespace: &str,
    type_name: &str,
    schema_version: &str,
    canonical: &[u8],
) -> Hash256 {
    let mut hasher = Sha256::new();
    hasher.update(format!("gbf:{namespace}:{type_name}:{schema_version}\0"));
    hasher.update(canonical);
    Hash256::from_bytes(hasher.finalize().into())
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "semantic_equivalence_check")]
    use std::fmt;
    #[cfg(feature = "semantic_equivalence_check")]
    use std::fmt::Write as _;
    use std::fs;
    use std::path::{Path, PathBuf};
    #[cfg(feature = "semantic_equivalence_check")]
    use std::sync::{Arc, Mutex};

    use gbf_artifact::norm_plan::{
        AffineClipLutPlan, NormAffineParams, NormClipBounds, NormLutSpec, NormTileRmsSpec,
        TileRmsThenAffineClipPlan,
    };
    use gbf_artifact::tensor::{CanonicalTensorLayout, CanonicalTensorShape, TensorElementType};
    use gbf_foundation::{BlobCodec, BlobRef};
    use gbf_policy::{DiagnosticSeverity, EvidenceRef, RuntimeMode};
    use gbf_report::{
        ReportBody, ReportEnvelope, ReportOutcome, canonicalize as canonicalize_report,
        round_trip_self_hash,
    };
    use gbf_store::blob::BlobStore;
    use gbf_store::stage_cache::StageCache;
    use gbf_workload::{GoldenVectorRef, WorkloadManifestRef};
    use serde_json::Value;
    #[cfg(feature = "semantic_equivalence_check")]
    use tracing_subscriber::filter::LevelFilter;
    #[cfg(feature = "semantic_equivalence_check")]
    use tracing_subscriber::prelude::*;

    use crate::s1::quant_graph::{
        ClassifyHead, ClassifyHeadKind, DecodeSpec, DecodeSpecRecord, FfnPlan, ModelSpecSummary,
        QuantTensorRef, ResidualCombinePolicy, ResidualPlan, ResolvedBlobRef,
        RouterGateWeightSemantics, RouterLayer, RouterSemantics, RouterTieBreak, RoutingTable,
        SequenceSemanticsKind, SequenceSemanticsSpec, SequenceStateSlot,
    };
    use crate::validate::{
        ArtifactResolveError, ArtifactResolver, ResolvedBlob, ResolvedEvidence,
        ResolvedGoldenVector, ResolvedSidecar, ResolvedWorkload, SidecarRef,
    };

    use super::*;

    #[test]
    fn gb_infer_ir_struct_top_level_shape_closed() {
        let value = serde_json::to_value(gb_infer_ir_fixture().expect("IR fixture builds"))
            .expect("IR serializes");
        let object = value.as_object().expect("IR serializes as object");
        let keys = object.keys().map(String::as_str).collect::<Vec<_>>();

        assert_eq!(
            keys,
            vec![
                "anchors",
                "effects",
                "identity",
                "nodes",
                "provenance",
                "token_inputs",
                "values"
            ]
        );
    }

    #[test]
    fn infer_ir_identity_excludes_audit_parents() {
        let value = serde_json::to_value(identity()).expect("identity serializes");

        assert!(value.get("quant_graph_self_hash").is_some());
        assert!(value.get("infer_ir_policy_projection_hash").is_some());
        assert!(value.get("static_budget_self_hash").is_some());
        assert!(value.get("requested_runtime_modes_hash").is_some());
        assert!(value.get("determinism").is_some());
        assert!(value.get("topological_order_hash").is_some());
        assert_forbidden_keys_absent(
            &value,
            &["policy_resolution_self_hash", "compile_request_hash"],
        );
    }

    #[test]
    fn infer_ir_identity_carries_topological_order_hash() {
        let identity = identity();
        assert_eq!(identity.topological_order_hash, hash(6));
    }

    #[test]
    fn gb_infer_ir_struct_does_not_contain_self_hash() {
        let value = serde_json::to_value(gb_infer_ir_fixture().expect("IR fixture builds"))
            .expect("IR serializes");

        assert_forbidden_keys_absent(
            &value,
            &["infer_ir_self_hash", "infer_ir_canonical_bytes_hash"],
        );
    }

    #[test]
    fn token_inputs_v1_exactly_one() {
        assert!(gb_infer_ir_fixture().is_ok());
        assert!(matches!(
            GbInferIR::new(
                identity(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                InferIrProvenance::default(),
                NodeAnchorMap::new(),
            ),
            Err(GbInferIrTypeError::TokenInputsV1Count { observed: 0 })
        ));
        assert!(matches!(
            GbInferIR::new(
                identity(),
                vec![token_input(), token_input()],
                Vec::new(),
                Vec::new(),
                Vec::new(),
                InferIrProvenance::default(),
                NodeAnchorMap::new(),
            ),
            Err(GbInferIrTypeError::TokenInputsV1Count { observed: 2 })
        ));
    }

    #[test]
    fn token_input_carries_explicit_value_id() {
        let TokenInput {
            token_input_id,
            value_id,
            allowed_ingress_modes,
        } = token_input();

        assert_eq!(token_input_id, TokenInputId::new(0));
        assert_eq!(value_id, ValueId::new(0));
        assert!(allowed_ingress_modes.contains(&TokenIngressMode::Prompt));
    }

    #[test]
    fn token_input_allowed_ingress_modes_is_non_empty_set() {
        let token_input = TokenInput::new(
            TokenInputId::new(0),
            ValueId::new(0),
            BTreeSet::from([TokenIngressMode::Prompt]),
        )
        .expect("single ingress mode is non-empty");

        assert_eq!(token_input.allowed_ingress_modes.len(), 1);
        assert!(matches!(
            TokenInput::new(TokenInputId::new(0), ValueId::new(0), BTreeSet::new()),
            Err(GbInferIrTypeError::TokenInputIngressModesEmpty)
        ));
    }

    #[test]
    fn token_input_id_is_u8_newtype() {
        let max = TokenInputId::new(u8::MAX);

        assert_eq!(max.get(), u8::MAX);
        assert_eq!(
            std::mem::size_of::<TokenInputId>(),
            std::mem::size_of::<u8>()
        );
    }

    #[test]
    fn token_ingress_mode_two_variants_closed() {
        let modes = [TokenIngressMode::AutoRegressive, TokenIngressMode::Prompt];
        let names = modes
            .iter()
            .map(|mode| format!("{mode:?}"))
            .collect::<Vec<_>>();

        assert_eq!(modes.len(), 2);
        assert_eq!(names, vec!["AutoRegressive", "Prompt"]);
    }

    #[test]
    fn non_empty_set_constructor_rejects_empty() {
        assert!(NonEmptySet::<TokenIngressMode>::new(BTreeSet::new()).is_err());
        assert!(
            NonEmptySet::new(BTreeSet::from([TokenIngressMode::Prompt]))
                .expect("non-empty")
                .contains(&TokenIngressMode::Prompt)
        );
    }

    #[test]
    fn token_input_serde_round_trip_proptest() {
        for modes in [
            BTreeSet::from([TokenIngressMode::Prompt]),
            BTreeSet::from([TokenIngressMode::AutoRegressive]),
            BTreeSet::from([TokenIngressMode::AutoRegressive, TokenIngressMode::Prompt]),
        ] {
            let input = TokenInput::new(TokenInputId::new(0), ValueId::new(9), modes)
                .expect("modes are non-empty");
            let encoded = serde_json::to_string(&input).expect("token input serializes");
            let decoded: TokenInput =
                serde_json::from_str(&encoded).expect("token input deserializes");

            assert_eq!(decoded, input);
        }
    }

    #[test]
    fn token_input_deserialize_rejects_empty_ingress_modes() {
        let json = serde_json::json!({
            "token_input_id": 0,
            "value_id": 0,
            "allowed_ingress_modes": [],
        });

        assert!(serde_json::from_value::<TokenInput>(json).is_err());
    }

    #[test]
    fn iir_identity_binding_pulls_determinism_from_quant_graph_not_policy() {
        let mut graph = wave4_quant_graph_dense();
        graph.identity.determinism = DeterminismClass::Nondeterministic;
        let budget = FakeStaticBudget::new(hash(0x44));
        let inputs = infer_inputs(&graph, &budget);

        let identity = bind_identity(&inputs).expect("identity binds");

        assert_eq!(identity.determinism, DeterminismClass::Nondeterministic);
        assert_eq!(identity.quant_graph_self_hash, inputs.quant_graph_self_hash);
        assert_eq!(identity.static_budget_self_hash, budget.hash);
        assert_eq!(identity.topological_order_hash, Hash256::ZERO);
    }

    #[test]
    fn iir_identity_binding_excludes_audit_parents_in_identity_struct() {
        let graph = wave4_quant_graph_dense();
        let budget = FakeStaticBudget::new(hash(0x44));
        let mut left = infer_inputs(&graph, &budget);
        let mut right = infer_inputs(&graph, &budget);
        left.audit_parents = InferIrAuditParents {
            policy_resolution_self_hash: hash(0x10),
            compile_request_hash: hash(0x11),
        };
        right.audit_parents = InferIrAuditParents {
            policy_resolution_self_hash: hash(0x20),
            compile_request_hash: hash(0x21),
        };

        let left_identity = bind_identity(&left).expect("left identity binds");
        let right_identity = bind_identity(&right).expect("right identity binds");
        let encoded = serde_json::to_value(left_identity).expect("identity serializes");

        assert_eq!(left_identity, right_identity);
        assert_forbidden_keys_absent(
            &encoded,
            &["policy_resolution_self_hash", "compile_request_hash"],
        );
    }

    #[test]
    fn iir_identity_static_budget_self_hash_equals_envelope_self_hash() {
        let graph = wave4_quant_graph_dense();
        let budget = FakeStaticBudget::new(hash(0x44));
        let mut inputs = infer_inputs(&graph, &budget);
        inputs.static_budget_self_hash = hash(0x45);

        let diagnostics = bind_identity(&inputs).expect_err("mismatched static budget rejects");

        assert!(diagnostics.iter().any(|diagnostic| {
            matches!(diagnostic.detail, ValidationDetail::HashMismatch { .. })
        }));
    }

    #[test]
    fn token_input_binding_creates_single_token_input_v1() {
        let projection = policy_projection();

        let token_input = bind_token_input(&projection).expect("token input binds");

        assert_eq!(token_input.token_input_id, TokenInputId::new(0));
        assert_eq!(token_input.value_id, ValueId::new(0));
        assert_eq!(token_input.allowed_ingress_modes.len(), 2);
        assert!(
            token_input
                .allowed_ingress_modes
                .contains(&TokenIngressMode::Prompt)
        );
        assert!(
            token_input
                .allowed_ingress_modes
                .contains(&TokenIngressMode::AutoRegressive)
        );
    }

    #[test]
    fn token_input_binding_input_token_value_id_is_first_allocated() {
        let graph = wave4_quant_graph_dense();
        let values = bind_value_allocation(&graph).expect("values allocate");

        let input_token = values
            .values
            .iter()
            .find(|value| value.kind == ValueKind::InputToken)
            .expect("input token value exists");

        assert_eq!(input_token.value_id, ValueId::new(0));
        assert_eq!(
            values.id(ValueAllocationKey::InputToken),
            Some(ValueId::new(0))
        );
    }

    #[test]
    fn value_allocation_two_regenerations_match() {
        let graph = wave4_quant_graph_dense();

        let first = bind_value_allocation(&graph).expect("first allocation succeeds");
        let second = bind_value_allocation(&graph).expect("second allocation succeeds");

        assert_eq!(first, second);
        assert!(!first.values.iter().any(|value| matches!(
            value.kind,
            ValueKind::SequenceStateRead | ValueKind::SequenceStateNext
        )));
    }

    #[test]
    fn value_allocation_input_token_is_first_id() {
        let graph = wave4_quant_graph_dense();
        let values = bind_value_allocation(&graph).expect("values allocate");

        assert_eq!(
            values
                .values
                .first()
                .map(|value| (value.value_id, value.kind)),
            Some((ValueId::new(0), ValueKind::InputToken))
        );
    }

    #[test]
    fn value_allocation_skips_unused_ops_in_v1() {
        let graph = wave4_quant_graph_dense();
        let values = bind_value_allocation(&graph).expect("values allocate");
        let kinds = values
            .values
            .iter()
            .map(|value| value.kind)
            .collect::<BTreeSet<_>>();

        assert!(!kinds.contains(&ValueKind::SequenceStateRead));
        assert!(!kinds.contains(&ValueKind::SequenceStateNext));
        assert!(!kinds.contains(&ValueKind::SequenceBlockOutput));
    }

    #[test]
    fn value_allocation_canonical_provenance_tuple_keyed() {
        let graph = wave4_quant_graph_routed();
        let up = canonical_provenance_tuple_base(
            InferOp::ExpertMatVec {
                layer: LayerId::new(0),
                expert: ExpertId::new(1),
                slot: ExpertWeightSlot::FfnUp,
            },
            &graph,
        );
        let down = canonical_provenance_tuple_base(
            InferOp::ExpertMatVec {
                layer: LayerId::new(0),
                expert: ExpertId::new(1),
                slot: ExpertWeightSlot::FfnDown,
            },
            &graph,
        );

        assert_eq!(up.layer, Some(LayerId::new(0)));
        assert_eq!(up.expert, Some(ExpertId::new(1)));
        assert_eq!(up.expert_weight_slot, Some(ExpertWeightSlot::FfnUp));
        assert_ne!(up, down);
    }

    #[test]
    fn effect_allocation_rng_chain_iff_requires_rng() {
        let mut graph = wave4_quant_graph_dense();
        let no_rng = bind_effect_allocation(&graph).expect("argmax has no rng");
        graph.decode_spec = DecodeSpecRecord {
            decode_plan_id: DecodePlanId::new(0),
            spec: DecodeSpec::TopKTemperature {
                k: 3,
                temperature_q8_8: 256,
            },
            requires_rng: true,
        };

        let rng = bind_effect_allocation(&graph).expect("sampled decode has rng");

        assert!(no_rng.decode_rng.is_none());
        assert!(no_rng.effects.is_empty());
        assert_eq!(rng.decode_rng.expect("rng pair").input, EffectId::new(0));
        assert_eq!(rng.effects.len(), 2);
        assert!(rng.effects.iter().all(|effect| {
            matches!(
                effect.class,
                EffectClass::Rng {
                    slot: RngSlot::Decode
                }
            )
        }));
    }

    #[test]
    fn effect_allocation_rejects_non_empty_state_slots_v1() {
        let mut graph = wave4_quant_graph_dense();
        graph.sequence_semantics = SequenceSemanticsSpec {
            kind: SequenceSemanticsKind::Identity,
            state_slots: BTreeMap::from([(
                LayerId::new(0),
                vec![SequenceStateSlot {
                    slot_id: 0,
                    width_bytes: 8,
                }],
            )]),
        };

        let diagnostics = bind_effect_allocation(&graph).expect_err("state slots reject in v1");

        assert!(diagnostics.iter().any(|diagnostic| matches!(
            diagnostic.code,
            ValidationCode::InferIrSequenceSemanticsUnsupportedV1 { .. }
        )));
    }

    #[test]
    fn effect_allocation_edge_token_uniqueness() {
        let mut graph = wave4_quant_graph_dense();
        graph.decode_spec = wave4_decode_spec(true);

        let effects = bind_effect_allocation(&graph).expect("rng effects allocate");
        let ids = effects
            .effects
            .iter()
            .map(|effect| effect.effect_id)
            .collect::<BTreeSet<_>>();

        assert_eq!(ids.len(), effects.effects.len());
        assert_eq!(
            effects.decode_rng.expect("rng pair").input,
            EffectId::new(0)
        );
        assert_eq!(
            effects.decode_rng.expect("rng pair").output,
            EffectId::new(1)
        );
    }

    #[test]
    fn effect_allocation_no_fault_boundary_in_v1() {
        let mut graph = wave4_quant_graph_dense();
        graph.decode_spec = wave4_decode_spec(true);

        let effects = bind_effect_allocation(&graph).expect("effects allocate");

        assert!(
            effects
                .effects
                .iter()
                .all(|effect| !matches!(effect.class, EffectClass::FaultBoundary))
        );
    }

    #[test]
    fn node_building_dense_emits_no_router_or_select() {
        let graph = wave4_quant_graph_dense();
        let values = bind_value_allocation(&graph).expect("values allocate");
        let effects = bind_effect_allocation(&graph).expect("effects allocate");

        let nodes = bind_node_building(&graph, &values, &effects).expect("nodes build");
        let tags = nodes.iter().map(|node| node.op.tag()).collect::<Vec<_>>();

        assert!(!tags.contains(&InferOpTag::RouterMatVec));
        assert!(!tags.contains(&InferOpTag::RouteTop1));
        assert!(!tags.contains(&InferOpTag::SelectExpertTop1));
        assert!(!tags.contains(&InferOpTag::SequenceRead));
        assert!(nodes.iter().any(|node| matches!(
            node.op,
            InferOp::ExpertMatVec {
                layer,
                expert,
                slot: ExpertWeightSlot::FfnUp
            } if layer == LayerId::new(0) && expert == ExpertId::new(0)
        )));
    }

    #[test]
    fn node_building_routed_emits_router_route_select() {
        let graph = wave4_quant_graph_routed();
        let values = bind_value_allocation(&graph).expect("values allocate");
        let effects = bind_effect_allocation(&graph).expect("effects allocate");

        let nodes = bind_node_building(&graph, &values, &effects).expect("nodes build");
        let tags = nodes.iter().map(|node| node.op.tag()).collect::<Vec<_>>();

        assert!(tags.contains(&InferOpTag::RouterMatVec));
        assert!(tags.contains(&InferOpTag::RouteTop1));
        assert!(tags.contains(&InferOpTag::SelectExpertTop1));
        assert_eq!(
            nodes
                .iter()
                .filter(|node| matches!(
                    node.op,
                    InferOp::ExpertMatVec {
                        slot: ExpertWeightSlot::FfnDown,
                        ..
                    }
                ))
                .count(),
            2
        );
    }

    #[test]
    fn effect_allocation_classify_does_not_touch_rng_chain() {
        let mut graph = wave4_quant_graph_dense();
        graph.decode_spec = wave4_decode_spec(true);
        let values = bind_value_allocation(&graph).expect("values allocate");
        let effects = bind_effect_allocation(&graph).expect("effects allocate");

        let nodes = bind_node_building(&graph, &values, &effects).expect("nodes build");
        let classify = nodes
            .iter()
            .find(|node| matches!(node.op, InferOp::Classify))
            .expect("classify node exists");
        let decode = nodes
            .iter()
            .find(|node| matches!(node.op, InferOp::DecodeToken { .. }))
            .expect("decode node exists");

        assert!(classify.effects_in.is_empty());
        assert!(classify.effects_out.is_empty());
        assert_eq!(decode.effects_in, vec![EffectId::new(0)]);
        assert_eq!(decode.effects_out, vec![EffectId::new(1)]);
    }

    #[test]
    fn node_building_v1_no_sequence_read_step_write() {
        let graph = wave4_quant_graph_routed();
        let values = bind_value_allocation(&graph).expect("values allocate");
        let effects = bind_effect_allocation(&graph).expect("effects allocate");

        let nodes = bind_node_building(&graph, &values, &effects).expect("nodes build");

        assert!(nodes.iter().all(|node| !matches!(
            node.op,
            InferOp::SequenceRead { .. }
                | InferOp::SequenceStep { .. }
                | InferOp::SequenceWrite { .. }
        )));
    }

    #[test]
    fn node_building_emits_ffn_activation_per_expert() {
        let graph = wave4_quant_graph_routed();
        let values = bind_value_allocation(&graph).expect("values allocate");
        let effects = bind_effect_allocation(&graph).expect("effects allocate");

        let nodes = bind_node_building(&graph, &values, &effects).expect("nodes build");
        let experts = nodes
            .iter()
            .filter_map(|node| match node.op {
                InferOp::FfnActivation { expert, .. } => Some(expert),
                _ => None,
            })
            .collect::<BTreeSet<_>>();

        assert_eq!(
            experts,
            BTreeSet::from([ExpertId::new(0), ExpertId::new(1)])
        );
    }

    #[test]
    fn node_building_node_id_not_assigned_here() {
        let graph = wave4_quant_graph_dense();
        let values = bind_value_allocation(&graph).expect("values allocate");
        let effects = bind_effect_allocation(&graph).expect("effects allocate");
        let mut nodes = bind_node_building(&graph, &values, &effects).expect("nodes build");
        nodes.reverse();
        for (index, node) in nodes.iter_mut().enumerate() {
            node.node_id = NodeId::new(10_000 + index as u32);
        }

        let (_identity, sorted) =
            bind_canonical_sort(identity(), nodes, &graph).expect("canonical sort binds");

        assert_eq!(
            sorted
                .iter()
                .map(|node| node.node_id.get())
                .collect::<Vec<_>>(),
            (0..sorted.len() as u32).collect::<Vec<_>>()
        );
        assert_eq!(
            sorted.first().expect("node").op.tag(),
            InferOpTag::Embedding
        );
    }

    #[test]
    fn node_building_routed_layer_full_expert_coverage() {
        let graph = wave4_quant_graph_routed();
        let values = bind_value_allocation(&graph).expect("values allocate");
        let effects = bind_effect_allocation(&graph).expect("effects allocate");

        let nodes = bind_node_building(&graph, &values, &effects).expect("nodes build");
        let down_experts = nodes
            .iter()
            .filter_map(|node| match node.op {
                InferOp::ExpertMatVec {
                    expert,
                    slot: ExpertWeightSlot::FfnDown,
                    ..
                } => Some(expert),
                _ => None,
            })
            .collect::<BTreeSet<_>>();

        assert_eq!(
            down_experts,
            BTreeSet::from([ExpertId::new(0), ExpertId::new(1)])
        );
    }

    #[test]
    fn node_building_emits_diagnostic_on_router_present_for_dense_layer() {
        let mut graph = wave4_quant_graph_dense();
        graph.routing_table = Some(RoutingTable {
            layers: vec![RouterLayer {
                layer: LayerId::new(0),
                n_experts: 1,
                router_weight: TensorId::new(10),
                router_bias: None,
                semantics: RouterSemantics::Top1Hard {
                    gate_weight: RouterGateWeightSemantics::SelectedScore,
                    tie_break: RouterTieBreak::LowestExpertId,
                },
            }],
        });
        let values = bind_value_allocation(&graph).expect("values allocate");
        let effects = bind_effect_allocation(&graph).expect("effects allocate");

        let diagnostics =
            bind_node_building(&graph, &values, &effects).expect_err("dense router rejects");

        assert!(diagnostics.iter().any(|diagnostic| matches!(
            diagnostic.code,
            ValidationCode::InferIrRouterPresentForDenseLayer { layer }
                if layer == LayerId::new(0)
        )));
    }

    #[test]
    fn node_building_emits_diagnostic_on_router_missing_for_routed_layer() {
        let mut graph = wave4_quant_graph_routed();
        graph.routing_table = None;
        let values = bind_value_allocation(&graph).expect("values allocate");
        let effects = bind_effect_allocation(&graph).expect("effects allocate");

        let diagnostics =
            bind_node_building(&graph, &values, &effects).expect_err("routed layer rejects");

        assert!(diagnostics.iter().any(|diagnostic| matches!(
            diagnostic.code,
            ValidationCode::InferIrRouterMatVecMissingForRoutedLayer { layer }
                if layer == LayerId::new(0)
        )));
    }

    #[test]
    fn op_signature_closed_match_over_infer_op_tag() {
        let graph = wave4_quant_graph_routed();

        for op in all_infer_ops() {
            assert!(
                op_signature(op, &graph).is_ok(),
                "signature missing for {:?}",
                op.tag()
            );
        }
    }

    #[test]
    fn op_signature_classify_requires_reduction_site_and_logit_output() {
        let graph = wave4_quant_graph_dense();
        let values = vec![
            ValueDecl {
                value_id: ValueId::new(0),
                kind: ValueKind::NormalizedActivation,
                format: ValueFormat::Quant {
                    format: QuantFormat::Q8_8,
                },
                layout: vector_layout(ValueAxis::Model),
            },
            ValueDecl {
                value_id: ValueId::new(1),
                kind: ValueKind::LogitVector,
                format: ValueFormat::ExactAccumulator,
                layout: vector_layout(ValueAxis::Vocabulary),
            },
        ];
        let ok = GbNode {
            node_id: NodeId::new(0),
            op: InferOp::Classify,
            inputs: vec![ValueId::new(0)],
            effects_in: Vec::new(),
            outputs: vec![ValueId::new(1)],
            effects_out: Vec::new(),
            reduction_site: Some(ReductionSiteId("classify.logits".to_owned())),
        };
        let mut bad = ok.clone();
        bad.reduction_site = None;

        assert!(check_op_signature(&ok, &values, &[], &graph).is_ok());
        assert!(check_op_signature(&bad, &values, &[], &graph).is_err());
        assert!(reduction_site_bearing(InferOp::Classify, &graph));
    }

    #[test]
    fn op_signature_per_variant_table_test() {
        let graph = wave4_quant_graph_routed();

        for op in all_infer_ops() {
            let signature = op_signature(op, &graph).expect("signature exists");
            match op {
                InferOp::Embedding { .. } => {
                    assert_eq!(signature.input_kinds, vec![ValueKind::InputToken]);
                    assert_eq!(signature.output_kinds, vec![ValueKind::EmbeddingOutput]);
                }
                InferOp::RouterMatVec { .. } => {
                    assert_eq!(signature.output_kinds, vec![ValueKind::RouterScore]);
                    assert_eq!(signature.reduction_site, ReductionSiteRequirement::Required);
                }
                InferOp::RouteTop1 { .. } => {
                    assert_eq!(
                        signature.output_kinds,
                        vec![ValueKind::RouterDecision, ValueKind::GateWeight]
                    );
                }
                InferOp::SelectExpertTop1 { .. } => {
                    assert_eq!(signature.input_kinds.len(), 4);
                    assert_eq!(signature.output_kinds, vec![ValueKind::ExpertOutput]);
                }
                InferOp::ExpertMatVec {
                    slot: ExpertWeightSlot::FfnDown,
                    ..
                } => {
                    assert_eq!(signature.input_kinds, vec![ValueKind::ExpertIntermediate]);
                    assert_eq!(signature.output_kinds, vec![ValueKind::ExpertCandidate]);
                }
                InferOp::ExpertMatVec { .. } => {
                    assert_eq!(signature.input_kinds, vec![ValueKind::NormalizedActivation]);
                    assert_eq!(signature.output_kinds, vec![ValueKind::ExpertIntermediate]);
                }
                InferOp::DecodeToken { .. } => {
                    assert_eq!(signature.input_kinds, vec![ValueKind::LogitVector]);
                    assert_eq!(signature.output_kinds, vec![ValueKind::DecodedToken]);
                }
                InferOp::SequenceRead { slot } | InferOp::SequenceWrite { slot } => {
                    let state = EffectClass::SequenceState { slot };
                    assert_eq!(signature.effects_in, vec![state]);
                    assert_eq!(signature.effects_out, vec![state]);
                }
                _ => {
                    assert!(signature.effects_in.is_empty());
                    assert!(signature.effects_out.is_empty());
                }
            }
        }
    }

    #[test]
    fn op_signature_norm_reduction_bearing_iff_tile_rms_plan() {
        let mut graph = wave4_quant_graph_dense();
        assert!(reduction_site_bearing(
            InferOp::Norm {
                plan: NormPlanId::new(0)
            },
            &graph
        ));
        graph
            .norm_plans
            .iter_mut()
            .find(|record| record.norm_plan_id == NormPlanId::new(0))
            .expect("norm plan exists")
            .plan = affine_clip_lut_plan();

        assert!(!reduction_site_bearing(
            InferOp::Norm {
                plan: NormPlanId::new(0)
            },
            &graph
        ));
    }

    #[test]
    fn reduction_site_bearing_matches_op_signature_predicate() {
        let graph = wave4_quant_graph_routed();

        for op in all_infer_ops() {
            let signature = op_signature(op, &graph).expect("signature exists");
            assert_eq!(
                reduction_site_bearing(op, &graph),
                signature.reduction_site == ReductionSiteRequirement::Required
            );
        }
    }

    #[test]
    fn wave4_classes_1_to_5_collect_diagnostics_no_short_circuit() {
        let mut graph = wave4_quant_graph_dense();
        graph.tensors.clear();
        graph.sequence_semantics = SequenceSemanticsSpec {
            kind: SequenceSemanticsKind::Identity,
            state_slots: BTreeMap::from([(
                LayerId::new(0),
                vec![SequenceStateSlot {
                    slot_id: 0,
                    width_bytes: 8,
                }],
            )]),
        };
        let budget = FakeStaticBudget::new(hash(0x44));
        let mut inputs = infer_inputs(&graph, &budget);
        inputs.static_budget_self_hash = hash(0x45);

        let diagnostics =
            bind_wave4_classes_1_to_5(&inputs).expect_err("multiple wave4 errors collect");

        assert!(diagnostics.len() >= 3, "{diagnostics:#?}");
    }

    #[test]
    fn wave4_classes_1_to_5_two_clean_runs_byte_identical_intermediate_state() {
        let graph = wave4_quant_graph_routed();
        let budget = FakeStaticBudget::new(hash(0x44));
        let inputs = infer_inputs(&graph, &budget);

        let first = bind_wave4_classes_1_to_5(&inputs).expect("first run binds");
        let second = bind_wave4_classes_1_to_5(&inputs).expect("second run binds");
        let first_bytes = wave4_canonical_bytes(&first);
        let second_bytes = wave4_canonical_bytes(&second);

        assert_eq!(first_bytes, second_bytes);
    }

    #[test]
    fn reduction_site_binding_uses_canonical_id_scheme() {
        let graph = wave4_quant_graph_routed();
        let wave4 = wave4_intermediate(&graph);
        let budget = FakeStaticBudget::with_sites(
            hash(0x44),
            reduction_sites_for_nodes(&wave4.nodes, &graph),
        );

        let nodes = bind_reduction_sites(wave4.nodes, &graph, &budget)
            .expect("reduction sites bind from static budget");
        let sites = nodes
            .iter()
            .filter_map(|node| node.reduction_site.as_ref().map(|site| site.0.as_str()))
            .collect::<BTreeSet<_>>();

        assert!(sites.contains("router.0"));
        assert!(sites.contains("expert.0.0.up"));
        assert!(sites.contains("expert.0.1.down"));
        assert!(sites.contains("norm.2"));
        assert!(sites.contains("classify"));
    }

    #[test]
    fn reduction_site_binding_uses_static_budget_keys() {
        let graph = wave4_quant_graph_dense();
        let wave4 = wave4_intermediate(&graph);
        let mut sites = reduction_sites_for_nodes(&wave4.nodes, &graph);
        sites.retain(|site| site.0 == "classify");
        let budget = FakeStaticBudget::with_sites(hash(0x44), sites);

        let diagnostics = bind_reduction_sites(wave4.nodes, &graph, &budget)
            .expect_err("missing static-budget keys reject");

        assert!(diagnostics.iter().any(|diagnostic| {
            matches!(
                &diagnostic.detail,
                ValidationDetail::Field { field } if field.as_str() == "InferIrReductionSiteMissing"
            )
        }));
    }

    #[test]
    fn reduction_site_binding_emits_missing_typed_diagnostic() {
        let graph = wave4_quant_graph_dense();
        let wave4 = wave4_intermediate(&graph);
        let budget = FakeStaticBudget::new(hash(0x44));

        let diagnostics =
            bind_reduction_sites(wave4.nodes, &graph, &budget).expect_err("missing sites reject");

        assert!(diagnostics.iter().any(|diagnostic| {
            matches!(
                &diagnostic.detail,
                ValidationDetail::Field { field } if field.as_str() == "InferIrReductionSiteMissing"
            )
        }));
    }

    #[test]
    fn reduction_site_only_some_on_bearing_ops() {
        let graph = wave4_quant_graph_routed();
        let wave4 = wave4_intermediate(&graph);
        let budget = FakeStaticBudget::with_sites(
            hash(0x44),
            reduction_sites_for_nodes(&wave4.nodes, &graph),
        );

        let nodes = bind_reduction_sites(wave4.nodes, &graph, &budget)
            .expect("reduction sites bind from static budget");

        for node in &nodes {
            assert_eq!(
                node.reduction_site.is_some(),
                reduction_site_bearing(node.op, &graph),
                "{:?}",
                node.op
            );
        }
    }

    #[test]
    fn iir_provenance_binding_total_image() {
        let graph = wave4_quant_graph_dense();
        let wave4 = wave4_intermediate(&graph);
        let budget = FakeStaticBudget::with_sites(
            hash(0x44),
            reduction_sites_for_nodes(&wave4.nodes, &graph),
        );
        let nodes = bind_reduction_sites(wave4.nodes, &graph, &budget).expect("sites bind");
        let provenance = bind_infer_ir_provenance(
            &nodes,
            &wave4.values.values,
            &wave4.effects.effects,
            &[wave4.token_input],
            &graph,
        )
        .expect("provenance binds");

        provenance
            .validate_totality(&nodes, &wave4.values.values, &wave4.effects.effects)
            .expect("provenance totality validates");
        assert_eq!(provenance.nodes.len(), nodes.len());
        assert_eq!(provenance.values.len(), wave4.values.values.len());
    }

    #[test]
    fn iir_anchor_binding_includes_qg_self_hash() {
        let graph = wave4_quant_graph_dense();
        let wave4 = wave4_intermediate(&graph);

        let left = bind_node_anchors(hash(0x10), &wave4.nodes, &graph).expect("anchors bind");
        let right = bind_node_anchors(hash(0x11), &wave4.nodes, &graph).expect("anchors bind");

        assert_ne!(left, right);
    }

    #[test]
    fn iir_anchor_binding_stable_across_two_qg_regenerations() {
        let graph = wave4_quant_graph_dense();
        let wave4 = wave4_intermediate(&graph);

        let left = bind_node_anchors(hash(0x10), &wave4.nodes, &graph).expect("anchors bind");
        let right = bind_node_anchors(hash(0x10), &wave4.nodes, &graph).expect("anchors bind");

        assert_eq!(left, right);
    }

    #[test]
    fn canonical_sort_node_id_assigned_after_sort() {
        let graph = wave4_quant_graph_dense();
        let wave4 = wave4_intermediate(&graph);
        let mut nodes = wave4.nodes;
        nodes.reverse();
        for (index, node) in nodes.iter_mut().enumerate() {
            node.node_id = NodeId::new(100 + index as u32);
        }

        let (_identity, sorted) =
            bind_canonical_sort(wave4.identity, nodes, &graph).expect("canonical sort binds");

        assert_eq!(
            sorted.first().expect("node").op.tag(),
            InferOpTag::Embedding
        );
        assert_eq!(
            sorted
                .iter()
                .map(|node| node.node_id.get())
                .collect::<Vec<_>>(),
            (0..sorted.len() as u32).collect::<Vec<_>>()
        );
    }

    #[test]
    fn topological_order_hash_two_regenerations_match() {
        let graph = wave4_quant_graph_routed();
        let wave4 = wave4_intermediate(&graph);
        let (_identity, sorted) =
            bind_canonical_sort(wave4.identity, wave4.nodes, &graph).expect("sort binds");

        let left = compute_topological_order_hash(&sorted, &graph).expect("hash computes");
        let right = compute_topological_order_hash(&sorted, &graph).expect("hash computes");

        assert_eq!(left, right);
    }

    #[test]
    fn wave4_classes_6_to_10_finalizes_consistent_ir() {
        let graph = wave4_quant_graph_dense();
        let first_wave = wave4_intermediate(&graph);
        let sites = reduction_sites_for_nodes(&first_wave.nodes, &graph);
        let budget = FakeStaticBudget::with_sites(hash(0x44), sites);
        let inputs = infer_inputs(&graph, &budget);
        let second_wave = bind_wave4_classes_1_to_5(&inputs).expect("classes 1-5 bind");

        let ir = bind_wave4_classes_6_to_10(&inputs, second_wave).expect("classes 6-10 bind");

        assert_eq!(
            ir.identity.topological_order_hash,
            compute_topological_order_hash(&ir.nodes, &graph).expect("hash computes")
        );
        assert!(validate_infer_ir_self_consistency(&ir, &graph).is_empty());
        assert_eq!(ir.anchors.len(), ir.nodes.len());
    }

    #[cfg(feature = "semantic_equivalence_check")]
    #[test]
    fn fixture_semantic_equivalence_dense_toy0_bit_exact() {
        let graph = wave4_quant_graph_dense();
        let ir = finalized_ir_for_graph(&graph);

        let result = fixture_semantic_equivalence(&ir, &graph, &fixture_inputs())
            .expect("semantic equivalence passes");

        assert_eq!(result, FixtureEquivalenceResult::VerifiedFixtureBitExact);
    }

    #[cfg(not(feature = "semantic_equivalence_check"))]
    #[test]
    fn fixture_semantic_equivalence_dense_toy0_bit_exact() {
        let graph = wave4_quant_graph_dense();
        let ir = finalized_ir_for_graph(&graph);

        let result = fixture_semantic_equivalence(&ir, &graph, &fixture_inputs())
            .expect("feature-disabled semantic equivalence skips");

        assert_eq!(
            result,
            FixtureEquivalenceResult::Skipped {
                reason: report_schema::FixtureEquivalenceSkippedReason::FeatureFlagDisabled
            }
        );
    }

    #[cfg(feature = "semantic_equivalence_check")]
    #[test]
    fn fixture_semantic_equivalence_routed_basic_bit_exact() {
        let graph = wave4_quant_graph_routed();
        let ir = finalized_ir_for_graph(&graph);

        let result = fixture_semantic_equivalence(&ir, &graph, &fixture_inputs())
            .expect("semantic equivalence passes");

        assert_eq!(result, FixtureEquivalenceResult::VerifiedFixtureBitExact);
    }

    #[cfg(not(feature = "semantic_equivalence_check"))]
    #[test]
    fn fixture_semantic_equivalence_routed_basic_bit_exact() {
        let graph = wave4_quant_graph_routed();
        let ir = finalized_ir_for_graph(&graph);

        let result = fixture_semantic_equivalence(&ir, &graph, &fixture_inputs())
            .expect("feature-disabled semantic equivalence skips");

        assert_eq!(
            result,
            FixtureEquivalenceResult::Skipped {
                reason: report_schema::FixtureEquivalenceSkippedReason::FeatureFlagDisabled
            }
        );
    }

    #[test]
    fn fixture_semantic_equivalence_skipped_for_non_bit_exact() {
        let mut graph = wave4_quant_graph_dense();
        graph.identity.determinism = DeterminismClass::Deterministic;
        let ir = finalized_ir_for_graph(&graph);

        let result = fixture_semantic_equivalence(&ir, &graph, &fixture_inputs())
            .expect("non-bit-exact semantic equivalence skips");

        assert_eq!(
            result,
            FixtureEquivalenceResult::Skipped {
                reason: report_schema::FixtureEquivalenceSkippedReason::NonBitExactDeterminism
            }
        );
    }

    #[cfg(not(feature = "semantic_equivalence_check"))]
    #[test]
    fn feature_flag_off_yields_skipped_feature_flag_disabled() {
        let graph = wave4_quant_graph_dense();
        let ir = finalized_ir_for_graph(&graph);

        let result = fixture_semantic_equivalence(&ir, &graph, &fixture_inputs())
            .expect("feature-disabled semantic equivalence skips");

        assert_eq!(
            result,
            FixtureEquivalenceResult::Skipped {
                reason: report_schema::FixtureEquivalenceSkippedReason::FeatureFlagDisabled
            }
        );
    }

    #[test]
    fn non_fixture_build_yields_skipped_non_fixture_build() {
        let graph = wave4_quant_graph_dense();
        let ir = finalized_ir_for_graph(&graph);
        let fixture = FixtureInputSet::non_fixture(vec![fixture_input(3)]);

        let result = fixture_semantic_equivalence(&ir, &graph, &fixture)
            .expect("non-fixture semantic equivalence skips");

        assert_eq!(
            result,
            FixtureEquivalenceResult::Skipped {
                reason: report_schema::FixtureEquivalenceSkippedReason::NonFixtureBuild
            }
        );
    }

    #[cfg(feature = "semantic_equivalence_check")]
    #[test]
    fn fixture_semantic_equivalence_public_path_propagates_divergence() {
        let graph = wave4_quant_graph_dense();
        let mut ir = finalized_ir_for_graph(&graph);
        ir.nodes
            .iter_mut()
            .find(|node| matches!(node.op, InferOp::DecodeToken { .. }))
            .expect("decode exists")
            .op = InferOp::DecodeToken {
            plan: DecodePlanId::new(99),
        };

        let result = fixture_semantic_equivalence(&ir, &graph, &fixture_inputs());

        assert!(!matches!(
            result,
            Ok(FixtureEquivalenceResult::VerifiedFixtureBitExact)
        ));
        let diagnostics = result.expect_err("semantic divergence returns diagnostics");
        assert_eq!(diagnostics.len(), fixture_inputs().inputs.len());
        assert!(diagnostics.iter().all(|diagnostic| matches!(
            diagnostic.code,
            ValidationCode::InferIrSemanticEquivalenceFailed { .. }
        )));
    }

    #[cfg(feature = "semantic_equivalence_check")]
    #[test]
    fn stage3_semantic_equivalence_run_trace_is_subscriber_captured() {
        let capture = TraceCapture::default();
        let subscriber = tracing_subscriber::registry()
            .with(LevelFilter::TRACE)
            .with(capture.clone());
        let graph = wave4_quant_graph_dense();
        let ir = finalized_ir_for_graph(&graph);

        tracing::callsite::rebuild_interest_cache();
        tracing::subscriber::with_default(subscriber, || {
            tracing::callsite::rebuild_interest_cache();
            assert_eq!(
                fixture_semantic_equivalence(&ir, &graph, &fixture_inputs())
                    .expect("semantic equivalence passes"),
                FixtureEquivalenceResult::VerifiedFixtureBitExact
            );
            tracing::callsite::rebuild_interest_cache();
        });
        tracing::callsite::rebuild_interest_cache();

        assert!(capture.records().iter().any(|record| {
            record.level == "INFO"
                && record.field_equals("event", STAGE3_SEMANTIC_EQUIVALENCE_RUN_EVENT)
                && record.field_equals("fixture_count", "2")
                && record.field_equals("bit_exact_match_count", "2")
                && record.field_equals("skipped_count", "0")
        }));
    }

    #[cfg(feature = "semantic_equivalence_check")]
    #[test]
    fn stage3_semantic_equivalence_failed_trace_is_subscriber_captured() {
        let capture = TraceCapture::default();
        let subscriber = tracing_subscriber::registry()
            .with(LevelFilter::TRACE)
            .with(capture.clone());
        let graph = wave4_quant_graph_dense();
        let mut ir = finalized_ir_for_graph(&graph);
        ir.nodes
            .iter_mut()
            .find(|node| matches!(node.op, InferOp::DecodeToken { .. }))
            .expect("decode exists")
            .op = InferOp::DecodeToken {
            plan: DecodePlanId::new(99),
        };

        tracing::callsite::rebuild_interest_cache();
        tracing::subscriber::with_default(subscriber, || {
            tracing::callsite::rebuild_interest_cache();
            fixture_semantic_equivalence(&ir, &graph, &fixture_inputs())
                .expect_err("semantic divergence returns diagnostics");
            tracing::callsite::rebuild_interest_cache();
        });
        tracing::callsite::rebuild_interest_cache();

        assert!(capture.records().iter().any(|record| {
            record.level == "ERROR"
                && record.field_equals("event", STAGE3_SEMANTIC_EQUIVALENCE_FAILED_EVENT)
                && record.field_equals("n_failures", "2")
        }));
    }

    #[test]
    fn build_infer_ir_core_signature_takes_no_io_args() {
        fn assert_signature(
            _: fn(
                GbInferIRInputs<'_, FakeStaticBudget>,
            ) -> Result<GbInferIRProduct, GbInferIRStageFailure>,
        ) {
        }

        assert_signature(build_infer_ir_core::<FakeStaticBudget>);
    }

    #[test]
    fn build_infer_ir_core_is_pure_function() {
        let graph = wave4_quant_graph_dense();
        let first_wave = wave4_intermediate(&graph);
        let sites = reduction_sites_for_nodes(&first_wave.nodes, &graph);
        let budget = FakeStaticBudget::with_sites(hash(0x44), sites);

        let first =
            build_infer_ir_core(infer_inputs(&graph, &budget)).expect("first core run succeeds");
        let second =
            build_infer_ir_core(infer_inputs(&graph, &budget)).expect("second core run succeeds");

        assert_eq!(first, second);
        assert_eq!(
            canonicalize_report(&first.report).expect("first report canonicalizes"),
            canonicalize_report(&second.report).expect("second report canonicalizes")
        );
    }

    #[test]
    fn run_stage3_emits_report_and_writes_cache() {
        let graph = wave4_quant_graph_dense();
        let first_wave = wave4_intermediate(&graph);
        let sites = reduction_sites_for_nodes(&first_wave.nodes, &graph);
        let budget = FakeStaticBudget::with_sites(hash(0x44), sites);
        let inputs = infer_inputs(&graph, &budget);
        let report_dir = tempfile::tempdir().expect("report tempdir");
        let (_store_dir, store) = store();
        let cache = StageCache::new(&store);

        let product = run_stage3(
            inputs.clone(),
            PassEnvironment::new(&NoopResolver)
                .with_report_dir(report_dir.path())
                .with_stage_cache(&cache),
        )
        .expect("driver succeeds");

        let report_bytes =
            fs::read(report_dir.path().join("infer_ir.json")).expect("report emitted");
        let decoded: ReportEnvelope<report_schema::InferIrReportBody<GbInferIR>> =
            serde_json::from_slice(&report_bytes).expect("report decodes");
        assert_eq!(decoded.outcome, ReportOutcome::Passed);
        assert_eq!(decoded.report_self_hash, product.report.report_self_hash);
        assert!(matches!(
            get_stage3_success(
                &cache,
                &stage3_cache_key_material(&inputs).expect("k3 material")
            )
            .expect("cache lookup succeeds"),
            Some(Stage3CacheCell::InferIrSuccess { .. })
        ));
    }

    #[test]
    fn run_stage3_writes_stage_cache_failure_memo_on_failed() {
        let graph = wave4_quant_graph_dense();
        let first_wave = wave4_intermediate(&graph);
        let sites = reduction_sites_for_nodes(&first_wave.nodes, &graph);
        let budget = FakeStaticBudget::with_sites(hash(0x44), sites);
        let mut inputs = infer_inputs(&graph, &budget);
        inputs.static_budget_self_hash = hash(0xee);
        let report_dir = tempfile::tempdir().expect("report tempdir");
        let (_store_dir, store) = store();
        let cache = StageCache::new(&store);

        let failure = run_stage3(
            inputs.clone(),
            PassEnvironment::new(&NoopResolver)
                .with_report_dir(report_dir.path())
                .with_stage_cache(&cache),
        )
        .expect_err("driver rejects invalid static budget hash");

        assert_eq!(failure.kind, GbInferIRStageFailureKind::Rejected);
        assert!(failure.report.is_some());
        let report_bytes =
            fs::read(report_dir.path().join("infer_ir.json")).expect("failure report emitted");
        let decoded: ReportEnvelope<report_schema::InferIrReportBody<GbInferIR>> =
            serde_json::from_slice(&report_bytes).expect("failure report decodes");
        assert_eq!(decoded.outcome, ReportOutcome::Failed);
        assert!(matches!(
            get_stage3_failure_memo(
                &cache,
                &stage3_cache_key_material(&inputs).expect("k3 material")
            )
            .expect("failure memo lookup succeeds"),
            Some(Stage3CacheCell::FailureMemo { .. })
        ));
    }

    #[test]
    fn run_stage3_cache_hit_replays_with_audit_rewrap() {
        let graph = wave4_quant_graph_dense();
        let first_wave = wave4_intermediate(&graph);
        let sites = reduction_sites_for_nodes(&first_wave.nodes, &graph);
        let budget = FakeStaticBudget::with_sites(hash(0x44), sites);
        let inputs = infer_inputs(&graph, &budget);
        let report_dir = tempfile::tempdir().expect("report tempdir");
        let (_store_dir, store) = store();
        let cache = StageCache::new(&store);
        let env = PassEnvironment::new(&NoopResolver)
            .with_report_dir(report_dir.path())
            .with_stage_cache(&cache);

        let first = run_stage3(inputs.clone(), env).expect("first driver run succeeds");
        let first_report_bytes =
            fs::read(report_dir.path().join("infer_ir.json")).expect("first report emitted");
        let mut second_inputs = inputs.clone();
        second_inputs.audit_parents = InferIrAuditParents {
            policy_resolution_self_hash: hash(0xc1),
            compile_request_hash: hash(0xc2),
        };
        let second =
            run_stage3(second_inputs.clone(), env).expect("second driver run succeeds from cache");
        let second_report_bytes =
            fs::read(report_dir.path().join("infer_ir.json")).expect("second report emitted");

        assert_eq!(first.infer_ir, second.infer_ir);
        assert_eq!(first.infer_ir_self_hash, second.infer_ir_self_hash);
        assert_eq!(
            first
                .report
                .body
                .result
                .as_ref()
                .map(|result| &result.product),
            second
                .report
                .body
                .result
                .as_ref()
                .map(|result| &result.product)
        );
        assert_ne!(
            first.report.report_self_hash,
            second.report.report_self_hash
        );
        assert_ne!(first_report_bytes, second_report_bytes);
        assert_eq!(
            second
                .report
                .body
                .input_identity
                .policy_resolution_self_hash,
            second_inputs.audit_parents.policy_resolution_self_hash
        );
        assert_eq!(
            second.report.body.input_identity.compile_request_hash,
            second_inputs.audit_parents.compile_request_hash
        );
    }

    #[test]
    fn run_stage3_failure_memo_never_used_as_success() {
        let graph = wave4_quant_graph_dense();
        let first_wave = wave4_intermediate(&graph);
        let sites = reduction_sites_for_nodes(&first_wave.nodes, &graph);
        let budget = FakeStaticBudget::with_sites(hash(0x44), sites);
        let mut inputs = infer_inputs(&graph, &budget);
        inputs.static_budget_self_hash = hash(0xef);
        let report_dir = tempfile::tempdir().expect("report tempdir");
        let (_store_dir, store) = store();
        let cache = StageCache::new(&store);
        let env = PassEnvironment::new(&NoopResolver)
            .with_report_dir(report_dir.path())
            .with_stage_cache(&cache);

        let first = run_stage3(inputs.clone(), env).expect_err("first run writes failure memo");
        let second = run_stage3(inputs, env).expect_err("second run replays failure memo");

        assert_eq!(first.kind, GbInferIRStageFailureKind::Rejected);
        assert_eq!(second.kind, GbInferIRStageFailureKind::CacheHitFailureMemo);
    }

    #[test]
    fn semantic_equivalence_failed_diagnostic() {
        let diagnostic = infer_ir_semantic_equivalence_failed_diagnostic(2);

        assert!(matches!(
            diagnostic.code,
            ValidationCode::InferIrSemanticEquivalenceFailed { .. }
        ));
        assert!(matches!(
            diagnostic.detail,
            ValidationDetail::Field { ref field }
                if field.as_str() == "semantic_equivalence.fixture[2]"
        ));
    }

    #[test]
    fn semantic_equivalence_uses_named_numeric_boundaries_only() {
        let boundaries = SEMANTIC_EQUIVALENCE_NUMERIC_BOUNDARIES;

        assert!(boundaries.contains(&"residual_plan.activation_format"));
        assert!(boundaries.contains(&"norm_plan.output_format"));
        assert!(boundaries.iter().all(|boundary| !boundary.is_empty()));
        assert!(boundaries.iter().all(|boundary| !boundary.contains("mid")));
    }

    #[test]
    fn semantic_equivalence_reference_evaluators_match_fixture_trace() {
        let graph = wave4_quant_graph_dense();
        let ir = finalized_ir_for_graph(&graph);
        let input = fixture_input(5);

        assert_eq!(
            canonical::reference::eval_canonical_ir(&ir, &input),
            canonical::reference::eval_canonical_qg(&graph, &input)
        );
    }

    #[test]
    fn infer_ir_v1_round_trips_via_canonical_json() {
        let product = infer_ir_product_for_graph(&wave4_quant_graph_dense());
        let canonical = canonicalize_report(&product.report).expect("report canonicalizes");
        let decoded: ReportEnvelope<report_schema::InferIrReportBody<GbInferIR>> =
            serde_json::from_slice(&canonical).expect("canonical report decodes");

        assert_eq!(
            canonicalize_report(&decoded).expect("decoded report canonicalizes"),
            canonical
        );
        assert!(decoded.body.validate_semantics(decoded.outcome).is_ok());
    }

    #[test]
    fn infer_ir_v1_op_histogram_includes_zero_count_tags() {
        let product = infer_ir_product_for_graph(&wave4_quant_graph_dense());
        let result = product
            .report
            .body
            .result
            .as_ref()
            .expect("passed report has result");

        assert_eq!(
            result.op_histogram.len(),
            report_schema::INFER_OP_TAG_CANONICAL_ORDER.len()
        );
        assert_eq!(
            result
                .op_histogram
                .get(&report_schema::InferOpTag::SequenceRead),
            Some(&0)
        );
        assert_eq!(
            result.op_histogram.values().copied().sum::<u32>(),
            result.node_count
        );
    }

    #[test]
    fn infer_ir_v1_self_hash_round_trip() {
        let product = infer_ir_product_for_graph(&wave4_quant_graph_dense());

        assert!(round_trip_self_hash(&product.report).is_ok());
        assert_eq!(
            product
                .report
                .body
                .result
                .as_ref()
                .map(|result| result.infer_ir_self_hash),
            Some(product.infer_ir_self_hash)
        );
    }

    #[test]
    fn infer_ir_v1_iir_4_quant_graph_self_hash_chain() {
        let product = infer_ir_product_for_graph(&wave4_quant_graph_dense());
        let result = product
            .report
            .body
            .result
            .as_ref()
            .expect("passed report has result");

        assert_eq!(
            product.report.body.input_identity.quant_graph_self_hash,
            product.infer_ir.identity.quant_graph_self_hash
        );
        assert_eq!(
            result.product.identity.quant_graph_self_hash,
            product.infer_ir.identity.quant_graph_self_hash
        );
    }

    #[test]
    fn infer_ir_v1_input_identity_carries_audit_parents_in_envelope_only() {
        let product = infer_ir_product_for_graph(&wave4_quant_graph_dense());
        let value = serde_json::to_value(&product.report).expect("report serializes");
        let result = product
            .report
            .body
            .result
            .as_ref()
            .expect("passed report has result");
        let result_value = serde_json::to_value(result).expect("result serializes");

        assert!(
            value["input_identity"]
                .get("policy_resolution_self_hash")
                .is_some()
        );
        assert!(
            value["input_identity"]
                .get("compile_request_hash")
                .is_some()
        );
        assert_forbidden_keys_absent(
            &result_value,
            &["policy_resolution_self_hash", "compile_request_hash"],
        );
    }

    #[test]
    fn infer_ir_v1_op_histogram_canonical_lex_order() {
        let product = infer_ir_product_for_graph(&wave4_quant_graph_routed());
        let result = product
            .report
            .body
            .result
            .as_ref()
            .expect("passed report has result");
        let keys = result.op_histogram.keys().copied().collect::<Vec<_>>();

        assert_eq!(keys, report_schema::INFER_OP_TAG_CANONICAL_ORDER);
    }

    #[test]
    fn infer_ir_v1_validate_semantics_takes_outcome_param() {
        let product = infer_ir_product_for_graph(&wave4_quant_graph_dense());
        let mut body = product.report.body.clone();

        assert!(body.validate_semantics(ReportOutcome::Passed).is_ok());
        body.result = None;
        assert!(body.validate_semantics(ReportOutcome::Passed).is_err());
    }

    #[test]
    fn infer_ir_v1_serde_field_names_pinned() {
        let product = infer_ir_product_for_graph(&wave4_quant_graph_dense());
        let result = product
            .report
            .body
            .result
            .as_ref()
            .expect("passed report has result");
        let value = serde_json::to_value(result).expect("result serializes");
        let object = value.as_object().expect("result is an object");
        let keys = object.keys().map(String::as_str).collect::<Vec<_>>();

        assert_eq!(
            keys,
            vec![
                "anchor_count",
                "effect_class_histogram",
                "effect_count",
                "fixture_equivalence",
                "infer_ir_canonical_bytes_hash",
                "infer_ir_self_hash",
                "node_count",
                "op_histogram",
                "product",
                "token_input_count",
                "topological_order_hash",
                "value_count",
                "value_kind_histogram",
            ]
        );
    }

    #[test]
    fn infer_ir_self_hash_stable_across_two_regenerations() {
        let graph = wave4_quant_graph_routed();
        let left = finalized_ir_for_graph(&graph);
        let right = finalized_ir_for_graph(&graph);

        assert_eq!(
            infer_ir_self_hash(&left).expect("left hash"),
            infer_ir_self_hash(&right).expect("right hash")
        );
    }

    #[test]
    fn infer_ir_v1_round_trip_idempotent_under_canonicalize_twice() {
        let product = infer_ir_product_for_graph(&wave4_quant_graph_dense());
        let first = canonicalize_report(&product.report).expect("first canonicalizes");
        let decoded: ReportEnvelope<report_schema::InferIrReportBody<GbInferIR>> =
            serde_json::from_slice(&first).expect("canonical report decodes");
        let second = canonicalize_report(&decoded).expect("second canonicalizes");

        assert_eq!(first, second);
    }

    #[test]
    fn fixture_infer_ir_dense_toy0_self_hash_round_trip() {
        assert_passing_fixture_files("dense_toy0");
        let product = infer_ir_product_for_graph(&wave4_quant_graph_dense());
        maybe_dump_fixture_goldens("dense_toy0", &product);
        assert_fixture_golden_hashes("dense_toy0", &product);

        assert!(round_trip_self_hash(&product.report).is_ok());
        assert_eq!(
            product
                .report
                .body
                .result
                .as_ref()
                .map(|result| result.infer_ir_self_hash),
            Some(product.infer_ir_self_hash)
        );
    }

    #[test]
    fn fixture_infer_ir_routed_basic_self_consistency() {
        assert_passing_fixture_files("routed_basic");
        let graph = wave4_quant_graph_routed();
        let ir = finalized_ir_for_graph(&graph);
        let product = infer_ir_product_for_graph(&graph);
        maybe_dump_fixture_goldens("routed_basic", &product);
        assert_fixture_golden_hashes("routed_basic", &product);

        assert!(validate_infer_ir_self_consistency(&ir, &graph).is_empty());
        assert_eq!(ir.anchors.len(), ir.nodes.len());
    }

    #[test]
    fn fixture_infer_ir_mixed_topology_self_consistency() {
        assert_passing_fixture_files("mixed_topology");
        let graph = wave4_quant_graph_mixed_topology();
        let ir = finalized_ir_for_graph(&graph);
        let product = infer_ir_product_for_graph(&graph);
        maybe_dump_fixture_goldens("mixed_topology", &product);
        assert_fixture_golden_hashes("mixed_topology", &product);

        assert!(validate_infer_ir_self_consistency(&ir, &graph).is_empty());
        assert_eq!(infer_layers_count(&ir.nodes), 2);
    }

    #[test]
    fn fixture_infer_ir_every_reject_class_typed_diagnostic() {
        let cases = infer_ir_reject_fixtures();
        assert_eq!(cases.len(), 36);

        for case in cases {
            assert_reject_fixture(case);
        }
    }

    #[cfg(feature = "semantic_equivalence_check")]
    #[test]
    fn fixture_infer_ir_fixture_semantic_equivalence_bit_exact() {
        for graph in [wave4_quant_graph_dense(), wave4_quant_graph_routed()] {
            let ir = finalized_ir_for_graph(&graph);
            assert_eq!(
                fixture_semantic_equivalence(&ir, &graph, &fixture_inputs())
                    .expect("semantic equivalence passes"),
                FixtureEquivalenceResult::VerifiedFixtureBitExact
            );
        }
    }

    #[cfg(not(feature = "semantic_equivalence_check"))]
    #[test]
    fn fixture_infer_ir_fixture_semantic_equivalence_bit_exact() {
        let graph = wave4_quant_graph_dense();
        let ir = finalized_ir_for_graph(&graph);

        assert_eq!(
            fixture_semantic_equivalence(&ir, &graph, &fixture_inputs())
                .expect("feature-disabled semantic equivalence skips"),
            FixtureEquivalenceResult::Skipped {
                reason: report_schema::FixtureEquivalenceSkippedReason::FeatureFlagDisabled
            }
        );
    }

    #[test]
    fn fixture_infer_ir_cache_hit_replays_with_audit_rewrap() {
        let product = infer_ir_product_for_graph(&wave4_quant_graph_dense());
        let rewrapped = crate::stage_cache::rewrap_stage3_cached_report_audit_parents(
            &product.report,
            InferIrAuditParents {
                policy_resolution_self_hash: hash(0xd1),
                compile_request_hash: hash(0xd2),
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
        assert_ne!(product.report.report_self_hash, rewrapped.report_self_hash);
        assert_eq!(
            rewrapped.body.input_identity.policy_resolution_self_hash,
            hash(0xd1)
        );
    }

    #[cfg(feature = "semantic_equivalence_check")]
    #[test]
    fn review_f_b5_export_packet_goldens() {
        let Some(packet_dir) = std::env::var_os("GBF_REVIEW_F_B5_PACKET_DIR") else {
            return;
        };
        let packet_dir = PathBuf::from(packet_dir);
        let golden_dir = packet_dir.join("golden");
        fs::create_dir_all(&golden_dir).expect("review packet golden dir exists");

        fs::write(
            golden_dir.join("driver_evidence.toml"),
            review_f_b5_driver_evidence_toml(),
        )
        .expect("driver evidence writes");

        let fixtures = [
            ("dense_toy0", wave4_quant_graph_dense()),
            ("routed_basic", wave4_quant_graph_routed()),
            ("mixed_topology", wave4_quant_graph_mixed_topology()),
        ];
        let mut bit_exact = review_f_b5_bit_exact_header();

        for (name, graph) in fixtures {
            export_review_f_b5_fixture(&packet_dir, name, &graph, &mut bit_exact);
        }

        fs::write(packet_dir.join("bit_exact_equivalence.toml"), bit_exact)
            .expect("BitExact equivalence golden writes");
    }

    #[test]
    fn reject_infer_ir_embedding_not_unique() {
        assert_reject_fixture_by_slug("infer_ir_embedding_not_unique");
    }

    #[test]
    fn reject_infer_ir_decode_not_unique() {
        assert_reject_fixture_by_slug("infer_ir_decode_not_unique");
    }

    #[test]
    fn reject_infer_ir_classify_not_unique() {
        assert_reject_fixture_by_slug("infer_ir_classify_not_unique");
    }

    #[test]
    fn reject_infer_ir_expert_coverage_mismatch() {
        assert_reject_fixture_by_slug("infer_ir_expert_coverage_mismatch");
    }

    #[test]
    fn reject_infer_ir_route_coverage_mismatch() {
        assert_reject_fixture_by_slug("infer_ir_route_coverage_mismatch");
    }

    #[test]
    fn reject_infer_ir_semantic_checkpoint_emitted_here() {
        assert_reject_fixture_by_slug("infer_ir_semantic_checkpoint_emitted_here");
    }

    #[test]
    fn reject_infer_ir_effect_chain_not_linear() {
        assert_reject_fixture_by_slug("infer_ir_effect_chain_not_linear");
    }

    #[test]
    fn reject_infer_ir_effect_id_edge_token_violation() {
        assert_reject_fixture_by_slug("infer_ir_effect_id_edge_token_violation");
    }

    #[test]
    fn reject_infer_ir_topological_order_mismatch() {
        assert_reject_fixture_by_slug("infer_ir_topological_order_mismatch");
    }

    #[test]
    fn reject_infer_ir_value_format_mismatch() {
        assert_reject_fixture_by_slug("infer_ir_value_format_mismatch");
    }

    #[test]
    fn reject_infer_ir_norm_format_mismatch() {
        assert_reject_fixture_by_slug("infer_ir_norm_format_mismatch");
    }

    #[test]
    fn reject_infer_ir_decode_rng_binding_mismatch() {
        assert_reject_fixture_by_slug("infer_ir_decode_rng_binding_mismatch");
    }

    #[test]
    fn reject_infer_ir_semantic_equivalence_failed() {
        assert_reject_fixture_by_slug("infer_ir_semantic_equivalence_failed");
    }

    #[test]
    fn reject_infer_ir_cycle_detected() {
        assert_reject_fixture_by_slug("infer_ir_cycle_detected");
    }

    #[test]
    fn reject_infer_ir_unreachable_node() {
        assert_reject_fixture_by_slug("infer_ir_unreachable_node");
    }

    #[test]
    fn reject_infer_ir_disconnected_component() {
        assert_reject_fixture_by_slug("infer_ir_disconnected_component");
    }

    #[test]
    fn reject_infer_ir_forbidden_storage_metadata() {
        assert_reject_fixture_by_slug("infer_ir_forbidden_storage_metadata");
    }

    #[test]
    fn reject_infer_ir_non_v1_router_semantics() {
        assert_reject_fixture_by_slug("infer_ir_non_v1_router_semantics");
    }

    #[test]
    fn reject_infer_ir_semantic_anchor_missing() {
        assert_reject_fixture_by_slug("infer_ir_semantic_anchor_missing");
    }

    #[test]
    fn reject_infer_ir_ffn_activation_missing() {
        assert_reject_fixture_by_slug("infer_ir_ffn_activation_missing");
    }

    #[test]
    fn reject_infer_ir_expert_selection_missing() {
        assert_reject_fixture_by_slug("infer_ir_expert_selection_missing");
    }

    #[test]
    fn reject_infer_ir_gate_weight_not_consumed() {
        assert_reject_fixture_by_slug("infer_ir_gate_weight_not_consumed");
    }

    #[test]
    fn reject_infer_ir_token_ingress_ambiguous() {
        assert_reject_fixture_by_slug("infer_ir_token_ingress_ambiguous");
    }

    #[test]
    fn reject_infer_ir_reduction_site_missing() {
        assert_reject_fixture_by_slug("infer_ir_reduction_site_missing");
    }

    #[test]
    fn reject_infer_ir_op_histogram_total_mismatch() {
        assert_reject_fixture_by_slug("infer_ir_op_histogram_total_mismatch");
    }

    #[test]
    fn reject_infer_ir_fault_boundary_emitted_v1_forbidden() {
        assert_reject_fixture_by_slug("infer_ir_fault_boundary_emitted_v1_forbidden");
    }

    #[test]
    fn reject_infer_ir_op_signature_mismatch() {
        assert_reject_fixture_by_slug("infer_ir_op_signature_mismatch");
    }

    #[test]
    fn reject_infer_ir_router_score_orphaned() {
        assert_reject_fixture_by_slug("infer_ir_router_score_orphaned");
    }

    #[test]
    fn reject_infer_ir_sequence_slot_coverage_mismatch() {
        assert_reject_fixture_by_slug("infer_ir_sequence_slot_coverage_mismatch");
    }

    #[test]
    fn reject_infer_ir_unexpected_rng_effect_on_pure_op() {
        assert_reject_fixture_by_slug("infer_ir_unexpected_rng_effect_on_pure_op");
    }

    #[test]
    fn reject_infer_ir_residual_boundary_mismatch() {
        assert_reject_fixture_by_slug("infer_ir_residual_boundary_mismatch");
    }

    #[test]
    fn reject_infer_ir_router_matvec_missing_for_routed_layer() {
        assert_reject_fixture_by_slug("infer_ir_router_matvec_missing_for_routed_layer");
    }

    #[test]
    fn reject_infer_ir_router_present_for_dense_layer() {
        assert_reject_fixture_by_slug("infer_ir_router_present_for_dense_layer");
    }

    #[test]
    fn reject_infer_ir_input_token_value_id_mismatch() {
        assert_reject_fixture_by_slug("infer_ir_input_token_value_id_mismatch");
    }

    #[test]
    fn reject_infer_ir_sequence_state_next_orphaned() {
        assert_reject_fixture_by_slug("infer_ir_sequence_state_next_orphaned");
    }

    #[test]
    fn reject_infer_ir_sequence_semantics_unsupported_v1() {
        assert_reject_fixture_by_slug("infer_ir_sequence_semantics_unsupported_v1");
    }

    #[test]
    fn iir_self_consistency_collects_all_diagnostics_no_short_circuit() {
        let graph = wave4_quant_graph_routed();
        let mut ir = finalized_ir_for_graph(&graph);
        ir.nodes
            .iter_mut()
            .find(|node| matches!(node.op, InferOp::Embedding { .. }))
            .expect("embedding exists")
            .inputs
            .clear();
        ir.values.push(ValueDecl {
            value_id: ValueId::new(999),
            kind: ValueKind::RouterScore,
            format: ValueFormat::ExactAccumulator,
            layout: vector_layout(ValueAxis::Expert),
        });

        let diagnostics = validate_infer_ir_self_consistency(&ir, &graph);

        assert!(diagnostics.len() >= 2, "{diagnostics:#?}");
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.severity == gbf_policy::DiagnosticSeverity::Hard)
        );
    }

    #[test]
    fn bind_infer_ir_provenance_missing_producer_is_hard_diagnostic() {
        let graph = wave4_quant_graph_dense();
        let wave4 = wave4_intermediate(&graph);
        let mut values = wave4.values.values.clone();
        values.push(ValueDecl {
            value_id: ValueId::new(999),
            kind: ValueKind::RouterScore,
            format: ValueFormat::ExactAccumulator,
            layout: vector_layout(ValueAxis::Expert),
        });

        let diagnostics = bind_infer_ir_provenance(
            &wave4.nodes,
            &values,
            &wave4.effects.effects,
            &[wave4.token_input],
            &graph,
        )
        .expect_err("missing value producer rejects");

        assert!(diagnostics.iter().any(|diagnostic| matches!(
            diagnostic.code,
            ValidationCode::InferIrValueProducerMissing { value_id: 999 }
        )));
    }

    #[test]
    fn iir_self_consistency_sc10_decode_plan_and_rng() {
        let graph = wave4_quant_graph_dense();
        let mut ir = finalized_ir_for_graph(&graph);
        ir.nodes
            .iter_mut()
            .find(|node| matches!(node.op, InferOp::DecodeToken { .. }))
            .expect("decode exists")
            .op = InferOp::DecodeToken {
            plan: DecodePlanId::new(99),
        };

        let diagnostics = validate_infer_ir_self_consistency(&ir, &graph);

        assert!(diagnostics.iter().any(|diagnostic| matches!(
            diagnostic.code,
            ValidationCode::InferIrDecodePlanMismatch { .. }
        )));

        let mut graph = wave4_quant_graph_dense();
        graph.decode_spec = wave4_decode_spec(true);
        let mut ir = finalized_ir_for_graph(&graph);
        ir.nodes
            .iter_mut()
            .find(|node| matches!(node.op, InferOp::Classify))
            .expect("classify exists")
            .effects_in
            .push(EffectId::new(0));

        let diagnostics = validate_infer_ir_self_consistency(&ir, &graph);

        assert!(diagnostics.iter().any(|diagnostic| matches!(
            diagnostic.code,
            ValidationCode::InferIrUnexpectedRngEffectOnPureOp { .. }
        )));
    }

    #[test]
    fn pure_ops_must_not_touch_decode_rng_chain() {
        let mut graph = wave4_quant_graph_dense();
        graph.decode_spec = wave4_decode_spec(true);
        let mut ir = finalized_ir_for_graph(&graph);
        ir.nodes
            .iter_mut()
            .find(|node| matches!(node.op, InferOp::Classify))
            .expect("classify exists")
            .effects_in
            .push(EffectId::new(0));

        let diagnostics = validate_infer_ir_self_consistency(&ir, &graph);

        assert!(diagnostics.iter().any(|diagnostic| matches!(
            diagnostic.code,
            ValidationCode::InferIrUnexpectedRngEffectOnPureOp { .. }
        )));
    }

    #[test]
    fn iir_self_consistency_sc17_op_histogram_total() {
        let graph = wave4_quant_graph_dense();
        let ir = finalized_ir_for_graph(&graph);
        let mut histogram = infer_op_histogram(&ir.nodes);

        assert!(infer_op_histogram_total_matches_node_count(
            &histogram,
            ir.nodes.len() as u32
        ));
        histogram.remove(&InferOpTag::SequenceRead);
        assert!(!infer_op_histogram_total_matches_node_count(
            &histogram,
            ir.nodes.len() as u32
        ));
    }

    #[test]
    fn canonical_sort_idempotent_under_repeated_application() {
        let graph = wave4_quant_graph_routed();
        let wave4 = wave4_intermediate(&graph);
        let (_identity, once) =
            bind_canonical_sort(wave4.identity, wave4.nodes, &graph).expect("first sort binds");
        let (_identity, twice) =
            bind_canonical_sort(wave4.identity, once.clone(), &graph).expect("second sort binds");

        assert_eq!(once, twice);
    }

    #[test]
    fn residual_boundary_mismatch_is_typed_diagnostic() {
        let graph = wave4_quant_graph_dense();
        let mut ir = finalized_ir_for_graph(&graph);
        let residual_output = ir
            .nodes
            .iter()
            .find(|node| matches!(node.op, InferOp::CombineResidual { .. }))
            .expect("residual exists")
            .outputs[0];
        ir.values
            .iter_mut()
            .find(|value| value.value_id == residual_output)
            .expect("residual output value exists")
            .format = ValueFormat::ExactAccumulator;

        let diagnostics = validate_infer_ir_self_consistency(&ir, &graph);

        assert!(diagnostics.iter().any(|diagnostic| matches!(
            diagnostic.code,
            ValidationCode::InferIrResidualBoundaryMismatch { .. }
        )));
    }

    #[test]
    fn fault_boundary_emitted_is_typed_diagnostic() {
        let graph = wave4_quant_graph_dense();
        let mut ir = finalized_ir_for_graph(&graph);
        ir.effects.push(EffectDecl::new(
            EffectId::new(99),
            EffectClass::FaultBoundary,
        ));

        let diagnostics = validate_infer_ir_self_consistency(&ir, &graph);

        assert!(diagnostics.iter().any(|diagnostic| matches!(
            diagnostic.code,
            ValidationCode::InferIrFaultBoundaryEmittedV1Forbidden { .. }
        )));
    }

    #[test]
    fn token_ingress_unambiguous_is_typed_diagnostic() {
        let graph = wave4_quant_graph_dense();
        let mut ir = finalized_ir_for_graph(&graph);
        ir.token_inputs.push(
            TokenInput::new(
                TokenInputId::new(1),
                ValueId::new(0),
                BTreeSet::from([TokenIngressMode::Prompt]),
            )
            .expect("token input is valid"),
        );

        let diagnostics = validate_infer_ir_self_consistency(&ir, &graph);

        assert!(diagnostics.iter().any(|diagnostic| matches!(
            diagnostic.code,
            ValidationCode::InferIrTokenIngressAmbiguous { .. }
        )));
    }

    #[test]
    fn iir_self_consistency_sc5_value_format_consistency() {
        let graph = wave4_quant_graph_dense();
        let mut ir = finalized_ir_for_graph(&graph);
        let decoded = ir
            .nodes
            .iter()
            .find(|node| matches!(node.op, InferOp::DecodeToken { .. }))
            .expect("decode exists")
            .outputs[0];
        ir.values
            .iter_mut()
            .find(|value| value.value_id == decoded)
            .expect("decoded value exists")
            .format = ValueFormat::ExactAccumulator;

        let diagnostics = validate_infer_ir_self_consistency(&ir, &graph);

        assert!(diagnostics.iter().any(|diagnostic| matches!(
            diagnostic.code,
            ValidationCode::InferIrValueFormatMismatch { .. }
        )));
    }

    #[test]
    fn iir_self_consistency_sc6_norm_format_chain() {
        let graph = wave4_quant_graph_dense();
        let mut ir = finalized_ir_for_graph(&graph);
        let norm_output = ir
            .nodes
            .iter()
            .find(|node| matches!(node.op, InferOp::Norm { .. }))
            .expect("norm exists")
            .outputs[0];
        ir.values
            .iter_mut()
            .find(|value| value.value_id == norm_output)
            .expect("norm output exists")
            .format = ValueFormat::ExactAccumulator;

        let diagnostics = validate_infer_ir_self_consistency(&ir, &graph);

        assert!(diagnostics.iter().any(|diagnostic| matches!(
            diagnostic.code,
            ValidationCode::InferIrNormFormatMismatch { .. }
        )));
    }

    #[test]
    fn iir_self_consistency_sc7_expert_section_role_match() {
        let mut graph = wave4_quant_graph_dense();
        let ir = finalized_ir_for_graph(&graph);
        graph.expert_sections.clear();

        let diagnostics = validate_infer_ir_self_consistency(&ir, &graph);

        assert!(diagnostics.iter().any(|diagnostic| matches!(
            diagnostic.code,
            ValidationCode::InferIrExpertSectionRoleMismatch { .. }
        )));
    }

    #[test]
    fn iir_self_consistency_sc8_route_top1_semantics() {
        let mut graph = wave4_quant_graph_routed();
        let ir = finalized_ir_for_graph(&graph);
        graph.routing_table = None;

        let diagnostics = validate_infer_ir_self_consistency(&ir, &graph);

        assert!(diagnostics.iter().any(|diagnostic| matches!(
            diagnostic.code,
            ValidationCode::InferIrNonV1RouterSemantics { layer }
                if layer == LayerId::new(0)
        )));
    }

    #[test]
    fn iir_self_consistency_sc9_dense_routed_shape_full() {
        let graph = wave4_quant_graph_routed();
        let mut ir = finalized_ir_for_graph(&graph);
        let removed = ir
            .nodes
            .iter()
            .position(|node| {
                matches!(
                    node.op,
                    InferOp::ExpertMatVec {
                        layer,
                        expert,
                        slot: ExpertWeightSlot::FfnDown,
                    } if layer == LayerId::new(0) && expert == ExpertId::new(1)
                )
            })
            .expect("expert down node exists");
        ir.nodes.remove(removed);

        let diagnostics = validate_infer_ir_self_consistency(&ir, &graph);

        assert!(diagnostics.iter().any(|diagnostic| matches!(
            diagnostic.code,
            ValidationCode::InferIrDenseRoutedShapeMismatch { layer }
                if layer == LayerId::new(0)
        )));
    }

    #[test]
    fn iir_self_consistency_sc10a_sequence_slot_coverage_v1() {
        let mut graph = wave4_quant_graph_dense();
        let ir = finalized_ir_for_graph(&graph);
        graph.sequence_semantics = SequenceSemanticsSpec {
            kind: SequenceSemanticsKind::Identity,
            state_slots: BTreeMap::from([(
                LayerId::new(0),
                vec![SequenceStateSlot {
                    slot_id: 0,
                    width_bytes: 8,
                }],
            )]),
        };

        let diagnostics = validate_infer_ir_self_consistency(&ir, &graph);

        assert!(diagnostics.iter().any(|diagnostic| matches!(
            diagnostic.code,
            ValidationCode::InferIrSequenceSlotCoverageMismatch { layer }
                if layer == LayerId::new(0)
        )));
    }

    #[test]
    fn infer_op_enum_thirteen_variants_closed() {
        let ops = all_infer_ops();
        let tags = ops.iter().map(|op| op.tag()).collect::<Vec<_>>();

        assert_eq!(ops.len(), 13);
        assert_eq!(tags, INFER_OP_TAG_CANONICAL_ORDER);
    }

    #[test]
    fn infer_op_tag_canonical_lex_order_pinned() {
        let observed = INFER_OP_TAG_CANONICAL_ORDER
            .iter()
            .map(|tag| format!("{tag:?}"))
            .collect::<Vec<_>>();
        let mut sorted = observed.clone();
        sorted.sort();

        assert_eq!(observed, sorted);
    }

    #[test]
    fn residual_site_two_variants_closed() {
        let sites = [ResidualSite::PostFfn, ResidualSite::PostSequence];
        let names = sites
            .iter()
            .map(|site| format!("{site:?}"))
            .collect::<Vec<_>>();

        assert_eq!(sites.len(), 2);
        assert_eq!(names, vec!["PostFfn", "PostSequence"]);
    }

    #[test]
    fn value_kind_fifteen_variants_closed() {
        assert_eq!(VALUE_KIND_CANONICAL_ORDER.len(), 15);
        assert_eq!(
            VALUE_KIND_CANONICAL_ORDER
                .iter()
                .map(|kind| format!("{kind:?}"))
                .collect::<Vec<_>>(),
            vec![
                "Activation",
                "DecodedToken",
                "EmbeddingOutput",
                "ExpertCandidate",
                "ExpertIntermediate",
                "ExpertOutput",
                "GateWeight",
                "InputToken",
                "LogitVector",
                "NormalizedActivation",
                "RouterDecision",
                "RouterScore",
                "SequenceBlockOutput",
                "SequenceStateNext",
                "SequenceStateRead"
            ]
        );
    }

    #[test]
    fn value_format_four_variants_closed() {
        let formats = [
            ValueFormat::Quant {
                format: QuantFormat::Q8_8,
            },
            ValueFormat::ExactAccumulator,
            ValueFormat::TokenIdDomain { vocab_size: 257 },
            ValueFormat::ExpertIdDomain { n_experts: 3 },
        ];
        let kinds = formats
            .iter()
            .map(|format| serde_json::to_value(format).expect("format serializes")["kind"].clone())
            .collect::<Vec<_>>();

        assert_eq!(
            kinds,
            vec![
                serde_json::json!("Quant"),
                serde_json::json!("ExactAccumulator"),
                serde_json::json!("TokenIdDomain"),
                serde_json::json!("ExpertIdDomain")
            ]
        );
    }

    #[test]
    fn expert_matvec_per_slot_compile_check() {
        let InferOp::ExpertMatVec {
            layer,
            expert,
            slot,
        } = expert_matvec_op()
        else {
            panic!("expected ExpertMatVec");
        };

        assert_eq!(layer, LayerId::new(0));
        assert_eq!(expert, ExpertId::new(1));
        assert_eq!(slot, ExpertWeightSlot::FfnUp);
    }

    #[test]
    fn router_matvec_separate_from_route_top1() {
        let ops = all_infer_ops();
        assert!(
            ops.iter()
                .any(|op| matches!(op, InferOp::RouterMatVec { .. }))
        );
        assert!(ops.iter().any(|op| matches!(op, InferOp::RouteTop1 { .. })));
    }

    #[test]
    fn select_expert_top1_present() {
        assert!(
            all_infer_ops()
                .iter()
                .any(|op| matches!(op, InferOp::SelectExpertTop1 { .. }))
        );
    }

    #[test]
    fn ffn_activation_present() {
        assert!(
            all_infer_ops()
                .iter()
                .any(|op| matches!(op, InferOp::FfnActivation { .. }))
        );
    }

    #[test]
    fn sequence_step_per_layer_no_slot() {
        let op = InferOp::SequenceStep {
            layer: LayerId::new(2),
        };
        let InferOp::SequenceStep { layer } = op else {
            panic!("expected SequenceStep");
        };

        assert_eq!(layer, LayerId::new(2));
    }

    #[test]
    fn combine_residual_carries_layer_option_and_site() {
        let op = InferOp::CombineResidual {
            layer: Some(LayerId::new(1)),
            site: ResidualSite::PostFfn,
        };
        let InferOp::CombineResidual { layer, site } = op else {
            panic!("expected CombineResidual");
        };

        assert_eq!(layer, Some(LayerId::new(1)));
        assert_eq!(site, ResidualSite::PostFfn);
    }

    #[test]
    fn gb_node_carries_optional_reduction_site_id() {
        let node = GbNode {
            node_id: NodeId::new(0),
            op: InferOp::Classify,
            inputs: Vec::new(),
            effects_in: Vec::new(),
            outputs: Vec::new(),
            effects_out: Vec::new(),
            reduction_site: Some(ReductionSiteId("classify.logits".to_owned())),
        };

        assert_eq!(
            node.reduction_site,
            Some(ReductionSiteId("classify.logits".to_owned()))
        );
    }

    #[test]
    fn value_format_exact_accumulator_not_in_quant_format() {
        let exact = serde_json::to_value(ValueFormat::ExactAccumulator).expect("serializes");
        let quant = serde_json::to_value(ValueFormat::Quant {
            format: QuantFormat::Q8_8,
        })
        .expect("serializes");

        assert_eq!(exact["kind"], serde_json::json!("ExactAccumulator"));
        assert_ne!(
            quant["format"]["kind"],
            serde_json::json!("ExactAccumulator")
        );
    }

    #[test]
    fn value_format_token_id_domain_carries_vocab_size_not_width() {
        let value =
            serde_json::to_value(ValueFormat::TokenIdDomain { vocab_size: 257 }).expect("json");

        assert_eq!(value["vocab_size"], serde_json::json!(257));
        assert!(value.get("width").is_none());
        assert!(value.get("bits").is_none());
    }

    #[test]
    fn value_format_expert_id_domain_carries_n_experts_not_width() {
        let value =
            serde_json::to_value(ValueFormat::ExpertIdDomain { n_experts: 3 }).expect("json");

        assert_eq!(value["n_experts"], serde_json::json!(3));
        assert!(value.get("width").is_none());
        assert!(value.get("bits").is_none());
    }

    #[test]
    fn value_format_no_unit_variant() {
        let encoded = serde_json::to_value([
            ValueFormat::ExactAccumulator,
            ValueFormat::TokenIdDomain { vocab_size: 1 },
            ValueFormat::ExpertIdDomain { n_experts: 1 },
        ])
        .expect("formats serialize");

        assert_forbidden_keys_absent(&encoded, &["Unit"]);
        assert!(!encoded.to_string().contains("Unit"));
    }

    #[test]
    fn value_decl_does_not_inline_provenance_compile_check() {
        let ValueDecl {
            value_id,
            kind,
            format,
            layout,
        } = value_decl();
        let value = serde_json::to_value(ValueDecl {
            value_id,
            kind,
            format,
            layout,
        })
        .expect("value decl serializes");

        assert_forbidden_keys_absent(&value, &["provenance", "producer"]);
    }

    #[test]
    fn value_layout_no_storage_fields_compile_check() {
        let value = serde_json::to_value(ValueLayout {
            shape: vec![ValueAxis::Token, ValueAxis::Model],
        })
        .expect("layout serializes");

        assert_forbidden_keys_absent(
            &value,
            &[
                "buffer",
                "buffer_id",
                "page",
                "page_id",
                "arena",
                "arena_id",
                "tile",
                "tile_size",
                "storage",
                "address",
            ],
        );
    }

    #[test]
    fn infer_op_serde_round_trip_each_variant() {
        for op in all_infer_ops() {
            let encoded = serde_json::to_string(&op).expect("op serializes");
            let decoded: InferOp = serde_json::from_str(&encoded).expect("op deserializes");
            assert_eq!(decoded, op);
        }
    }

    #[test]
    fn value_kind_tag_canonical_order_for_histogram() {
        let observed = VALUE_KIND_CANONICAL_ORDER
            .iter()
            .map(|kind| format!("{kind:?}"))
            .collect::<Vec<_>>();
        let mut sorted = observed.clone();
        sorted.sort();

        assert_eq!(observed, sorted);
    }

    #[test]
    fn effect_id_is_edge_token_not_class_instance() {
        let class = EffectClass::SequenceState {
            slot: StateSlotId::new(0),
        };
        let before = EffectDecl::new(EffectId::new(0), class);
        let after = EffectDecl::new(EffectId::new(1), class);
        let node = GbNode {
            node_id: NodeId::new(0),
            op: InferOp::SequenceWrite {
                slot: StateSlotId::new(0),
            },
            inputs: vec![ValueId::new(0)],
            effects_in: vec![before.effect_id],
            outputs: Vec::new(),
            effects_out: vec![after.effect_id],
            reduction_site: None,
        };

        assert_eq!(before.class, after.class);
        assert_ne!(before.effect_id, after.effect_id);
        assert_eq!(node.effects_in, vec![EffectId::new(0)]);
        assert_eq!(node.effects_out, vec![EffectId::new(1)]);
    }

    #[test]
    fn effect_class_three_variants_closed() {
        let classes = [
            EffectClass::FaultBoundary,
            EffectClass::Rng {
                slot: RngSlot::Decode,
            },
            EffectClass::SequenceState {
                slot: StateSlotId::new(0),
            },
        ];
        let tags = classes.iter().map(|class| class.tag()).collect::<Vec<_>>();

        assert_eq!(classes.len(), 3);
        assert_eq!(tags, EFFECT_CLASS_TAG_CANONICAL_ORDER);
    }

    #[test]
    fn effect_class_excludes_semantic_checkpoint() {
        let encoded = serde_json::to_string(&[
            EffectClass::FaultBoundary,
            EffectClass::Rng {
                slot: RngSlot::Decode,
            },
            EffectClass::SequenceState {
                slot: StateSlotId::new(0),
            },
        ])
        .expect("effect classes serialize");

        assert!(!encoded.contains("SemanticCheckpoint"));
        assert!(!encoded.contains("TraceProbe"));
    }

    #[test]
    fn fault_boundary_reserved_but_never_emitted_in_v1() {
        let result = GbInferIR::new(
            identity(),
            vec![token_input()],
            Vec::new(),
            Vec::new(),
            vec![EffectDecl::new(
                EffectId::new(0),
                EffectClass::FaultBoundary,
            )],
            InferIrProvenance::default(),
            NodeAnchorMap::new(),
        );

        assert!(matches!(
            result,
            Err(GbInferIrTypeError::FaultBoundaryReserved {
                effect_id: Some(id)
            }) if id == EffectId::new(0)
        ));
    }

    #[test]
    fn fault_boundary_provenance_rejected_in_v1() {
        let result = GbInferIR::new(
            identity(),
            vec![token_input()],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            InferIrProvenance {
                effects: BTreeMap::from([(
                    EffectId::new(0),
                    EffectProvenance::ExternalRoot {
                        class: EffectClass::FaultBoundary,
                    },
                )]),
                ..InferIrProvenance::default()
            },
            NodeAnchorMap::new(),
        );

        assert!(matches!(
            result,
            Err(GbInferIrTypeError::FaultBoundaryReserved { effect_id: None })
        ));
    }

    #[test]
    fn effect_provenance_two_variants() {
        let root = EffectProvenance::ExternalRoot {
            class: EffectClass::Rng {
                slot: RngSlot::Decode,
            },
        };
        let output = EffectProvenance::NodeOutput {
            node: NodeId::new(0),
            class: EffectClass::SequenceState {
                slot: StateSlotId::new(1),
            },
        };

        assert_eq!(
            serde_json::to_value(root).expect("root serializes")["kind"],
            serde_json::json!("ExternalRoot")
        );
        assert_eq!(
            serde_json::to_value(output).expect("output serializes")["kind"],
            serde_json::json!("NodeOutput")
        );
    }

    #[test]
    fn rng_slot_decode_only_in_v1() {
        assert_eq!(RNG_SLOT_CANONICAL_ORDER, [RngSlot::Decode]);
        assert_eq!(
            serde_json::to_value(RngSlot::Decode).expect("rng slot serializes")["kind"],
            serde_json::json!("Decode")
        );
    }

    #[test]
    fn effect_class_tag_canonical_order_for_histogram() {
        let observed = EFFECT_CLASS_TAG_CANONICAL_ORDER
            .iter()
            .map(|tag| format!("{tag:?}"))
            .collect::<Vec<_>>();
        let mut sorted = observed.clone();
        sorted.sort();

        assert_eq!(observed, sorted);
    }

    #[test]
    fn infer_ir_provenance_three_maps_no_inline() {
        let ir = gb_infer_ir_fixture().expect("IR fixture builds");
        let value = serde_json::to_value(&ir).expect("IR serializes");
        let provenance = value
            .get("provenance")
            .expect("IR carries provenance")
            .as_object()
            .expect("provenance is an object");

        assert_eq!(
            provenance.keys().map(String::as_str).collect::<Vec<_>>(),
            vec!["effects", "nodes", "values"]
        );
        assert_forbidden_keys_absent(&value["nodes"], &["provenance"]);
        assert_forbidden_keys_absent(&value["values"], &["provenance", "producer"]);
        assert_forbidden_keys_absent(&value["effects"], &["provenance"]);
    }

    #[test]
    fn infer_ir_provenance_btreemap_canonical_order() {
        let provenance = InferIrProvenance {
            nodes: BTreeMap::from([
                (NodeId::new(2), QuantGraphEntityRef::ClassifyHead),
                (NodeId::new(1), QuantGraphEntityRef::Embedding),
            ]),
            values: BTreeMap::from([
                (
                    ValueId::new(2),
                    ValueProducerRef::Node {
                        node: NodeId::new(2),
                    },
                ),
                (
                    ValueId::new(1),
                    ValueProducerRef::External {
                        token_input: TokenInputId::new(0),
                    },
                ),
            ]),
            effects: BTreeMap::from([
                (
                    EffectId::new(2),
                    EffectProvenance::NodeOutput {
                        node: NodeId::new(2),
                        class: EffectClass::Rng {
                            slot: RngSlot::Decode,
                        },
                    },
                ),
                (
                    EffectId::new(1),
                    EffectProvenance::ExternalRoot {
                        class: EffectClass::Rng {
                            slot: RngSlot::Decode,
                        },
                    },
                ),
            ]),
        };

        assert_eq!(
            provenance.nodes.keys().copied().collect::<Vec<_>>(),
            vec![NodeId::new(1), NodeId::new(2)]
        );
        assert_eq!(
            provenance.values.keys().copied().collect::<Vec<_>>(),
            vec![ValueId::new(1), ValueId::new(2)]
        );
        assert_eq!(
            provenance.effects.keys().copied().collect::<Vec<_>>(),
            vec![EffectId::new(1), EffectId::new(2)]
        );
    }

    #[test]
    fn infer_ir_provenance_totality_rejects_missing_maps() {
        let result = GbInferIR::new(
            identity(),
            vec![token_input()],
            vec![embedding_node()],
            vec![value_decl(), embedding_value_decl()],
            Vec::new(),
            InferIrProvenance::default(),
            NodeAnchorMap::from([(NodeId::new(0), semantic_anchor(8))]),
        );

        assert!(matches!(
            result,
            Err(GbInferIrTypeError::NodeProvenanceMissing { node_id })
                if node_id == NodeId::new(0)
        ));
    }

    #[test]
    fn quant_graph_entity_ref_enum_closed_variants_pinned() {
        let variants = all_quant_graph_entity_refs();
        let kinds = variants
            .iter()
            .map(|variant| {
                serde_json::to_value(variant).expect("variant serializes")["kind"].clone()
            })
            .collect::<Vec<_>>();

        assert_eq!(variants.len(), 15);
        assert_eq!(
            kinds,
            vec![
                serde_json::json!("Embedding"),
                serde_json::json!("NormPlan"),
                serde_json::json!("NormSite"),
                serde_json::json!("RouterLayer"),
                serde_json::json!("RouterTensor"),
                serde_json::json!("RouterSelection"),
                serde_json::json!("ExpertSection"),
                serde_json::json!("ExpertTensor"),
                serde_json::json!("FfnActivationSite"),
                serde_json::json!("ResidualSiteRef"),
                serde_json::json!("DecodePlan"),
                serde_json::json!("ClassifyHead"),
                serde_json::json!("SequenceSlot"),
                serde_json::json!("SequenceStep"),
                serde_json::json!("TokenInput"),
            ]
        );
    }

    #[test]
    fn value_producer_ref_two_variants() {
        let refs = [
            ValueProducerRef::Node {
                node: NodeId::new(0),
            },
            ValueProducerRef::External {
                token_input: TokenInputId::new(0),
            },
        ];
        let kinds = refs
            .iter()
            .map(|producer| {
                serde_json::to_value(producer).expect("producer serializes")["kind"].clone()
            })
            .collect::<Vec<_>>();

        assert_eq!(
            kinds,
            vec![serde_json::json!("Node"), serde_json::json!("External")]
        );
    }

    #[test]
    fn provenance_refs_public_json_shape_uses_tagged_objects() {
        assert_eq!(
            serde_json::to_value(QuantGraphEntityRef::NormPlan {
                plan: NormPlanId::new(7),
            })
            .expect("entity ref serializes"),
            serde_json::json!({
                "kind": "NormPlan",
                "plan": 7,
            })
        );
        assert_eq!(
            serde_json::to_value(QuantGraphEntityRef::TokenInput {
                token_input: TokenInputId::new(0),
            })
            .expect("entity ref serializes"),
            serde_json::json!({
                "kind": "TokenInput",
                "token_input": 0,
            })
        );
        assert_eq!(
            serde_json::to_value(ValueProducerRef::Node {
                node: NodeId::new(2),
            })
            .expect("producer ref serializes"),
            serde_json::json!({
                "kind": "Node",
                "node": 2,
            })
        );
        assert_eq!(
            serde_json::to_value(ValueProducerRef::External {
                token_input: TokenInputId::new(0),
            })
            .expect("producer ref serializes"),
            serde_json::json!({
                "kind": "External",
                "token_input": 0,
            })
        );
        assert_eq!(
            serde_json::to_value(semantic_anchor(8)).expect("anchor serializes"),
            serde_json::json!({
                "anchor_id": hash(8),
            })
        );
    }

    #[test]
    fn infer_ir_provenance_serde_round_trip_proptest() {
        let provenance = provenance_fixture();
        let encoded = serde_json::to_string(&provenance).expect("provenance serializes");
        let decoded: InferIrProvenance =
            serde_json::from_str(&encoded).expect("provenance deserializes");

        assert_eq!(decoded, provenance);
    }

    #[test]
    fn node_anchor_map_is_btreemap_canonical() {
        let anchors = NodeAnchorMap::from([
            (NodeId::new(2), semantic_anchor(2)),
            (NodeId::new(1), semantic_anchor(1)),
        ]);

        assert_eq!(
            anchors.keys().copied().collect::<Vec<_>>(),
            vec![NodeId::new(1), NodeId::new(2)]
        );
    }

    #[test]
    fn semantic_anchor_id_uses_domain_hash_with_qg_self_hash() {
        let tuple = canonical_provenance_tuple_fixture();
        let left = compute_semantic_anchor(hash(1), NodeId::new(0), InferOpTag::Embedding, &tuple)
            .expect("anchor computes");
        let right = compute_semantic_anchor(hash(2), NodeId::new(0), InferOpTag::Embedding, &tuple)
            .expect("anchor computes");

        assert_ne!(left, right);
    }

    #[test]
    fn semantic_anchor_id_stable_across_two_qg_regenerations() {
        let tuple = canonical_provenance_tuple_fixture();
        let left = compute_semantic_anchor(hash(1), NodeId::new(0), InferOpTag::Embedding, &tuple)
            .expect("anchor computes");
        let right = compute_semantic_anchor(hash(1), NodeId::new(0), InferOpTag::Embedding, &tuple)
            .expect("anchor computes");

        assert_eq!(left, right);
    }

    #[test]
    fn node_anchor_map_excludes_semantic_checkpoint_ids() {
        let value =
            serde_json::to_value(NodeAnchorMap::from([(NodeId::new(0), semantic_anchor(8))]))
                .expect("anchors serialize");

        assert_forbidden_keys_absent(&value, &["SemanticCheckpointId", "TraceProbeId"]);
        assert!(!value.to_string().contains("SemanticCheckpoint"));
        assert!(!value.to_string().contains("TraceProbe"));
    }

    #[test]
    fn compute_semantic_anchor_includes_canonical_provenance_tuple() {
        let mut left_tuple = canonical_provenance_tuple_fixture();
        let mut right_tuple = canonical_provenance_tuple_fixture();
        left_tuple.occurrence_index = 0;
        right_tuple.occurrence_index = 1;

        let left =
            compute_semantic_anchor(hash(1), NodeId::new(0), InferOpTag::Embedding, &left_tuple)
                .expect("anchor computes");
        let right =
            compute_semantic_anchor(hash(1), NodeId::new(0), InferOpTag::Embedding, &right_tuple)
                .expect("anchor computes");

        assert_ne!(left, right);
    }

    #[test]
    fn semantic_anchor_collision_resistance_proptest() {
        let mut observed = BTreeSet::new();

        for index in 0..16 {
            let tuple = CanonicalProvenanceTuple {
                occurrence_index: index,
                ..canonical_provenance_tuple_fixture()
            };
            let anchor = compute_semantic_anchor(
                hash(index as u8),
                NodeId::new(index),
                InferOpTag::Embedding,
                &tuple,
            )
            .expect("anchor computes");

            assert!(observed.insert(anchor.anchor_id));
        }
    }

    #[test]
    fn semantic_anchor_missing_rejected_for_node() {
        let result = GbInferIR::new(
            identity(),
            vec![token_input()],
            vec![embedding_node()],
            vec![value_decl(), embedding_value_decl()],
            Vec::new(),
            provenance_fixture(),
            NodeAnchorMap::new(),
        );

        assert!(matches!(
            result,
            Err(GbInferIrTypeError::SemanticAnchorMissing { node_id })
                if node_id == NodeId::new(0)
        ));
    }

    #[derive(Debug)]
    struct FakeStaticBudget {
        hash: Hash256,
        sites: Vec<ReductionSiteId>,
    }

    impl FakeStaticBudget {
        fn new(hash: Hash256) -> Self {
            Self {
                hash,
                sites: Vec::new(),
            }
        }

        fn with_sites(hash: Hash256, sites: Vec<ReductionSiteId>) -> Self {
            Self { hash, sites }
        }
    }

    impl StaticBudgetReportSelfHash for FakeStaticBudget {
        fn report_self_hash(&self) -> Hash256 {
            self.hash
        }
    }

    impl StaticBudgetReductionSites for FakeStaticBudget {
        fn reduction_site_ids(&self) -> Vec<ReductionSiteId> {
            self.sites.clone()
        }
    }

    fn infer_inputs<'a>(
        quant_graph: &'a QuantGraph,
        static_budget: &'a FakeStaticBudget,
    ) -> GbInferIRInputs<'a, FakeStaticBudget> {
        GbInferIRInputs {
            quant_graph,
            quant_graph_self_hash: hash(0x33),
            policy_projection: policy_projection(),
            audit_parents: InferIrAuditParents {
                policy_resolution_self_hash: hash(0x55),
                compile_request_hash: hash(0x56),
            },
            static_budget,
            static_budget_self_hash: static_budget.hash,
        }
    }

    fn policy_projection() -> InferIrPolicyProjection {
        InferIrPolicyProjection {
            requested_runtime_modes: BTreeSet::from([RuntimeMode::Interactive, RuntimeMode::Safe]),
            allowed_ingress_modes: BTreeSet::from([
                TokenIngressMode::AutoRegressive,
                TokenIngressMode::Prompt,
            ]),
        }
    }

    fn wave4_intermediate(quant_graph: &QuantGraph) -> InferIrWave4 {
        let budget = FakeStaticBudget::new(hash(0x44));
        let inputs = infer_inputs(quant_graph, &budget);
        bind_wave4_classes_1_to_5(&inputs).expect("classes 1-5 bind")
    }

    fn finalized_ir_for_graph(quant_graph: &QuantGraph) -> GbInferIR {
        let first_wave = wave4_intermediate(quant_graph);
        let sites = reduction_sites_for_nodes(&first_wave.nodes, quant_graph);
        let budget = FakeStaticBudget::with_sites(hash(0x44), sites);
        let inputs = infer_inputs(quant_graph, &budget);
        let second_wave = bind_wave4_classes_1_to_5(&inputs).expect("classes 1-5 bind");
        bind_wave4_classes_6_to_10(&inputs, second_wave).expect("classes 6-10 bind")
    }

    #[derive(Clone)]
    struct InferIrRejectFixture {
        id: u8,
        slug: &'static str,
        code_name: &'static str,
        code: ValidationCode,
    }

    fn assert_passing_fixture_files(name: &str) {
        let dir = infer_ir_fixture_root().join(name);
        assert!(
            dir.is_dir(),
            "missing passing fixture dir {}",
            dir.display()
        );
        for file in [
            "README.md",
            "inputs.toml",
            "expected.toml",
            "infer_ir_self_hash",
            "canonical_bytes_hash",
            "topological_order_hash",
        ] {
            let path = dir.join(file);
            assert!(
                path.is_file(),
                "missing passing fixture file {}",
                path.display()
            );
            assert!(
                !fs::read_to_string(&path)
                    .expect("fixture file reads")
                    .trim()
                    .is_empty(),
                "fixture file {} must be non-empty",
                path.display()
            );
        }
    }

    fn assert_reject_fixture_by_slug(slug: &str) {
        let case = infer_ir_reject_fixtures()
            .into_iter()
            .find(|case| case.slug == slug)
            .expect("reject slug exists in table");
        assert_reject_fixture(case);
    }

    fn assert_reject_fixture(case: InferIrRejectFixture) {
        let dir = infer_ir_fixture_root().join("reject").join(case.slug);
        assert!(dir.is_dir(), "missing reject fixture dir {}", dir.display());
        for file in ["README.md", "inputs.toml", "expected.toml"] {
            assert!(
                dir.join(file).is_file(),
                "missing reject fixture file {}",
                dir.join(file).display()
            );
        }
        let expected = fs::read_to_string(dir.join("expected.toml")).expect("expected TOML reads");
        assert!(expected.contains(&format!("reject_id = {}", case.id)));
        assert!(expected.contains(&format!("code = \"{}\"", case.code_name)));
        let diagnostic = ValidationDiagnostic::hard(
            ValidationOrigin::SemanticCore,
            case.code.clone(),
            ValidationDetail::Field {
                field: FieldPath::from(case.slug),
            },
            Vec::new(),
        );

        assert_eq!(diagnostic.severity, DiagnosticSeverity::Hard);
        assert_eq!(validation_code_kind(&diagnostic.code), case.code_name);
    }

    fn validation_code_kind(code: &ValidationCode) -> String {
        let value = serde_json::to_value(code).expect("validation code serializes");
        value
            .get("kind")
            .and_then(|value| value.as_str())
            .expect("validation code has kind")
            .to_owned()
    }

    fn infer_ir_fixture_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("gbf-codegen has workspace parent")
            .join("fixtures")
            .join("infer_ir")
    }

    fn maybe_dump_fixture_goldens(name: &str, product: &GbInferIRProduct) {
        if std::env::var_os("GBF_DUMP_INFER_IR_GOLDENS").is_some() {
            eprintln!(
                "{name} infer_ir_self_hash={} canonical_bytes_hash={} topological_order_hash={}",
                product.infer_ir_self_hash,
                product.infer_ir_canonical_bytes_hash,
                product.infer_ir.identity.topological_order_hash
            );
        }
    }

    fn assert_fixture_golden_hashes(name: &str, product: &GbInferIRProduct) {
        let dir = infer_ir_fixture_root().join(name);
        assert_eq!(
            read_trimmed(&dir.join("infer_ir_self_hash")),
            product.infer_ir_self_hash.to_string()
        );
        assert_eq!(
            read_trimmed(&dir.join("canonical_bytes_hash")),
            product.infer_ir_canonical_bytes_hash.to_string()
        );
        assert_eq!(
            read_trimmed(&dir.join("topological_order_hash")),
            product.infer_ir.identity.topological_order_hash.to_string()
        );
    }

    fn read_trimmed(path: &Path) -> String {
        fs::read_to_string(path)
            .expect("fixture file reads")
            .trim()
            .to_owned()
    }

    fn infer_ir_reject_fixtures() -> Vec<InferIrRejectFixture> {
        vec![
            reject_case(
                1,
                "infer_ir_embedding_not_unique",
                "InferIrEmbeddingNotUnique",
                ValidationCode::InferIrEmbeddingNotUnique {
                    field: FieldPath::from("nodes.embedding"),
                },
            ),
            reject_case(
                2,
                "infer_ir_decode_not_unique",
                "InferIrDecodeNotUnique",
                ValidationCode::InferIrDecodeNotUnique {
                    field: FieldPath::from("nodes.decode"),
                },
            ),
            reject_case(
                3,
                "infer_ir_classify_not_unique",
                "InferIrClassifyNotUnique",
                ValidationCode::InferIrClassifyNotUnique {
                    field: FieldPath::from("nodes.classify"),
                },
            ),
            reject_case(
                4,
                "infer_ir_expert_coverage_mismatch",
                "InferIrExpertCoverageMismatch",
                ValidationCode::InferIrExpertCoverageMismatch {
                    layer: LayerId::new(0),
                    expert: ExpertId::new(0),
                },
            ),
            reject_case(
                5,
                "infer_ir_route_coverage_mismatch",
                "InferIrRouteCoverageMismatch",
                ValidationCode::InferIrRouteCoverageMismatch {
                    layer: LayerId::new(0),
                },
            ),
            reject_case(
                6,
                "infer_ir_semantic_checkpoint_emitted_here",
                "InferIrSemanticCheckpointEmittedHere",
                ValidationCode::InferIrSemanticCheckpointEmittedHere {
                    field: FieldPath::from("anchors.semantic_checkpoint"),
                },
            ),
            reject_case(
                7,
                "infer_ir_effect_chain_not_linear",
                "InferIrEffectChainNotLinear",
                ValidationCode::InferIrEffectChainNotLinear {
                    field: FieldPath::from("effects"),
                },
            ),
            reject_case(
                8,
                "infer_ir_effect_id_edge_token_violation",
                "InferIrEffectIdEdgeTokenViolation",
                ValidationCode::InferIrEffectIdEdgeTokenViolation {
                    field: FieldPath::from("effects"),
                },
            ),
            reject_case(
                9,
                "infer_ir_topological_order_mismatch",
                "InferIrTopologicalOrderMismatch",
                ValidationCode::InferIrTopologicalOrderMismatch {
                    field: FieldPath::from("nodes"),
                },
            ),
            reject_case(
                10,
                "infer_ir_value_format_mismatch",
                "InferIrValueFormatMismatch",
                ValidationCode::InferIrValueFormatMismatch {
                    field: FieldPath::from("values.format"),
                },
            ),
            reject_case(
                11,
                "infer_ir_norm_format_mismatch",
                "InferIrNormFormatMismatch",
                ValidationCode::InferIrNormFormatMismatch {
                    field: FieldPath::from("nodes.norm"),
                },
            ),
            reject_case(
                12,
                "infer_ir_decode_rng_binding_mismatch",
                "InferIrDecodeRngBindingMismatch",
                ValidationCode::InferIrDecodeRngBindingMismatch {
                    field: FieldPath::from("nodes.decode.effects"),
                },
            ),
            reject_case(
                13,
                "infer_ir_semantic_equivalence_failed",
                "InferIrSemanticEquivalenceFailed",
                ValidationCode::InferIrSemanticEquivalenceFailed {
                    field: FieldPath::from("semantic_equivalence.fixture"),
                },
            ),
            reject_case(
                14,
                "infer_ir_cycle_detected",
                "InferIrCycleDetected",
                ValidationCode::InferIrCycleDetected {
                    field: FieldPath::from("nodes"),
                },
            ),
            reject_case(
                15,
                "infer_ir_unreachable_node",
                "InferIrUnreachableNode",
                ValidationCode::InferIrUnreachableNode {
                    field: FieldPath::from("nodes"),
                },
            ),
            reject_case(
                16,
                "infer_ir_disconnected_component",
                "InferIrDisconnectedComponent",
                ValidationCode::InferIrDisconnectedComponent {
                    field: FieldPath::from("nodes"),
                },
            ),
            reject_case(
                17,
                "infer_ir_forbidden_storage_metadata",
                "InferIrForbiddenStorageMetadata",
                ValidationCode::InferIrForbiddenStorageMetadata {
                    field: FieldPath::from("values.layout"),
                },
            ),
            reject_case(
                18,
                "infer_ir_non_v1_router_semantics",
                "InferIrNonV1RouterSemantics",
                ValidationCode::InferIrNonV1RouterSemantics {
                    layer: LayerId::new(0),
                },
            ),
            reject_case(
                19,
                "infer_ir_semantic_anchor_missing",
                "InferIrSemanticAnchorMissing",
                ValidationCode::InferIrSemanticAnchorMissing {
                    field: FieldPath::from("anchors"),
                },
            ),
            reject_case(
                20,
                "infer_ir_ffn_activation_missing",
                "InferIrFfnActivationMissing",
                ValidationCode::InferIrFfnActivationMissing {
                    layer: LayerId::new(0),
                    expert: ExpertId::new(0),
                },
            ),
            reject_case(
                21,
                "infer_ir_expert_selection_missing",
                "InferIrExpertSelectionMissing",
                ValidationCode::InferIrExpertSelectionMissing {
                    layer: LayerId::new(0),
                },
            ),
            reject_case(
                22,
                "infer_ir_gate_weight_not_consumed",
                "InferIrGateWeightNotConsumed",
                ValidationCode::InferIrGateWeightNotConsumed {
                    field: FieldPath::from("values.gate_weight"),
                },
            ),
            reject_case(
                23,
                "infer_ir_token_ingress_ambiguous",
                "InferIrTokenIngressAmbiguous",
                ValidationCode::InferIrTokenIngressAmbiguous {
                    field: FieldPath::from("token_inputs"),
                },
            ),
            reject_case(
                24,
                "infer_ir_reduction_site_missing",
                "InferIrReductionSiteMissing",
                ValidationCode::InferIrReductionSiteMissing {
                    field: FieldPath::from("nodes.reduction_site"),
                },
            ),
            reject_case(
                25,
                "infer_ir_op_histogram_total_mismatch",
                "InferIrOpHistogramTotalMismatch",
                ValidationCode::InferIrOpHistogramTotalMismatch {
                    field: FieldPath::from("result.op_histogram"),
                },
            ),
            reject_case(
                26,
                "infer_ir_fault_boundary_emitted_v1_forbidden",
                "InferIrFaultBoundaryEmittedV1Forbidden",
                ValidationCode::InferIrFaultBoundaryEmittedV1Forbidden {
                    field: FieldPath::from("effects"),
                },
            ),
            reject_case(
                27,
                "infer_ir_op_signature_mismatch",
                "InferIrOpSignatureMismatch",
                ValidationCode::InferIrOpSignatureMismatch {
                    field: FieldPath::from("nodes.signature"),
                },
            ),
            reject_case(
                28,
                "infer_ir_router_score_orphaned",
                "InferIrRouterScoreOrphaned",
                ValidationCode::InferIrRouterScoreOrphaned {
                    field: FieldPath::from("values.router_score"),
                },
            ),
            reject_case(
                29,
                "infer_ir_sequence_slot_coverage_mismatch",
                "InferIrSequenceSlotCoverageMismatch",
                ValidationCode::InferIrSequenceSlotCoverageMismatch {
                    layer: LayerId::new(0),
                },
            ),
            reject_case(
                30,
                "infer_ir_unexpected_rng_effect_on_pure_op",
                "InferIrUnexpectedRngEffectOnPureOp",
                ValidationCode::InferIrUnexpectedRngEffectOnPureOp {
                    field: FieldPath::from("nodes.classify.effects"),
                },
            ),
            reject_case(
                31,
                "infer_ir_residual_boundary_mismatch",
                "InferIrResidualBoundaryMismatch",
                ValidationCode::InferIrResidualBoundaryMismatch {
                    field: FieldPath::from("nodes.combine_residual"),
                },
            ),
            reject_case(
                32,
                "infer_ir_router_matvec_missing_for_routed_layer",
                "InferIrRouterMatVecMissingForRoutedLayer",
                ValidationCode::InferIrRouterMatVecMissingForRoutedLayer {
                    layer: LayerId::new(0),
                },
            ),
            reject_case(
                33,
                "infer_ir_router_present_for_dense_layer",
                "InferIrRouterPresentForDenseLayer",
                ValidationCode::InferIrRouterPresentForDenseLayer {
                    layer: LayerId::new(0),
                },
            ),
            reject_case(
                34,
                "infer_ir_input_token_value_id_mismatch",
                "InferIrInputTokenValueIdMismatch",
                ValidationCode::InferIrInputTokenValueIdMismatch {
                    field: FieldPath::from("token_inputs.value_id"),
                },
            ),
            reject_case(
                35,
                "infer_ir_sequence_state_next_orphaned",
                "InferIrSequenceStateNextOrphaned",
                ValidationCode::InferIrSequenceStateNextOrphaned {
                    field: FieldPath::from("values.sequence_state_next"),
                },
            ),
            reject_case(
                36,
                "infer_ir_sequence_semantics_unsupported_v1",
                "InferIrSequenceSemanticsUnsupportedV1",
                ValidationCode::InferIrSequenceSemanticsUnsupportedV1 {
                    field: FieldPath::from("sequence_semantics.state_slots"),
                },
            ),
        ]
    }

    fn reject_case(
        id: u8,
        slug: &'static str,
        code_name: &'static str,
        code: ValidationCode,
    ) -> InferIrRejectFixture {
        InferIrRejectFixture {
            id,
            slug,
            code_name,
            code,
        }
    }

    fn infer_ir_product_for_graph(quant_graph: &QuantGraph) -> GbInferIRProduct {
        let infer_ir = finalized_ir_for_graph(quant_graph);
        let fixture_equivalence = report_schema::FixtureEquivalenceTag::from(
            fixture_semantic_equivalence(&infer_ir, quant_graph, &fixture_inputs())
                .expect("fixture semantic equivalence resolves"),
        );
        GbInferIRProduct::new(
            infer_ir,
            InferIrAuditParents {
                policy_resolution_self_hash: hash(0x52),
                compile_request_hash: hash(0x53),
            },
            BTreeSet::from([RuntimeMode::Interactive, RuntimeMode::Safe]),
            fixture_equivalence,
        )
        .expect("infer_ir product builds")
    }

    #[cfg(feature = "semantic_equivalence_check")]
    fn export_review_f_b5_fixture(
        packet_dir: &Path,
        name: &str,
        quant_graph: &QuantGraph,
        bit_exact: &mut String,
    ) {
        let first_wave = wave4_intermediate(quant_graph);
        let sites = reduction_sites_for_nodes(&first_wave.nodes, quant_graph);
        let budget = FakeStaticBudget::with_sites(hash(0x44), sites);
        let inputs = infer_inputs(quant_graph, &budget);
        let fixture_dir = packet_dir.join("golden").join(name);
        fs::create_dir_all(&fixture_dir).expect("fixture export dir exists");

        let product = run_stage3(
            inputs,
            PassEnvironment::new(&NoopResolver).with_report_dir(&fixture_dir),
        )
        .expect("Stage 3 fixture export succeeds");
        assert_fixture_golden_hashes(name, &product);

        let report_bytes =
            fs::read(fixture_dir.join("infer_ir.json")).expect("driver report exists");
        let decoded: ReportEnvelope<report_schema::InferIrReportBody<GbInferIR>> =
            serde_json::from_slice(&report_bytes).expect("driver report decodes");
        assert_eq!(decoded.outcome, ReportOutcome::Passed);
        assert_eq!(
            decoded.body.result.as_ref().map(|result| &result.product),
            Some(&product.infer_ir)
        );
        assert_eq!(
            canonicalize_report(&decoded).expect("driver report canonicalizes"),
            report_bytes
        );

        fs::write(
            fixture_dir.join("hashes.toml"),
            review_f_b5_hashes_toml(name, &product),
        )
        .expect("fixture hashes write");
        fs::write(
            fixture_dir.join("anchor_ids.toml"),
            review_f_b5_anchor_ids_toml(name, &product),
        )
        .expect("fixture anchor ids write");

        write!(
            bit_exact,
            r#"
[[fixtures]]
name = "{name}"
determinism = "BitExact"
fixture_equivalence = "VerifiedFixtureBitExact"
infer_ir_json = "golden/{name}/infer_ir.json"
infer_ir_self_hash = "{}"
infer_ir_canonical_bytes_hash = "{}"
topological_order_hash = "{}"
anchor_count = {}
"#,
            product.infer_ir_self_hash,
            product.infer_ir_canonical_bytes_hash,
            product.infer_ir.identity.topological_order_hash,
            product.infer_ir.anchors.len()
        )
        .expect("BitExact fixture TOML formats");

        for (sample_index, input) in fixture_inputs().inputs.iter().enumerate() {
            let ir_output = canonical::reference::eval_canonical_ir(&product.infer_ir, input);
            let qg_output = canonical::reference::eval_canonical_qg(quant_graph, input);
            assert_eq!(ir_output, qg_output);
            write!(
                bit_exact,
                r#"
[[samples]]
fixture = "{name}"
sample_index = {sample_index}
token_id = {}
sequence_state_seed = "{}"
rng_seed = "{}"
ir_trace_hash = "{}"
qg_trace_hash = "{}"
match = true
"#,
                input.token_id,
                input.sequence_state_seed,
                input.rng_seed,
                ir_output.trace_hash,
                qg_output.trace_hash
            )
            .expect("BitExact sample TOML formats");
        }
    }

    #[cfg(feature = "semantic_equivalence_check")]
    fn review_f_b5_hashes_toml(name: &str, product: &GbInferIRProduct) -> String {
        format!(
            r#"schema = "f_b5.review_golden_hashes.v1"
fixture = "{name}"
source_fixture = "fixtures/infer_ir/{name}"
infer_ir_json = "golden/{name}/infer_ir.json"
infer_ir_self_hash = "{}"
infer_ir_canonical_bytes_hash = "{}"
topological_order_hash = "{}"
report_self_hash = "{}"
anchor_count = {}
driver_product_report_status = "exported"
"#,
            product.infer_ir_self_hash,
            product.infer_ir_canonical_bytes_hash,
            product.infer_ir.identity.topological_order_hash,
            product.report.report_self_hash,
            product.infer_ir.anchors.len()
        )
    }

    #[cfg(feature = "semantic_equivalence_check")]
    fn review_f_b5_anchor_ids_toml(name: &str, product: &GbInferIRProduct) -> String {
        let mut toml = format!(
            r#"schema = "f_b5.review_anchor_ids.v1"
fixture = "{name}"
status = "exported"
source = "golden/{name}/infer_ir.json.result.product.anchors"
anchor_count = {}
"#,
            product.infer_ir.anchors.len()
        );
        for node in &product.infer_ir.nodes {
            let anchor = product
                .infer_ir
                .anchors
                .get(&node.node_id)
                .expect("anchor exists for node");
            let reduction_site = node
                .reduction_site
                .as_ref()
                .map_or_else(|| "none".to_owned(), |site| site.0.clone());
            write!(
                toml,
                r#"
[[anchors]]
node_id = {}
op_tag = "{:?}"
anchor_id = "{}"
reduction_site = "{}"
"#,
                node.node_id.get(),
                node.op.tag(),
                anchor.anchor_id,
                reduction_site
            )
            .expect("anchor TOML formats");
        }
        toml
    }

    #[cfg(feature = "semantic_equivalence_check")]
    fn review_f_b5_driver_evidence_toml() -> String {
        r#"schema = "f_b5.stage3_driver_evidence.v1"
stage3_driver_gate = "scripts/e2e/stage3.sh"
build_core_tests = "s3::infer_ir::tests::build_infer_ir_core"
run_stage3_tests = "s3::infer_ir::tests::run_stage3"
driver_report_emission_test = "s3::infer_ir::tests::run_stage3_emits_report_and_writes_cache"
driver_cache_rewrap_test = "s3::infer_ir::tests::run_stage3_cache_hit_replays_with_audit_rewrap"
driver_product_report_status = "exported"
fixture_export_status = "exported"
product_reports = [
  "golden/dense_toy0/infer_ir.json",
  "golden/routed_basic/infer_ir.json",
  "golden/mixed_topology/infer_ir.json",
]
"#
        .to_owned()
    }

    #[cfg(feature = "semantic_equivalence_check")]
    fn review_f_b5_bit_exact_header() -> String {
        r#"schema = "f_b5.bit_exact_equivalence_golden.v1"
source = "fixtures/infer_ir"
feature = "semantic_equivalence_check"
closure_gate = "cargo test -p gbf-codegen --features semantic_equivalence_check --lib fixture_infer_ir_fixture_semantic_equivalence_bit_exact"
status = "VerifiedFixtureBitExact"
stage3_driver_evidence = "scripts/e2e/stage3.sh verifies run_stage3 emits product-bearing infer_ir.json to its report_dir."
driver_materialization = "exported"
fixture_export_status = "exported"
"#
        .to_owned()
    }

    fn fixture_inputs() -> FixtureInputSet {
        FixtureInputSet::fixture(vec![fixture_input(0), fixture_input(7)])
    }

    fn fixture_input(token_id: u32) -> FixtureInput {
        FixtureInput {
            token_id,
            sequence_state_seed: hash(0x61),
            rng_seed: hash(0x62),
        }
    }

    fn reduction_sites_for_nodes(
        nodes: &[GbNode],
        quant_graph: &QuantGraph,
    ) -> Vec<ReductionSiteId> {
        nodes
            .iter()
            .filter_map(|node| reduction_site_key_for_op(node.op, quant_graph))
            .map(canonical_reduction_site_id)
            .collect()
    }

    fn wave4_canonical_bytes(wave4: &InferIrWave4) -> Vec<u8> {
        let value = serde_json::json!({
            "identity": wave4.identity,
            "token_input": wave4.token_input,
            "values": wave4.values.values,
            "effects": wave4.effects.effects,
            "nodes": wave4.nodes,
        });
        canonicalize_value(&value).expect("wave4 intermediate canonicalizes")
    }

    fn wave4_quant_graph_dense() -> QuantGraph {
        let layer = LayerId::new(0);
        QuantGraph {
            identity: wave4_identity(ModelSpecSummary {
                n_layers: 1,
                n_experts: BTreeMap::from([(layer, 1)]),
                d_model: 8,
                d_ff: 16,
                vocab_size: 80,
                ffn_kind: BTreeMap::from([(layer, FfnKindTag::Dense)]),
            }),
            tensors: vec![
                wave4_tensor(
                    1,
                    QuantTensorRole::EmbeddingTable,
                    QuantFormat::I8,
                    &[80, 8],
                ),
                wave4_expert_weight_tensor(20, layer, ExpertId::new(0), ExpertWeightSlot::FfnUp),
                wave4_expert_weight_tensor(21, layer, ExpertId::new(0), ExpertWeightSlot::FfnDown),
            ],
            norm_plans: vec![
                wave4_norm_record(
                    0,
                    NormSite::LayerSequence { layer },
                    QuantFormat::I8,
                    QuantFormat::Q8_8,
                ),
                wave4_norm_record(
                    1,
                    NormSite::LayerFfn { layer },
                    QuantFormat::Q8_8,
                    QuantFormat::Q8_8,
                ),
                wave4_norm_record(2, NormSite::Final, QuantFormat::Q8_8, QuantFormat::Q8_8),
            ],
            layer_norms: BTreeMap::from([(
                layer,
                crate::s1::quant_graph::LayerNorms {
                    pre_sequence: NormPlanId::new(0),
                    pre_ffn: NormPlanId::new(1),
                },
            )]),
            routing_table: None,
            expert_sections: vec![crate::s1::quant_graph::ExpertSection {
                layer,
                expert: ExpertId::new(0),
                tensor_refs: vec![TensorId::new(20), TensorId::new(21)],
            }],
            ffn_plans: BTreeMap::from([(
                layer,
                FfnPlan {
                    layer,
                    activation_kind: FfnActivationKind::Relu,
                    intermediate_format: QuantFormat::Q8_8,
                },
            )]),
            decode_spec: wave4_decode_spec(false),
            sequence_semantics: SequenceSemanticsSpec::identity(),
            provenance: BTreeMap::new(),
            classify_head: wave4_classify_head(),
            residual_plan: wave4_residual_plan(),
        }
    }

    fn wave4_quant_graph_routed() -> QuantGraph {
        let layer = LayerId::new(0);
        let mut graph = wave4_quant_graph_dense();
        graph.identity.model_spec_summary.ffn_kind = BTreeMap::from([(layer, FfnKindTag::Routed)]);
        graph.identity.model_spec_summary.n_experts = BTreeMap::from([(layer, 2)]);
        graph.tensors.push(wave4_tensor(
            10,
            QuantTensorRole::RouterWeight { layer },
            QuantFormat::I8,
            &[8, 2],
        ));
        graph.tensors.push(wave4_expert_weight_tensor(
            22,
            layer,
            ExpertId::new(1),
            ExpertWeightSlot::FfnUp,
        ));
        graph.tensors.push(wave4_expert_weight_tensor(
            23,
            layer,
            ExpertId::new(1),
            ExpertWeightSlot::FfnDown,
        ));
        graph
            .expert_sections
            .push(crate::s1::quant_graph::ExpertSection {
                layer,
                expert: ExpertId::new(1),
                tensor_refs: vec![TensorId::new(22), TensorId::new(23)],
            });
        graph.routing_table = Some(RoutingTable {
            layers: vec![RouterLayer {
                layer,
                n_experts: 2,
                router_weight: TensorId::new(10),
                router_bias: None,
                semantics: RouterSemantics::Top1Hard {
                    gate_weight: RouterGateWeightSemantics::SelectedScore,
                    tie_break: RouterTieBreak::LowestExpertId,
                },
            }],
        });
        graph
    }

    fn wave4_quant_graph_mixed_topology() -> QuantGraph {
        let dense_layer = LayerId::new(0);
        let routed_layer = LayerId::new(1);
        QuantGraph {
            identity: wave4_identity(ModelSpecSummary {
                n_layers: 2,
                n_experts: BTreeMap::from([(dense_layer, 1), (routed_layer, 2)]),
                d_model: 8,
                d_ff: 16,
                vocab_size: 80,
                ffn_kind: BTreeMap::from([
                    (dense_layer, FfnKindTag::Dense),
                    (routed_layer, FfnKindTag::Routed),
                ]),
            }),
            tensors: vec![
                wave4_tensor(
                    1,
                    QuantTensorRole::EmbeddingTable,
                    QuantFormat::I8,
                    &[80, 8],
                ),
                wave4_tensor(
                    30,
                    QuantTensorRole::RouterWeight {
                        layer: routed_layer,
                    },
                    QuantFormat::I8,
                    &[8, 2],
                ),
                wave4_expert_weight_tensor(
                    20,
                    dense_layer,
                    ExpertId::new(0),
                    ExpertWeightSlot::FfnUp,
                ),
                wave4_expert_weight_tensor(
                    21,
                    dense_layer,
                    ExpertId::new(0),
                    ExpertWeightSlot::FfnDown,
                ),
                wave4_expert_weight_tensor(
                    40,
                    routed_layer,
                    ExpertId::new(0),
                    ExpertWeightSlot::FfnUp,
                ),
                wave4_expert_weight_tensor(
                    41,
                    routed_layer,
                    ExpertId::new(0),
                    ExpertWeightSlot::FfnDown,
                ),
                wave4_expert_weight_tensor(
                    42,
                    routed_layer,
                    ExpertId::new(1),
                    ExpertWeightSlot::FfnUp,
                ),
                wave4_expert_weight_tensor(
                    43,
                    routed_layer,
                    ExpertId::new(1),
                    ExpertWeightSlot::FfnDown,
                ),
            ],
            norm_plans: vec![
                wave4_norm_record(
                    0,
                    NormSite::LayerSequence { layer: dense_layer },
                    QuantFormat::I8,
                    QuantFormat::Q8_8,
                ),
                wave4_norm_record(
                    1,
                    NormSite::LayerFfn { layer: dense_layer },
                    QuantFormat::Q8_8,
                    QuantFormat::Q8_8,
                ),
                wave4_norm_record(
                    2,
                    NormSite::LayerSequence {
                        layer: routed_layer,
                    },
                    QuantFormat::Q8_8,
                    QuantFormat::Q8_8,
                ),
                wave4_norm_record(
                    3,
                    NormSite::LayerFfn {
                        layer: routed_layer,
                    },
                    QuantFormat::Q8_8,
                    QuantFormat::Q8_8,
                ),
                wave4_norm_record(4, NormSite::Final, QuantFormat::Q8_8, QuantFormat::Q8_8),
            ],
            layer_norms: BTreeMap::from([
                (
                    dense_layer,
                    crate::s1::quant_graph::LayerNorms {
                        pre_sequence: NormPlanId::new(0),
                        pre_ffn: NormPlanId::new(1),
                    },
                ),
                (
                    routed_layer,
                    crate::s1::quant_graph::LayerNorms {
                        pre_sequence: NormPlanId::new(2),
                        pre_ffn: NormPlanId::new(3),
                    },
                ),
            ]),
            routing_table: Some(RoutingTable {
                layers: vec![RouterLayer {
                    layer: routed_layer,
                    n_experts: 2,
                    router_weight: TensorId::new(30),
                    router_bias: None,
                    semantics: RouterSemantics::Top1Hard {
                        gate_weight: RouterGateWeightSemantics::SelectedScore,
                        tie_break: RouterTieBreak::LowestExpertId,
                    },
                }],
            }),
            expert_sections: vec![
                crate::s1::quant_graph::ExpertSection {
                    layer: dense_layer,
                    expert: ExpertId::new(0),
                    tensor_refs: vec![TensorId::new(20), TensorId::new(21)],
                },
                crate::s1::quant_graph::ExpertSection {
                    layer: routed_layer,
                    expert: ExpertId::new(0),
                    tensor_refs: vec![TensorId::new(40), TensorId::new(41)],
                },
                crate::s1::quant_graph::ExpertSection {
                    layer: routed_layer,
                    expert: ExpertId::new(1),
                    tensor_refs: vec![TensorId::new(42), TensorId::new(43)],
                },
            ],
            ffn_plans: BTreeMap::from([
                (
                    dense_layer,
                    FfnPlan {
                        layer: dense_layer,
                        activation_kind: FfnActivationKind::Relu,
                        intermediate_format: QuantFormat::Q8_8,
                    },
                ),
                (
                    routed_layer,
                    FfnPlan {
                        layer: routed_layer,
                        activation_kind: FfnActivationKind::Relu,
                        intermediate_format: QuantFormat::Q8_8,
                    },
                ),
            ]),
            decode_spec: wave4_decode_spec(false),
            sequence_semantics: SequenceSemanticsSpec::identity(),
            provenance: BTreeMap::new(),
            classify_head: wave4_classify_head(),
            residual_plan: wave4_residual_plan(),
        }
    }

    fn wave4_identity(
        model_spec_summary: ModelSpecSummary,
    ) -> crate::s1::quant_graph::QuantGraphIdentity {
        crate::s1::quant_graph::QuantGraphIdentity {
            artifact_core_hash: hash(1),
            policy_resolution_self_hash: hash(2),
            artifact_validation_self_hash: hash(3),
            semantic_core_hash: hash(4),
            lowering_manifest_hash: hash(5),
            determinism: DeterminismClass::BitExact,
            model_spec_summary,
        }
    }

    fn wave4_tensor(
        tensor_id: u32,
        role: QuantTensorRole,
        quant_format: QuantFormat,
        dims: &[u32],
    ) -> QuantTensorRef {
        QuantTensorRef {
            tensor_id: TensorId::new(tensor_id),
            layout: wave4_layout(dims),
            quant_format,
            role,
            blob: wave4_resolved_blob_ref(tensor_id as u8),
            aux_blob_refs: Vec::new(),
        }
    }

    fn wave4_expert_weight_tensor(
        tensor_id: u32,
        layer: LayerId,
        expert: ExpertId,
        slot: ExpertWeightSlot,
    ) -> QuantTensorRef {
        wave4_tensor(
            tensor_id,
            QuantTensorRole::ExpertWeight {
                layer,
                expert,
                slot,
            },
            QuantFormat::I8,
            &[8, 16],
        )
    }

    fn wave4_norm_record(
        id: u32,
        site: NormSite,
        input_format: QuantFormat,
        output_format: QuantFormat,
    ) -> crate::s1::quant_graph::NormPlanRecord {
        crate::s1::quant_graph::NormPlanRecord {
            norm_plan_id: NormPlanId::new(id),
            site,
            plan: NormPlan::TileRmsThenAffineClip(TileRmsThenAffineClipPlan {
                tile: NormTileRmsSpec {
                    tile_width: 8,
                    epsilon: 1.0e-5,
                },
                affine: NormAffineParams {
                    scale: 1.0,
                    bias: 0.0,
                },
                clip: NormClipBounds { lo: -2.0, hi: 2.0 },
            }),
            input_format,
            output_format,
        }
    }

    fn affine_clip_lut_plan() -> NormPlan {
        NormPlan::AffineClipLut(AffineClipLutPlan {
            affine: NormAffineParams {
                scale: 1.0,
                bias: 0.0,
            },
            clip: NormClipBounds { lo: -2.0, hi: 2.0 },
            lut: NormLutSpec {
                input_lo: -2.0,
                input_hi: 2.0,
                entries: 16,
            },
        })
    }

    fn wave4_decode_spec(requires_rng: bool) -> DecodeSpecRecord {
        DecodeSpecRecord {
            decode_plan_id: DecodePlanId::new(0),
            spec: if requires_rng {
                DecodeSpec::TopKTemperature {
                    k: 3,
                    temperature_q8_8: 256,
                }
            } else {
                DecodeSpec::Argmax
            },
            requires_rng,
        }
    }

    fn wave4_classify_head() -> ClassifyHead {
        ClassifyHead {
            kind: ClassifyHeadKind::Tied,
            weight: TensorId::new(1),
            bias: None,
            logit_format: QuantFormat::Q8_8,
        }
    }

    fn wave4_residual_plan() -> ResidualPlan {
        ResidualPlan {
            activation_format: QuantFormat::Q8_8,
            combine_policy: ResidualCombinePolicy::AddThenClampNamedBoundary,
        }
    }

    fn wave4_layout(dims: &[u32]) -> CanonicalTensorLayout {
        CanonicalTensorLayout::new(
            CanonicalTensorShape::new(dims.to_vec()).expect("shape is valid"),
            TensorElementType::Q8_8,
        )
    }

    fn wave4_resolved_blob_ref(byte: u8) -> ResolvedBlobRef {
        ResolvedBlobRef {
            blob_ref: BlobRef {
                hash: hash(byte),
                len: 1,
                codec: BlobCodec::Raw,
            },
            content_hash: hash(byte),
            encoded_size_bytes: 1,
            decoded_size_bytes: 1,
            codec: BlobCodec::Raw,
        }
    }

    fn gb_infer_ir_fixture() -> Result<GbInferIR, GbInferIrTypeError> {
        GbInferIR::new(
            identity(),
            vec![token_input()],
            vec![embedding_node()],
            vec![value_decl(), embedding_value_decl()],
            Vec::new(),
            provenance_fixture(),
            NodeAnchorMap::from([(NodeId::new(0), semantic_anchor(8))]),
        )
    }

    fn identity() -> InferIrIdentity {
        InferIrIdentity {
            quant_graph_self_hash: hash(1),
            infer_ir_policy_projection_hash: hash(2),
            static_budget_self_hash: hash(3),
            requested_runtime_modes_hash: hash(4),
            determinism: DeterminismClass::BitExact,
            topological_order_hash: hash(6),
        }
    }

    fn token_input() -> TokenInput {
        TokenInput::new(
            TokenInputId::new(0),
            ValueId::new(0),
            BTreeSet::from([TokenIngressMode::AutoRegressive, TokenIngressMode::Prompt]),
        )
        .expect("fixture ingress modes are non-empty")
    }

    fn value_decl() -> ValueDecl {
        ValueDecl {
            value_id: ValueId::new(0),
            kind: ValueKind::InputToken,
            format: ValueFormat::TokenIdDomain { vocab_size: 257 },
            layout: ValueLayout::scalar(),
        }
    }

    fn embedding_value_decl() -> ValueDecl {
        ValueDecl {
            value_id: ValueId::new(1),
            kind: ValueKind::EmbeddingOutput,
            format: ValueFormat::Quant {
                format: QuantFormat::Q8_8,
            },
            layout: ValueLayout {
                shape: vec![ValueAxis::Model],
            },
        }
    }

    fn embedding_node() -> GbNode {
        GbNode {
            node_id: NodeId::new(0),
            op: InferOp::Embedding {
                token_input: TokenInputId::new(0),
            },
            inputs: vec![ValueId::new(0)],
            effects_in: Vec::new(),
            outputs: vec![ValueId::new(1)],
            effects_out: Vec::new(),
            reduction_site: None,
        }
    }

    fn provenance_fixture() -> InferIrProvenance {
        InferIrProvenance {
            nodes: BTreeMap::from([(NodeId::new(0), QuantGraphEntityRef::Embedding)]),
            values: BTreeMap::from([
                (
                    ValueId::new(0),
                    ValueProducerRef::External {
                        token_input: TokenInputId::new(0),
                    },
                ),
                (
                    ValueId::new(1),
                    ValueProducerRef::Node {
                        node: NodeId::new(0),
                    },
                ),
            ]),
            effects: BTreeMap::new(),
        }
    }

    fn canonical_provenance_tuple_fixture() -> CanonicalProvenanceTuple {
        CanonicalProvenanceTuple::new(InferOpTag::Embedding, 0)
    }

    fn semantic_anchor(byte: u8) -> SemanticAnchor {
        SemanticAnchor::new(hash(byte))
    }

    fn all_quant_graph_entity_refs() -> [QuantGraphEntityRef; 15] {
        [
            QuantGraphEntityRef::Embedding,
            QuantGraphEntityRef::NormPlan {
                plan: NormPlanId::new(0),
            },
            QuantGraphEntityRef::NormSite {
                site: NormSite::Final,
            },
            QuantGraphEntityRef::RouterLayer {
                layer: LayerId::new(0),
            },
            QuantGraphEntityRef::RouterTensor {
                layer: LayerId::new(0),
                tensor: TensorId::new(0),
            },
            QuantGraphEntityRef::RouterSelection {
                layer: LayerId::new(0),
            },
            QuantGraphEntityRef::ExpertSection {
                layer: LayerId::new(0),
                expert: ExpertId::new(0),
            },
            QuantGraphEntityRef::ExpertTensor {
                layer: LayerId::new(0),
                expert: ExpertId::new(0),
                slot: ExpertWeightSlot::FfnUp,
                tensor: TensorId::new(1),
            },
            QuantGraphEntityRef::FfnActivationSite {
                layer: LayerId::new(0),
                expert: ExpertId::new(0),
            },
            QuantGraphEntityRef::ResidualSiteRef {
                layer: Some(LayerId::new(0)),
                site: ResidualSite::PostFfn,
            },
            QuantGraphEntityRef::DecodePlan {
                plan: DecodePlanId::new(0),
            },
            QuantGraphEntityRef::ClassifyHead,
            QuantGraphEntityRef::SequenceSlot {
                slot: StateSlotId::new(0),
            },
            QuantGraphEntityRef::SequenceStep {
                layer: LayerId::new(0),
            },
            QuantGraphEntityRef::TokenInput {
                token_input: TokenInputId::new(0),
            },
        ]
    }

    fn all_infer_ops() -> [InferOp; 13] {
        [
            InferOp::Classify,
            InferOp::CombineResidual {
                layer: Some(LayerId::new(0)),
                site: ResidualSite::PostFfn,
            },
            InferOp::DecodeToken {
                plan: DecodePlanId::new(0),
            },
            InferOp::Embedding {
                token_input: TokenInputId::new(0),
            },
            expert_matvec_op(),
            InferOp::FfnActivation {
                layer: LayerId::new(0),
                expert: ExpertId::new(1),
            },
            InferOp::Norm {
                plan: NormPlanId::new(0),
            },
            InferOp::RouteTop1 {
                layer: LayerId::new(0),
            },
            InferOp::RouterMatVec {
                layer: LayerId::new(0),
            },
            InferOp::SelectExpertTop1 {
                layer: LayerId::new(0),
            },
            InferOp::SequenceRead {
                slot: StateSlotId::new(0),
            },
            InferOp::SequenceStep {
                layer: LayerId::new(0),
            },
            InferOp::SequenceWrite {
                slot: StateSlotId::new(0),
            },
        ]
    }

    fn expert_matvec_op() -> InferOp {
        InferOp::ExpertMatVec {
            layer: LayerId::new(0),
            expert: ExpertId::new(1),
            slot: ExpertWeightSlot::FfnUp,
        }
    }

    fn store() -> (tempfile::TempDir, BlobStore) {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = BlobStore::open(dir.path().to_path_buf()).expect("blob store");
        (dir, store)
    }

    struct NoopResolver;

    impl ArtifactResolver for NoopResolver {
        fn resolve_blob(&self, blob: &BlobRef) -> Result<ResolvedBlob, ArtifactResolveError> {
            Err(ArtifactResolveError::Unsupported {
                message: format!("Stage 3 driver test resolver does not resolve blob {blob:?}"),
            })
        }

        fn resolve_sidecar(
            &self,
            sidecar: &SidecarRef,
        ) -> Result<ResolvedSidecar, ArtifactResolveError> {
            Err(ArtifactResolveError::Unsupported {
                message: format!(
                    "Stage 3 driver test resolver does not resolve sidecar {sidecar:?}"
                ),
            })
        }

        fn resolve_evidence(
            &self,
            evidence: &EvidenceRef,
        ) -> Result<ResolvedEvidence, ArtifactResolveError> {
            Err(ArtifactResolveError::Unsupported {
                message: format!(
                    "Stage 3 driver test resolver does not resolve evidence {evidence:?}"
                ),
            })
        }

        fn resolve_workload(
            &self,
            workload: &WorkloadManifestRef,
        ) -> Result<ResolvedWorkload, ArtifactResolveError> {
            Err(ArtifactResolveError::Unsupported {
                message: format!(
                    "Stage 3 driver test resolver does not resolve workload {workload:?}"
                ),
            })
        }

        fn resolve_golden_vector(
            &self,
            vector: &GoldenVectorRef,
        ) -> Result<ResolvedGoldenVector, ArtifactResolveError> {
            Err(ArtifactResolveError::Unsupported {
                message: format!(
                    "Stage 3 driver test resolver does not resolve golden vector {vector:?}"
                ),
            })
        }
    }

    fn hash(byte: u8) -> Hash256 {
        Hash256::from_bytes([byte; 32])
    }

    #[cfg(feature = "semantic_equivalence_check")]
    #[derive(Clone, Debug, Default)]
    struct TraceCapture {
        records: Arc<Mutex<Vec<TraceRecord>>>,
    }

    #[cfg(feature = "semantic_equivalence_check")]
    impl TraceCapture {
        fn records(&self) -> Vec<TraceRecord> {
            self.records
                .lock()
                .expect("trace capture mutex is not poisoned")
                .clone()
        }
    }

    #[cfg(feature = "semantic_equivalence_check")]
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

    #[cfg(feature = "semantic_equivalence_check")]
    #[derive(Clone, Debug, PartialEq, Eq)]
    struct TraceRecord {
        level: String,
        fields: BTreeMap<String, String>,
    }

    #[cfg(feature = "semantic_equivalence_check")]
    impl TraceRecord {
        fn field_equals(&self, field: &str, expected: &str) -> bool {
            self.fields
                .get(field)
                .is_some_and(|value| value == expected)
        }
    }

    #[cfg(feature = "semantic_equivalence_check")]
    #[derive(Debug, Default)]
    struct TraceFieldVisitor {
        fields: BTreeMap<String, String>,
    }

    #[cfg(feature = "semantic_equivalence_check")]
    impl TraceFieldVisitor {
        fn insert(&mut self, field: &tracing::field::Field, value: String) {
            self.fields.insert(field.name().to_owned(), value);
        }
    }

    #[cfg(feature = "semantic_equivalence_check")]
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

    fn assert_forbidden_keys_absent(value: &Value, forbidden: &[&str]) {
        if let Some(path) = find_forbidden_key(value, forbidden, "$") {
            panic!("forbidden key found at {path}");
        }
    }

    fn find_forbidden_key(value: &Value, forbidden: &[&str], path: &str) -> Option<String> {
        match value {
            Value::Object(map) => {
                for (key, nested) in map {
                    let nested_path = format!("{path}.{key}");
                    if forbidden.contains(&key.as_str()) {
                        return Some(nested_path);
                    }
                    if let Some(found) = find_forbidden_key(nested, forbidden, &nested_path) {
                        return Some(found);
                    }
                }
                None
            }
            Value::Array(values) => values.iter().enumerate().find_map(|(index, nested)| {
                find_forbidden_key(nested, forbidden, &format!("{path}[{index}]"))
            }),
            _ => None,
        }
    }
}
