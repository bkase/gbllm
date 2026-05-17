//! Stage 4 ObservationPlan core types and identity.
#![allow(clippy::large_enum_variant, clippy::result_large_err)]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::io;
use std::marker::PhantomData;
use std::path::Path;
use std::time::Instant;

use gbf_abi::{
    CompactCheckpointId, ProbeLevel, SemanticCheckpointId, SemanticCheckpointSchema,
    SemanticStratum, TraceBudget, TraceDropPolicy as AbiTraceDropPolicy,
};
use gbf_foundation::{
    CompileProfileId, EvidenceRef, ExpertId, FieldPath, Hash256, LayerId, WorkloadId,
};
use gbf_policy::MetricSource as RegistryMetricSource;
use gbf_policy::{
    ABI_TRACE_EVENT_PAYLOAD_BYTES, DiagnosticSeverity, EffectClass as PolicyEffectClass,
    InferOpTag as PolicyInferOpTag, MetricAggregation, MetricId, MetricRegistryEntry,
    MetricRegistrySnapshot, ObservabilityMode, ObservationProfileCaps, ProbeImportanceClass,
    ProbeRegistryEntry, ProbeRegistrySnapshot, ProbeSourceSelector, ProbeTiming,
    TraceBudget as PolicyTraceBudget, TraceDropPolicy as PolicyTraceDropPolicy,
    TraceEventLayoutRegistrySnapshot, TraceEventShape, ValidationCode, ValidationDetail,
    ValidationOrigin, ValueRole as PolicyValueRole,
};
use gbf_policy::{TraceFrequencyBound, TraceProbeId};
use gbf_report::{
    ReportBody, ReportEnvelope, ReportOutcome, ValidationDiagnostic,
    canonicalize as canonicalize_report, canonicalize_value,
};
use gbf_store::stage_cache::StageCache as StoreStageCache;
use gbf_workload::manifest as workload_manifest;
use serde::de::{MapAccess, Visitor};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::s1::quant_graph::{DeterminismClass, ExpertWeightSlot, NormSite};
use crate::s3::infer_ir::{
    CanonicalProvenanceTuple, EffectClass, EffectClassTag, EffectId, GbInferIR, GbInferIRProduct,
    GbNode, InferOp, InferOpTag, NodeId, ResidualSite, RngSlot, SemanticAnchor, StateSlotId,
    ValueId, ValueKind,
};
use crate::stage_cache::{
    CodegenStageCacheError, Stage4CacheKeyMaterial, Stage4ReportRewrapContext,
    get_stage4_failure_memo, get_stage4_success, put_stage4_failure_memo, put_stage4_success,
    rewrap_stage4_cached_failure, rewrap_stage4_cached_success,
};

pub const OBSERVATION_PLAN_SCHEMA_VERSION: &str = "observation_plan.v1";
pub const BUILD_ACTIVE_CHECKPOINT_SCHEMA_VERSION: &str =
    "build_active_semantic_checkpoint_schema.v1";
pub const OPERATIONAL_PROBE_SCHEMA_VERSION: &str = "operational_probe_schema.v1";
pub const OBSERVATION_REPORT_SCHEMA_SEMVER: &str = "1.0.0";
pub const OBSERVATION_POLICY_PROJECTION_HASH_COMPUTED_EVENT: &str =
    "gbf_codegen.observation_policy_projection.hash_computed";
pub const OBSERVATION_CORE_PRODUCT_HASH_COMPUTED_EVENT: &str =
    "gbf_codegen.observation_plan.core_product.hash_computed";
pub const OBSERVATION_CORE_PRODUCT_AUDIT_DRIFT_DETECTED_EVENT: &str =
    "gbf_codegen.observation_plan.core_product.audit_drift_detected";
pub const OBSERVATION_SC_HASH_MISMATCH_CODE: &str = "OBSERVATION-SC-HASH-MISMATCH";
pub const OBSERVATION_DETERMINISM_MISMATCH_CODE: &str = "OBSERVATION-DETERMINISM-MISMATCH";
pub const OBSERVATION_COMPARE_DOMAIN_MISMATCH_CODE: &str = "OBSERVATION-COMPARE-DOMAIN-MISMATCH";
pub const OBSERVATION_WORKLOAD_DETERMINISM_MISMATCH_CODE: &str =
    "OBSERVATION-WORKLOAD-DETERMINISM-MISMATCH";
pub const OBSERVATION_POLICY_WORKLOAD_DETERMINISM_MISMATCH_CODE: &str =
    "OBSERVATION-POLICY-WORKLOAD-DETERMINISM-MISMATCH";
pub const OBSERVATION_MANDATORY_CHECKPOINT_NOT_FEASIBLE_CODE: &str =
    "OBSERVATION-MANDATORY-CHECKPOINT-NOT-FEASIBLE";
pub const OBSERVATION_WORKLOAD_CHECKPOINT_NOT_FEASIBLE_CODE: &str =
    "OBSERVATION-WORKLOAD-CHECKPOINT-NOT-FEASIBLE";
pub const OBSERVATION_CHECKPOINT_NOT_IN_SCHEMA_CODE: &str = "OBSERVATION-CHECKPOINT-NOT-IN-SCHEMA";
pub const OBSERVATION_CHECKPOINT_NOT_ATTACHABLE_CODE: &str =
    "OBSERVATION-CHECKPOINT-NOT-ATTACHABLE";
pub const OBSERVATION_CHECKPOINT_AMBIGUOUS_CODE: &str = "OBSERVATION-CHECKPOINT-AMBIGUOUS";
pub const OBSERVATION_ENCODING_INVALID_FOR_CHECKPOINT_CODE: &str =
    "OBSERVATION-ENCODING-INVALID-FOR-CHECKPOINT";
pub const OBSERVATION_PROBE_ID_UNKNOWN_CODE: &str = "OBSERVATION-PROBE-ID-UNKNOWN";
pub const OBSERVATION_METRIC_ID_UNKNOWN_CODE: &str = "OBSERVATION-METRIC-ID-UNKNOWN";
pub const OBSERVATION_REQUIRED_PROBE_DISABLED_CODE: &str = "OBSERVATION-REQUIRED-PROBE-DISABLED";
pub const OBSERVATION_METRIC_SOURCE_RESERVED_V1_CODE: &str =
    "OBSERVATION-METRIC-SOURCE-RESERVED-V1";
pub const OBSERVATION_METRIC_HISTOGRAM_BUCKET_COUNT_ZERO_CODE: &str =
    "OBSERVATION-METRIC-HISTOGRAM-BUCKET-COUNT-ZERO";
pub const OBSERVATION_PROBE_SOURCE_INVALID_CODE: &str = "OBSERVATION-PROBE-SOURCE-INVALID";
pub const OBSERVATION_RESERVED_EFFECT_PROBE_CODE: &str = "OBSERVATION-RESERVED-EFFECT-PROBE";
pub const OBSERVATION_SEQUENCE_STATE_PROBE_RESERVED_CODE: &str =
    "OBSERVATION-SEQUENCE-STATE-PROBE-RESERVED";
pub const OBSERVATION_FAULT_BOUNDARY_PROBE_RESERVED_CODE: &str =
    "OBSERVATION-FAULT-BOUNDARY-PROBE-RESERVED";
pub const OBSERVATION_PROBE_CLASS_CAP_EXCEEDED_CODE: &str = "OBSERVATION-PROBE-CLASS-CAP-EXCEEDED";
pub const OBSERVATION_INVARIANT_MODE_BUDGET_BUSTED_CODE: &str =
    "OBSERVATION-INVARIANT-MODE-BUDGET-BUSTED";
pub const OBSERVATION_IDENTITY_BIND_EVENT: &str = "stage4.observation_plan.identity_bind";
pub const OBSERVATION_SCHEMA_INGEST_EVENT: &str = "stage4.observation_plan.schema_ingest";
pub const OBSERVATION_BUILD_FEASIBILITY_FILTER_EVENT: &str =
    "stage4.observation_plan.build_feasibility_filter";
pub const OBSERVATION_SEMANTIC_SELECTION_EVENT: &str = "stage4.observation_plan.semantic_selection";
pub const OBSERVATION_SEMANTIC_ANCHOR_BINDING_EVENT: &str =
    "stage4.observation_plan.semantic_anchor_binding";
pub const OBSERVATION_ENCODING_BINDING_EVENT: &str =
    "stage4.observation_plan.observation_encoding_binding";
/// Emitted once per probe registry entry/template; `instantiated_count`
/// records how many concrete `OperationalProbe` instances that entry produced.
pub const OBSERVATION_PROBE_REGISTRY_INSTANTIATION_EVENT: &str =
    "stage4.observation_plan.probe_registry_instantiation";
pub const OBSERVATION_PROBE_BUDGET_GOVERNANCE_EVENT: &str =
    "stage4.observation_plan.probe_budget_governance";
pub const OBSERVATION_PROBE_ORDERING_EVENT: &str = "stage4.observation_plan.probe_ordering";
pub const OBSERVATION_METRIC_REGISTRY_FILTER_EVENT: &str =
    "stage4.observation_plan.metric_registry_filter";
pub const OBSERVATION_METRIC_SELECTION_EVENT: &str = "stage4.observation_plan.metric_selection";
pub const OBSERVATION_METRIC_ORDERING_EVENT: &str = "stage4.observation_plan.metric_ordering";
pub const OBSERVATION_ANCHOR_TABLE_BIND_EVENT: &str = "stage4.observation_plan.anchor_table_bind";
pub const OBSERVATION_PROVENANCE_BIND_EVENT: &str = "stage4.observation_plan.provenance_bind";
pub const OBSERVATION_SCHEMA_RE_EMIT_EVENT: &str = "stage4.observation_plan.schema_re_emit";
pub const OBSERVATION_OPERATIONAL_PROBE_SCHEMA_EMIT_EVENT: &str =
    "stage4.observation_plan.operational_probe_schema_emit";
pub const OBSERVATION_INVARIANT_BUDGET_CHECK_EVENT: &str =
    "stage4.observation_plan.invariant_budget_check";
pub const OBSERVATION_SELF_CONSISTENCY_EVENT: &str = "stage4.observation_plan.self_consistency";
pub const OBSERVATION_CANONICAL_SORT_EVENT: &str = "stage4.observation_plan.canonical_sort";
pub const STAGE4_DRIVER_REPORT_EMIT_EVENT: &str = "stage4.driver.report_emit";
pub const STAGE4_DRIVER_FAILURE_MEMO_EVENT: &str = "stage4.driver.failure_memo";
pub const STAGE4_DRIVER_RUN_EVENT: &str = "stage4.driver.run";

#[cfg(test)]
thread_local! {
    static FINALIZATION_EVENT_LOG: std::cell::RefCell<Vec<&'static str>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

#[cfg(test)]
fn record_finalization_event(event: &'static str) {
    FINALIZATION_EVENT_LOG.with(|log| log.borrow_mut().push(event));
}

#[cfg(not(test))]
fn record_finalization_event(_event: &'static str) {}

#[cfg(test)]
fn take_recorded_finalization_events() -> Vec<&'static str> {
    FINALIZATION_EVENT_LOG.with(|log| std::mem::take(&mut *log.borrow_mut()))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationPlanInputs {
    pub infer_ir_product: GbInferIRProduct,
    pub infer_ir_self_hash: Hash256,
    pub quant_graph_self_hash: Hash256,
    pub semantic_checkpoint_schema: SemanticCheckpointSchema,
    pub semantic_checkpoint_schema_hash: Hash256,
    pub artifact_declared_semantic_checkpoint_schema_hash: Hash256,
    pub probe_registry: ProbeRegistrySnapshot,
    pub probe_registry_hash: Hash256,
    pub metric_registry: MetricRegistrySnapshot,
    pub metric_registry_hash: Hash256,
    pub trace_event_layout_registry: TraceEventLayoutRegistrySnapshot,
    pub trace_event_layout_registry_hash: Hash256,
    pub op_policy_projection: ObservationPolicyProjection,
    pub audit_parents: ObservationPlanAuditParents,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationPolicyProjection {
    pub profile_id: CompileProfileId,
    pub profile_observation_caps: ObservationProfileCaps,
    pub determinism_class: DeterminismClass,
    pub observability_mode: ObservabilityMode,
    pub trace_budget: PolicyTraceBudget,
    pub trace_demotion: TraceDemotionLevel,
    pub optional_probe_floor: ProbeImportanceClass,
    pub workload_observation: WorkloadObservationProjection,
    pub disabled_optional_probes: BTreeSet<TraceProbeId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkloadObservationProjection {
    pub workload_id: WorkloadId,
    pub checkpoint_selection: workload_manifest::CheckpointSelection,
    pub trace_level: workload_manifest::TraceLevel,
    pub compare_domain_workload: workload_manifest::CompareDomain,
    pub compare_domain_policy: CompareDomain,
    pub determinism_requirement: workload_manifest::DeterminismRequirement,
    pub determinism_class_v1: DeterminismClass,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum CompareDomain {
    CanonicalValue,
    TokenIdOnly,
    ExpertIdOnly,
    EnvelopeQ8_8,
    EnvelopeQ16_16,
}

impl From<workload_manifest::CompareDomain> for CompareDomain {
    fn from(value: workload_manifest::CompareDomain) -> Self {
        match value {
            workload_manifest::CompareDomain::TokenLogits => Self::CanonicalValue,
            workload_manifest::CompareDomain::GeneratedBytes => Self::TokenIdOnly,
        }
    }
}

impl From<workload_manifest::DeterminismRequirement> for DeterminismClass {
    fn from(value: workload_manifest::DeterminismRequirement) -> Self {
        match value {
            workload_manifest::DeterminismRequirement::SeededDecode => Self::BitExact,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum TraceDemotionLevel {
    None,
    DropBestEffort,
    DropDiagnosticAndBestEffort,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationPlanAuditParents {
    pub policy_resolution_self_hash: Hash256,
    pub compile_request_hash: Hash256,
    pub static_budget_self_hash: Hash256,
    pub artifact_aux_hash: Hash256,
    pub locked_observation_knobs: LockedObservationKnobs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockedObservationKnobs {
    pub trace_demotion_locked: bool,
    pub optional_probe_floor_locked: bool,
    pub probe_selection_locked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObservationPlanInputError {
    SemanticCheckpointSchemaHashMismatch {
        expected: Hash256,
        observed: Hash256,
    },
    DeterminismMismatch {
        projection: DeterminismClass,
        infer_ir: DeterminismClass,
    },
    CompareDomainProjectionDrift {
        workload: workload_manifest::CompareDomain,
        expected_policy: CompareDomain,
        observed_policy: CompareDomain,
    },
    WorkloadDeterminismProjectionDrift {
        requirement: workload_manifest::DeterminismRequirement,
        expected_class: DeterminismClass,
        observed_class: DeterminismClass,
    },
    PolicyWorkloadDeterminismDrift {
        policy: DeterminismClass,
        workload: DeterminismClass,
    },
}

impl ObservationPlanInputError {
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::SemanticCheckpointSchemaHashMismatch { .. } => OBSERVATION_SC_HASH_MISMATCH_CODE,
            Self::DeterminismMismatch { .. } => OBSERVATION_DETERMINISM_MISMATCH_CODE,
            Self::CompareDomainProjectionDrift { .. } => OBSERVATION_COMPARE_DOMAIN_MISMATCH_CODE,
            Self::WorkloadDeterminismProjectionDrift { .. } => {
                OBSERVATION_WORKLOAD_DETERMINISM_MISMATCH_CODE
            }
            Self::PolicyWorkloadDeterminismDrift { .. } => {
                OBSERVATION_POLICY_WORKLOAD_DETERMINISM_MISMATCH_CODE
            }
        }
    }
}

pub fn validate_observation_plan_inputs(
    inputs: &ObservationPlanInputs,
) -> Result<(), ObservationPlanInputError> {
    if inputs.semantic_checkpoint_schema_hash
        != inputs.artifact_declared_semantic_checkpoint_schema_hash
    {
        return Err(
            ObservationPlanInputError::SemanticCheckpointSchemaHashMismatch {
                expected: inputs.artifact_declared_semantic_checkpoint_schema_hash,
                observed: inputs.semantic_checkpoint_schema_hash,
            },
        );
    }

    let infer_ir_determinism = inputs.infer_ir_product.infer_ir.identity.determinism;
    let workload_observation = &inputs.op_policy_projection.workload_observation;
    let expected_compare_domain = CompareDomain::from(workload_observation.compare_domain_workload);
    if workload_observation.compare_domain_policy != expected_compare_domain {
        return Err(ObservationPlanInputError::CompareDomainProjectionDrift {
            workload: workload_observation.compare_domain_workload,
            expected_policy: expected_compare_domain,
            observed_policy: workload_observation.compare_domain_policy,
        });
    }

    let expected_workload_determinism =
        DeterminismClass::from(workload_observation.determinism_requirement);
    if workload_observation.determinism_class_v1 != expected_workload_determinism {
        return Err(
            ObservationPlanInputError::WorkloadDeterminismProjectionDrift {
                requirement: workload_observation.determinism_requirement,
                expected_class: expected_workload_determinism,
                observed_class: workload_observation.determinism_class_v1,
            },
        );
    }

    let policy_determinism = inputs.op_policy_projection.determinism_class;
    if policy_determinism != workload_observation.determinism_class_v1 {
        return Err(ObservationPlanInputError::PolicyWorkloadDeterminismDrift {
            policy: policy_determinism,
            workload: workload_observation.determinism_class_v1,
        });
    }

    if policy_determinism != infer_ir_determinism {
        return Err(ObservationPlanInputError::DeterminismMismatch {
            projection: policy_determinism,
            infer_ir: infer_ir_determinism,
        });
    }

    Ok(())
}

pub fn observation_policy_projection_hash(
    projection: &ObservationPolicyProjection,
) -> Result<Hash256, serde_json::Error> {
    let hash = domain_hash(
        "ObservationPolicyProjection",
        OBSERVATION_PLAN_SCHEMA_VERSION,
        projection,
    )?;
    tracing::info!(
        event = %OBSERVATION_POLICY_PROJECTION_HASH_COMPUTED_EVENT,
        hash = %hash,
    );
    Ok(hash)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationPlan {
    pub identity: ObservationPlanIdentity,
    pub semantic: Vec<SemanticObservation>,
    pub probes: Vec<OperationalProbe>,
    pub metrics: Vec<MetricProbe>,
    pub anchor_table: AnchorAttachmentTable,
    pub provenance: ObservationProvenance,
    pub trace_budget_projection: TraceBudgetProjection,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationPlanIdentity {
    pub infer_ir_self_hash: Hash256,
    pub quant_graph_self_hash: Hash256,
    pub semantic_checkpoint_schema_hash: Hash256,
    pub observation_policy_projection_hash: Hash256,
    pub determinism: DeterminismClass,
    pub observability_mode: ObservabilityMode,
    pub trace_budget: TraceBudget,
    pub workload_id: WorkloadId,
    pub probe_registry_hash: Hash256,
    pub metric_registry_hash: Hash256,
    pub trace_event_layout_registry_hash: Hash256,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationPlanCoreProduct {
    pub observation_plan: ObservationPlan,
    pub observation_plan_self_hash: Hash256,
    pub build_active_checkpoint_schema: BuildActiveCheckpointSchema,
    pub build_active_checkpoint_schema_hash: Hash256,
    pub operational_probe_schema: OperationalProbeSchema,
    pub operational_probe_schema_hash: Hash256,
}

pub type ObservationPlanProduct = ObservationPlanCoreProduct;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationPlanStageOutput {
    pub product: ObservationPlanCoreProduct,
    pub report: ReportEnvelope<ObservationPlanReportBody>,
    pub sc_re_emit_report: ReportEnvelope<SemanticCheckpointSchemaReEmitBody>,
    pub operational_probe_report: ReportEnvelope<OperationalProbeSchemaBody>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationPlanStageFailure {
    pub report: ReportEnvelope<ObservationPlanReportBody>,
    pub sc_re_emit_report: Option<ReportEnvelope<SemanticCheckpointSchemaReEmitBody>>,
    pub operational_probe_report: Option<ReportEnvelope<OperationalProbeSchemaBody>>,
    pub diagnostics: NonEmptyList<ValidationDiagnostic>,
}

#[derive(Debug)]
pub enum RunStage4Error {
    StageFailure(ObservationPlanStageFailure),
    StageCache(CodegenStageCacheError),
    ReportIo(io::Error),
}

#[derive(Clone, Copy)]
pub struct Stage4PassEnvironment<'a> {
    pub resolved_observability_mode: ObservabilityMode,
    pub report_dir: Option<&'a Path>,
    pub stage_cache: Option<&'a StoreStageCache<'a>>,
}

impl<'a> Stage4PassEnvironment<'a> {
    #[must_use]
    pub const fn new(resolved_observability_mode: ObservabilityMode) -> Self {
        Self {
            resolved_observability_mode,
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

impl RunStage4Error {
    #[must_use]
    pub const fn stage_failure(&self) -> Option<&ObservationPlanStageFailure> {
        match self {
            Self::StageFailure(failure) => Some(failure),
            Self::StageCache(_) | Self::ReportIo(_) => None,
        }
    }
}

impl fmt::Display for RunStage4Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StageFailure(failure) => write!(
                f,
                "Stage 4 observation plan failed with {} diagnostic(s)",
                failure.diagnostics.as_slice().len()
            ),
            Self::StageCache(err) => write!(f, "Stage 4 cache error: {err}"),
            Self::ReportIo(err) => write!(f, "Stage 4 report I/O error: {err}"),
        }
    }
}

impl std::error::Error for RunStage4Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::StageFailure(_) => None,
            Self::StageCache(err) => Some(err),
            Self::ReportIo(err) => Some(err),
        }
    }
}

impl From<CodegenStageCacheError> for RunStage4Error {
    fn from(value: CodegenStageCacheError) -> Self {
        Self::StageCache(value)
    }
}

impl From<io::Error> for RunStage4Error {
    fn from(value: io::Error) -> Self {
        Self::ReportIo(value)
    }
}

impl From<serde_json::Error> for RunStage4Error {
    fn from(value: serde_json::Error) -> Self {
        Self::ReportIo(io::Error::other(value.to_string()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationPlanCoreSuccess {
    pub product: ObservationPlanCoreProduct,
    pub observation_plan_body: ObservationPlanReportBody,
    pub sc_re_emit_body: SemanticCheckpointSchemaReEmitBody,
    pub operational_probe_body: OperationalProbeSchemaBody,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationPlanCoreFailure {
    pub observation_plan_body: ObservationPlanReportBody,
    pub sc_re_emit_body: Option<SemanticCheckpointSchemaReEmitBody>,
    pub operational_probe_body: Option<OperationalProbeSchemaBody>,
    pub diagnostics: NonEmptyList<ValidationDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NonEmptyList<T> {
    items: Vec<T>,
}

impl<T> NonEmptyList<T> {
    pub fn new(items: Vec<T>) -> Result<Self, NonEmptyListError> {
        if items.is_empty() {
            return Err(NonEmptyListError);
        }
        Ok(Self { items })
    }

    #[must_use]
    pub fn as_slice(&self) -> &[T] {
        &self.items
    }

    #[must_use]
    pub fn into_vec(self) -> Vec<T> {
        self.items
    }
}

impl<T> AsRef<[T]> for NonEmptyList<T> {
    fn as_ref(&self) -> &[T] {
        self.as_slice()
    }
}

impl<'de, T> Deserialize<'de> for NonEmptyList<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct NonEmptyListWire<T> {
            items: Vec<T>,
        }

        let wire = NonEmptyListWire::deserialize(deserializer)?;
        Self::new(wire.items)
            .map_err(|_| serde::de::Error::custom("NonEmptyList must contain at least one item"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NonEmptyListError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildActiveCheckpointSchema {
    pub checkpoints: Vec<ReEmittedCheckpointEntry>,
    pub build_active_count: u16,
    pub mandatory_count: u16,
    pub optional_count: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationalProbeSchema {
    pub probes: Vec<ProbeSchemaEntry>,
    pub metrics: Vec<MetricSchemaEntry>,
    pub probe_count: u16,
    pub metric_count: u16,
    pub per_class_probe_weight_total: PerClassWeightTotal,
    pub per_class_metric_weight_total: PerClassWeightTotal,
    pub per_class_total_weight: PerClassWeightTotal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProbeSchemaEntry {
    pub instance_id: ProbeInstanceId,
    pub probe_id: TraceProbeId,
    pub level: ProbeLevel,
    pub importance: ProbeImportanceClass,
    pub event_shape: TraceEventShape,
    pub source: ProbeSource,
    pub weight: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetricSchemaEntry {
    pub metric: MetricId,
    pub aggregation: MetricAggregation,
    pub source: RegistryMetricSource,
    pub importance: ProbeImportanceClass,
    pub weight: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PerClassWeightTotal {
    pub required: u32,
    pub important: u32,
    pub diagnostic: u32,
    pub best_effort: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PerClassCount {
    pub required: u16,
    pub important: u16,
    pub diagnostic: u16,
    pub best_effort: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationPlanReportBody {
    pub input_identity: ObservationPlanReportInputIdentity,
    pub result: Option<ObservationPlanReportResult>,
    pub diagnostics: Vec<ValidationDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationPlanReportInputIdentity {
    pub infer_ir_self_hash: Hash256,
    pub quant_graph_self_hash: Hash256,
    pub semantic_checkpoint_schema_hash: Hash256,
    pub observation_policy_projection_hash: Hash256,
    pub static_budget_self_hash: Hash256,
    pub policy_resolution_self_hash: Hash256,
    pub compile_request_hash: Hash256,
    pub artifact_aux_hash: Hash256,
    pub determinism: DeterminismClass,
    pub observability_mode: ObservabilityMode,
    pub trace_budget: TraceBudget,
    pub profile_id: CompileProfileId,
    pub workload_id: WorkloadId,
}

impl ObservationPlanReportInputIdentity {
    #[must_use]
    pub fn from_inputs(inputs: &ObservationPlanInputs, identity: &ObservationPlanIdentity) -> Self {
        Self {
            infer_ir_self_hash: identity.infer_ir_self_hash,
            quant_graph_self_hash: identity.quant_graph_self_hash,
            semantic_checkpoint_schema_hash: identity.semantic_checkpoint_schema_hash,
            observation_policy_projection_hash: identity.observation_policy_projection_hash,
            static_budget_self_hash: inputs.audit_parents.static_budget_self_hash,
            policy_resolution_self_hash: inputs.audit_parents.policy_resolution_self_hash,
            compile_request_hash: inputs.audit_parents.compile_request_hash,
            artifact_aux_hash: inputs.audit_parents.artifact_aux_hash,
            determinism: identity.determinism,
            observability_mode: identity.observability_mode,
            trace_budget: identity.trace_budget,
            profile_id: inputs.op_policy_projection.profile_id.clone(),
            workload_id: identity.workload_id.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationPlanReportResult {
    pub product: ObservationPlan,
    pub semantic_count: u16,
    pub probe_count: u16,
    pub metric_count: u16,
    pub mandatory_semantic_count: u16,
    pub optional_semantic_count: u16,
    pub per_class_probe_count: PerClassCount,
    pub per_class_metric_count: PerClassCount,
    pub sc_re_emit_report_self_hash: Hash256,
    pub operational_probe_schema_report_self_hash: Hash256,
    pub observation_plan_self_hash: Hash256,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticCheckpointSchemaReEmitBody {
    pub input_identity: SemanticCheckpointSchemaReEmitInputIdentity,
    pub result: Option<SemanticCheckpointSchemaReEmitResult>,
    pub diagnostics: Vec<ValidationDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticCheckpointSchemaReEmitInputIdentity {
    pub observation_plan_self_hash: Option<Hash256>,
    pub original_schema_hash: Hash256,
    pub infer_ir_self_hash: Hash256,
    pub quant_graph_self_hash: Hash256,
    pub artifact_aux_hash: Hash256,
    pub determinism: DeterminismClass,
    pub workload_id: WorkloadId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticCheckpointSchemaReEmitResult {
    pub schema_hash: Hash256,
    pub checkpoints: Vec<ReEmittedCheckpointEntry>,
    pub build_active_count: u16,
    pub mandatory_count: u16,
    pub optional_count: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationalProbeSchemaBody {
    pub input_identity: OperationalProbeSchemaInputIdentity,
    pub result: Option<OperationalProbeSchemaResult>,
    pub diagnostics: Vec<ValidationDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationalProbeSchemaInputIdentity {
    pub observation_plan_self_hash: Option<Hash256>,
    pub infer_ir_self_hash: Hash256,
    pub quant_graph_self_hash: Hash256,
    pub determinism: DeterminismClass,
    pub observability_mode: ObservabilityMode,
    pub trace_budget: TraceBudget,
    pub profile_id: CompileProfileId,
    pub workload_id: WorkloadId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationalProbeSchemaResult {
    pub schema_hash: Hash256,
    pub probes: Vec<ProbeSchemaEntry>,
    pub metrics: Vec<MetricSchemaEntry>,
    pub probe_count: u16,
    pub metric_count: u16,
    pub per_class_probe_weight_total: PerClassWeightTotal,
    pub per_class_metric_weight_total: PerClassWeightTotal,
    pub per_class_total_weight: PerClassWeightTotal,
}

impl ReportBody for ObservationPlanReportBody {
    const REPORT_TYPE: &'static str = "ObservationPlanReport";
    const SCHEMA_ID: &'static str = OBSERVATION_PLAN_SCHEMA_VERSION;
    const SCHEMA_VERSION: &'static str = OBSERVATION_REPORT_SCHEMA_SEMVER;

    fn validate_semantics(&self, outcome: ReportOutcome) -> Result<(), Vec<ValidationDiagnostic>> {
        validate_product_report_body(outcome, self.result.is_some(), &self.diagnostics)
    }
}

impl ReportBody for SemanticCheckpointSchemaReEmitBody {
    const REPORT_TYPE: &'static str = "SemanticCheckpointSchemaReEmit";
    const SCHEMA_ID: &'static str = BUILD_ACTIVE_CHECKPOINT_SCHEMA_VERSION;
    const SCHEMA_VERSION: &'static str = OBSERVATION_REPORT_SCHEMA_SEMVER;

    fn validate_semantics(&self, outcome: ReportOutcome) -> Result<(), Vec<ValidationDiagnostic>> {
        validate_product_report_body(outcome, self.result.is_some(), &self.diagnostics)
    }
}

impl ReportBody for OperationalProbeSchemaBody {
    const REPORT_TYPE: &'static str = "OperationalProbeSchema";
    const SCHEMA_ID: &'static str = OPERATIONAL_PROBE_SCHEMA_VERSION;
    const SCHEMA_VERSION: &'static str = OBSERVATION_REPORT_SCHEMA_SEMVER;

    fn validate_semantics(&self, outcome: ReportOutcome) -> Result<(), Vec<ValidationDiagnostic>> {
        validate_product_report_body(outcome, self.result.is_some(), &self.diagnostics)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticObservation {
    pub checkpoint: SemanticCheckpointId,
    pub kind: SemanticCheckpointKind,
    pub compact: CompactCheckpointId,
    pub stratum: SemanticStratum,
    pub source: ObservationSource,
    pub encoding: ObservationEncoding,
    pub anchor: SemanticAnchor,
    pub artifact_role: SemanticCheckpointRole,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
#[allow(clippy::enum_variant_names)]
pub enum SemanticCheckpointKind {
    PostEmbedding { layer: LayerId },
    PostRouter { layer: LayerId },
    PostExpertDowncast { layer: LayerId, expert: ExpertId },
    PostLogits,
    PostDecode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ObservationSource {
    NodeOutput {
        node: NodeId,
        value: ValueId,
    },
    RouterDecision {
        node: NodeId,
        decision: ValueId,
        weight: ValueId,
    },
    ExpertCandidate {
        node: NodeId,
        candidate: ValueId,
        layer: LayerId,
        expert: ExpertId,
    },
    LogitVector {
        node: NodeId,
        value: ValueId,
    },
    DecodedToken {
        node: NodeId,
        value: ValueId,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ObservationEncoding {
    Canonical,
    TokenId,
    ExpertId,
    QuantizedQ8_8,
    QuantizedQ16_16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum SemanticCheckpointRole {
    Mandatory,
    Optional,
}

impl From<SemanticStratum> for SemanticCheckpointRole {
    fn from(stratum: SemanticStratum) -> Self {
        match stratum {
            SemanticStratum::Denotation | SemanticStratum::Artifact => Self::Mandatory,
            SemanticStratum::Operational => Self::Optional,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationalProbe {
    pub instance_id: ProbeInstanceId,
    pub probe_id: TraceProbeId,
    pub source: ProbeSource,
    pub level: ProbeLevel,
    pub importance: ProbeImportanceClass,
    pub event_shape: TraceEventShape,
    pub frequency_bound: TraceFrequencyBound,
    pub weight: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProbeInstanceId {
    pub probe_id: TraceProbeId,
    pub source_fingerprint: Hash256,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ProbeSource {
    NodePreEntry {
        node: NodeId,
    },
    NodePostEntry {
        node: NodeId,
    },
    ValueEdge {
        value: ValueId,
    },
    EffectEdge {
        effect: EffectId,
        class: EffectClass,
    },
    Anchor {
        anchor: SemanticAnchor,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetricProbe {
    pub metric: MetricId,
    pub source: RegistryMetricSource,
    pub aggregation: MetricAggregation,
    pub importance: ProbeImportanceClass,
    pub weight: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnchorAttachmentTable {
    #[serde(with = "semantic_attachment_map")]
    pub semantic: BTreeMap<SemanticCheckpointId, SemanticAttachment>,
    #[serde(with = "probe_source_map")]
    pub probes: BTreeMap<ProbeInstanceId, ProbeSource>,
    #[serde(with = "metric_source_map")]
    pub metrics: BTreeMap<MetricId, RegistryMetricSource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticAttachment {
    pub anchor: SemanticAnchor,
    pub source: ObservationSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationProvenance {
    #[serde(with = "semantic_evidence_map")]
    pub semantic_provenance: BTreeMap<SemanticCheckpointId, EvidenceRef>,
    #[serde(with = "probe_evidence_map")]
    pub probe_provenance: BTreeMap<ProbeInstanceId, EvidenceRef>,
    #[serde(with = "metric_evidence_map")]
    pub metric_provenance: BTreeMap<MetricId, EvidenceRef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TraceBudgetProjection {
    pub projected_max_events_per_slice: u32,
    pub projected_max_bytes_per_frame: u32,
    pub fits_declared_budget: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReEmittedCheckpointEntry {
    pub id: SemanticCheckpointId,
    pub kind: SemanticCheckpointKind,
    pub artifact_role: SemanticCheckpointRole,
    pub original_checkpoint_metadata: CheckpointEntryView,
    pub encoding: ObservationEncoding,
    pub source: ObservationSource,
    pub attachment_node_id: NodeId,
    pub attachment_anchor: SemanticAnchor,
    #[serde(serialize_with = "canonical_provenance_tuple_json::serialize")]
    pub canonical_provenance_tuple: CanonicalProvenanceTuple,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticCheckpointMetadata {
    pub compact: CompactCheckpointId,
    pub stratum: SemanticStratum,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_op: Option<String>,
}

pub type CheckpointEntryView = SemanticCheckpointMetadata;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticSchemaIngestion {
    pub entries: BTreeMap<SemanticCheckpointId, SemanticSchemaEntry>,
    pub mandatory: BTreeSet<SemanticCheckpointId>,
    pub optional: BTreeSet<SemanticCheckpointId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticSchemaEntry {
    pub id: SemanticCheckpointId,
    pub compact: CompactCheckpointId,
    pub stratum: SemanticStratum,
    pub role: SemanticCheckpointRole,
    pub source_op: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticCheckpointAnchorCandidate {
    pub checkpoint: SemanticCheckpointId,
    pub kind: SemanticCheckpointKind,
    pub node_id: NodeId,
    pub anchor: Option<SemanticAnchor>,
    pub source: Option<ObservationSource>,
    pub canonical_provenance_tuple: CanonicalProvenanceTuple,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundSemanticObservation {
    pub observation: SemanticObservation,
    pub attachment_node_id: NodeId,
    pub canonical_provenance_tuple: CanonicalProvenanceTuple,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticObservationBindings {
    pub identity: ObservationPlanIdentity,
    pub schema: SemanticSchemaIngestion,
    pub feasible: BTreeMap<SemanticCheckpointId, Vec<SemanticCheckpointAnchorCandidate>>,
    pub selected: Vec<BoundSemanticObservation>,
    pub mandatory_count: u16,
    pub optional_count: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeMetricSelection {
    pub probes: Vec<OperationalProbe>,
    pub metrics: Vec<MetricProbe>,
    pub per_class_probe_weight_total: PerClassWeightTotal,
    pub per_class_metric_weight_total: PerClassWeightTotal,
    pub per_class_total_weight: PerClassWeightTotal,
}

pub fn semantic_checkpoint_kind_to_id(kind: SemanticCheckpointKind) -> SemanticCheckpointId {
    let id = match kind {
        SemanticCheckpointKind::PostEmbedding { layer } => {
            format!("layer.{layer}.post_embedding")
        }
        SemanticCheckpointKind::PostRouter { layer } => {
            format!("layer.{layer}.post_router")
        }
        SemanticCheckpointKind::PostExpertDowncast { layer, expert } => {
            format!("layer.{layer}.expert.{expert}.post_downcast")
        }
        SemanticCheckpointKind::PostLogits => "post_logits".to_owned(),
        SemanticCheckpointKind::PostDecode => "post_decode".to_owned(),
    };

    SemanticCheckpointId::from_owned(id).expect("semantic checkpoint kind encoder is grammar-valid")
}

#[must_use]
pub fn try_parse_semantic_checkpoint_kind(
    id: &SemanticCheckpointId,
) -> Option<SemanticCheckpointKind> {
    let text = id.as_str();
    match text {
        "post_logits" => return Some(SemanticCheckpointKind::PostLogits),
        "post_decode" => return Some(SemanticCheckpointKind::PostDecode),
        _ => {}
    }

    let parts = text.split('.').collect::<Vec<_>>();
    match parts.as_slice() {
        ["layer", layer, "post_embedding"] => Some(SemanticCheckpointKind::PostEmbedding {
            layer: parse_layer(layer)?,
        }),
        ["layer", layer, "post_router"] => Some(SemanticCheckpointKind::PostRouter {
            layer: parse_layer(layer)?,
        }),
        ["layer", layer, "expert", expert, "post_downcast"] => {
            Some(SemanticCheckpointKind::PostExpertDowncast {
                layer: parse_layer(layer)?,
                expert: parse_expert(expert)?,
            })
        }
        _ => None,
    }
}

pub fn bind_observation_plan_identity(
    inputs: &ObservationPlanInputs,
) -> Result<ObservationPlanIdentity, serde_json::Error> {
    let observation_policy_projection_hash =
        observation_policy_projection_hash(&inputs.op_policy_projection)?;
    let identity = ObservationPlanIdentity {
        infer_ir_self_hash: inputs.infer_ir_self_hash,
        quant_graph_self_hash: inputs.quant_graph_self_hash,
        semantic_checkpoint_schema_hash: inputs.semantic_checkpoint_schema_hash,
        observation_policy_projection_hash,
        determinism: inputs.op_policy_projection.determinism_class,
        observability_mode: inputs.op_policy_projection.observability_mode,
        trace_budget: abi_trace_budget(inputs.op_policy_projection.trace_budget),
        workload_id: inputs
            .op_policy_projection
            .workload_observation
            .workload_id
            .clone(),
        probe_registry_hash: inputs.probe_registry_hash,
        metric_registry_hash: inputs.metric_registry_hash,
        trace_event_layout_registry_hash: inputs.trace_event_layout_registry_hash,
    };

    record_finalization_event(OBSERVATION_IDENTITY_BIND_EVENT);
    tracing::info!(
        event = %OBSERVATION_IDENTITY_BIND_EVENT,
        infer_ir_self_hash = %identity.infer_ir_self_hash,
        quant_graph_self_hash = %identity.quant_graph_self_hash,
        semantic_checkpoint_schema_hash = %identity.semantic_checkpoint_schema_hash,
        observation_policy_projection_hash = %identity.observation_policy_projection_hash,
        workload_id = identity.workload_id.as_str(),
    );

    Ok(identity)
}

#[must_use]
pub fn ingest_semantic_checkpoint_schema(
    schema: &SemanticCheckpointSchema,
) -> SemanticSchemaIngestion {
    let mut entries = BTreeMap::new();
    let mut mandatory = BTreeSet::new();
    let mut optional = BTreeSet::new();

    for checkpoint in &schema.checkpoints {
        let role = SemanticCheckpointRole::from(checkpoint.stratum);
        let id = checkpoint.semantic.clone();
        match role {
            SemanticCheckpointRole::Mandatory => {
                mandatory.insert(id.clone());
            }
            SemanticCheckpointRole::Optional => {
                optional.insert(id.clone());
            }
        }
        entries.insert(
            id.clone(),
            SemanticSchemaEntry {
                id,
                compact: checkpoint.compact,
                stratum: checkpoint.stratum,
                role,
                source_op: checkpoint.source_op.as_ref().map(ToString::to_string),
            },
        );
    }

    record_finalization_event(OBSERVATION_SCHEMA_INGEST_EVENT);
    tracing::info!(
        event = %OBSERVATION_SCHEMA_INGEST_EVENT,
        mandatory = mandatory.len() as u64,
        optional = optional.len() as u64,
    );

    SemanticSchemaIngestion {
        entries,
        mandatory,
        optional,
    }
}

#[must_use]
pub fn build_feasible_set(
    infer_ir: &GbInferIR,
) -> BTreeMap<SemanticCheckpointId, Vec<SemanticCheckpointAnchorCandidate>> {
    let mut feasible =
        BTreeMap::<SemanticCheckpointId, Vec<SemanticCheckpointAnchorCandidate>>::new();
    let tuples = canonical_provenance_tuples_for_ir(infer_ir);

    for node in &infer_ir.nodes {
        let Some(tuple) = tuples.get(&node.node_id).cloned() else {
            continue;
        };
        let Some(kind) = anchor_to_checkpoint(tuple.clone()) else {
            continue;
        };
        let checkpoint = semantic_checkpoint_kind_to_id(kind);
        feasible
            .entry(checkpoint.clone())
            .or_default()
            .push(SemanticCheckpointAnchorCandidate {
                checkpoint,
                kind,
                node_id: node.node_id,
                anchor: infer_ir.anchors.get(&node.node_id).cloned(),
                source: observation_source_for_node(kind, node),
                canonical_provenance_tuple: tuple,
            });
    }

    feasible
}

pub fn select_semantic_checkpoints_v1(
    schema: &SemanticSchemaIngestion,
    feasible: &BTreeMap<SemanticCheckpointId, Vec<SemanticCheckpointAnchorCandidate>>,
) -> Result<Vec<SemanticCheckpointId>, NonEmptyList<ValidationDiagnostic>> {
    let mut diagnostics = Vec::new();
    let mut selected = BTreeSet::new();
    let mut dropped_codes = BTreeSet::new();

    for checkpoint in &schema.mandatory {
        if feasible.contains_key(checkpoint) {
            selected.insert(checkpoint.clone());
        } else {
            dropped_codes.insert(OBSERVATION_MANDATORY_CHECKPOINT_NOT_FEASIBLE_CODE);
            diagnostics.push(observation_checkpoint_diagnostic(
                ValidationCode::ObservationMandatoryCheckpointNotFeasible {
                    checkpoint: checkpoint.clone(),
                },
                "semantic_checkpoint_schema.checkpoints",
            ));
        }
    }

    for checkpoint in &schema.optional {
        if feasible.contains_key(checkpoint) {
            selected.insert(checkpoint.clone());
        }
    }

    let schema_ids = schema.entries.keys().collect::<BTreeSet<_>>();
    let dropped_count = schema_ids
        .iter()
        .filter(|checkpoint| !feasible.contains_key(*checkpoint))
        .count();
    let dropped_codes = dropped_codes.into_iter().collect::<Vec<_>>();
    record_finalization_event(OBSERVATION_BUILD_FEASIBILITY_FILTER_EVENT);
    tracing::info!(
        event = %OBSERVATION_BUILD_FEASIBILITY_FILTER_EVENT,
        feasible_count = feasible.len() as u64,
        dropped_count = dropped_count as u64,
        dropped_codes = ?dropped_codes,
    );

    if !diagnostics.is_empty() {
        return Err(NonEmptyList::new(diagnostics).expect("diagnostics are non-empty"));
    }

    let selected = selected.into_iter().collect::<Vec<_>>();
    let mandatory_count = selected
        .iter()
        .filter(|checkpoint| schema.mandatory.contains(*checkpoint))
        .count();
    let optional_count = selected
        .iter()
        .filter(|checkpoint| schema.optional.contains(*checkpoint))
        .count();
    record_finalization_event(OBSERVATION_SEMANTIC_SELECTION_EVENT);
    tracing::info!(
        event = %OBSERVATION_SEMANTIC_SELECTION_EVENT,
        selected_count = selected.len() as u64,
        mandatory_count = mandatory_count as u64,
        workload_required_count = 0_u64,
        workload_optional_count = optional_count as u64,
    );

    Ok(selected)
}

pub fn bind_semantic_observations_v1(
    inputs: &ObservationPlanInputs,
) -> Result<SemanticObservationBindings, NonEmptyList<ValidationDiagnostic>> {
    if let Err(error) = validate_observation_plan_inputs(inputs) {
        return Err(
            NonEmptyList::new(vec![observation_input_error_diagnostic(error)])
                .expect("input validation produced a diagnostic"),
        );
    }

    let identity = bind_observation_plan_identity(inputs).map_err(|error| {
        NonEmptyList::new(vec![observation_hash_diagnostic(error.to_string())])
            .expect("hash failure produced a diagnostic")
    })?;
    let schema = ingest_semantic_checkpoint_schema(&inputs.semantic_checkpoint_schema);
    let feasible = build_feasible_set(&inputs.infer_ir_product.infer_ir);
    let selected_ids = select_semantic_checkpoints_v1(&schema, &feasible)?;
    let mut selected = Vec::with_capacity(selected_ids.len());
    let mut diagnostics = Vec::new();
    let compare_domain = inputs
        .op_policy_projection
        .workload_observation
        .compare_domain_policy;
    let determinism = inputs.op_policy_projection.determinism_class;

    for checkpoint in selected_ids {
        let Some(candidates) = feasible.get(&checkpoint) else {
            diagnostics.push(observation_checkpoint_diagnostic(
                ValidationCode::ObservationCheckpointNotAttachable {
                    checkpoint: checkpoint.clone(),
                },
                "infer_ir.anchors",
            ));
            continue;
        };

        if candidates.len() > 1 {
            diagnostics.push(observation_checkpoint_diagnostic(
                ValidationCode::ObservationCheckpointAmbiguous {
                    checkpoint: checkpoint.clone(),
                },
                "infer_ir.anchors",
            ));
            continue;
        }

        let candidate = &candidates[0];
        let (Some(anchor), Some(source)) = (&candidate.anchor, &candidate.source) else {
            diagnostics.push(observation_checkpoint_diagnostic(
                ValidationCode::ObservationCheckpointNotAttachable {
                    checkpoint: checkpoint.clone(),
                },
                "infer_ir.anchors",
            ));
            continue;
        };

        let encoding = match try_encoding_for(candidate.kind, compare_domain, determinism) {
            Ok(encoding) => encoding,
            Err(diagnostic) => {
                diagnostics.push(*diagnostic);
                continue;
            }
        };
        let schema_entry = schema
            .entries
            .get(&checkpoint)
            .expect("selected checkpoint came from ingested schema");
        let observation = SemanticObservation {
            checkpoint: checkpoint.clone(),
            kind: candidate.kind,
            compact: schema_entry.compact,
            stratum: schema_entry.stratum,
            source: source.clone(),
            encoding,
            anchor: anchor.clone(),
            artifact_role: schema_entry.role,
        };

        record_finalization_event(OBSERVATION_SEMANTIC_ANCHOR_BINDING_EVENT);
        tracing::info!(
            event = %OBSERVATION_SEMANTIC_ANCHOR_BINDING_EVENT,
            checkpoint = checkpoint.as_str(),
            anchor = %anchor.anchor_id,
            node_id = candidate.node_id.get() as u64,
        );
        record_finalization_event(OBSERVATION_ENCODING_BINDING_EVENT);
        tracing::info!(
            event = %OBSERVATION_ENCODING_BINDING_EVENT,
            checkpoint = checkpoint.as_str(),
            encoding = ?encoding,
            compare_domain = ?compare_domain,
        );

        selected.push(BoundSemanticObservation {
            observation,
            attachment_node_id: candidate.node_id,
            canonical_provenance_tuple: candidate.canonical_provenance_tuple.clone(),
        });
    }

    if !diagnostics.is_empty() {
        return Err(NonEmptyList::new(diagnostics).expect("diagnostics are non-empty"));
    }

    let mandatory_count = checked_len_to_u16(
        selected
            .iter()
            .filter(|entry| entry.observation.artifact_role == SemanticCheckpointRole::Mandatory)
            .count(),
    );
    let optional_count = checked_len_to_u16(
        selected
            .iter()
            .filter(|entry| entry.observation.artifact_role == SemanticCheckpointRole::Optional)
            .count(),
    );

    Ok(SemanticObservationBindings {
        identity,
        schema,
        feasible,
        selected,
        mandatory_count,
        optional_count,
    })
}

pub fn build_probe_metric_selection_v1(
    inputs: &ObservationPlanInputs,
) -> Result<ProbeMetricSelection, NonEmptyList<ValidationDiagnostic>> {
    precheck_disabled_probe_ids(inputs)?;
    let instantiated = instantiate_registry_probes_v1(inputs)?;
    let governed = govern_probe_budget_v1(instantiated, &inputs.op_policy_projection)?;
    let probes = canonical_order_operational_probes_v1(governed)?;
    let metric_entries = filter_metric_registry_v1(&inputs.metric_registry)?;
    let metrics = select_metrics_v1(metric_entries, &inputs.op_policy_projection)?;
    let metrics = canonical_order_metric_probes_v1(metrics)?;
    let per_class_probe_weight_total = per_class_probe_weight_total(&probes);
    let per_class_metric_weight_total = per_class_metric_weight_total(&metrics);
    let per_class_total_weight =
        combine_weight_totals(per_class_probe_weight_total, per_class_metric_weight_total);
    enforce_per_class_weight_caps(
        per_class_total_weight,
        inputs.op_policy_projection.profile_observation_caps,
    )?;

    Ok(ProbeMetricSelection {
        probes,
        metrics,
        per_class_probe_weight_total,
        per_class_metric_weight_total,
        per_class_total_weight,
    })
}

pub fn build_observation_plan_core(
    inputs: &ObservationPlanInputs,
) -> Result<ObservationPlanCoreSuccess, ObservationPlanCoreFailure> {
    let semantic_bindings = match bind_semantic_observations_v1(inputs) {
        Ok(bindings) => bindings,
        Err(diagnostics) => {
            let identity = failure_identity(inputs);
            return Err(observation_plan_core_failure(
                inputs,
                &identity,
                diagnostics,
                None,
                None,
            ));
        }
    };

    let probe_metric_selection = match build_probe_metric_selection_v1(inputs) {
        Ok(selection) => selection,
        Err(diagnostics) => {
            return Err(observation_plan_core_failure(
                inputs,
                &semantic_bindings.identity,
                diagnostics,
                None,
                None,
            ));
        }
    };

    let anchor_table = bind_anchor_attachment_table(
        &semantic_bindings.selected,
        &probe_metric_selection.probes,
        &probe_metric_selection.metrics,
    );
    let provenance = match bind_observation_provenance(
        inputs,
        &semantic_bindings.selected,
        &probe_metric_selection.probes,
        &probe_metric_selection.metrics,
    ) {
        Ok(provenance) => provenance,
        Err(diagnostics) => {
            return Err(observation_plan_core_failure(
                inputs,
                &semantic_bindings.identity,
                diagnostics,
                None,
                None,
            ));
        }
    };
    let trace_budget_projection = project_trace_budget(
        &probe_metric_selection.probes,
        semantic_bindings.identity.trace_budget,
    );

    let mut observation_plan = ObservationPlan {
        identity: semantic_bindings.identity.clone(),
        semantic: semantic_bindings
            .selected
            .iter()
            .map(|entry| entry.observation.clone())
            .collect(),
        probes: probe_metric_selection.probes.clone(),
        metrics: probe_metric_selection.metrics.clone(),
        anchor_table,
        provenance,
        trace_budget_projection,
    };
    let mut build_active_checkpoint_schema =
        emit_build_active_checkpoint_schema(&semantic_bindings);
    let mut operational_probe_schema = emit_operational_probe_schema(&probe_metric_selection);

    let pre_sort_observation_plan_self_hash =
        observation_plan_self_hash(&observation_plan).expect("observation plan hashes");
    let pre_sort_build_active_checkpoint_schema_hash =
        build_active_checkpoint_schema_hash(&build_active_checkpoint_schema)
            .expect("build-active checkpoint schema hashes");
    let pre_sort_operational_probe_schema_hash =
        operational_probe_schema_hash(&operational_probe_schema)
            .expect("operational probe schema hashes");
    let sc_re_emit_body = semantic_checkpoint_schema_re_emit_body(
        inputs,
        &observation_plan.identity,
        Some(pre_sort_observation_plan_self_hash),
        &build_active_checkpoint_schema,
        pre_sort_build_active_checkpoint_schema_hash,
        Vec::new(),
    );
    let operational_probe_body = operational_probe_schema_body(
        inputs,
        &observation_plan.identity,
        Some(pre_sort_observation_plan_self_hash),
        &operational_probe_schema,
        pre_sort_operational_probe_schema_hash,
        Vec::new(),
    );

    if let Err(diagnostics) = enforce_invariant_budget(
        inputs.op_policy_projection.observability_mode,
        observation_plan.trace_budget_projection,
        observation_plan.identity.trace_budget,
    ) {
        return Err(observation_plan_core_failure(
            inputs,
            &observation_plan.identity,
            diagnostics,
            Some(sc_re_emit_body),
            Some(operational_probe_body),
        ));
    }

    if let Err(diagnostics) = validate_observation_plan_self_consistency(
        inputs,
        &semantic_bindings,
        &probe_metric_selection,
        &observation_plan,
        &build_active_checkpoint_schema,
        &operational_probe_schema,
        pre_sort_observation_plan_self_hash,
    ) {
        return Err(observation_plan_core_failure(
            inputs,
            &observation_plan.identity,
            diagnostics,
            Some(sc_re_emit_body),
            Some(operational_probe_body),
        ));
    }

    canonical_sort_finalized_observation_plan(
        &mut observation_plan,
        &mut build_active_checkpoint_schema,
        &mut operational_probe_schema,
    );

    let final_observation_plan_self_hash =
        observation_plan_self_hash(&observation_plan).expect("observation plan hashes after sort");
    let final_build_active_checkpoint_schema_hash =
        build_active_checkpoint_schema_hash(&build_active_checkpoint_schema)
            .expect("build-active checkpoint schema hashes after sort");
    let final_operational_probe_schema_hash =
        operational_probe_schema_hash(&operational_probe_schema)
            .expect("operational probe schema hashes after sort");

    if final_observation_plan_self_hash != pre_sort_observation_plan_self_hash
        || final_build_active_checkpoint_schema_hash != pre_sort_build_active_checkpoint_schema_hash
        || final_operational_probe_schema_hash != pre_sort_operational_probe_schema_hash
    {
        let diagnostics = NonEmptyList::new(vec![observation_checkpointless_invariant_diagnostic(
            "canonical_sort",
        )])
        .expect("canonical sort diagnostic is non-empty");
        return Err(observation_plan_core_failure(
            inputs,
            &observation_plan.identity,
            diagnostics,
            Some(sc_re_emit_body),
            Some(operational_probe_body),
        ));
    }

    let product = ObservationPlanCoreProduct {
        observation_plan,
        observation_plan_self_hash: final_observation_plan_self_hash,
        build_active_checkpoint_schema,
        build_active_checkpoint_schema_hash: final_build_active_checkpoint_schema_hash,
        operational_probe_schema,
        operational_probe_schema_hash: final_operational_probe_schema_hash,
    };
    let sc_re_emit_body = semantic_checkpoint_schema_re_emit_body(
        inputs,
        &product.observation_plan.identity,
        Some(product.observation_plan_self_hash),
        &product.build_active_checkpoint_schema,
        product.build_active_checkpoint_schema_hash,
        Vec::new(),
    );
    let operational_probe_body = operational_probe_schema_body(
        inputs,
        &product.observation_plan.identity,
        Some(product.observation_plan_self_hash),
        &product.operational_probe_schema,
        product.operational_probe_schema_hash,
        Vec::new(),
    );
    let sc_re_emit_report_self_hash =
        report_self_hash_for(ReportOutcome::Passed, sc_re_emit_body.clone());
    let operational_probe_schema_report_self_hash =
        report_self_hash_for(ReportOutcome::Passed, operational_probe_body.clone());
    let observation_plan_body = observation_plan_report_body(
        inputs,
        &product,
        sc_re_emit_report_self_hash,
        operational_probe_schema_report_self_hash,
        Vec::new(),
    );

    Ok(ObservationPlanCoreSuccess {
        product,
        observation_plan_body,
        sc_re_emit_body,
        operational_probe_body,
    })
}

#[allow(clippy::result_large_err)]
pub fn run_stage4(
    inputs: ObservationPlanInputs,
    env: Stage4PassEnvironment<'_>,
) -> Result<ObservationPlanStageOutput, RunStage4Error> {
    let started = Instant::now();
    let material = Stage4CacheKeyMaterial::from_inputs(&inputs)?;
    let context = Stage4ReportRewrapContext::from_inputs(&inputs);

    if let Err(diagnostics) =
        validate_stage4_driver_preconditions(&inputs, env.resolved_observability_mode)
    {
        let identity = failure_identity(&inputs);
        let failure = observation_plan_core_failure(&inputs, &identity, diagnostics, None, None);
        let stage_failure = wrap_stage4_failure(failure.clone())?;
        emit_stage4_failure_reports(env.report_dir, &stage_failure)?;
        if let Some(cache) = env.stage_cache {
            put_stage4_failure_memo(cache, &material, &failure)?;
            emit_stage4_failure_memo_event(&stage_failure);
        }
        tracing::info!(
            event = %STAGE4_DRIVER_RUN_EVENT,
            cache_state = "precondition_failure",
            audit_rewrap = false,
            total_ms = started.elapsed().as_millis() as u64,
        );
        return Err(RunStage4Error::StageFailure(stage_failure));
    }

    if let Some(cache) = env.stage_cache {
        if let Some(product) = get_stage4_success(cache, &material)? {
            let output = rewrap_stage4_cached_success(&product, &context)?;
            emit_stage4_success_reports(env.report_dir, &output)?;
            tracing::info!(
                event = %STAGE4_DRIVER_RUN_EVENT,
                observation_plan_self_hash = %output.product.observation_plan_self_hash,
                cache_state = "hit_success",
                audit_rewrap = true,
                total_ms = started.elapsed().as_millis() as u64,
            );
            return Ok(output);
        }

        if let Some(failure) = get_stage4_failure_memo(cache, &material)? {
            let replay = rewrap_stage4_cached_failure(&failure, &context)?;
            let stage_failure = ObservationPlanStageFailure {
                report: replay.report,
                sc_re_emit_report: replay.sc_re_emit_report,
                operational_probe_report: replay.operational_probe_report,
                diagnostics: replay.diagnostics,
            };
            emit_stage4_failure_reports(env.report_dir, &stage_failure)?;
            tracing::info!(
                event = %STAGE4_DRIVER_RUN_EVENT,
                cache_state = "hit_failure_memo",
                audit_rewrap = true,
                total_ms = started.elapsed().as_millis() as u64,
            );
            return Err(RunStage4Error::StageFailure(stage_failure));
        }
    }

    match build_observation_plan_core(&inputs) {
        Ok(success) => {
            let output = wrap_stage4_success(success)?;
            emit_stage4_success_reports(env.report_dir, &output)?;
            if let Some(cache) = env.stage_cache {
                put_stage4_success(cache, &material, &output.product)?;
            }
            tracing::info!(
                event = %STAGE4_DRIVER_RUN_EVENT,
                observation_plan_self_hash = %output.product.observation_plan_self_hash,
                cache_state = "miss_success",
                audit_rewrap = false,
                total_ms = started.elapsed().as_millis() as u64,
            );
            Ok(output)
        }
        Err(failure) => {
            let stage_failure = wrap_stage4_failure(failure.clone())?;
            emit_stage4_failure_reports(env.report_dir, &stage_failure)?;
            if let Some(cache) = env.stage_cache {
                put_stage4_failure_memo(cache, &material, &failure)?;
                emit_stage4_failure_memo_event(&stage_failure);
            }
            tracing::info!(
                event = %STAGE4_DRIVER_RUN_EVENT,
                cache_state = "miss_failure",
                audit_rewrap = false,
                total_ms = started.elapsed().as_millis() as u64,
            );
            Err(RunStage4Error::StageFailure(stage_failure))
        }
    }
}

fn validate_stage4_driver_preconditions(
    inputs: &ObservationPlanInputs,
    resolved_observability_mode: ObservabilityMode,
) -> Result<(), NonEmptyList<ValidationDiagnostic>> {
    let mut diagnostics = Vec::new();

    match crate::s3::infer_ir::infer_ir_self_hash(&inputs.infer_ir_product.infer_ir) {
        Ok(computed) => {
            if inputs.infer_ir_self_hash != computed
                || inputs.infer_ir_product.infer_ir_self_hash != computed
            {
                diagnostics.push(stage4_hash_mismatch_diagnostic(
                    "infer_ir_self_hash",
                    computed,
                    inputs.infer_ir_self_hash,
                ));
            }
        }
        Err(error) => diagnostics.push(stage4_precondition_diagnostic_with_field(
            "infer_ir_self_hash",
            error.to_string(),
        )),
    }

    if inputs.semantic_checkpoint_schema_hash
        != inputs.artifact_declared_semantic_checkpoint_schema_hash
    {
        diagnostics.push(stage4_observation_sc_hash_mismatch_diagnostic(
            inputs.artifact_declared_semantic_checkpoint_schema_hash,
            inputs.semantic_checkpoint_schema_hash,
        ));
    }

    if inputs.op_policy_projection.observability_mode != resolved_observability_mode {
        diagnostics.push(stage4_precondition_diagnostic(
            "op_policy_projection.observability_mode",
        ));
    }

    let infer_ir_determinism = inputs.infer_ir_product.infer_ir.identity.determinism;
    if inputs.op_policy_projection.determinism_class != infer_ir_determinism {
        diagnostics.push(stage4_observation_determinism_mismatch_diagnostic(
            "op_policy_projection.determinism_class",
        ));
    }

    if inputs.audit_parents.static_budget_self_hash
        != inputs
            .infer_ir_product
            .infer_ir
            .identity
            .static_budget_self_hash
    {
        diagnostics.push(stage4_precondition_diagnostic(
            "audit_parents.static_budget_self_hash",
        ));
    }

    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(NonEmptyList::new(diagnostics).expect("precondition diagnostics are non-empty"))
    }
}

fn wrap_stage4_success(
    success: ObservationPlanCoreSuccess,
) -> Result<ObservationPlanStageOutput, RunStage4Error> {
    let report = stage4_report_envelope(ReportOutcome::Passed, success.observation_plan_body)?;
    let sc_re_emit_report = stage4_report_envelope(ReportOutcome::Passed, success.sc_re_emit_body)?;
    let operational_probe_report =
        stage4_report_envelope(ReportOutcome::Passed, success.operational_probe_body)?;
    Ok(ObservationPlanStageOutput {
        product: success.product,
        report,
        sc_re_emit_report,
        operational_probe_report,
    })
}

fn wrap_stage4_failure(
    failure: ObservationPlanCoreFailure,
) -> Result<ObservationPlanStageFailure, RunStage4Error> {
    let report = stage4_report_envelope(ReportOutcome::Failed, failure.observation_plan_body)?;
    let sc_re_emit_report = failure
        .sc_re_emit_body
        .map(|body| stage4_report_envelope(ReportOutcome::Failed, body))
        .transpose()?;
    let operational_probe_report = failure
        .operational_probe_body
        .map(|body| stage4_report_envelope(ReportOutcome::Failed, body))
        .transpose()?;
    Ok(ObservationPlanStageFailure {
        report,
        sc_re_emit_report,
        operational_probe_report,
        diagnostics: failure.diagnostics,
    })
}

fn stage4_report_envelope<B>(
    outcome: ReportOutcome,
    body: B,
) -> Result<ReportEnvelope<B>, RunStage4Error>
where
    B: ReportBody + Serialize,
{
    ReportEnvelope::new(outcome, body)
        .map_err(|err| RunStage4Error::ReportIo(io::Error::other(err.to_string())))?
        .with_computed_self_hash()
        .map_err(|err| RunStage4Error::ReportIo(io::Error::other(err.to_string())))
}

fn emit_stage4_success_reports(
    report_dir: Option<&Path>,
    output: &ObservationPlanStageOutput,
) -> Result<(), RunStage4Error> {
    emit_stage4_report_files(
        report_dir,
        vec![
            (
                "observation_plan.json",
                canonicalize_stage4_report(&output.report, "observation_plan.json")?,
            ),
            (
                "semantic_checkpoint_schema.json",
                canonicalize_stage4_report(
                    &output.sc_re_emit_report,
                    "semantic_checkpoint_schema.json",
                )?,
            ),
            (
                "operational_probe_schema.json",
                canonicalize_stage4_report(
                    &output.operational_probe_report,
                    "operational_probe_schema.json",
                )?,
            ),
        ],
    )
}

fn emit_stage4_failure_reports(
    report_dir: Option<&Path>,
    failure: &ObservationPlanStageFailure,
) -> Result<(), RunStage4Error> {
    let mut reports = vec![(
        "observation_plan.json",
        canonicalize_stage4_report(&failure.report, "observation_plan.json")?,
    )];
    if let Some(report) = &failure.sc_re_emit_report {
        reports.push((
            "semantic_checkpoint_schema.json",
            canonicalize_stage4_report(report, "semantic_checkpoint_schema.json")?,
        ));
    }
    if let Some(report) = &failure.operational_probe_report {
        reports.push((
            "operational_probe_schema.json",
            canonicalize_stage4_report(report, "operational_probe_schema.json")?,
        ));
    }
    emit_stage4_report_files(report_dir, reports)
}

fn canonicalize_stage4_report<B>(
    report: &ReportEnvelope<B>,
    file_name: &'static str,
) -> Result<Vec<u8>, RunStage4Error>
where
    B: ReportBody + Serialize,
{
    canonicalize_report(report).map_err(|err| {
        RunStage4Error::ReportIo(io::Error::other(format!(
            "Stage 4 report {file_name} did not canonicalize: {err}"
        )))
    })
}

fn emit_stage4_report_files(
    report_dir: Option<&Path>,
    reports: Vec<(&'static str, Vec<u8>)>,
) -> Result<(), RunStage4Error> {
    let Some(report_dir) = report_dir else {
        return Ok(());
    };
    fs::create_dir_all(report_dir).map_err(|err| {
        io::Error::new(
            err.kind(),
            format!(
                "failed to create Stage 4 report directory {}: {err}",
                report_dir.display()
            ),
        )
    })?;
    for (file_name, bytes) in reports {
        let path = report_dir.join(file_name);
        fs::write(&path, &bytes).map_err(|err| {
            io::Error::new(
                err.kind(),
                format!("failed to write Stage 4 report {}: {err}", path.display()),
            )
        })?;
        tracing::debug!(
            event = %STAGE4_DRIVER_REPORT_EMIT_EVENT,
            canonical_bytes_len = bytes.len() as u64,
            report_path = %path.display(),
            "stage4.driver.report_emit"
        );
    }
    Ok(())
}

fn emit_stage4_failure_memo_event(failure: &ObservationPlanStageFailure) {
    #[cfg(test)]
    tracing_core::callsite::rebuild_interest_cache();

    tracing::info!(
        event = %STAGE4_DRIVER_FAILURE_MEMO_EVENT,
        diagnostic_count = failure.diagnostics.as_slice().len() as u64,
        "stage4.driver.failure_memo"
    );
}

fn stage4_hash_mismatch_diagnostic(
    field: &'static str,
    expected: Hash256,
    observed: Hash256,
) -> ValidationDiagnostic {
    ValidationDiagnostic::hard(
        ValidationOrigin::SemanticCore,
        ValidationCode::SemanticCoreHashMismatch,
        ValidationDetail::HashMismatch { expected, observed },
        vec![EvidenceRef {
            kind: "stage4-precondition".to_owned(),
            reference: field.to_owned(),
            hash: Some(observed),
        }],
    )
}

fn stage4_observation_sc_hash_mismatch_diagnostic(
    expected: Hash256,
    observed: Hash256,
) -> ValidationDiagnostic {
    ValidationDiagnostic::hard(
        ValidationOrigin::ObservationPlanConstruction,
        ValidationCode::ObservationScHashMismatch { expected, observed },
        ValidationDetail::HashMismatch { expected, observed },
        vec![EvidenceRef {
            kind: "stage4-precondition".to_owned(),
            reference: OBSERVATION_SC_HASH_MISMATCH_CODE.to_owned(),
            hash: Some(observed),
        }],
    )
}

fn stage4_observation_determinism_mismatch_diagnostic(field: &'static str) -> ValidationDiagnostic {
    ValidationDiagnostic::hard(
        ValidationOrigin::ObservationPlanConstruction,
        ValidationCode::ObservationDeterminismMismatch {
            field: FieldPath::from(field),
        },
        ValidationDetail::Field {
            field: FieldPath::from(field),
        },
        vec![EvidenceRef {
            kind: "stage4-precondition".to_owned(),
            reference: OBSERVATION_DETERMINISM_MISMATCH_CODE.to_owned(),
            hash: Some(Hash256::ZERO),
        }],
    )
}

fn stage4_precondition_diagnostic(field: &'static str) -> ValidationDiagnostic {
    stage4_precondition_diagnostic_with_field(field, field.to_owned())
}

fn stage4_precondition_diagnostic_with_field(
    field: &'static str,
    reference: String,
) -> ValidationDiagnostic {
    ValidationDiagnostic::hard(
        ValidationOrigin::SemanticCore,
        ValidationCode::ReportSemanticInvariantViolated {
            field: FieldPath::from(field),
        },
        ValidationDetail::Field {
            field: FieldPath::from(field),
        },
        vec![EvidenceRef {
            kind: "stage4-precondition".to_owned(),
            reference,
            hash: Some(Hash256::ZERO),
        }],
    )
}

fn failure_identity(inputs: &ObservationPlanInputs) -> ObservationPlanIdentity {
    bind_observation_plan_identity(inputs).expect("failure identity hashes")
}

fn observation_plan_core_failure(
    inputs: &ObservationPlanInputs,
    identity: &ObservationPlanIdentity,
    diagnostics: NonEmptyList<ValidationDiagnostic>,
    sc_re_emit_body: Option<SemanticCheckpointSchemaReEmitBody>,
    operational_probe_body: Option<OperationalProbeSchemaBody>,
) -> ObservationPlanCoreFailure {
    let diagnostics_vec = diagnostics.as_slice().to_vec();
    ObservationPlanCoreFailure {
        observation_plan_body: ObservationPlanReportBody {
            input_identity: ObservationPlanReportInputIdentity::from_inputs(inputs, identity),
            result: None,
            diagnostics: diagnostics_vec,
        },
        sc_re_emit_body,
        operational_probe_body,
        diagnostics,
    }
}

fn bind_anchor_attachment_table(
    semantic: &[BoundSemanticObservation],
    probes: &[OperationalProbe],
    metrics: &[MetricProbe],
) -> AnchorAttachmentTable {
    let table = AnchorAttachmentTable {
        semantic: semantic
            .iter()
            .map(|entry| {
                (
                    entry.observation.checkpoint.clone(),
                    SemanticAttachment {
                        anchor: entry.observation.anchor.clone(),
                        source: entry.observation.source.clone(),
                    },
                )
            })
            .collect(),
        probes: probes
            .iter()
            .map(|probe| (probe.instance_id, probe.source.clone()))
            .collect(),
        metrics: metrics
            .iter()
            .map(|metric| (metric.metric.clone(), metric.source))
            .collect(),
    };

    record_finalization_event(OBSERVATION_ANCHOR_TABLE_BIND_EVENT);
    tracing::info!(
        event = %OBSERVATION_ANCHOR_TABLE_BIND_EVENT,
        sem = table.semantic.len() as u64,
        probes = table.probes.len() as u64,
        metrics = table.metrics.len() as u64,
    );

    table
}

fn bind_observation_provenance(
    inputs: &ObservationPlanInputs,
    semantic: &[BoundSemanticObservation],
    probes: &[OperationalProbe],
    metrics: &[MetricProbe],
) -> Result<ObservationProvenance, NonEmptyList<ValidationDiagnostic>> {
    let probe_evidence = inputs
        .probe_registry
        .entries
        .iter()
        .map(|entry| (entry.probe_id, entry.evidence.clone()))
        .collect::<BTreeMap<_, _>>();
    let metric_evidence = inputs
        .metric_registry
        .entries
        .iter()
        .map(|entry| (entry.metric.clone(), entry.evidence.clone()))
        .collect::<BTreeMap<_, _>>();

    let mut diagnostics = Vec::new();
    let mut probe_provenance = BTreeMap::new();
    for probe in probes {
        let Some(evidence) = probe_evidence.get(&probe.probe_id) else {
            diagnostics.push(observation_registry_diagnostic(
                ValidationCode::ObservationProbeIdUnknown {
                    probe_id: probe.probe_id,
                },
                "probe_registry.entries",
                Vec::new(),
            ));
            continue;
        };
        probe_provenance.insert(
            probe.instance_id,
            evidence_with_default_hash(evidence.clone(), inputs.probe_registry_hash),
        );
    }

    let mut metric_provenance = BTreeMap::new();
    for metric in metrics {
        let Some(evidence) = metric_evidence.get(&metric.metric) else {
            diagnostics.push(observation_registry_diagnostic(
                ValidationCode::ObservationMetricIdUnknown {
                    metric: metric.metric.clone(),
                },
                "metric_registry.entries",
                Vec::new(),
            ));
            continue;
        };
        metric_provenance.insert(
            metric.metric.clone(),
            evidence_with_default_hash(evidence.clone(), inputs.metric_registry_hash),
        );
    }

    if !diagnostics.is_empty() {
        return Err(NonEmptyList::new(diagnostics).expect("diagnostics are non-empty"));
    }

    let provenance = ObservationProvenance {
        semantic_provenance: semantic
            .iter()
            .map(|entry| {
                (
                    entry.observation.checkpoint.clone(),
                    semantic_schema_evidence(inputs, &entry.observation.checkpoint),
                )
            })
            .collect(),
        probe_provenance,
        metric_provenance,
    };

    record_finalization_event(OBSERVATION_PROVENANCE_BIND_EVENT);
    tracing::info!(
        event = %OBSERVATION_PROVENANCE_BIND_EVENT,
        sem = provenance.semantic_provenance.len() as u64,
        probes = provenance.probe_provenance.len() as u64,
        metrics = provenance.metric_provenance.len() as u64,
    );

    Ok(provenance)
}

fn semantic_schema_evidence(
    inputs: &ObservationPlanInputs,
    checkpoint: &SemanticCheckpointId,
) -> EvidenceRef {
    EvidenceRef {
        kind: "semantic_checkpoint_schema".to_owned(),
        reference: checkpoint.as_str().to_owned(),
        hash: Some(inputs.semantic_checkpoint_schema_hash),
    }
}

fn evidence_with_default_hash(mut evidence: EvidenceRef, fallback_hash: Hash256) -> EvidenceRef {
    if evidence.hash.is_none() {
        evidence.hash = Some(fallback_hash);
    }
    evidence
}

fn emit_build_active_checkpoint_schema(
    bindings: &SemanticObservationBindings,
) -> BuildActiveCheckpointSchema {
    let mut checkpoints = bindings
        .selected
        .iter()
        .map(|entry| {
            let observation = &entry.observation;
            let schema_entry = bindings
                .schema
                .entries
                .get(&observation.checkpoint)
                .expect("selected checkpoint came from schema");
            ReEmittedCheckpointEntry {
                id: observation.checkpoint.clone(),
                kind: observation.kind,
                artifact_role: observation.artifact_role,
                original_checkpoint_metadata: SemanticCheckpointMetadata {
                    compact: schema_entry.compact,
                    stratum: schema_entry.stratum,
                    source_op: schema_entry.source_op.clone(),
                },
                encoding: observation.encoding,
                source: observation.source.clone(),
                attachment_node_id: entry.attachment_node_id,
                attachment_anchor: observation.anchor.clone(),
                canonical_provenance_tuple: entry.canonical_provenance_tuple.clone(),
            }
        })
        .collect::<Vec<_>>();
    checkpoints.sort_by(|left, right| left.id.cmp(&right.id));
    let schema = BuildActiveCheckpointSchema {
        build_active_count: checked_len_to_u16(checkpoints.len()),
        mandatory_count: bindings.mandatory_count,
        optional_count: bindings.optional_count,
        checkpoints,
    };
    let schema_hash =
        build_active_checkpoint_schema_hash(&schema).expect("build-active schema hashes");

    record_finalization_event(OBSERVATION_SCHEMA_RE_EMIT_EVENT);
    tracing::info!(
        event = %OBSERVATION_SCHEMA_RE_EMIT_EVENT,
        count = schema.checkpoints.len() as u64,
        schema_hash = %schema_hash,
    );

    schema
}

fn emit_operational_probe_schema(selection: &ProbeMetricSelection) -> OperationalProbeSchema {
    let mut probes = selection
        .probes
        .iter()
        .map(|probe| ProbeSchemaEntry {
            instance_id: probe.instance_id,
            probe_id: probe.probe_id,
            level: probe.level,
            importance: probe.importance,
            event_shape: probe.event_shape.clone(),
            source: probe.source.clone(),
            weight: probe.weight,
        })
        .collect::<Vec<_>>();
    probes.sort_by_key(|probe| {
        (
            probe.instance_id.probe_id,
            probe.instance_id.source_fingerprint,
        )
    });
    let mut metrics = selection
        .metrics
        .iter()
        .map(|metric| MetricSchemaEntry {
            metric: metric.metric.clone(),
            aggregation: metric.aggregation,
            source: metric.source,
            importance: metric.importance,
            weight: metric.weight,
        })
        .collect::<Vec<_>>();
    metrics.sort_by(|left, right| left.metric.cmp(&right.metric));

    let schema = OperationalProbeSchema {
        probe_count: checked_len_to_u16(probes.len()),
        metric_count: checked_len_to_u16(metrics.len()),
        probes,
        metrics,
        per_class_probe_weight_total: selection.per_class_probe_weight_total,
        per_class_metric_weight_total: selection.per_class_metric_weight_total,
        per_class_total_weight: selection.per_class_total_weight,
    };
    debug_assert_eq!(
        combine_weight_totals(
            schema.per_class_probe_weight_total,
            schema.per_class_metric_weight_total,
        ),
        schema.per_class_total_weight,
        "operational probe schema per-class probe + metric totals must equal total",
    );

    record_finalization_event(OBSERVATION_OPERATIONAL_PROBE_SCHEMA_EMIT_EVENT);
    tracing::info!(
        event = %OBSERVATION_OPERATIONAL_PROBE_SCHEMA_EMIT_EVENT,
        probe_count = schema.probe_count as u64,
        metric_count = schema.metric_count as u64,
        per_class_total = ?schema.per_class_total_weight,
    );

    schema
}

fn project_trace_budget(probes: &[OperationalProbe], budget: TraceBudget) -> TraceBudgetProjection {
    let mut projected_max_events_per_slice = 0_u64;
    let mut projected_max_bytes_per_frame = 0_u64;
    for probe in probes {
        let events = u64::from(trace_frequency_bound_events(probe.frequency_bound));
        projected_max_events_per_slice = projected_max_events_per_slice.saturating_add(events);
        projected_max_bytes_per_frame = projected_max_bytes_per_frame
            .saturating_add(events.saturating_mul(u64::from(probe.event_shape.max_payload_bytes)));
    }

    let projected_max_events_per_slice =
        u32::try_from(projected_max_events_per_slice).unwrap_or(u32::MAX);
    let projected_max_bytes_per_frame =
        u32::try_from(projected_max_bytes_per_frame).unwrap_or(u32::MAX);
    TraceBudgetProjection {
        projected_max_events_per_slice,
        projected_max_bytes_per_frame,
        fits_declared_budget: projected_max_events_per_slice
            <= u32::from(budget.max_events_per_slice)
            && projected_max_bytes_per_frame <= u32::from(budget.max_bytes_per_frame),
    }
}

fn trace_frequency_bound_events(bound: TraceFrequencyBound) -> u32 {
    match bound {
        TraceFrequencyBound::PerPass { max_events } => max_events,
        TraceFrequencyBound::PerToken {
            max_events_per_token,
        } => max_events_per_token,
        TraceFrequencyBound::PerNodeExecution {
            max_events_per_execution,
        } => max_events_per_execution,
        TraceFrequencyBound::PerFrame {
            max_events_per_frame,
        }
        | TraceFrequencyBound::FaultOnly {
            max_events_per_frame,
        } => max_events_per_frame,
    }
}

fn enforce_invariant_budget(
    mode: ObservabilityMode,
    projection: TraceBudgetProjection,
    budget: TraceBudget,
) -> Result<(), NonEmptyList<ValidationDiagnostic>> {
    record_finalization_event(OBSERVATION_INVARIANT_BUDGET_CHECK_EVENT);
    tracing::info!(
        event = %OBSERVATION_INVARIANT_BUDGET_CHECK_EVENT,
        projected_events = projection.projected_max_events_per_slice as u64,
        projected_bytes = projection.projected_max_bytes_per_frame as u64,
        fits = projection.fits_declared_budget,
        mode = ?mode,
    );

    if mode == ObservabilityMode::Invariant && !projection.fits_declared_budget {
        return Err(
            NonEmptyList::new(vec![observation_invariant_budget_diagnostic(
                projection, budget,
            )])
            .expect("budget diagnostic is non-empty"),
        );
    }

    Ok(())
}

fn observation_invariant_budget_diagnostic(
    projection: TraceBudgetProjection,
    budget: TraceBudget,
) -> ValidationDiagnostic {
    ValidationDiagnostic::hard(
        ValidationOrigin::ObservationPlanConstruction,
        observation_invariant_budget_code(projection, budget),
        ValidationDetail::Field {
            field: FieldPath::from("trace_budget_projection"),
        },
        Vec::new(),
    )
}

fn observation_invariant_budget_code(
    projection: TraceBudgetProjection,
    budget: TraceBudget,
) -> ValidationCode {
    ValidationCode::ObservationInvariantModeBudgetBusted {
        projected_max_events_per_slice: projection.projected_max_events_per_slice,
        projected_max_bytes_per_frame: projection.projected_max_bytes_per_frame,
        max_events_per_slice: budget.max_events_per_slice,
        max_bytes_per_frame: budget.max_bytes_per_frame,
    }
}

#[derive(Clone, Copy)]
struct OpScCheck {
    id: &'static str,
    field: &'static str,
    description: &'static str,
}

const OP_SC_UNIQUE_SEMANTIC: OpScCheck = OpScCheck {
    id: "OP-SC-1",
    field: "semantic.checkpoint",
    description: "selected semantic checkpoints are unique",
};
const OP_SC_UNIQUE_PROBE: OpScCheck = OpScCheck {
    id: "OP-SC-2",
    field: "probes.instance_id",
    description: "operational probe instances are unique",
};
const OP_SC_UNIQUE_METRIC: OpScCheck = OpScCheck {
    id: "OP-SC-3",
    field: "metrics.metric",
    description: "selected metric identifiers are unique",
};
const OP_SC_SCHEMA_MEMBER: OpScCheck = OpScCheck {
    id: "OP-SC-4",
    field: "semantic.checkpoint",
    description: "selected semantic checkpoints are members of the ingested schema",
};
const OP_SC_ANCHOR_BINDING: OpScCheck = OpScCheck {
    id: "OP-SC-5",
    field: "semantic.anchor",
    description: "semantic anchors reference feasible infer-ir anchors",
};
const OP_SC_ENCODING_BINDING: OpScCheck = OpScCheck {
    id: "OP-SC-6",
    field: "semantic.encoding",
    description: "semantic checkpoint encodings match policy and determinism",
};
const OP_SC_MANDATORY_SELECTION: OpScCheck = OpScCheck {
    id: "OP-SC-7",
    field: "semantic.mandatory",
    description: "mandatory feasible checkpoints are selected as mandatory",
};
const OP_SC_GOVERNANCE_SELECTION: OpScCheck = OpScCheck {
    id: "OP-SC-8",
    field: "selection.governance",
    description: "selected probes and metrics survive governance filters",
};
const OP_SC_ANCHOR_TABLE: OpScCheck = OpScCheck {
    id: "OP-SC-9",
    field: "anchor_table",
    description: "anchor attachment table matches selected vectors",
};
const OP_SC_WEIGHT_TOTALS: OpScCheck = OpScCheck {
    id: "OP-SC-10",
    field: "per_class_weight_total",
    description: "per-class probe and metric totals compose exactly",
};
const OP_SC_BUDGET_AND_CAPS: OpScCheck = OpScCheck {
    id: "OP-SC-11",
    field: "budget_and_caps",
    description: "profile caps and invariant-mode trace budgets are enforced",
};
const OP_SC_PROVENANCE: OpScCheck = OpScCheck {
    id: "OP-SC-12",
    field: "provenance",
    description: "provenance maps are total and keyed by final identifiers",
};
const OP_SC_BUILD_SCHEMA_COUNTS: OpScCheck = OpScCheck {
    id: "OP-SC-13",
    field: "build_active_checkpoint_schema.counts",
    description: "build-active checkpoint schema counts are internally consistent",
};
const OP_SC_BUILD_SCHEMA_IDS: OpScCheck = OpScCheck {
    id: "OP-SC-14",
    field: "build_active_checkpoint_schema.checkpoints",
    description: "build-active checkpoint schema ids match selected checkpoints",
};
const OP_SC_BUILD_SCHEMA_ENTRY: OpScCheck = OpScCheck {
    id: "OP-SC-15",
    field: "build_active_checkpoint_schema.entry",
    description: "re-emitted checkpoint entries mirror selected schema metadata",
};
const OP_SC_OPERATIONAL_SCHEMA_COUNTS: OpScCheck = OpScCheck {
    id: "OP-SC-16",
    field: "operational_probe_schema.counts",
    description: "operational schema counts and totals mirror the selection",
};
const OP_SC_OPERATIONAL_SCHEMA_IDS: OpScCheck = OpScCheck {
    id: "OP-SC-17",
    field: "operational_probe_schema.entries",
    description: "operational schema entries match selected probes and metrics",
};
const OP_SC_SELF_HASH: OpScCheck = OpScCheck {
    id: "OP-SC-18",
    field: "observation_plan_self_hash",
    description: "observation plan self hash matches the finalized product",
};
const OP_SC_REGISTRY_BINDING: OpScCheck = OpScCheck {
    id: "OP-SC-19",
    field: "registry_binding",
    description: "selected probe and metric registry bindings remain faithful",
};

const OP_SC_CHECKS: [OpScCheck; 19] = [
    OP_SC_UNIQUE_SEMANTIC,
    OP_SC_UNIQUE_PROBE,
    OP_SC_UNIQUE_METRIC,
    OP_SC_SCHEMA_MEMBER,
    OP_SC_ANCHOR_BINDING,
    OP_SC_ENCODING_BINDING,
    OP_SC_MANDATORY_SELECTION,
    OP_SC_GOVERNANCE_SELECTION,
    OP_SC_ANCHOR_TABLE,
    OP_SC_WEIGHT_TOTALS,
    OP_SC_BUDGET_AND_CAPS,
    OP_SC_PROVENANCE,
    OP_SC_BUILD_SCHEMA_COUNTS,
    OP_SC_BUILD_SCHEMA_IDS,
    OP_SC_BUILD_SCHEMA_ENTRY,
    OP_SC_OPERATIONAL_SCHEMA_COUNTS,
    OP_SC_OPERATIONAL_SCHEMA_IDS,
    OP_SC_SELF_HASH,
    OP_SC_REGISTRY_BINDING,
];

fn validate_observation_plan_self_consistency(
    inputs: &ObservationPlanInputs,
    bindings: &SemanticObservationBindings,
    selection: &ProbeMetricSelection,
    plan: &ObservationPlan,
    build_schema: &BuildActiveCheckpointSchema,
    operational_schema: &OperationalProbeSchema,
    observation_plan_self_hash_value: Hash256,
) -> Result<(), NonEmptyList<ValidationDiagnostic>> {
    let mut diagnostics = Vec::new();

    check_unique_by(
        plan.semantic.iter().map(|entry| entry.checkpoint.clone()),
        "semantic.checkpoint",
        OP_SC_UNIQUE_SEMANTIC,
        &mut diagnostics,
    );
    check_unique_by(
        plan.probes.iter().map(|entry| entry.instance_id),
        "probes.instance_id",
        OP_SC_UNIQUE_PROBE,
        &mut diagnostics,
    );
    check_unique_by(
        plan.metrics.iter().map(|entry| entry.metric.clone()),
        "metrics.metric",
        OP_SC_UNIQUE_METRIC,
        &mut diagnostics,
    );

    let node_ids = inputs
        .infer_ir_product
        .infer_ir
        .nodes
        .iter()
        .map(|node| node.node_id)
        .collect::<BTreeSet<_>>();
    let value_ids = inputs
        .infer_ir_product
        .infer_ir
        .values
        .iter()
        .map(|value| value.value_id)
        .collect::<BTreeSet<_>>();
    let effect_ids = inputs
        .infer_ir_product
        .infer_ir
        .effects
        .iter()
        .map(|effect| effect.effect_id)
        .collect::<BTreeSet<_>>();
    let anchor_ids = inputs
        .infer_ir_product
        .infer_ir
        .anchors
        .values()
        .map(|anchor| anchor.anchor_id)
        .collect::<BTreeSet<_>>();
    let selected_checkpoint_ids = plan
        .semantic
        .iter()
        .map(|entry| entry.checkpoint.clone())
        .collect::<BTreeSet<_>>();
    let selected_probe_ids = plan
        .probes
        .iter()
        .map(|entry| entry.instance_id)
        .collect::<BTreeSet<_>>();
    let selected_metric_ids = plan
        .metrics
        .iter()
        .map(|entry| entry.metric.clone())
        .collect::<BTreeSet<_>>();
    let schema_ids = bindings
        .schema
        .entries
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    let probe_registry = inputs
        .probe_registry
        .entries
        .iter()
        .map(|entry| (entry.probe_id, entry))
        .collect::<BTreeMap<_, _>>();
    let metric_registry = inputs
        .metric_registry
        .entries
        .iter()
        .map(|entry| (entry.metric.clone(), entry))
        .collect::<BTreeMap<_, _>>();

    for entry in &plan.semantic {
        if !schema_ids.contains(&entry.checkpoint) {
            diagnostics.push(observation_self_consistency_diagnostic(
                OP_SC_SCHEMA_MEMBER,
                "semantic.checkpoint",
            ));
        }
        if !anchor_ids.contains(&entry.anchor.anchor_id) {
            diagnostics.push(observation_self_consistency_diagnostic_with_code(
                ValidationCode::ObservationCheckpointNotAttachable {
                    checkpoint: entry.checkpoint.clone(),
                },
                OP_SC_ANCHOR_BINDING,
                "semantic.anchor",
                Vec::new(),
            ));
        }
        let has_matching_feasible_candidate = bindings
            .feasible
            .get(&entry.checkpoint)
            .into_iter()
            .flat_map(|candidates| candidates.iter())
            .any(|candidate| candidate.anchor.as_ref() == Some(&entry.anchor));
        if !has_matching_feasible_candidate {
            diagnostics.push(observation_self_consistency_diagnostic(
                OP_SC_ANCHOR_BINDING,
                "semantic.anchor_to_checkpoint",
            ));
        }
        let expected_encoding = try_encoding_for(
            entry.kind,
            inputs
                .op_policy_projection
                .workload_observation
                .compare_domain_policy,
            inputs.op_policy_projection.determinism_class,
        );
        if !matches!(expected_encoding, Ok(expected) if expected == entry.encoding) {
            diagnostics.push(observation_self_consistency_diagnostic(
                OP_SC_ENCODING_BINDING,
                "semantic.encoding",
            ));
        }
        if entry.artifact_role == SemanticCheckpointRole::Mandatory
            && !bindings.schema.mandatory.contains(&entry.checkpoint)
        {
            diagnostics.push(observation_self_consistency_diagnostic(
                OP_SC_MANDATORY_SELECTION,
                "semantic.artifact_role",
            ));
        }
    }

    let feasible_checkpoint_ids = bindings.feasible.keys().cloned().collect::<BTreeSet<_>>();
    for checkpoint in bindings
        .schema
        .mandatory
        .intersection(&feasible_checkpoint_ids)
    {
        let selected_as_mandatory = plan.semantic.iter().any(|entry| {
            &entry.checkpoint == checkpoint
                && entry.artifact_role == SemanticCheckpointRole::Mandatory
        });
        if !selected_as_mandatory {
            diagnostics.push(observation_self_consistency_diagnostic(
                OP_SC_MANDATORY_SELECTION,
                "semantic.mandatory",
            ));
        }
    }

    for probe in &plan.probes {
        if !survives_optional_floor(
            probe.importance,
            inputs.op_policy_projection.optional_probe_floor,
        ) || trace_demotion_drops(inputs.op_policy_projection.trace_demotion, probe.importance)
            || inputs
                .op_policy_projection
                .disabled_optional_probes
                .contains(&probe.probe_id)
        {
            diagnostics.push(observation_self_consistency_diagnostic(
                OP_SC_GOVERNANCE_SELECTION,
                "probes.importance",
            ));
        }
        let Some(registry_entry) = probe_registry.get(&probe.probe_id) else {
            diagnostics.push(observation_self_consistency_diagnostic_with_code(
                ValidationCode::ObservationProbeIdUnknown {
                    probe_id: probe.probe_id,
                },
                OP_SC_REGISTRY_BINDING,
                "probe_registry.entries",
                Vec::new(),
            ));
            continue;
        };
        if probe.level != registry_entry.level
            || probe.importance != registry_entry.importance
            || probe.event_shape != registry_entry.event_shape
            || probe.frequency_bound != registry_entry.frequency_bound
            || probe.weight != registry_entry.weight
        {
            diagnostics.push(observation_self_consistency_diagnostic(
                OP_SC_REGISTRY_BINDING,
                "probes.registry_binding",
            ));
        }
        if !probe_source_references_valid(probe, &node_ids, &value_ids, &effect_ids, &anchor_ids) {
            diagnostics.push(observation_self_consistency_diagnostic_with_code(
                ValidationCode::ObservationProbeSourceInvalid {
                    probe_id: probe.probe_id,
                },
                OP_SC_REGISTRY_BINDING,
                "probes.source",
                Vec::new(),
            ));
        }
    }

    for metric in &plan.metrics {
        if !survives_optional_floor(
            metric.importance,
            inputs.op_policy_projection.optional_probe_floor,
        ) || trace_demotion_drops(
            inputs.op_policy_projection.trace_demotion,
            metric.importance,
        ) {
            diagnostics.push(observation_self_consistency_diagnostic(
                OP_SC_GOVERNANCE_SELECTION,
                "metrics.importance",
            ));
        }
        if !metric_registry.contains_key(&metric.metric) {
            diagnostics.push(observation_self_consistency_diagnostic_with_code(
                ValidationCode::ObservationMetricIdUnknown {
                    metric: metric.metric.clone(),
                },
                OP_SC_REGISTRY_BINDING,
                "metrics.registry_binding",
                Vec::new(),
            ));
        }
    }

    if selection.per_class_probe_weight_total != per_class_probe_weight_total(&plan.probes)
        || selection.per_class_metric_weight_total != per_class_metric_weight_total(&plan.metrics)
        || selection.per_class_total_weight
            != combine_weight_totals(
                selection.per_class_probe_weight_total,
                selection.per_class_metric_weight_total,
            )
    {
        diagnostics.push(observation_self_consistency_diagnostic(
            OP_SC_WEIGHT_TOTALS,
            "per_class_weight_total",
        ));
    }
    if enforce_per_class_weight_caps(
        selection.per_class_total_weight,
        inputs.op_policy_projection.profile_observation_caps,
    )
    .is_err()
    {
        diagnostics.push(observation_self_consistency_diagnostic(
            OP_SC_BUDGET_AND_CAPS,
            "profile_observation_caps",
        ));
    }
    if inputs.op_policy_projection.observability_mode == ObservabilityMode::Invariant
        && !plan.trace_budget_projection.fits_declared_budget
    {
        diagnostics.push(observation_self_consistency_diagnostic_with_code(
            observation_invariant_budget_code(
                plan.trace_budget_projection,
                plan.identity.trace_budget,
            ),
            OP_SC_BUDGET_AND_CAPS,
            "trace_budget_projection",
            Vec::new(),
        ));
    }

    if !anchor_table_matches(plan) {
        diagnostics.push(observation_self_consistency_diagnostic(
            OP_SC_ANCHOR_TABLE,
            "anchor_table",
        ));
    }
    if plan
        .provenance
        .semantic_provenance
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>()
        != selected_checkpoint_ids
        || plan
            .provenance
            .probe_provenance
            .keys()
            .copied()
            .collect::<BTreeSet<_>>()
            != selected_probe_ids
        || plan
            .provenance
            .metric_provenance
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>()
            != selected_metric_ids
    {
        diagnostics.push(observation_self_consistency_diagnostic(
            OP_SC_PROVENANCE,
            "provenance",
        ));
    }

    if build_schema.checkpoints.len() != usize::from(build_schema.build_active_count)
        || u32::from(build_schema.mandatory_count) + u32::from(build_schema.optional_count)
            != u32::from(build_schema.build_active_count)
    {
        diagnostics.push(observation_self_consistency_diagnostic(
            OP_SC_BUILD_SCHEMA_COUNTS,
            "build_active_checkpoint_schema.counts",
        ));
    }
    let build_schema_ids = build_schema
        .checkpoints
        .iter()
        .map(|entry| entry.id.clone())
        .collect::<BTreeSet<_>>();
    if build_schema_ids != selected_checkpoint_ids {
        diagnostics.push(observation_self_consistency_diagnostic(
            OP_SC_BUILD_SCHEMA_IDS,
            "build_active_checkpoint_schema.checkpoints",
        ));
    }
    for entry in &build_schema.checkpoints {
        let Some(plan_entry) = plan
            .semantic
            .iter()
            .find(|semantic| semantic.checkpoint == entry.id)
        else {
            diagnostics.push(observation_self_consistency_diagnostic(
                OP_SC_BUILD_SCHEMA_ENTRY,
                "build_active_checkpoint_schema.entry",
            ));
            continue;
        };
        let Some(schema_entry) = bindings.schema.entries.get(&entry.id) else {
            diagnostics.push(observation_self_consistency_diagnostic(
                OP_SC_BUILD_SCHEMA_ENTRY,
                "build_active_checkpoint_schema.original_checkpoint_metadata",
            ));
            continue;
        };
        if entry.source != plan_entry.source
            || entry.kind != plan_entry.kind
            || entry.encoding != plan_entry.encoding
            || entry.attachment_anchor != plan_entry.anchor
            || entry.artifact_role != plan_entry.artifact_role
            || entry.original_checkpoint_metadata.compact != schema_entry.compact
            || entry.original_checkpoint_metadata.stratum != schema_entry.stratum
            || entry.original_checkpoint_metadata.source_op != schema_entry.source_op
        {
            diagnostics.push(observation_self_consistency_diagnostic(
                OP_SC_BUILD_SCHEMA_ENTRY,
                "build_active_checkpoint_schema.entry",
            ));
        }
    }

    if operational_schema.probes.len() != usize::from(operational_schema.probe_count)
        || operational_schema.metrics.len() != usize::from(operational_schema.metric_count)
        || operational_schema.per_class_probe_weight_total != selection.per_class_probe_weight_total
        || operational_schema.per_class_metric_weight_total
            != selection.per_class_metric_weight_total
        || operational_schema.per_class_total_weight != selection.per_class_total_weight
    {
        diagnostics.push(observation_self_consistency_diagnostic(
            OP_SC_OPERATIONAL_SCHEMA_COUNTS,
            "operational_probe_schema.counts",
        ));
    }
    let operational_probe_ids = operational_schema
        .probes
        .iter()
        .map(|probe| probe.instance_id)
        .collect::<BTreeSet<_>>();
    let operational_metric_ids = operational_schema
        .metrics
        .iter()
        .map(|metric| metric.metric.clone())
        .collect::<BTreeSet<_>>();
    if operational_probe_ids != selected_probe_ids || operational_metric_ids != selected_metric_ids
    {
        diagnostics.push(observation_self_consistency_diagnostic(
            OP_SC_OPERATIONAL_SCHEMA_IDS,
            "operational_probe_schema.entries",
        ));
    }

    if observation_plan_self_hash(plan).ok() != Some(observation_plan_self_hash_value) {
        diagnostics.push(observation_self_consistency_diagnostic(
            OP_SC_SELF_HASH,
            "observation_plan_self_hash",
        ));
    }

    let invariant_ids = OP_SC_CHECKS
        .iter()
        .map(|check| check.id)
        .collect::<Vec<_>>();
    let invariant_fields = OP_SC_CHECKS
        .iter()
        .map(|check| check.field)
        .collect::<Vec<_>>();
    let invariant_descriptions = OP_SC_CHECKS
        .iter()
        .map(|check| check.description)
        .collect::<Vec<_>>();
    record_finalization_event(OBSERVATION_SELF_CONSISTENCY_EVENT);
    tracing::info!(
        event = %OBSERVATION_SELF_CONSISTENCY_EVENT,
        invariants_checked = invariant_ids.len() as u64,
        invariant_ids = ?invariant_ids,
        invariant_fields = ?invariant_fields,
        invariant_descriptions = ?invariant_descriptions,
        violations = diagnostics.len() as u64,
    );

    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(NonEmptyList::new(diagnostics).expect("diagnostics are non-empty"))
    }
}

fn check_unique_by<K>(
    keys: impl IntoIterator<Item = K>,
    field: &'static str,
    check: OpScCheck,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) where
    K: Ord,
{
    let mut seen = BTreeSet::new();
    for key in keys {
        if !seen.insert(key) {
            diagnostics.push(observation_self_consistency_diagnostic(check, field));
            return;
        }
    }
}

fn anchor_table_matches(plan: &ObservationPlan) -> bool {
    plan.semantic.iter().all(|entry| {
        plan.anchor_table
            .semantic
            .get(&entry.checkpoint)
            .is_some_and(|attachment| {
                attachment.anchor == entry.anchor && attachment.source == entry.source
            })
    }) && plan.probes.iter().all(|probe| {
        plan.anchor_table
            .probes
            .get(&probe.instance_id)
            .is_some_and(|source| *source == probe.source)
    }) && plan.metrics.iter().all(|metric| {
        plan.anchor_table
            .metrics
            .get(&metric.metric)
            .is_some_and(|source| *source == metric.source)
    }) && plan.anchor_table.semantic.len() == plan.semantic.len()
        && plan.anchor_table.probes.len() == plan.probes.len()
        && plan.anchor_table.metrics.len() == plan.metrics.len()
}

fn probe_source_references_valid(
    probe: &OperationalProbe,
    node_ids: &BTreeSet<NodeId>,
    value_ids: &BTreeSet<ValueId>,
    effect_ids: &BTreeSet<EffectId>,
    anchor_ids: &BTreeSet<Hash256>,
) -> bool {
    match &probe.source {
        ProbeSource::NodePreEntry { node } | ProbeSource::NodePostEntry { node } => {
            node_ids.contains(node)
        }
        ProbeSource::ValueEdge { value } => value_ids.contains(value),
        ProbeSource::EffectEdge { effect, class } => {
            effect_ids.contains(effect) && is_v1_emitted_effect_class(*class)
        }
        ProbeSource::Anchor { anchor } => anchor_ids.contains(&anchor.anchor_id),
    }
}

fn canonical_sort_finalized_observation_plan(
    plan: &mut ObservationPlan,
    build_schema: &mut BuildActiveCheckpointSchema,
    operational_schema: &mut OperationalProbeSchema,
) {
    plan.semantic
        .sort_by(|left, right| left.checkpoint.cmp(&right.checkpoint));
    plan.probes.sort_by_key(|probe| {
        (
            probe.instance_id.probe_id,
            probe.instance_id.source_fingerprint,
        )
    });
    plan.metrics
        .sort_by(|left, right| left.metric.cmp(&right.metric));
    build_schema
        .checkpoints
        .sort_by(|left, right| left.id.cmp(&right.id));
    operational_schema.probes.sort_by_key(|probe| {
        (
            probe.instance_id.probe_id,
            probe.instance_id.source_fingerprint,
        )
    });
    operational_schema
        .metrics
        .sort_by(|left, right| left.metric.cmp(&right.metric));

    record_finalization_event(OBSERVATION_CANONICAL_SORT_EVENT);
    tracing::info!(
        event = %OBSERVATION_CANONICAL_SORT_EVENT,
        semantic_count = plan.semantic.len() as u64,
        probe_count = plan.probes.len() as u64,
        metric_count = plan.metrics.len() as u64,
    );
}

fn semantic_checkpoint_schema_re_emit_body(
    inputs: &ObservationPlanInputs,
    identity: &ObservationPlanIdentity,
    observation_plan_self_hash: Option<Hash256>,
    schema: &BuildActiveCheckpointSchema,
    schema_hash: Hash256,
    diagnostics: Vec<ValidationDiagnostic>,
) -> SemanticCheckpointSchemaReEmitBody {
    SemanticCheckpointSchemaReEmitBody {
        input_identity: SemanticCheckpointSchemaReEmitInputIdentity {
            observation_plan_self_hash,
            original_schema_hash: inputs.semantic_checkpoint_schema_hash,
            infer_ir_self_hash: identity.infer_ir_self_hash,
            quant_graph_self_hash: identity.quant_graph_self_hash,
            artifact_aux_hash: inputs.audit_parents.artifact_aux_hash,
            determinism: identity.determinism,
            workload_id: identity.workload_id.clone(),
        },
        result: Some(SemanticCheckpointSchemaReEmitResult {
            schema_hash,
            checkpoints: schema.checkpoints.clone(),
            build_active_count: schema.build_active_count,
            mandatory_count: schema.mandatory_count,
            optional_count: schema.optional_count,
        }),
        diagnostics,
    }
}

fn operational_probe_schema_body(
    inputs: &ObservationPlanInputs,
    identity: &ObservationPlanIdentity,
    observation_plan_self_hash: Option<Hash256>,
    schema: &OperationalProbeSchema,
    schema_hash: Hash256,
    diagnostics: Vec<ValidationDiagnostic>,
) -> OperationalProbeSchemaBody {
    OperationalProbeSchemaBody {
        input_identity: OperationalProbeSchemaInputIdentity {
            observation_plan_self_hash,
            infer_ir_self_hash: identity.infer_ir_self_hash,
            quant_graph_self_hash: identity.quant_graph_self_hash,
            determinism: identity.determinism,
            observability_mode: identity.observability_mode,
            trace_budget: identity.trace_budget,
            profile_id: inputs.op_policy_projection.profile_id.clone(),
            workload_id: identity.workload_id.clone(),
        },
        result: Some(OperationalProbeSchemaResult {
            schema_hash,
            probes: schema.probes.clone(),
            metrics: schema.metrics.clone(),
            probe_count: schema.probe_count,
            metric_count: schema.metric_count,
            per_class_probe_weight_total: schema.per_class_probe_weight_total,
            per_class_metric_weight_total: schema.per_class_metric_weight_total,
            per_class_total_weight: schema.per_class_total_weight,
        }),
        diagnostics,
    }
}

fn observation_plan_report_body(
    inputs: &ObservationPlanInputs,
    product: &ObservationPlanCoreProduct,
    sc_re_emit_report_self_hash: Hash256,
    operational_probe_schema_report_self_hash: Hash256,
    diagnostics: Vec<ValidationDiagnostic>,
) -> ObservationPlanReportBody {
    ObservationPlanReportBody {
        input_identity: ObservationPlanReportInputIdentity::from_inputs(
            inputs,
            &product.observation_plan.identity,
        ),
        result: Some(ObservationPlanReportResult {
            product: product.observation_plan.clone(),
            semantic_count: checked_len_to_u16(product.observation_plan.semantic.len()),
            probe_count: checked_len_to_u16(product.observation_plan.probes.len()),
            metric_count: checked_len_to_u16(product.observation_plan.metrics.len()),
            mandatory_semantic_count: checked_len_to_u16(
                product
                    .observation_plan
                    .semantic
                    .iter()
                    .filter(|entry| entry.artifact_role == SemanticCheckpointRole::Mandatory)
                    .count(),
            ),
            optional_semantic_count: checked_len_to_u16(
                product
                    .observation_plan
                    .semantic
                    .iter()
                    .filter(|entry| entry.artifact_role == SemanticCheckpointRole::Optional)
                    .count(),
            ),
            per_class_probe_count: per_class_probe_count(&product.observation_plan.probes),
            per_class_metric_count: per_class_metric_count(&product.observation_plan.metrics),
            sc_re_emit_report_self_hash,
            operational_probe_schema_report_self_hash,
            observation_plan_self_hash: product.observation_plan_self_hash,
        }),
        diagnostics,
    }
}

fn report_self_hash_for<R>(outcome: ReportOutcome, body: R) -> Hash256
where
    R: ReportBody + Serialize,
{
    ReportEnvelope::new(outcome, body)
        .expect("report envelope builds")
        .with_computed_self_hash()
        .expect("report self hash computes")
        .report_self_hash
}

fn per_class_probe_count(probes: &[OperationalProbe]) -> PerClassCount {
    let mut total = PerClassCount::default();
    for probe in probes {
        add_importance_count(&mut total, probe.importance);
    }
    total
}

fn per_class_metric_count(metrics: &[MetricProbe]) -> PerClassCount {
    let mut total = PerClassCount::default();
    for metric in metrics {
        add_importance_count(&mut total, metric.importance);
    }
    total
}

fn add_importance_count(total: &mut PerClassCount, class: ProbeImportanceClass) {
    match class {
        ProbeImportanceClass::Required => total.required += 1,
        ProbeImportanceClass::Important => total.important += 1,
        ProbeImportanceClass::Diagnostic => total.diagnostic += 1,
        ProbeImportanceClass::BestEffort => total.best_effort += 1,
    }
}

fn precheck_disabled_probe_ids(
    inputs: &ObservationPlanInputs,
) -> Result<(), NonEmptyList<ValidationDiagnostic>> {
    let mut diagnostics = Vec::new();
    let mut required_rejected = 0_u64;
    let mut unknown_rejected = 0_u64;

    for probe_id in &inputs.op_policy_projection.disabled_optional_probes {
        let Some(entry) = inputs
            .probe_registry
            .entries
            .iter()
            .find(|entry| entry.probe_id == *probe_id)
        else {
            unknown_rejected += 1;
            diagnostics.push(observation_registry_diagnostic(
                ValidationCode::ObservationProbeIdUnknown {
                    probe_id: *probe_id,
                },
                "op_policy_projection.disabled_optional_probes",
                Vec::new(),
            ));
            continue;
        };

        if entry.importance == ProbeImportanceClass::Required {
            required_rejected += 1;
            diagnostics.push(observation_registry_diagnostic(
                ValidationCode::ObservationRequiredProbeDisabled {
                    probe_id: *probe_id,
                },
                "op_policy_projection.disabled_optional_probes",
                vec![entry.evidence.clone()],
            ));
        }
    }

    if !diagnostics.is_empty() {
        record_finalization_event(OBSERVATION_PROBE_BUDGET_GOVERNANCE_EVENT);
        tracing::info!(
            event = %OBSERVATION_PROBE_BUDGET_GOVERNANCE_EVENT,
            surviving_count = 0_u64,
            dropped_floor = 0_u64,
            dropped_demotion = 0_u64,
            dropped_disabled = 0_u64,
            dropped_required_rejected_pre = required_rejected,
            dropped_unknown_rejected_pre = unknown_rejected,
        );
        return Err(NonEmptyList::new(diagnostics).expect("diagnostics are non-empty"));
    }

    Ok(())
}

fn instantiate_registry_probes_v1(
    inputs: &ObservationPlanInputs,
) -> Result<Vec<OperationalProbe>, NonEmptyList<ValidationDiagnostic>> {
    let mut probes = Vec::new();
    let mut diagnostics = Vec::new();

    for entry in &inputs.probe_registry.entries {
        match instantiate_probe_entry_v1(entry, &inputs.infer_ir_product.infer_ir) {
            Ok(mut instances) => {
                record_finalization_event(OBSERVATION_PROBE_REGISTRY_INSTANTIATION_EVENT);
                tracing::info!(
                    event = %OBSERVATION_PROBE_REGISTRY_INSTANTIATION_EVENT,
                    instantiated_count = instances.len() as u64,
                    selector_kind = selector_kind_name(&entry.source_selector),
                    importance_class = importance_class_wire(entry.importance),
                );
                probes.append(&mut instances);
            }
            Err(mut entry_diagnostics) => {
                record_finalization_event(OBSERVATION_PROBE_REGISTRY_INSTANTIATION_EVENT);
                tracing::info!(
                    event = %OBSERVATION_PROBE_REGISTRY_INSTANTIATION_EVENT,
                    instantiated_count = 0_u64,
                    selector_kind = selector_kind_name(&entry.source_selector),
                    importance_class = importance_class_wire(entry.importance),
                );
                diagnostics.append(&mut entry_diagnostics);
            }
        }
    }

    if !diagnostics.is_empty() {
        return Err(NonEmptyList::new(diagnostics).expect("diagnostics are non-empty"));
    }

    Ok(probes)
}

fn instantiate_probe_entry_v1(
    entry: &ProbeRegistryEntry,
    infer_ir: &GbInferIR,
) -> Result<Vec<OperationalProbe>, Vec<ValidationDiagnostic>> {
    if entry.event_shape.max_payload_bytes > ABI_TRACE_EVENT_PAYLOAD_BYTES {
        return Err(vec![observation_registry_diagnostic(
            ValidationCode::ReportSemanticInvariantViolated {
                field: FieldPath::from("probe_registry.entries.event_shape.max_payload_bytes"),
            },
            "probe_registry.entries.event_shape.max_payload_bytes",
            vec![entry.evidence.clone()],
        )]);
    }

    let sources = instantiate_probe_sources_v1(entry, infer_ir)?;
    let mut probes = Vec::with_capacity(sources.len());
    let mut diagnostics = Vec::new();

    for source in sources {
        match probe_instance_source_fingerprint(entry.probe_id, &source) {
            Ok(source_fingerprint) => probes.push(OperationalProbe {
                instance_id: ProbeInstanceId {
                    probe_id: entry.probe_id,
                    source_fingerprint,
                },
                probe_id: entry.probe_id,
                source,
                level: entry.level,
                importance: entry.importance,
                event_shape: entry.event_shape.clone(),
                frequency_bound: entry.frequency_bound,
                weight: entry.weight,
            }),
            Err(_error) => diagnostics.push(observation_registry_diagnostic(
                ValidationCode::ObservationProbeSourceInvalid {
                    probe_id: entry.probe_id,
                },
                "probe_registry.entries.source_selector",
                vec![entry.evidence.clone()],
            )),
        }
    }

    if diagnostics.is_empty() {
        Ok(probes)
    } else {
        Err(diagnostics)
    }
}

fn instantiate_probe_sources_v1(
    entry: &ProbeRegistryEntry,
    infer_ir: &GbInferIR,
) -> Result<Vec<ProbeSource>, Vec<ValidationDiagnostic>> {
    match &entry.source_selector {
        ProbeSourceSelector::ByAnchorCheckpoint { checkpoint, .. } => {
            let feasible = build_feasible_set(infer_ir);
            Ok(feasible
                .get(checkpoint)
                .into_iter()
                .flat_map(|candidates| candidates.iter())
                .filter_map(|candidate| {
                    candidate
                        .anchor
                        .clone()
                        .map(|anchor| ProbeSource::Anchor { anchor })
                })
                .collect())
        }
        ProbeSourceSelector::ByInferOpTag { op_tag, timing } => Ok(infer_ir
            .nodes
            .iter()
            .filter(|node| infer_op_tag_matches(*op_tag, node.op.tag()))
            .map(|node| node_probe_source(node.node_id, *timing))
            .collect()),
        ProbeSourceSelector::ByEffectClass { class } => {
            let mut diagnostics = Vec::new();
            let mut sources = Vec::new();
            for effect in &infer_ir.effects {
                if !effect_class_matches(*class, effect.class) {
                    continue;
                }
                if is_v1_emitted_effect_class(effect.class) {
                    sources.push(ProbeSource::EffectEdge {
                        effect: effect.effect_id,
                        class: effect.class,
                    });
                } else {
                    diagnostics.push(reserved_effect_probe_diagnostic(entry, effect.class));
                }
            }
            if diagnostics.is_empty() {
                Ok(sources)
            } else {
                Err(diagnostics)
            }
        }
        ProbeSourceSelector::ByValueRole { role } => Ok(infer_ir
            .values
            .iter()
            .filter(|value| value_role_matches(*role, value.kind))
            .map(|value| ProbeSource::ValueEdge {
                value: value.value_id,
            })
            .collect()),
    }
}

fn govern_probe_budget_v1(
    probes: Vec<OperationalProbe>,
    policy: &ObservationPolicyProjection,
) -> Result<Vec<OperationalProbe>, NonEmptyList<ValidationDiagnostic>> {
    let mut surviving = Vec::new();
    let mut dropped_floor = 0_u64;
    let mut dropped_demotion = 0_u64;
    let mut dropped_disabled = 0_u64;

    for probe in probes {
        if !survives_optional_floor(probe.importance, policy.optional_probe_floor) {
            dropped_floor += 1;
            continue;
        }
        if trace_demotion_drops(policy.trace_demotion, probe.importance) {
            dropped_demotion += 1;
            continue;
        }
        if policy.disabled_optional_probes.contains(&probe.probe_id) {
            dropped_disabled += 1;
            continue;
        }
        surviving.push(probe);
    }

    record_finalization_event(OBSERVATION_PROBE_BUDGET_GOVERNANCE_EVENT);
    tracing::info!(
        event = %OBSERVATION_PROBE_BUDGET_GOVERNANCE_EVENT,
        surviving_count = surviving.len() as u64,
        dropped_floor,
        dropped_demotion,
        dropped_disabled,
        dropped_required_rejected_pre = 0_u64,
        dropped_unknown_rejected_pre = 0_u64,
    );

    Ok(surviving)
}

pub fn canonical_order_operational_probes_v1(
    mut probes: Vec<OperationalProbe>,
) -> Result<Vec<OperationalProbe>, NonEmptyList<ValidationDiagnostic>> {
    probes.sort_by_key(|probe| {
        (
            probe.instance_id.probe_id,
            probe.instance_id.source_fingerprint,
        )
    });

    let mut diagnostics = Vec::new();
    for pair in probes.windows(2) {
        if pair[0].instance_id == pair[1].instance_id {
            diagnostics.push(observation_registry_diagnostic(
                ValidationCode::ObservationProbeSourceInvalid {
                    probe_id: pair[0].probe_id,
                },
                "probes.instance_id",
                Vec::new(),
            ));
        }
    }

    record_finalization_event(OBSERVATION_PROBE_ORDERING_EVENT);
    tracing::info!(
        event = %OBSERVATION_PROBE_ORDERING_EVENT,
        final_count = probes.len() as u64,
    );

    if diagnostics.is_empty() {
        Ok(probes)
    } else {
        Err(NonEmptyList::new(diagnostics).expect("diagnostics are non-empty"))
    }
}

fn filter_metric_registry_v1(
    registry: &MetricRegistrySnapshot,
) -> Result<Vec<&MetricRegistryEntry>, NonEmptyList<ValidationDiagnostic>> {
    let mut surviving = Vec::new();
    let mut diagnostics = Vec::new();
    let mut dropped_per_slice_reserved = 0_u64;

    for entry in &registry.entries {
        // MetricRegistrySnapshot::new is the primary policy-side guard. This
        // codegen guard catches deserialized or directly-constructed snapshots.
        if entry.source == RegistryMetricSource::PerSliceReserved {
            dropped_per_slice_reserved += 1;
            diagnostics.push(observation_registry_diagnostic(
                ValidationCode::ObservationMetricSourceReservedV1 {
                    metric: entry.metric.clone(),
                },
                "metric_registry.entries.source",
                vec![entry.evidence.clone()],
            ));
            continue;
        }
        surviving.push(entry);
    }

    record_finalization_event(OBSERVATION_METRIC_REGISTRY_FILTER_EVENT);
    tracing::info!(
        event = %OBSERVATION_METRIC_REGISTRY_FILTER_EVENT,
        surviving_count = surviving.len() as u64,
        dropped_per_slice_reserved,
    );

    if diagnostics.is_empty() {
        Ok(surviving)
    } else {
        Err(NonEmptyList::new(diagnostics).expect("diagnostics are non-empty"))
    }
}

fn select_metrics_v1(
    entries: Vec<&MetricRegistryEntry>,
    policy: &ObservationPolicyProjection,
) -> Result<Vec<MetricProbe>, NonEmptyList<ValidationDiagnostic>> {
    let mut metrics = Vec::new();
    let mut diagnostics = Vec::new();
    let mut dropped_floor = 0_u64;
    let mut dropped_demotion = 0_u64;

    for entry in entries {
        // MetricAggregation::new is the primary policy-side guard. This
        // codegen guard catches deserialized or directly-constructed snapshots.
        if let MetricAggregation::Histogram { bucket_count: 0 } = entry.aggregation {
            diagnostics.push(observation_registry_diagnostic(
                ValidationCode::ObservationMetricHistogramBucketCountZero {
                    metric: entry.metric.clone(),
                },
                "metric_registry.entries.aggregation.bucket_count",
                vec![entry.evidence.clone()],
            ));
            continue;
        }
        if !survives_optional_floor(entry.importance, policy.optional_probe_floor) {
            dropped_floor += 1;
            continue;
        }
        if trace_demotion_drops(policy.trace_demotion, entry.importance) {
            dropped_demotion += 1;
            continue;
        }
        metrics.push(MetricProbe {
            metric: entry.metric.clone(),
            source: entry.source,
            aggregation: entry.aggregation,
            importance: entry.importance,
            weight: entry.weight,
        });
    }

    record_finalization_event(OBSERVATION_METRIC_SELECTION_EVENT);
    tracing::info!(
        event = %OBSERVATION_METRIC_SELECTION_EVENT,
        surviving_count = metrics.len() as u64,
        dropped_floor,
        dropped_demotion,
    );

    if diagnostics.is_empty() {
        Ok(metrics)
    } else {
        Err(NonEmptyList::new(diagnostics).expect("diagnostics are non-empty"))
    }
}

fn canonical_order_metric_probes_v1(
    mut metrics: Vec<MetricProbe>,
) -> Result<Vec<MetricProbe>, NonEmptyList<ValidationDiagnostic>> {
    metrics.sort_by(|left, right| left.metric.cmp(&right.metric));

    let mut diagnostics = Vec::new();
    for pair in metrics.windows(2) {
        if pair[0].metric == pair[1].metric {
            diagnostics.push(observation_registry_diagnostic(
                ValidationCode::ReportSemanticInvariantViolated {
                    field: FieldPath::from("metrics.metric"),
                },
                "metrics.metric",
                Vec::new(),
            ));
        }
    }

    record_finalization_event(OBSERVATION_METRIC_ORDERING_EVENT);
    tracing::info!(
        event = %OBSERVATION_METRIC_ORDERING_EVENT,
        final_count = metrics.len() as u64,
    );

    if diagnostics.is_empty() {
        Ok(metrics)
    } else {
        Err(NonEmptyList::new(diagnostics).expect("diagnostics are non-empty"))
    }
}

fn enforce_per_class_weight_caps(
    totals: PerClassWeightTotal,
    caps: ObservationProfileCaps,
) -> Result<(), NonEmptyList<ValidationDiagnostic>> {
    let mut diagnostics = Vec::new();
    for class in ProbeImportanceClass::ALL {
        let Some(cap) = cap_for_importance(caps, class) else {
            continue;
        };
        let observed = weight_for_importance(totals, class);
        if observed > cap {
            diagnostics.push(observation_registry_diagnostic(
                ValidationCode::ObservationProbeClassCapExceeded {
                    class,
                    observed,
                    cap,
                },
                "op_policy_projection.profile_observation_caps",
                Vec::new(),
            ));
        }
    }

    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(NonEmptyList::new(diagnostics).expect("diagnostics are non-empty"))
    }
}

#[must_use]
pub fn anchor_to_checkpoint(tuple: CanonicalProvenanceTuple) -> Option<SemanticCheckpointKind> {
    match tuple {
        CanonicalProvenanceTuple {
            op_tag: InferOpTag::Embedding,
            occurrence_index: 0,
            ..
        } => Some(SemanticCheckpointKind::PostEmbedding {
            layer: LayerId::new(0),
        }),
        CanonicalProvenanceTuple {
            op_tag: InferOpTag::CombineResidual,
            residual_site: Some(ResidualSite::PostSequence),
            occurrence_index: 0,
            ..
        } => None,
        CanonicalProvenanceTuple {
            op_tag: InferOpTag::RouteTop1,
            layer: Some(layer),
            occurrence_index: 0,
            ..
        } => Some(SemanticCheckpointKind::PostRouter { layer }),
        CanonicalProvenanceTuple {
            op_tag: InferOpTag::ExpertMatVec,
            layer: Some(layer),
            expert: Some(expert),
            expert_weight_slot: Some(ExpertWeightSlot::FfnDown),
            occurrence_index: 0,
            ..
        } => Some(SemanticCheckpointKind::PostExpertDowncast { layer, expert }),
        CanonicalProvenanceTuple {
            op_tag: InferOpTag::Classify,
            occurrence_index: 0,
            ..
        } => Some(SemanticCheckpointKind::PostLogits),
        CanonicalProvenanceTuple {
            op_tag: InferOpTag::DecodeToken,
            occurrence_index: 0,
            ..
        } => Some(SemanticCheckpointKind::PostDecode),
        _ => None,
    }
}

pub fn encoding_for(
    checkpoint: SemanticCheckpointKind,
    compare_domain: CompareDomain,
    determinism: DeterminismClass,
) -> ObservationEncoding {
    match compare_domain {
        CompareDomain::ExpertIdOnly
        | CompareDomain::EnvelopeQ8_8
        | CompareDomain::EnvelopeQ16_16 => {
            unreachable!(
                "policy compare domain {compare_domain:?} is unreachable in v1 workload projection"
            )
        }
        CompareDomain::CanonicalValue | CompareDomain::TokenIdOnly => {
            encoding_for_reachable_v1(checkpoint, compare_domain, determinism)
        }
    }
}

pub fn try_encoding_for(
    checkpoint: SemanticCheckpointKind,
    compare_domain: CompareDomain,
    determinism: DeterminismClass,
) -> Result<ObservationEncoding, Box<ValidationDiagnostic>> {
    match compare_domain {
        CompareDomain::ExpertIdOnly
        | CompareDomain::EnvelopeQ8_8
        | CompareDomain::EnvelopeQ16_16 => Err(Box::new(observation_checkpoint_diagnostic(
            ValidationCode::ObservationEncodingInvalidForCheckpoint {
                checkpoint: semantic_checkpoint_kind_to_id(checkpoint),
            },
            "op_policy_projection.workload_observation.compare_domain_policy",
        ))),
        CompareDomain::CanonicalValue | CompareDomain::TokenIdOnly => {
            Ok(encoding_for(checkpoint, compare_domain, determinism))
        }
    }
}

fn encoding_for_reachable_v1(
    checkpoint: SemanticCheckpointKind,
    compare_domain: CompareDomain,
    determinism: DeterminismClass,
) -> ObservationEncoding {
    match (checkpoint, compare_domain) {
        (SemanticCheckpointKind::PostDecode, _) => ObservationEncoding::TokenId,
        (SemanticCheckpointKind::PostRouter { .. }, CompareDomain::TokenIdOnly) => {
            ObservationEncoding::ExpertId
        }
        (SemanticCheckpointKind::PostRouter { .. }, CompareDomain::CanonicalValue)
        | (SemanticCheckpointKind::PostEmbedding { .. }, _)
        | (SemanticCheckpointKind::PostExpertDowncast { .. }, _) => ObservationEncoding::Canonical,
        (SemanticCheckpointKind::PostLogits, CompareDomain::TokenIdOnly) => {
            ObservationEncoding::TokenId
        }
        (SemanticCheckpointKind::PostLogits, CompareDomain::CanonicalValue) => match determinism {
            DeterminismClass::BitExact => ObservationEncoding::Canonical,
            DeterminismClass::Deterministic => ObservationEncoding::QuantizedQ8_8,
            DeterminismClass::Nondeterministic => ObservationEncoding::QuantizedQ16_16,
        },
        (
            _,
            CompareDomain::ExpertIdOnly
            | CompareDomain::EnvelopeQ8_8
            | CompareDomain::EnvelopeQ16_16,
        ) => {
            unreachable!("reserved v1 compare domains are filtered before reachable encoding")
        }
    }
}

fn abi_trace_budget(budget: PolicyTraceBudget) -> TraceBudget {
    TraceBudget::new(
        budget.max_events_per_slice,
        budget.max_bytes_per_frame,
        abi_trace_drop_policy(budget.drop_policy),
    )
    .expect("validated policy trace budget fits ABI trace budget")
}

fn abi_trace_drop_policy(policy: PolicyTraceDropPolicy) -> AbiTraceDropPolicy {
    match policy {
        PolicyTraceDropPolicy::DropOldest => AbiTraceDropPolicy::DropOldest,
        PolicyTraceDropPolicy::DropNewest => AbiTraceDropPolicy::DropNewest,
        PolicyTraceDropPolicy::HaltAndFault => AbiTraceDropPolicy::HaltAndFault,
    }
}

fn node_probe_source(node: NodeId, timing: ProbeTiming) -> ProbeSource {
    match timing {
        ProbeTiming::PreEntry => ProbeSource::NodePreEntry { node },
        ProbeTiming::PostEntry => ProbeSource::NodePostEntry { node },
    }
}

fn infer_op_tag_matches(policy: PolicyInferOpTag, actual: InferOpTag) -> bool {
    matches!(
        (policy, actual),
        (PolicyInferOpTag::Classify, InferOpTag::Classify)
            | (
                PolicyInferOpTag::CombineResidual,
                InferOpTag::CombineResidual
            )
            | (PolicyInferOpTag::DecodeToken, InferOpTag::DecodeToken)
            | (PolicyInferOpTag::Embedding, InferOpTag::Embedding)
            | (PolicyInferOpTag::ExpertMatVec, InferOpTag::ExpertMatVec)
            | (PolicyInferOpTag::FfnActivation, InferOpTag::FfnActivation)
            | (PolicyInferOpTag::Norm, InferOpTag::Norm)
            | (PolicyInferOpTag::RouteTop1, InferOpTag::RouteTop1)
            | (PolicyInferOpTag::RouterMatVec, InferOpTag::RouterMatVec)
            | (
                PolicyInferOpTag::SelectExpertTop1,
                InferOpTag::SelectExpertTop1
            )
            | (PolicyInferOpTag::SequenceRead, InferOpTag::SequenceRead)
            | (PolicyInferOpTag::SequenceStep, InferOpTag::SequenceStep)
            | (PolicyInferOpTag::SequenceWrite, InferOpTag::SequenceWrite)
    )
}

fn effect_class_matches(policy: PolicyEffectClass, actual: EffectClass) -> bool {
    matches!(
        (policy, actual.tag()),
        (
            PolicyEffectClass::FaultBoundary,
            EffectClassTag::FaultBoundary
        ) | (PolicyEffectClass::Rng, EffectClassTag::Rng)
            | (
                PolicyEffectClass::SequenceState,
                EffectClassTag::SequenceState
            )
    )
}

fn value_role_matches(policy: PolicyValueRole, actual: ValueKind) -> bool {
    matches!(
        (policy, actual),
        (PolicyValueRole::Activation, ValueKind::Activation)
            | (PolicyValueRole::DecodedToken, ValueKind::DecodedToken)
            | (PolicyValueRole::EmbeddingOutput, ValueKind::EmbeddingOutput)
            | (PolicyValueRole::ExpertCandidate, ValueKind::ExpertCandidate)
            | (
                PolicyValueRole::ExpertIntermediate,
                ValueKind::ExpertIntermediate
            )
            | (PolicyValueRole::ExpertOutput, ValueKind::ExpertOutput)
            | (PolicyValueRole::GateWeight, ValueKind::GateWeight)
            | (PolicyValueRole::InputToken, ValueKind::InputToken)
            | (PolicyValueRole::LogitVector, ValueKind::LogitVector)
            | (
                PolicyValueRole::NormalizedActivation,
                ValueKind::NormalizedActivation
            )
            | (PolicyValueRole::RouterDecision, ValueKind::RouterDecision)
            | (PolicyValueRole::RouterScore, ValueKind::RouterScore)
            | (
                PolicyValueRole::SequenceBlockOutput,
                ValueKind::SequenceBlockOutput
            )
            | (
                PolicyValueRole::SequenceStateNext,
                ValueKind::SequenceStateNext
            )
            | (
                PolicyValueRole::SequenceStateRead,
                ValueKind::SequenceStateRead
            )
    )
}

fn is_v1_emitted_effect_class(class: EffectClass) -> bool {
    matches!(
        class,
        EffectClass::Rng {
            slot: RngSlot::Decode
        }
    )
}

fn reserved_effect_probe_diagnostic(
    entry: &ProbeRegistryEntry,
    class: EffectClass,
) -> ValidationDiagnostic {
    let code = match class {
        EffectClass::SequenceState { .. } => {
            ValidationCode::ObservationSequenceStateProbeReserved {
                probe_id: entry.probe_id,
            }
        }
        EffectClass::FaultBoundary => ValidationCode::ObservationFaultBoundaryProbeReserved {
            probe_id: entry.probe_id,
        },
        _ => ValidationCode::ObservationReservedEffectProbe {
            probe_id: entry.probe_id,
        },
    };
    observation_registry_diagnostic(
        code,
        "probe_registry.entries.source_selector",
        vec![entry.evidence.clone()],
    )
}

fn survives_optional_floor(class: ProbeImportanceClass, floor: ProbeImportanceClass) -> bool {
    class <= floor
}

fn trace_demotion_drops(demotion: TraceDemotionLevel, class: ProbeImportanceClass) -> bool {
    match demotion {
        TraceDemotionLevel::None => false,
        TraceDemotionLevel::DropBestEffort => class == ProbeImportanceClass::BestEffort,
        TraceDemotionLevel::DropDiagnosticAndBestEffort => {
            matches!(
                class,
                ProbeImportanceClass::Diagnostic | ProbeImportanceClass::BestEffort
            )
        }
    }
}

fn per_class_probe_weight_total(probes: &[OperationalProbe]) -> PerClassWeightTotal {
    let mut total = PerClassWeightTotal::default();
    for probe in probes {
        add_importance_weight(&mut total, probe.importance, probe.weight);
    }
    total
}

fn per_class_metric_weight_total(metrics: &[MetricProbe]) -> PerClassWeightTotal {
    let mut total = PerClassWeightTotal::default();
    for metric in metrics {
        add_importance_weight(&mut total, metric.importance, metric.weight);
    }
    total
}

fn add_importance_weight(
    total: &mut PerClassWeightTotal,
    class: ProbeImportanceClass,
    weight: u16,
) {
    let weight = u32::from(weight);
    match class {
        ProbeImportanceClass::Required => total.required = total.required.saturating_add(weight),
        ProbeImportanceClass::Important => total.important = total.important.saturating_add(weight),
        ProbeImportanceClass::Diagnostic => {
            total.diagnostic = total.diagnostic.saturating_add(weight);
        }
        ProbeImportanceClass::BestEffort => {
            total.best_effort = total.best_effort.saturating_add(weight);
        }
    }
}

fn combine_weight_totals(
    left: PerClassWeightTotal,
    right: PerClassWeightTotal,
) -> PerClassWeightTotal {
    PerClassWeightTotal {
        required: left.required.saturating_add(right.required),
        important: left.important.saturating_add(right.important),
        diagnostic: left.diagnostic.saturating_add(right.diagnostic),
        best_effort: left.best_effort.saturating_add(right.best_effort),
    }
}

fn weight_for_importance(total: PerClassWeightTotal, class: ProbeImportanceClass) -> u32 {
    match class {
        ProbeImportanceClass::Required => total.required,
        ProbeImportanceClass::Important => total.important,
        ProbeImportanceClass::Diagnostic => total.diagnostic,
        ProbeImportanceClass::BestEffort => total.best_effort,
    }
}

fn cap_for_importance(caps: ObservationProfileCaps, class: ProbeImportanceClass) -> Option<u32> {
    match class {
        ProbeImportanceClass::Required => caps.required_max.map(u32::from),
        ProbeImportanceClass::Important => Some(u32::from(caps.important_max)),
        ProbeImportanceClass::Diagnostic => Some(u32::from(caps.diagnostic_max)),
        ProbeImportanceClass::BestEffort => Some(u32::from(caps.best_effort_max)),
    }
}

fn selector_kind_name(selector: &ProbeSourceSelector) -> &'static str {
    match selector {
        ProbeSourceSelector::ByAnchorCheckpoint { .. } => "by_anchor_checkpoint",
        ProbeSourceSelector::ByInferOpTag { .. } => "by_infer_op_tag",
        ProbeSourceSelector::ByEffectClass { .. } => "by_effect_class",
        ProbeSourceSelector::ByValueRole { .. } => "by_value_role",
    }
}

fn importance_class_wire(class: ProbeImportanceClass) -> &'static str {
    match class {
        ProbeImportanceClass::Required => "required",
        ProbeImportanceClass::Important => "important",
        ProbeImportanceClass::Diagnostic => "diagnostic",
        ProbeImportanceClass::BestEffort => "best_effort",
    }
}

fn canonical_provenance_tuples_for_ir(
    infer_ir: &GbInferIR,
) -> BTreeMap<NodeId, CanonicalProvenanceTuple> {
    let mut counts = BTreeMap::<String, u32>::new();
    let mut tuples = BTreeMap::new();
    for node in &infer_ir.nodes {
        let mut tuple = canonical_provenance_tuple_for_op(node.op);
        let key = serde_json::to_string(&tuple)
            .expect("canonical provenance tuple serializes for occurrence key");
        let occurrence = counts.entry(key).or_default();
        tuple.occurrence_index = *occurrence;
        *occurrence += 1;
        tuples.insert(node.node_id, tuple);
    }
    tuples
}

fn canonical_provenance_tuple_for_op(op: InferOp) -> CanonicalProvenanceTuple {
    let mut tuple = CanonicalProvenanceTuple::new(op.tag(), 0);
    match op {
        InferOp::Embedding { .. } | InferOp::Classify | InferOp::DecodeToken { .. } => {}
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
        InferOp::Norm { .. } => {}
    }
    tuple
}

fn observation_source_for_node(
    kind: SemanticCheckpointKind,
    node: &GbNode,
) -> Option<ObservationSource> {
    match kind {
        SemanticCheckpointKind::PostEmbedding { .. } => {
            node.outputs
                .first()
                .copied()
                .map(|value| ObservationSource::NodeOutput {
                    node: node.node_id,
                    value,
                })
        }
        SemanticCheckpointKind::PostRouter { .. } => {
            let decision = node.outputs.first().copied()?;
            let weight = node.outputs.get(1).copied()?;
            Some(ObservationSource::RouterDecision {
                node: node.node_id,
                decision,
                weight,
            })
        }
        SemanticCheckpointKind::PostExpertDowncast { layer, expert } => node
            .outputs
            .first()
            .copied()
            .map(|candidate| ObservationSource::ExpertCandidate {
                node: node.node_id,
                candidate,
                layer,
                expert,
            }),
        SemanticCheckpointKind::PostLogits => {
            node.outputs
                .first()
                .copied()
                .map(|value| ObservationSource::LogitVector {
                    node: node.node_id,
                    value,
                })
        }
        SemanticCheckpointKind::PostDecode => {
            node.outputs
                .first()
                .copied()
                .map(|value| ObservationSource::DecodedToken {
                    node: node.node_id,
                    value,
                })
        }
    }
}

fn observation_checkpoint_diagnostic(
    code: ValidationCode,
    field: &'static str,
) -> ValidationDiagnostic {
    ValidationDiagnostic::hard(
        ValidationOrigin::ObservationPlanConstruction,
        code,
        ValidationDetail::Field {
            field: FieldPath::from(field),
        },
        Vec::new(),
    )
}

fn observation_registry_diagnostic(
    code: ValidationCode,
    field: &'static str,
    provenance: Vec<EvidenceRef>,
) -> ValidationDiagnostic {
    let provenance = provenance
        .into_iter()
        .map(|mut evidence| {
            if evidence.hash.is_none() {
                evidence.hash = Some(Hash256::ZERO);
            }
            evidence
        })
        .collect();
    ValidationDiagnostic::hard(
        ValidationOrigin::ObservationPlanConstruction,
        code,
        ValidationDetail::Field {
            field: FieldPath::from(field),
        },
        provenance,
    )
}

fn observation_input_error_diagnostic(error: ObservationPlanInputError) -> ValidationDiagnostic {
    match error {
        ObservationPlanInputError::SemanticCheckpointSchemaHashMismatch { expected, observed } => {
            stage4_observation_sc_hash_mismatch_diagnostic(expected, observed)
        }
        ObservationPlanInputError::DeterminismMismatch { .. } => {
            stage4_observation_determinism_mismatch_diagnostic(
                "op_policy_projection.determinism_class",
            )
        }
        ObservationPlanInputError::CompareDomainProjectionDrift { .. }
        | ObservationPlanInputError::WorkloadDeterminismProjectionDrift { .. }
        | ObservationPlanInputError::PolicyWorkloadDeterminismDrift { .. } => {
            observation_checkpointless_invariant_diagnostic("op_policy_projection")
        }
    }
}

fn observation_hash_diagnostic(message: String) -> ValidationDiagnostic {
    let _ = message;
    observation_checkpointless_invariant_diagnostic("observation_policy_projection_hash")
}

fn observation_checkpointless_invariant_diagnostic(field: &'static str) -> ValidationDiagnostic {
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

fn observation_self_consistency_diagnostic(
    check: OpScCheck,
    field: &'static str,
) -> ValidationDiagnostic {
    observation_self_consistency_diagnostic_with_code(
        ValidationCode::ReportSemanticInvariantViolated {
            field: op_sc_field(check, field),
        },
        check,
        field,
        Vec::new(),
    )
}

fn observation_self_consistency_diagnostic_with_code(
    code: ValidationCode,
    check: OpScCheck,
    field: &'static str,
    provenance: Vec<EvidenceRef>,
) -> ValidationDiagnostic {
    let field = op_sc_field(check, field);
    ValidationDiagnostic::hard(
        ValidationOrigin::SemanticCore,
        code,
        ValidationDetail::Field { field },
        provenance,
    )
}

fn op_sc_field(check: OpScCheck, field: &'static str) -> FieldPath {
    FieldPath::from(format!("{}.{}", check.id, field))
}

fn checked_len_to_u16(len: usize) -> u16 {
    u16::try_from(len).expect("Stage 4 semantic checkpoint count fits u16")
}

pub fn probe_instance_source_fingerprint(
    probe_id: TraceProbeId,
    source: &ProbeSource,
) -> Result<Hash256, serde_json::Error> {
    #[derive(Serialize)]
    struct FingerprintMaterial<'a> {
        probe_id: TraceProbeId,
        source: &'a ProbeSource,
    }

    domain_hash(
        "ProbeInstanceSource",
        OPERATIONAL_PROBE_SCHEMA_VERSION,
        &FingerprintMaterial { probe_id, source },
    )
}

pub fn observation_plan_self_hash(plan: &ObservationPlan) -> Result<Hash256, serde_json::Error> {
    domain_hash("ObservationPlan", OBSERVATION_PLAN_SCHEMA_VERSION, plan)
}

pub fn observation_plan_core_product_hash(
    product: &ObservationPlanCoreProduct,
) -> Result<Hash256, serde_json::Error> {
    let hash = domain_hash(
        "ObservationPlanCoreProduct",
        OBSERVATION_PLAN_SCHEMA_VERSION,
        product,
    )?;
    tracing::info!(
        event = %OBSERVATION_CORE_PRODUCT_HASH_COMPUTED_EVENT,
        hash = %hash,
    );
    Ok(hash)
}

pub fn build_active_checkpoint_schema_hash(
    schema: &BuildActiveCheckpointSchema,
) -> Result<Hash256, serde_json::Error> {
    domain_hash(
        "BuildActiveCheckpointSchema",
        BUILD_ACTIVE_CHECKPOINT_SCHEMA_VERSION,
        schema.checkpoints.as_slice(),
    )
}

pub fn operational_probe_schema_hash(
    schema: &OperationalProbeSchema,
) -> Result<Hash256, serde_json::Error> {
    #[derive(Serialize)]
    struct OperationalProbeSchemaHashProjection<'a> {
        probes: &'a [ProbeSchemaEntry],
        metrics: &'a [MetricSchemaEntry],
    }

    domain_hash(
        "OperationalProbeSchema",
        OPERATIONAL_PROBE_SCHEMA_VERSION,
        &OperationalProbeSchemaHashProjection {
            probes: schema.probes.as_slice(),
            metrics: schema.metrics.as_slice(),
        },
    )
}

pub fn log_observation_plan_core_product_audit_drift(
    audit_field: &'static str,
    old_hash: Hash256,
    new_hash: Hash256,
) {
    tracing::debug!(
        event = %OBSERVATION_CORE_PRODUCT_AUDIT_DRIFT_DETECTED_EVENT,
        audit_field,
        old_hash = %old_hash,
        new_hash = %new_hash,
    );
}

fn validate_product_report_body(
    outcome: ReportOutcome,
    has_result: bool,
    diagnostics: &[ValidationDiagnostic],
) -> Result<(), Vec<ValidationDiagnostic>> {
    let has_hard = diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Hard);

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

/// Stage 4 report/product hash helper.
///
/// This intentionally keeps the gbf-codegen domain tuple
/// `gbf-codegen:<type>:<schema>` used by existing ObservationPlan artifacts.
/// Do not substitute `gbf_policy::canonical::domain_hash`, whose first
/// argument is the policy crate/domain string and whose callers own
/// policy-registry hashes rather than codegen report/product hashes.
fn domain_hash<T: Serialize + ?Sized>(
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

fn canonical_json_bytes<T: Serialize + ?Sized>(value: &T) -> Result<Vec<u8>, serde_json::Error> {
    let value = serde_json::to_value(value)?;
    Ok(canonicalize_value(&value).expect("Stage 4 material canonicalizes"))
}

fn deserialize_unique_string_keyed_map<'de, D, K, V>(
    deserializer: D,
    key_label: &'static str,
) -> Result<BTreeMap<K, V>, D::Error>
where
    D: serde::Deserializer<'de>,
    K: Deserialize<'de> + Ord,
    V: Deserialize<'de>,
{
    struct UniqueStringKeyedMapVisitor<K, V> {
        key_label: &'static str,
        marker: PhantomData<(K, V)>,
    }

    impl<'de, K, V> Visitor<'de> for UniqueStringKeyedMapVisitor<K, V>
    where
        K: Deserialize<'de> + Ord,
        V: Deserialize<'de>,
    {
        type Value = BTreeMap<K, V>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(formatter, "a map with unique {} keys", self.key_label)
        }

        fn visit_map<A>(self, mut access: A) -> Result<Self::Value, A::Error>
        where
            A: MapAccess<'de>,
        {
            let mut map = BTreeMap::new();
            while let Some((key, value)) = access.next_entry::<K, V>()? {
                if map.insert(key, value).is_some() {
                    return Err(serde::de::Error::custom(format!(
                        "duplicate {}",
                        self.key_label
                    )));
                }
            }
            Ok(map)
        }
    }

    deserializer.deserialize_map(UniqueStringKeyedMapVisitor {
        key_label,
        marker: PhantomData,
    })
}

fn parse_layer(value: &str) -> Option<LayerId> {
    parse_canonical_u16(value).map(LayerId::new)
}

fn parse_expert(value: &str) -> Option<ExpertId> {
    parse_canonical_u16(value).map(ExpertId::new)
}

fn parse_canonical_u16(value: &str) -> Option<u16> {
    let parsed = value.parse::<u16>().ok()?;
    if parsed.to_string() == value {
        Some(parsed)
    } else {
        None
    }
}

mod canonical_provenance_tuple_json {
    use gbf_foundation::{ExpertId, LayerId};
    use serde::{Serialize, Serializer};

    use super::{
        CanonicalProvenanceTuple, ExpertWeightSlot, InferOpTag, NormSite, ResidualSite, StateSlotId,
    };

    #[derive(Serialize)]
    struct TupleProjection {
        op_tag: InferOpTag,
        #[serde(skip_serializing_if = "Option::is_none")]
        layer: Option<LayerId>,
        #[serde(skip_serializing_if = "Option::is_none")]
        expert: Option<ExpertId>,
        #[serde(skip_serializing_if = "Option::is_none")]
        expert_weight_slot: Option<ExpertWeightSlot>,
        #[serde(skip_serializing_if = "Option::is_none")]
        norm_site: Option<NormSite>,
        #[serde(skip_serializing_if = "Option::is_none")]
        state_slot: Option<StateSlotId>,
        #[serde(skip_serializing_if = "Option::is_none")]
        residual_site: Option<ResidualSite>,
        occurrence_index: u32,
    }

    pub fn serialize<S>(tuple: &CanonicalProvenanceTuple, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        TupleProjection {
            op_tag: tuple.op_tag,
            layer: tuple.layer,
            expert: tuple.expert,
            expert_weight_slot: tuple.expert_weight_slot,
            norm_site: tuple.norm_site,
            state_slot: tuple.state_slot,
            residual_site: tuple.residual_site,
            occurrence_index: tuple.occurrence_index,
        }
        .serialize(serializer)
    }
}

mod semantic_attachment_map {
    use std::collections::BTreeMap;

    use serde::{Deserializer, Serialize, Serializer};

    use super::{SemanticAttachment, SemanticCheckpointId, deserialize_unique_string_keyed_map};

    pub fn serialize<S>(
        map: &BTreeMap<SemanticCheckpointId, SemanticAttachment>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        map.serialize(serializer)
    }

    pub fn deserialize<'de, D>(
        deserializer: D,
    ) -> Result<BTreeMap<SemanticCheckpointId, SemanticAttachment>, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_unique_string_keyed_map(deserializer, "semantic checkpoint id")
    }
}

mod metric_source_map {
    use std::collections::BTreeMap;

    use gbf_policy::{MetricId, MetricSource as RegistryMetricSource};
    use serde::{Deserializer, Serialize, Serializer};

    use super::deserialize_unique_string_keyed_map;

    pub fn serialize<S>(
        map: &BTreeMap<MetricId, RegistryMetricSource>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        map.serialize(serializer)
    }

    pub fn deserialize<'de, D>(
        deserializer: D,
    ) -> Result<BTreeMap<MetricId, RegistryMetricSource>, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_unique_string_keyed_map(deserializer, "metric id")
    }
}

mod semantic_evidence_map {
    use std::collections::BTreeMap;

    use gbf_foundation::EvidenceRef;
    use serde::{Deserializer, Serialize, Serializer};

    use super::{SemanticCheckpointId, deserialize_unique_string_keyed_map};

    pub fn serialize<S>(
        map: &BTreeMap<SemanticCheckpointId, EvidenceRef>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        map.serialize(serializer)
    }

    pub fn deserialize<'de, D>(
        deserializer: D,
    ) -> Result<BTreeMap<SemanticCheckpointId, EvidenceRef>, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_unique_string_keyed_map(deserializer, "semantic checkpoint id")
    }
}

mod metric_evidence_map {
    use std::collections::BTreeMap;

    use gbf_foundation::EvidenceRef;
    use gbf_policy::MetricId;
    use serde::{Deserializer, Serialize, Serializer};

    use super::deserialize_unique_string_keyed_map;

    pub fn serialize<S>(
        map: &BTreeMap<MetricId, EvidenceRef>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        map.serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<BTreeMap<MetricId, EvidenceRef>, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_unique_string_keyed_map(deserializer, "metric id")
    }
}

mod probe_source_map {
    use std::collections::BTreeMap;

    use serde::ser::Error as _;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use super::{ProbeInstanceId, ProbeSource, canonical_json_bytes};

    #[derive(Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Entry {
        key: ProbeInstanceId,
        value: ProbeSource,
    }

    pub fn serialize<S>(
        map: &BTreeMap<ProbeInstanceId, ProbeSource>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut entries = Vec::with_capacity(map.len());
        for (key, value) in map {
            entries.push((
                canonical_json_bytes(key).map_err(S::Error::custom)?,
                Entry {
                    key: *key,
                    value: value.clone(),
                },
            ));
        }
        entries.sort_by(|left, right| left.0.cmp(&right.0));
        entries
            .into_iter()
            .map(|(_, entry)| entry)
            .collect::<Vec<_>>()
            .serialize(serializer)
    }

    pub fn deserialize<'de, D>(
        deserializer: D,
    ) -> Result<BTreeMap<ProbeInstanceId, ProbeSource>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let mut map = BTreeMap::new();
        for entry in Vec::<Entry>::deserialize(deserializer)? {
            if map.insert(entry.key, entry.value).is_some() {
                return Err(serde::de::Error::custom("duplicate probe instance id"));
            }
        }
        Ok(map)
    }
}

mod probe_evidence_map {
    use std::collections::BTreeMap;

    use gbf_foundation::EvidenceRef;
    use serde::ser::Error as _;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use super::{ProbeInstanceId, canonical_json_bytes};

    #[derive(Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Entry {
        key: ProbeInstanceId,
        value: EvidenceRef,
    }

    pub fn serialize<S>(
        map: &BTreeMap<ProbeInstanceId, EvidenceRef>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut entries = Vec::with_capacity(map.len());
        for (key, value) in map {
            entries.push((
                canonical_json_bytes(key).map_err(S::Error::custom)?,
                Entry {
                    key: *key,
                    value: value.clone(),
                },
            ));
        }
        entries.sort_by(|left, right| left.0.cmp(&right.0));
        entries
            .into_iter()
            .map(|(_, entry)| entry)
            .collect::<Vec<_>>()
            .serialize(serializer)
    }

    pub fn deserialize<'de, D>(
        deserializer: D,
    ) -> Result<BTreeMap<ProbeInstanceId, EvidenceRef>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let mut map = BTreeMap::new();
        for entry in Vec::<Entry>::deserialize(deserializer)? {
            if map.insert(entry.key, entry.value).is_some() {
                return Err(serde::de::Error::custom("duplicate probe instance id"));
            }
        }
        Ok(map)
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    use crate::s1::quant_graph::{DecodePlanId, QuantFormat};
    use crate::s3::infer_ir::{
        EffectClass, EffectDecl, EffectProvenance, GbInferIR, InferIrAuditParents, InferIrIdentity,
        InferIrProvenance, InferOpTag, QuantGraphEntityRef, RngSlot, StateSlotId, TokenIngressMode,
        TokenInput, TokenInputId, ValueDecl, ValueFormat, ValueKind, ValueLayout, ValueProducerRef,
    };
    use gbf_abi::{CURRENT_ABI, CheckpointEntry, TraceDropPolicy};
    use gbf_foundation::FieldPath;
    use gbf_policy::{
        EffectClass as PolicyEffectClass, InferOpTag as PolicyInferOpTag, MetricRegistryEntry,
        MetricSource, ProbeRegistryEntry, ProbeSourceSelector, ProbeTiming,
        TraceDropPolicy as PolicyTraceDropPolicy, ValueRole as PolicyValueRole, metric_registry_v1,
        probe_registry_v1, trace_event_layout_registry_v1,
    };
    use gbf_policy::{TraceEventPayloadLayout, TraceEventTupleSpecId};
    use gbf_policy::{ValidationCode, ValidationDetail, ValidationOrigin};
    use gbf_report::report_schemas::infer_ir_v1::FixtureEquivalenceTag;
    use gbf_store::blob::BlobStore;
    use gbf_store::stage_cache::StageCache;
    use gbf_workload::manifest::{
        CheckpointSelection, CompareDomain as WorkloadCompareDomain, DeterminismRequirement,
        TraceLevel,
    };
    use tracing::field::{Field, Visit};
    use tracing::{Event, Subscriber};
    use tracing_subscriber::Layer;
    use tracing_subscriber::layer::Context;
    use tracing_subscriber::prelude::*;

    #[derive(Clone, Default)]
    struct CapturedTracingLayer {
        events: std::sync::Arc<std::sync::Mutex<Vec<CapturedEvent>>>,
    }

    #[derive(Debug, Clone)]
    struct CapturedEvent {
        name: String,
        fields: BTreeMap<String, String>,
    }

    impl CapturedTracingLayer {
        fn events(&self) -> Vec<CapturedEvent> {
            self.events.lock().expect("events lock").clone()
        }
    }

    impl<S> Layer<S> for CapturedTracingLayer
    where
        S: Subscriber,
    {
        fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
            let mut visitor = CapturedFieldVisitor::default();
            event.record(&mut visitor);
            let name = visitor
                .fields
                .get("event")
                .cloned()
                .unwrap_or_else(|| event.metadata().name().to_owned());
            self.events
                .lock()
                .expect("events lock")
                .push(CapturedEvent {
                    name,
                    fields: visitor.fields,
                });
        }
    }

    #[derive(Default)]
    struct CapturedFieldVisitor {
        fields: BTreeMap<String, String>,
    }

    impl Visit for CapturedFieldVisitor {
        fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
            self.fields
                .insert(field.name().to_owned(), format!("{value:?}"));
        }

        fn record_str(&mut self, field: &Field, value: &str) {
            self.fields
                .insert(field.name().to_owned(), value.to_owned());
        }

        fn record_u64(&mut self, field: &Field, value: u64) {
            self.fields
                .insert(field.name().to_owned(), value.to_string());
        }

        fn record_i64(&mut self, field: &Field, value: i64) {
            self.fields
                .insert(field.name().to_owned(), value.to_string());
        }

        fn record_bool(&mut self, field: &Field, value: bool) {
            self.fields
                .insert(field.name().to_owned(), value.to_string());
        }
    }

    fn inputs_fixture() -> ObservationPlanInputs {
        ObservationPlanInputs {
            infer_ir_product: infer_ir_product_fixture(DeterminismClass::BitExact),
            infer_ir_self_hash: hash(0x30),
            quant_graph_self_hash: hash(0x31),
            semantic_checkpoint_schema: semantic_checkpoint_schema_fixture(),
            semantic_checkpoint_schema_hash: hash(0x32),
            artifact_declared_semantic_checkpoint_schema_hash: hash(0x32),
            probe_registry: probe_registry_v1(),
            probe_registry_hash: hash(0x33),
            metric_registry: metric_registry_v1(),
            metric_registry_hash: hash(0x34),
            trace_event_layout_registry: trace_event_layout_registry_v1(),
            trace_event_layout_registry_hash: hash(0x35),
            op_policy_projection: policy_projection_fixture(),
            audit_parents: audit_parents_fixture(0x40),
        }
    }

    fn infer_ir_product_fixture(determinism: DeterminismClass) -> GbInferIRProduct {
        let infer_ir = GbInferIR::new(
            InferIrIdentity {
                quant_graph_self_hash: hash(0x31),
                infer_ir_policy_projection_hash: hash(0x36),
                static_budget_self_hash: hash(0x37),
                requested_runtime_modes_hash: hash(0x38),
                determinism,
                topological_order_hash: hash(0x39),
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
                policy_resolution_self_hash: hash(0x41),
                compile_request_hash: hash(0x42),
            },
            BTreeSet::new(),
            FixtureEquivalenceTag::VerifiedFixtureBitExact,
        )
        .expect("infer ir product builds")
    }

    fn semantic_checkpoint_schema_fixture() -> SemanticCheckpointSchema {
        SemanticCheckpointSchema {
            schema_version: 1,
            abi_version: CURRENT_ABI,
            build_hash: [1; 32],
            compile_request_hash: [2; 32],
            checkpoints: vec![CheckpointEntry {
                semantic: SemanticCheckpointId::from_static("layer.0.post_embedding")
                    .expect("semantic id is valid"),
                compact: CompactCheckpointId(1),
                stratum: SemanticStratum::Denotation,
                source_op: Some("embedding".into()),
            }],
        }
    }

    fn semantic_checkpoint_schema_from_entries(
        entries: Vec<(SemanticCheckpointKind, u16, SemanticStratum, &'static str)>,
    ) -> SemanticCheckpointSchema {
        SemanticCheckpointSchema {
            schema_version: 1,
            abi_version: CURRENT_ABI,
            build_hash: [1; 32],
            compile_request_hash: [2; 32],
            checkpoints: entries
                .into_iter()
                .map(|(kind, compact, stratum, source_op)| CheckpointEntry {
                    semantic: semantic_checkpoint_kind_to_id(kind),
                    compact: CompactCheckpointId(compact),
                    stratum,
                    source_op: Some(source_op.into()),
                })
                .collect(),
        }
    }

    fn semantic_checkpoint_schema_all_v1() -> SemanticCheckpointSchema {
        semantic_checkpoint_schema_from_entries(vec![
            (
                SemanticCheckpointKind::PostEmbedding {
                    layer: LayerId::new(0),
                },
                1,
                SemanticStratum::Denotation,
                "embedding",
            ),
            (
                SemanticCheckpointKind::PostRouter {
                    layer: LayerId::new(0),
                },
                2,
                SemanticStratum::Operational,
                "route_top1",
            ),
            (
                SemanticCheckpointKind::PostExpertDowncast {
                    layer: LayerId::new(0),
                    expert: ExpertId::new(1),
                },
                3,
                SemanticStratum::Operational,
                "expert_down",
            ),
            (
                SemanticCheckpointKind::PostLogits,
                4,
                SemanticStratum::Artifact,
                "classify",
            ),
            (
                SemanticCheckpointKind::PostDecode,
                5,
                SemanticStratum::Denotation,
                "decode",
            ),
        ])
    }

    fn semantic_binding_inputs_fixture() -> ObservationPlanInputs {
        let mut inputs = inputs_fixture();
        let product = semantic_infer_ir_product_fixture();
        inputs.infer_ir_self_hash = product.infer_ir_self_hash;
        inputs.quant_graph_self_hash = product.infer_ir.identity.quant_graph_self_hash;
        inputs.infer_ir_product = product;
        inputs.semantic_checkpoint_schema = semantic_checkpoint_schema_all_v1();
        inputs
    }

    fn semantic_infer_ir_product_fixture() -> GbInferIRProduct {
        let token_input = TokenInputId::new(0);
        let input = ValueId::new(0);
        let embedding_value = ValueId::new(1);
        let router_decision = ValueId::new(2);
        let gate_weight = ValueId::new(3);
        let expert_candidate = ValueId::new(4);
        let logits = ValueId::new(5);
        let decoded = ValueId::new(6);
        let embedding = NodeId::new(0);
        let route = NodeId::new(1);
        let expert = NodeId::new(2);
        let classify = NodeId::new(3);
        let decode = NodeId::new(4);
        let layer = LayerId::new(0);
        let expert_id = ExpertId::new(1);
        let plan = DecodePlanId::new(0);

        let values = vec![
            ValueDecl {
                value_id: input,
                kind: ValueKind::InputToken,
                format: ValueFormat::TokenIdDomain { vocab_size: 257 },
                layout: ValueLayout::scalar(),
            },
            ValueDecl {
                value_id: embedding_value,
                kind: ValueKind::EmbeddingOutput,
                format: ValueFormat::Quant {
                    format: QuantFormat::Q8_8,
                },
                layout: ValueLayout::scalar(),
            },
            ValueDecl {
                value_id: router_decision,
                kind: ValueKind::RouterDecision,
                format: ValueFormat::ExpertIdDomain { n_experts: 2 },
                layout: ValueLayout::scalar(),
            },
            ValueDecl {
                value_id: gate_weight,
                kind: ValueKind::GateWeight,
                format: ValueFormat::Quant {
                    format: QuantFormat::Q8_8,
                },
                layout: ValueLayout::scalar(),
            },
            ValueDecl {
                value_id: expert_candidate,
                kind: ValueKind::ExpertCandidate,
                format: ValueFormat::ExactAccumulator,
                layout: ValueLayout::scalar(),
            },
            ValueDecl {
                value_id: logits,
                kind: ValueKind::LogitVector,
                format: ValueFormat::ExactAccumulator,
                layout: ValueLayout::scalar(),
            },
            ValueDecl {
                value_id: decoded,
                kind: ValueKind::DecodedToken,
                format: ValueFormat::TokenIdDomain { vocab_size: 257 },
                layout: ValueLayout::scalar(),
            },
        ];
        let nodes = vec![
            GbNode {
                node_id: embedding,
                op: InferOp::Embedding { token_input },
                inputs: vec![input],
                effects_in: Vec::new(),
                outputs: vec![embedding_value],
                effects_out: Vec::new(),
                reduction_site: None,
            },
            GbNode {
                node_id: route,
                op: InferOp::RouteTop1 { layer },
                inputs: vec![embedding_value],
                effects_in: Vec::new(),
                outputs: vec![router_decision, gate_weight],
                effects_out: Vec::new(),
                reduction_site: None,
            },
            GbNode {
                node_id: expert,
                op: InferOp::ExpertMatVec {
                    layer,
                    expert: expert_id,
                    slot: ExpertWeightSlot::FfnDown,
                },
                inputs: vec![embedding_value],
                effects_in: Vec::new(),
                outputs: vec![expert_candidate],
                effects_out: Vec::new(),
                reduction_site: None,
            },
            GbNode {
                node_id: classify,
                op: InferOp::Classify,
                inputs: vec![embedding_value],
                effects_in: Vec::new(),
                outputs: vec![logits],
                effects_out: Vec::new(),
                reduction_site: None,
            },
            GbNode {
                node_id: decode,
                op: InferOp::DecodeToken { plan },
                inputs: vec![logits],
                effects_in: Vec::new(),
                outputs: vec![decoded],
                effects_out: Vec::new(),
                reduction_site: None,
            },
        ];
        let provenance = InferIrProvenance {
            nodes: BTreeMap::from([
                (embedding, QuantGraphEntityRef::Embedding),
                (route, QuantGraphEntityRef::RouterSelection { layer }),
                (
                    expert,
                    QuantGraphEntityRef::ExpertSection {
                        layer,
                        expert: expert_id,
                    },
                ),
                (classify, QuantGraphEntityRef::ClassifyHead),
                (decode, QuantGraphEntityRef::DecodePlan { plan }),
            ]),
            values: BTreeMap::from([
                (input, ValueProducerRef::External { token_input }),
                (embedding_value, ValueProducerRef::Node { node: embedding }),
                (router_decision, ValueProducerRef::Node { node: route }),
                (gate_weight, ValueProducerRef::Node { node: route }),
                (expert_candidate, ValueProducerRef::Node { node: expert }),
                (logits, ValueProducerRef::Node { node: classify }),
                (decoded, ValueProducerRef::Node { node: decode }),
            ]),
            effects: BTreeMap::new(),
        };
        let anchors = BTreeMap::from([
            (embedding, anchor(0x80)),
            (route, anchor(0x81)),
            (expert, anchor(0x82)),
            (classify, anchor(0x83)),
            (decode, anchor(0x84)),
        ]);
        let infer_ir = GbInferIR::new(
            InferIrIdentity {
                quant_graph_self_hash: hash(0x31),
                infer_ir_policy_projection_hash: hash(0x36),
                static_budget_self_hash: hash(0x37),
                requested_runtime_modes_hash: hash(0x38),
                determinism: DeterminismClass::BitExact,
                topological_order_hash: hash(0x39),
            },
            vec![
                TokenInput::new(
                    token_input,
                    input,
                    BTreeSet::from([TokenIngressMode::Prompt]),
                )
                .expect("token input is valid"),
            ],
            nodes,
            values,
            Vec::new(),
            provenance,
            anchors,
        )
        .expect("semantic infer ir fixture is valid");

        GbInferIRProduct::new(
            infer_ir,
            InferIrAuditParents {
                policy_resolution_self_hash: hash(0x41),
                compile_request_hash: hash(0x42),
            },
            BTreeSet::new(),
            FixtureEquivalenceTag::VerifiedFixtureBitExact,
        )
        .expect("semantic infer ir product builds")
    }

    fn with_schema(
        mut inputs: ObservationPlanInputs,
        schema: SemanticCheckpointSchema,
    ) -> ObservationPlanInputs {
        inputs.semantic_checkpoint_schema = schema;
        inputs
    }

    fn with_workload_compare_domain(
        mut inputs: ObservationPlanInputs,
        compare_domain: WorkloadCompareDomain,
    ) -> ObservationPlanInputs {
        inputs
            .op_policy_projection
            .workload_observation
            .compare_domain_workload = compare_domain;
        inputs
            .op_policy_projection
            .workload_observation
            .compare_domain_policy = CompareDomain::from(compare_domain);
        inputs
    }

    fn policy_projection_fixture() -> ObservationPolicyProjection {
        ObservationPolicyProjection {
            profile_id: CompileProfileId::from("Default"),
            profile_observation_caps: ObservationProfileCaps {
                required_max: None,
                important_max: 256,
                diagnostic_max: 128,
                best_effort_max: 64,
            },
            determinism_class: DeterminismClass::BitExact,
            observability_mode: ObservabilityMode::Invariant,
            trace_budget: PolicyTraceBudget {
                max_events_per_slice: 128,
                max_bytes_per_frame: 2048,
                drop_policy: PolicyTraceDropPolicy::DropOldest,
            },
            trace_demotion: TraceDemotionLevel::None,
            optional_probe_floor: ProbeImportanceClass::Important,
            workload_observation: WorkloadObservationProjection {
                workload_id: WorkloadId::from("workload.fixture"),
                checkpoint_selection: CheckpointSelection::SemanticAndOperational,
                trace_level: TraceLevel::Checkpoints,
                compare_domain_workload: WorkloadCompareDomain::TokenLogits,
                compare_domain_policy: CompareDomain::from(WorkloadCompareDomain::TokenLogits),
                determinism_requirement: DeterminismRequirement::SeededDecode,
                determinism_class_v1: DeterminismClass::BitExact,
            },
            disabled_optional_probes: BTreeSet::new(),
        }
    }

    fn audit_parents_fixture(byte: u8) -> ObservationPlanAuditParents {
        ObservationPlanAuditParents {
            policy_resolution_self_hash: hash(byte),
            compile_request_hash: hash(byte + 1),
            static_budget_self_hash: hash(byte + 2),
            artifact_aux_hash: hash(byte + 3),
            locked_observation_knobs: LockedObservationKnobs {
                trace_demotion_locked: false,
                optional_probe_floor_locked: false,
                probe_selection_locked: false,
            },
        }
    }

    fn hash(byte: u8) -> Hash256 {
        Hash256::from_bytes([byte; 32])
    }

    fn expected_codegen_domain_hash<T: serde::Serialize>(
        type_name: &str,
        schema_version: &str,
        value: &T,
    ) -> Hash256 {
        let canonical = canonical_json_bytes(value).expect("fixture canonicalizes");
        let mut hasher = Sha256::new();
        hasher.update(format!("gbf:gbf-codegen:{type_name}:{schema_version}\0"));
        hasher.update(canonical);
        Hash256::from_bytes(hasher.finalize().into())
    }

    fn legacy_nul_tuple_domain_hash<T: serde::Serialize>(
        type_name: &str,
        schema_version: &str,
        value: &T,
    ) -> Hash256 {
        let canonical = canonical_json_bytes(value).expect("fixture canonicalizes");
        let mut hasher = Sha256::new();
        hasher.update(b"gbf-codegen");
        hasher.update([0]);
        hasher.update(type_name.as_bytes());
        hasher.update([0]);
        hasher.update(schema_version.as_bytes());
        hasher.update([0]);
        hasher.update(canonical);
        Hash256::from_bytes(hasher.finalize().into())
    }

    fn evidence(reference: &str, byte: u8) -> EvidenceRef {
        EvidenceRef {
            kind: "fixture".to_owned(),
            reference: reference.to_owned(),
            hash: Some(hash(byte)),
        }
    }

    fn anchor(byte: u8) -> SemanticAnchor {
        SemanticAnchor::new(hash(byte))
    }

    fn observation_source() -> ObservationSource {
        ObservationSource::NodeOutput {
            node: NodeId::new(1),
            value: ValueId::new(2),
        }
    }

    fn probe_source(node: u32) -> ProbeSource {
        ProbeSource::NodePostEntry {
            node: NodeId::new(node),
        }
    }

    fn trace_shape(name: &str) -> TraceEventShape {
        TraceEventShape::new(
            TraceEventPayloadLayout::U16,
            2,
            TraceEventTupleSpecId(name.to_owned()),
        )
        .expect("fixture shape fits ABI slot")
    }

    fn trace_shape_with_layout(
        name: &str,
        layout: TraceEventPayloadLayout,
        max_payload_bytes: u16,
    ) -> TraceEventShape {
        TraceEventShape::new(
            layout,
            max_payload_bytes,
            TraceEventTupleSpecId(name.to_owned()),
        )
        .expect("fixture shape fits ABI slot")
    }

    fn registry_evidence(reference: &str) -> EvidenceRef {
        EvidenceRef {
            kind: "fixture".to_owned(),
            reference: reference.to_owned(),
            hash: None,
        }
    }

    fn probe_registry_entry(
        probe_id: u16,
        selector: ProbeSourceSelector,
        level: ProbeLevel,
        importance: ProbeImportanceClass,
        weight: u16,
    ) -> ProbeRegistryEntry {
        let (stable_id, payload_layout, max_payload_bytes) = match probe_id {
            10 => ("checkpoint.empty", TraceEventPayloadLayout::Empty, 0),
            11 => ("op.counter_u16", TraceEventPayloadLayout::U16, 2),
            12 => ("effect.rng_u32", TraceEventPayloadLayout::U32, 4),
            _ => ("value.q8_8", TraceEventPayloadLayout::Q8_8, 2),
        };
        ProbeRegistryEntry {
            probe_id: TraceProbeId(probe_id),
            source_selector: selector,
            level,
            importance,
            event_shape: trace_shape_with_layout(stable_id, payload_layout, max_payload_bytes),
            frequency_bound: TraceFrequencyBound::PerPass { max_events: 1 },
            weight,
            evidence: registry_evidence(&format!("probe/{probe_id}")),
        }
    }

    fn all_selector_probe_registry() -> ProbeRegistrySnapshot {
        ProbeRegistrySnapshot::new(vec![
            probe_registry_entry(
                10,
                ProbeSourceSelector::ByAnchorCheckpoint {
                    checkpoint: semantic_checkpoint_kind_to_id(
                        SemanticCheckpointKind::PostEmbedding {
                            layer: LayerId::new(0),
                        },
                    ),
                    timing: ProbeTiming::PostEntry,
                },
                ProbeLevel::Always,
                ProbeImportanceClass::Required,
                3,
            ),
            probe_registry_entry(
                11,
                ProbeSourceSelector::ByInferOpTag {
                    op_tag: PolicyInferOpTag::RouteTop1,
                    timing: ProbeTiming::PreEntry,
                },
                ProbeLevel::Verbose,
                ProbeImportanceClass::Important,
                5,
            ),
            probe_registry_entry(
                12,
                ProbeSourceSelector::ByEffectClass {
                    class: PolicyEffectClass::Rng,
                },
                ProbeLevel::OnError,
                ProbeImportanceClass::Diagnostic,
                7,
            ),
            probe_registry_entry(
                13,
                ProbeSourceSelector::ByValueRole {
                    role: PolicyValueRole::LogitVector,
                },
                ProbeLevel::Always,
                ProbeImportanceClass::BestEffort,
                11,
            ),
        ])
        .expect("probe registry fixture is valid")
    }

    fn metric_registry_entry(
        metric: &'static str,
        source: MetricSource,
        aggregation: MetricAggregation,
        importance: ProbeImportanceClass,
        weight: u16,
    ) -> MetricRegistryEntry {
        MetricRegistryEntry {
            metric: MetricId::from_static(metric).expect("metric id is valid"),
            source,
            aggregation,
            importance,
            weight,
            evidence: registry_evidence(metric),
        }
    }

    fn probe_metric_inputs_fixture() -> ObservationPlanInputs {
        let mut inputs = semantic_binding_inputs_fixture();
        inputs.probe_registry = all_selector_probe_registry();
        inputs.op_policy_projection.optional_probe_floor = ProbeImportanceClass::BestEffort;
        inputs.op_policy_projection.trace_demotion = TraceDemotionLevel::None;
        add_effect_to_inputs(
            &mut inputs,
            EffectId::new(0),
            EffectClass::Rng {
                slot: RngSlot::Decode,
            },
        );
        inputs
    }

    fn add_effect_to_inputs(
        inputs: &mut ObservationPlanInputs,
        effect_id: EffectId,
        class: EffectClass,
    ) {
        inputs
            .infer_ir_product
            .infer_ir
            .effects
            .push(EffectDecl::new(effect_id, class));
        inputs
            .infer_ir_product
            .infer_ir
            .provenance
            .effects
            .insert(effect_id, EffectProvenance::ExternalRoot { class });
    }

    fn metric_ids(metrics: &[MetricProbe]) -> Vec<String> {
        metrics
            .iter()
            .map(|metric| metric.metric.as_str().to_owned())
            .collect()
    }

    fn probe_by_id(probes: &[OperationalProbe], id: u16) -> &OperationalProbe {
        probes
            .iter()
            .find(|probe| probe.probe_id == TraceProbeId(id))
            .expect("probe id exists")
    }

    fn probe_for_ordering(
        probe_id: u16,
        source_fingerprint: Hash256,
        source_node: u32,
    ) -> OperationalProbe {
        let probe_id = TraceProbeId(probe_id);
        OperationalProbe {
            instance_id: ProbeInstanceId {
                probe_id,
                source_fingerprint,
            },
            probe_id,
            source: probe_source(source_node),
            level: ProbeLevel::Always,
            importance: ProbeImportanceClass::Important,
            event_shape: trace_shape("probe.order"),
            frequency_bound: TraceFrequencyBound::PerPass { max_events: 1 },
            weight: 1,
        }
    }

    fn hard_diagnostic() -> ValidationDiagnostic {
        ValidationDiagnostic::hard(
            ValidationOrigin::SemanticCore,
            ValidationCode::ReportSemanticInvariantViolated {
                field: FieldPath::from("fixture"),
            },
            ValidationDetail::Field {
                field: FieldPath::from("fixture"),
            },
            Vec::new(),
        )
    }

    fn observation_plan_json_with_duplicate_anchor_table_entry(
        field: &str,
        key: &str,
        value_json: &str,
    ) -> String {
        let plan_value = serde_json::to_value(plan_fixture()).expect("plan serializes");
        let anchor_table_value = &plan_value["anchor_table"];
        let original_anchor_table_json =
            serde_json::to_string(anchor_table_value).expect("anchor table serializes");
        let mut duplicate_anchor_table_json = original_anchor_table_json.clone();
        let marker = format!(r#""{field}":{{"#);
        let replacement = format!(r#"{marker}"{key}":{value_json},"#);
        duplicate_anchor_table_json =
            duplicate_anchor_table_json.replacen(&marker, &replacement, 1);
        assert_ne!(duplicate_anchor_table_json, original_anchor_table_json);

        let original_plan_json = serde_json::to_string(&plan_value).expect("plan json");
        let duplicate_plan_json = original_plan_json.replacen(
            &original_anchor_table_json,
            &duplicate_anchor_table_json,
            1,
        );
        assert_ne!(duplicate_plan_json, original_plan_json);
        duplicate_plan_json
    }

    #[test]
    fn observation_plan_inputs_serde_round_trip() {
        let inputs = inputs_fixture();
        let first_value = serde_json::to_value(&inputs).expect("inputs serialize");
        let first = canonicalize_value(&first_value).expect("inputs canonicalize");
        let decoded: ObservationPlanInputs = serde_json::from_slice(&first).expect("inputs decode");
        let second_value = serde_json::to_value(&decoded).expect("decoded inputs serialize");
        let second = canonicalize_value(&second_value).expect("decoded inputs canonicalize");

        assert_eq!(decoded, inputs);
        assert_eq!(second, first);
    }

    #[test]
    fn observation_policy_projection_hash_deterministic() {
        let projection = policy_projection_fixture();

        assert_eq!(
            observation_policy_projection_hash(&projection).expect("first hash"),
            observation_policy_projection_hash(&projection).expect("second hash")
        );
    }

    fn projection_hash_from_value(value: &serde_json::Value) -> Hash256 {
        let canonical = canonicalize_value(value).expect("projection value canonicalizes");
        domain_hash_from_canonical(
            "ObservationPolicyProjection",
            OBSERVATION_PLAN_SCHEMA_VERSION,
            &canonical,
        )
    }

    fn replace_json_leaf(
        value: &mut serde_json::Value,
        path: &[&str],
        replacement: serde_json::Value,
    ) {
        let mut cursor = value;
        let (leaf, parents) = path
            .split_last()
            .expect("projection leaf path is non-empty");
        for parent in parents {
            cursor = cursor
                .get_mut(*parent)
                .unwrap_or_else(|| panic!("projection path contains missing parent {parent}"));
        }

        let slot = cursor
            .get_mut(*leaf)
            .unwrap_or_else(|| panic!("projection path contains missing leaf {leaf}"));
        assert_ne!(
            slot,
            &replacement,
            "replacement for projection leaf {} must change the value",
            path.join(".")
        );
        *slot = replacement;
    }

    #[test]
    fn observation_policy_projection_hash_changes_on_every_serialized_leaf_field_change() {
        let base = policy_projection_fixture();
        let base_hash = observation_policy_projection_hash(&base).expect("base hash");
        let base_value = serde_json::to_value(&base).expect("projection serializes");
        assert_eq!(projection_hash_from_value(&base_value), base_hash);

        let cases = vec![
            ("profile_id", vec!["profile_id"], serde_json::json!("Trace")),
            (
                "profile_observation_caps.required_max",
                vec!["profile_observation_caps", "required_max"],
                serde_json::json!(1),
            ),
            (
                "profile_observation_caps.important_max",
                vec!["profile_observation_caps", "important_max"],
                serde_json::json!(257),
            ),
            (
                "profile_observation_caps.diagnostic_max",
                vec!["profile_observation_caps", "diagnostic_max"],
                serde_json::json!(129),
            ),
            (
                "profile_observation_caps.best_effort_max",
                vec!["profile_observation_caps", "best_effort_max"],
                serde_json::json!(65),
            ),
            (
                "determinism_class.kind",
                vec!["determinism_class", "kind"],
                serde_json::json!("Deterministic"),
            ),
            (
                "observability_mode.kind",
                vec!["observability_mode", "kind"],
                serde_json::json!("Flexible"),
            ),
            (
                "trace_budget.max_events_per_slice",
                vec!["trace_budget", "max_events_per_slice"],
                serde_json::json!(129),
            ),
            (
                "trace_budget.max_bytes_per_frame",
                vec!["trace_budget", "max_bytes_per_frame"],
                serde_json::json!(2049),
            ),
            (
                "trace_budget.drop_policy.kind",
                vec!["trace_budget", "drop_policy", "kind"],
                serde_json::json!("DropNewest"),
            ),
            (
                "trace_demotion.kind",
                vec!["trace_demotion", "kind"],
                serde_json::json!("DropBestEffort"),
            ),
            (
                "optional_probe_floor.kind",
                vec!["optional_probe_floor", "kind"],
                serde_json::json!("Diagnostic"),
            ),
            (
                "workload_observation.workload_id",
                vec!["workload_observation", "workload_id"],
                serde_json::json!("workload.changed"),
            ),
            (
                "workload_observation.checkpoint_selection",
                vec!["workload_observation", "checkpoint_selection"],
                serde_json::json!("explicit_required_and_optional"),
            ),
            (
                "workload_observation.trace_level",
                vec!["workload_observation", "trace_level"],
                serde_json::json!("summary"),
            ),
            (
                "workload_observation.compare_domain_workload",
                vec!["workload_observation", "compare_domain_workload"],
                serde_json::json!("generated_bytes"),
            ),
            (
                "workload_observation.compare_domain_policy.kind",
                vec!["workload_observation", "compare_domain_policy", "kind"],
                serde_json::json!("TokenIdOnly"),
            ),
            (
                "workload_observation.determinism_requirement",
                vec!["workload_observation", "determinism_requirement"],
                serde_json::json!("explicit_seed"),
            ),
            (
                "workload_observation.determinism_class_v1.kind",
                vec!["workload_observation", "determinism_class_v1", "kind"],
                serde_json::json!("Deterministic"),
            ),
            (
                "disabled_optional_probes",
                vec!["disabled_optional_probes"],
                serde_json::json!([99]),
            ),
        ];

        for (name, path, replacement) in cases {
            let mut changed = base_value.clone();
            replace_json_leaf(&mut changed, &path, replacement);
            assert_ne!(
                projection_hash_from_value(&changed),
                base_hash,
                "projection leaf {name} did not affect the hash"
            );
        }
    }

    #[test]
    fn op_pre_2_artifact_declared_hash_mismatch_rejected() {
        let mut inputs = inputs_fixture();
        inputs.artifact_declared_semantic_checkpoint_schema_hash = hash(0xee);
        let err =
            validate_observation_plan_inputs(&inputs).expect_err("schema hash mismatch rejects");

        assert_eq!(err.code(), OBSERVATION_SC_HASH_MISMATCH_CODE);
    }

    #[test]
    fn op_pre_3a_determinism_class_mismatch_rejected() {
        let mut inputs = inputs_fixture();
        inputs.infer_ir_product = infer_ir_product_fixture(DeterminismClass::Deterministic);
        let err =
            validate_observation_plan_inputs(&inputs).expect_err("determinism mismatch rejects");

        assert_eq!(err.code(), OBSERVATION_DETERMINISM_MISMATCH_CODE);
    }

    #[test]
    fn op_pre_3b_compare_domain_projection_mismatch_rejected() {
        let mut inputs = inputs_fixture();
        inputs
            .op_policy_projection
            .workload_observation
            .compare_domain_policy = CompareDomain::TokenIdOnly;
        let err =
            validate_observation_plan_inputs(&inputs).expect_err("compare domain mismatch rejects");

        assert_eq!(err.code(), OBSERVATION_COMPARE_DOMAIN_MISMATCH_CODE);
    }

    #[test]
    fn op_pre_3c_workload_determinism_requirement_mismatch_rejected() {
        let mut inputs = inputs_fixture();
        inputs
            .op_policy_projection
            .workload_observation
            .determinism_class_v1 = DeterminismClass::Deterministic;
        let err = validate_observation_plan_inputs(&inputs)
            .expect_err("workload determinism mismatch rejects");

        assert_eq!(err.code(), OBSERVATION_WORKLOAD_DETERMINISM_MISMATCH_CODE);
    }

    #[test]
    fn op_pre_3d_policy_workload_determinism_mismatch_rejected() {
        let mut inputs = inputs_fixture();
        inputs.op_policy_projection.determinism_class = DeterminismClass::Deterministic;
        let err = validate_observation_plan_inputs(&inputs)
            .expect_err("policy/workload determinism mismatch rejects");

        assert_eq!(
            err.code(),
            OBSERVATION_POLICY_WORKLOAD_DETERMINISM_MISMATCH_CODE
        );
    }

    #[test]
    fn compare_domain_serde_round_trip_all_five_variants() {
        for domain in [
            CompareDomain::CanonicalValue,
            CompareDomain::TokenIdOnly,
            CompareDomain::ExpertIdOnly,
            CompareDomain::EnvelopeQ8_8,
            CompareDomain::EnvelopeQ16_16,
        ] {
            let encoded = serde_json::to_vec(&domain).expect("domain serializes");
            let decoded: CompareDomain = serde_json::from_slice(&encoded).expect("domain decodes");

            assert_eq!(decoded, domain);
        }
    }

    #[test]
    fn workload_compare_domain_projection_v1_maps_real_variants() {
        assert_eq!(
            CompareDomain::from(WorkloadCompareDomain::TokenLogits),
            CompareDomain::CanonicalValue
        );
        assert_eq!(
            CompareDomain::from(WorkloadCompareDomain::GeneratedBytes),
            CompareDomain::TokenIdOnly
        );
        assert_eq!(
            DeterminismClass::from(DeterminismRequirement::SeededDecode),
            DeterminismClass::BitExact
        );
    }

    #[test]
    fn reserved_policy_compare_domain_variants_unreachable_from_v1_workload() {
        let mapped = [
            CompareDomain::from(WorkloadCompareDomain::TokenLogits),
            CompareDomain::from(WorkloadCompareDomain::GeneratedBytes),
        ];

        assert!(!mapped.contains(&CompareDomain::ExpertIdOnly));
        assert!(!mapped.contains(&CompareDomain::EnvelopeQ8_8));
        assert!(!mapped.contains(&CompareDomain::EnvelopeQ16_16));
    }

    #[test]
    fn audit_parents_do_not_affect_projection_hash() {
        let inputs = inputs_fixture();
        let mut changed = inputs.clone();
        changed.audit_parents = audit_parents_fixture(0x60);

        assert_ne!(inputs.audit_parents, changed.audit_parents);
        assert_eq!(
            observation_policy_projection_hash(&inputs.op_policy_projection)
                .expect("original projection hash"),
            observation_policy_projection_hash(&changed.op_policy_projection)
                .expect("changed projection hash")
        );
    }

    #[test]
    fn locked_observation_knobs_live_in_audit_parents_not_projection() {
        let inputs = inputs_fixture();
        let projection_value =
            serde_json::to_value(&inputs.op_policy_projection).expect("projection serializes");
        let audit_value = serde_json::to_value(&inputs.audit_parents).expect("audit serializes");

        assert!(projection_value.get("locked_observation_knobs").is_none());
        assert!(audit_value.get("locked_observation_knobs").is_some());
    }

    #[test]
    fn locked_observation_knobs_shape_matches_rfc_8_1() {
        let inputs = inputs_fixture();
        let value = serde_json::to_value(inputs.audit_parents.locked_observation_knobs)
            .expect("locked knobs serialize");

        assert_eq!(
            value,
            serde_json::json!({
                "trace_demotion_locked": false,
                "optional_probe_floor_locked": false,
                "probe_selection_locked": false,
            })
        );
    }

    fn semantic_observation(kind: SemanticCheckpointKind, compact: u16) -> SemanticObservation {
        let stratum = SemanticStratum::Denotation;
        SemanticObservation {
            checkpoint: semantic_checkpoint_kind_to_id(kind),
            kind,
            compact: CompactCheckpointId(compact),
            stratum,
            source: observation_source(),
            encoding: ObservationEncoding::Canonical,
            anchor: anchor(0x41),
            artifact_role: SemanticCheckpointRole::from(stratum),
        }
    }

    fn plan_fixture() -> ObservationPlan {
        let semantic = vec![semantic_observation(
            SemanticCheckpointKind::PostEmbedding {
                layer: LayerId::new(0),
            },
            1,
        )];
        let probe_id = TraceProbeId(7);
        let probe_source = probe_source(3);
        let instance_id = ProbeInstanceId {
            probe_id,
            source_fingerprint: probe_instance_source_fingerprint(probe_id, &probe_source)
                .expect("fingerprint hashes"),
        };
        let metric_id = MetricId::from_static("token.latency").expect("metric id");

        ObservationPlan {
            identity: ObservationPlanIdentity {
                infer_ir_self_hash: hash(0x10),
                quant_graph_self_hash: hash(0x11),
                semantic_checkpoint_schema_hash: hash(0x12),
                observation_policy_projection_hash: hash(0x13),
                determinism: DeterminismClass::BitExact,
                observability_mode: ObservabilityMode::Invariant,
                trace_budget: TraceBudget::new(8, 128, TraceDropPolicy::DropOldest)
                    .expect("budget"),
                workload_id: WorkloadId::from("workload.fixture"),
                probe_registry_hash: hash(0x14),
                metric_registry_hash: hash(0x15),
                trace_event_layout_registry_hash: hash(0x16),
            },
            semantic: semantic.clone(),
            probes: vec![OperationalProbe {
                instance_id,
                probe_id,
                source: probe_source.clone(),
                level: ProbeLevel::Always,
                importance: ProbeImportanceClass::Important,
                event_shape: trace_shape("probe.node.post"),
                frequency_bound: TraceFrequencyBound::PerNodeExecution {
                    max_events_per_execution: 1,
                },
                weight: 3,
            }],
            metrics: vec![MetricProbe {
                metric: metric_id.clone(),
                source: MetricSource::PerToken,
                aggregation: MetricAggregation::Mean,
                importance: ProbeImportanceClass::Diagnostic,
                weight: 5,
            }],
            anchor_table: AnchorAttachmentTable {
                semantic: BTreeMap::from([(
                    semantic[0].checkpoint.clone(),
                    SemanticAttachment {
                        anchor: semantic[0].anchor.clone(),
                        source: semantic[0].source.clone(),
                    },
                )]),
                probes: BTreeMap::from([(instance_id, probe_source)]),
                metrics: BTreeMap::from([(metric_id.clone(), MetricSource::PerToken)]),
            },
            provenance: ObservationProvenance {
                semantic_provenance: BTreeMap::from([(
                    semantic[0].checkpoint.clone(),
                    evidence("semantic", 0x21),
                )]),
                probe_provenance: BTreeMap::from([(instance_id, evidence("probe", 0x22))]),
                metric_provenance: BTreeMap::from([(metric_id, evidence("metric", 0x23))]),
            },
            trace_budget_projection: TraceBudgetProjection {
                projected_max_events_per_slice: 4,
                projected_max_bytes_per_frame: 32,
                fits_declared_budget: true,
            },
        }
    }

    fn build_active_checkpoint_schema_fixture() -> BuildActiveCheckpointSchema {
        let checkpoints = vec![ReEmittedCheckpointEntry {
            id: semantic_checkpoint_kind_to_id(SemanticCheckpointKind::PostEmbedding {
                layer: LayerId::new(0),
            }),
            kind: SemanticCheckpointKind::PostEmbedding {
                layer: LayerId::new(0),
            },
            artifact_role: SemanticCheckpointRole::Mandatory,
            original_checkpoint_metadata: SemanticCheckpointMetadata {
                compact: CompactCheckpointId(1),
                stratum: SemanticStratum::Denotation,
                source_op: Some("embedding".to_owned()),
            },
            encoding: ObservationEncoding::Canonical,
            source: observation_source(),
            attachment_node_id: NodeId::new(1),
            attachment_anchor: anchor(0x41),
            canonical_provenance_tuple: CanonicalProvenanceTuple {
                op_tag: InferOpTag::Embedding,
                layer: Some(LayerId::new(0)),
                expert: None,
                expert_weight_slot: None,
                norm_site: None,
                state_slot: None,
                residual_site: None,
                occurrence_index: 0,
            },
        }];

        BuildActiveCheckpointSchema {
            checkpoints,
            build_active_count: 1,
            mandatory_count: 1,
            optional_count: 0,
        }
    }

    fn operational_probe_schema_fixture() -> OperationalProbeSchema {
        let plan = plan_fixture();
        let probe = plan.probes[0].clone();
        let metric = plan.metrics[0].clone();

        OperationalProbeSchema {
            probes: vec![ProbeSchemaEntry {
                instance_id: probe.instance_id,
                probe_id: probe.probe_id,
                level: probe.level,
                importance: probe.importance,
                event_shape: probe.event_shape,
                source: probe.source,
                weight: probe.weight,
            }],
            metrics: vec![MetricSchemaEntry {
                metric: metric.metric,
                aggregation: metric.aggregation,
                source: metric.source,
                importance: metric.importance,
                weight: metric.weight,
            }],
            probe_count: 1,
            metric_count: 1,
            per_class_probe_weight_total: PerClassWeightTotal {
                important: 3,
                ..PerClassWeightTotal::default()
            },
            per_class_metric_weight_total: PerClassWeightTotal {
                diagnostic: 5,
                ..PerClassWeightTotal::default()
            },
            per_class_total_weight: PerClassWeightTotal {
                important: 3,
                diagnostic: 5,
                ..PerClassWeightTotal::default()
            },
        }
    }

    fn core_product_fixture() -> ObservationPlanCoreProduct {
        let observation_plan = plan_fixture();
        let build_active_checkpoint_schema = build_active_checkpoint_schema_fixture();
        let operational_probe_schema = operational_probe_schema_fixture();
        ObservationPlanCoreProduct {
            observation_plan_self_hash: observation_plan_self_hash(&observation_plan)
                .expect("plan hashes"),
            build_active_checkpoint_schema_hash: build_active_checkpoint_schema_hash(
                &build_active_checkpoint_schema,
            )
            .expect("checkpoint schema hashes"),
            operational_probe_schema_hash: operational_probe_schema_hash(&operational_probe_schema)
                .expect("probe schema hashes"),
            observation_plan,
            build_active_checkpoint_schema,
            operational_probe_schema,
        }
    }

    fn observation_plan_report_body_fixture(
        inputs: &ObservationPlanInputs,
        product: &ObservationPlanCoreProduct,
    ) -> ObservationPlanReportBody {
        ObservationPlanReportBody {
            input_identity: ObservationPlanReportInputIdentity::from_inputs(
                inputs,
                &product.observation_plan.identity,
            ),
            result: Some(ObservationPlanReportResult {
                product: product.observation_plan.clone(),
                semantic_count: 1,
                probe_count: 1,
                metric_count: 1,
                mandatory_semantic_count: 1,
                optional_semantic_count: 0,
                per_class_probe_count: PerClassCount {
                    important: 1,
                    ..PerClassCount::default()
                },
                per_class_metric_count: PerClassCount {
                    diagnostic: 1,
                    ..PerClassCount::default()
                },
                sc_re_emit_report_self_hash: hash(0x71),
                operational_probe_schema_report_self_hash: hash(0x72),
                observation_plan_self_hash: product.observation_plan_self_hash,
            }),
            diagnostics: Vec::new(),
        }
    }

    fn sc_re_emit_body_fixture(
        inputs: &ObservationPlanInputs,
        product: &ObservationPlanCoreProduct,
    ) -> SemanticCheckpointSchemaReEmitBody {
        SemanticCheckpointSchemaReEmitBody {
            input_identity: SemanticCheckpointSchemaReEmitInputIdentity {
                observation_plan_self_hash: Some(product.observation_plan_self_hash),
                original_schema_hash: inputs.semantic_checkpoint_schema_hash,
                infer_ir_self_hash: product.observation_plan.identity.infer_ir_self_hash,
                quant_graph_self_hash: product.observation_plan.identity.quant_graph_self_hash,
                artifact_aux_hash: inputs.audit_parents.artifact_aux_hash,
                determinism: product.observation_plan.identity.determinism,
                workload_id: product.observation_plan.identity.workload_id.clone(),
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

    fn operational_probe_body_fixture(
        inputs: &ObservationPlanInputs,
        product: &ObservationPlanCoreProduct,
    ) -> OperationalProbeSchemaBody {
        OperationalProbeSchemaBody {
            input_identity: OperationalProbeSchemaInputIdentity {
                observation_plan_self_hash: Some(product.observation_plan_self_hash),
                infer_ir_self_hash: product.observation_plan.identity.infer_ir_self_hash,
                quant_graph_self_hash: product.observation_plan.identity.quant_graph_self_hash,
                determinism: product.observation_plan.identity.determinism,
                observability_mode: product.observation_plan.identity.observability_mode,
                trace_budget: product.observation_plan.identity.trace_budget,
                profile_id: inputs.op_policy_projection.profile_id.clone(),
                workload_id: product.observation_plan.identity.workload_id.clone(),
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

    #[test]
    fn observation_plan_serde_round_trip() {
        let plan = plan_fixture();
        let first_value = serde_json::to_value(&plan).expect("plan serializes");
        let first = canonicalize_value(&first_value).expect("plan canonicalizes");
        let decoded: ObservationPlan = serde_json::from_slice(&first).expect("plan decodes");
        let second_value = serde_json::to_value(&decoded).expect("decoded plan serializes");
        let second = canonicalize_value(&second_value).expect("decoded plan canonicalizes");

        assert_eq!(decoded, plan);
        assert_eq!(second, first);
    }

    #[test]
    fn anchor_table_wire_shape_pins_semantic_probes_metrics() {
        let plan_value = serde_json::to_value(plan_fixture()).expect("plan serializes");
        let anchor_table = &plan_value["anchor_table"];
        let semantic = anchor_table["semantic"]
            .as_object()
            .expect("semantic anchors use object shape");
        let probes = anchor_table["probes"]
            .as_array()
            .expect("probe anchors use key/value array shape");
        let metrics = anchor_table["metrics"]
            .as_object()
            .expect("metric anchors use object shape");

        assert_eq!(semantic.len(), 1);
        assert!(semantic.get("layer.0.post_embedding").is_some());
        assert_eq!(
            semantic["layer.0.post_embedding"]["source"]["kind"],
            serde_json::json!("NodeOutput")
        );
        assert_eq!(probes.len(), 1);
        assert_eq!(probes[0]["key"]["probe_id"], serde_json::json!(7));
        assert!(probes[0]["key"].get("source_fingerprint").is_some());
        assert_eq!(
            probes[0]["value"]["kind"],
            serde_json::json!("NodePostEntry")
        );
        assert_eq!(metrics.len(), 1);
        assert_eq!(
            metrics["token.latency"],
            serde_json::json!({ "kind": "PerToken" })
        );
    }

    #[test]
    fn observation_plan_deserialization_rejects_duplicate_anchor_table_semantic_keys() {
        let plan = plan_fixture();
        let key = plan.semantic[0].checkpoint.as_str();
        let value_json = serde_json::to_string(
            plan.anchor_table
                .semantic
                .get(&plan.semantic[0].checkpoint)
                .expect("semantic attachment exists"),
        )
        .expect("semantic attachment serializes");
        let json =
            observation_plan_json_with_duplicate_anchor_table_entry("semantic", key, &value_json);

        let err = serde_json::from_str::<ObservationPlan>(&json)
            .expect_err("duplicate semantic key rejects");

        assert!(err.to_string().contains("duplicate semantic checkpoint id"));
    }

    #[test]
    fn observation_plan_deserialization_rejects_duplicate_anchor_table_metric_keys() {
        let plan = plan_fixture();
        let key = plan.metrics[0].metric.as_str();
        let value_json = serde_json::to_string(
            plan.anchor_table
                .metrics
                .get(&plan.metrics[0].metric)
                .expect("metric source exists"),
        )
        .expect("metric source serializes");
        let json =
            observation_plan_json_with_duplicate_anchor_table_entry("metrics", key, &value_json);

        let err =
            serde_json::from_str::<ObservationPlan>(&json).expect_err("duplicate metric rejects");

        assert!(err.to_string().contains("duplicate metric id"));
    }

    #[test]
    fn observation_plan_self_hash_deterministic() {
        let first = observation_plan_self_hash(&plan_fixture()).expect("first hash");
        let second = observation_plan_self_hash(&plan_fixture()).expect("second hash");

        assert_eq!(first, second);
    }

    #[test]
    fn observation_plan_self_hash_uses_codegen_domain_prefix() {
        let plan = plan_fixture();
        let actual = observation_plan_self_hash(&plan).expect("plan hashes");

        assert_eq!(
            actual,
            expected_codegen_domain_hash("ObservationPlan", OBSERVATION_PLAN_SCHEMA_VERSION, &plan)
        );
        assert_ne!(
            actual,
            legacy_nul_tuple_domain_hash("ObservationPlan", OBSERVATION_PLAN_SCHEMA_VERSION, &plan)
        );
    }

    #[test]
    fn observation_plan_core_product_serde_round_trip() {
        let product = core_product_fixture();
        let first_value = serde_json::to_value(&product).expect("product serializes");
        let first = canonicalize_value(&first_value).expect("product canonicalizes");
        let decoded: ObservationPlanCoreProduct =
            serde_json::from_slice(&first).expect("product decodes");
        let second_value = serde_json::to_value(&decoded).expect("decoded product serializes");
        let second = canonicalize_value(&second_value).expect("decoded product canonicalizes");

        assert_eq!(decoded, product);
        assert_eq!(second, first);
    }

    #[test]
    fn observation_plan_core_product_hash_deterministic() {
        let first = observation_plan_core_product_hash(&core_product_fixture())
            .expect("first product hash");
        let second = observation_plan_core_product_hash(&core_product_fixture())
            .expect("second product hash");

        assert_eq!(first, second);
    }

    #[test]
    fn observation_plan_core_product_byte_stable_across_audit_drift() {
        let product = core_product_fixture();
        let first_inputs = inputs_fixture();
        let mut second_inputs = first_inputs.clone();
        second_inputs.audit_parents.compile_request_hash = hash(0xee);
        second_inputs.audit_parents.policy_resolution_self_hash = hash(0xef);
        second_inputs.audit_parents.static_budget_self_hash = hash(0xf0);
        second_inputs.audit_parents.artifact_aux_hash = hash(0xf1);
        second_inputs
            .audit_parents
            .locked_observation_knobs
            .probe_selection_locked = true;
        second_inputs
            .audit_parents
            .locked_observation_knobs
            .trace_demotion_locked = true;
        let first_body = observation_plan_report_body_fixture(&first_inputs, &product);
        let second_body = observation_plan_report_body_fixture(&second_inputs, &product);
        let first_product_bytes = canonical_json_bytes(&product).expect("first product bytes");
        let second_product_bytes = canonical_json_bytes(&product).expect("second product bytes");
        let product_json = String::from_utf8(first_product_bytes.clone()).expect("json is utf8");

        assert_ne!(
            first_body.input_identity.compile_request_hash,
            second_body.input_identity.compile_request_hash
        );
        assert_ne!(
            first_body.input_identity.policy_resolution_self_hash,
            second_body.input_identity.policy_resolution_self_hash
        );
        assert_ne!(
            first_body.input_identity.static_budget_self_hash,
            second_body.input_identity.static_budget_self_hash
        );
        assert_ne!(
            first_body.input_identity.artifact_aux_hash,
            second_body.input_identity.artifact_aux_hash
        );
        for audit_field in [
            "audit_parents",
            "policy_resolution_self_hash",
            "compile_request_hash",
            "static_budget_self_hash",
            "artifact_aux_hash",
            "locked_observation_knobs",
            "trace_demotion_locked",
            "optional_probe_floor_locked",
            "probe_selection_locked",
        ] {
            assert!(
                !product_json.contains(audit_field),
                "core product unexpectedly serialized audit-parent field {audit_field}"
            );
        }
        assert_eq!(first_product_bytes, second_product_bytes);
        assert_eq!(
            observation_plan_core_product_hash(&product).expect("first product hash"),
            observation_plan_core_product_hash(&product).expect("second product hash")
        );
    }

    #[test]
    fn observation_plan_stage_output_envelopes_carry_audit_parents() {
        let inputs = inputs_fixture();
        let product = core_product_fixture();
        let report_body = observation_plan_report_body_fixture(&inputs, &product);
        let sc_body = sc_re_emit_body_fixture(&inputs, &product);
        let op_body = operational_probe_body_fixture(&inputs, &product);
        let output = ObservationPlanStageOutput {
            product: product.clone(),
            report: ReportEnvelope::new(ReportOutcome::Passed, report_body)
                .expect("observation report envelope"),
            sc_re_emit_report: ReportEnvelope::new(ReportOutcome::Passed, sc_body)
                .expect("sc re-emit envelope"),
            operational_probe_report: ReportEnvelope::new(ReportOutcome::Passed, op_body)
                .expect("operational probe envelope"),
        };
        let product_value = serde_json::to_value(&output.product).expect("product serializes");

        assert_eq!(
            output.report.body.input_identity.compile_request_hash,
            inputs.audit_parents.compile_request_hash
        );
        assert_eq!(
            output
                .report
                .body
                .input_identity
                .policy_resolution_self_hash,
            inputs.audit_parents.policy_resolution_self_hash
        );
        assert_eq!(
            output.report.body.input_identity.static_budget_self_hash,
            inputs.audit_parents.static_budget_self_hash
        );
        assert_eq!(
            output.report.body.input_identity.artifact_aux_hash,
            inputs.audit_parents.artifact_aux_hash
        );
        assert_eq!(
            output
                .sc_re_emit_report
                .body
                .input_identity
                .artifact_aux_hash,
            inputs.audit_parents.artifact_aux_hash
        );
        assert!(product_value.get("input_identity").is_none());
        assert!(
            serde_json::to_string(&product_value)
                .expect("product value stringifies")
                .find("compile_request_hash")
                .is_none()
        );
    }

    #[test]
    fn observation_plan_core_success_carries_three_bodies() {
        let inputs = inputs_fixture();
        let product = core_product_fixture();
        let success = ObservationPlanCoreSuccess {
            observation_plan_body: observation_plan_report_body_fixture(&inputs, &product),
            sc_re_emit_body: sc_re_emit_body_fixture(&inputs, &product),
            operational_probe_body: operational_probe_body_fixture(&inputs, &product),
            product,
        };

        assert!(success.observation_plan_body.result.is_some());
        assert!(success.sc_re_emit_body.result.is_some());
        assert!(success.operational_probe_body.result.is_some());
    }

    #[test]
    fn ancillary_report_results_are_flattened_schema_shapes() {
        let inputs = inputs_fixture();
        let product = core_product_fixture();
        let sc_body = sc_re_emit_body_fixture(&inputs, &product);
        let op_body = operational_probe_body_fixture(&inputs, &product);
        let sc_result =
            serde_json::to_value(sc_body.result.expect("sc result")).expect("sc result serializes");
        let op_result =
            serde_json::to_value(op_body.result.expect("op result")).expect("op result serializes");

        assert!(sc_result.get("schema").is_none());
        assert!(sc_result.get("schema_hash").is_some());
        assert!(sc_result.get("checkpoints").is_some());
        assert!(op_result.get("schema").is_none());
        assert!(op_result.get("schema_hash").is_some());
        assert!(op_result.get("probes").is_some());
        assert!(op_result.get("metrics").is_some());
    }

    #[test]
    fn observation_plan_core_failure_has_optional_ancillaries() {
        let inputs = inputs_fixture();
        let product = core_product_fixture();
        let failure = ObservationPlanCoreFailure {
            observation_plan_body: ObservationPlanReportBody {
                input_identity: ObservationPlanReportInputIdentity::from_inputs(
                    &inputs,
                    &product.observation_plan.identity,
                ),
                result: None,
                diagnostics: vec![hard_diagnostic()],
            },
            sc_re_emit_body: None,
            operational_probe_body: Some(operational_probe_body_fixture(&inputs, &product)),
            diagnostics: NonEmptyList::new(vec![hard_diagnostic()])
                .expect("failure diagnostics are non-empty"),
        };

        assert!(failure.observation_plan_body.result.is_none());
        assert!(failure.sc_re_emit_body.is_none());
        assert!(failure.operational_probe_body.is_some());
        assert_eq!(failure.diagnostics.as_slice().len(), 1);
    }

    #[test]
    fn non_empty_list_deserialization_rejects_empty_items() {
        let err = serde_json::from_str::<NonEmptyList<ValidationDiagnostic>>(r#"{"items":[]}"#)
            .expect_err("empty items reject");

        assert!(
            err.to_string()
                .contains("NonEmptyList must contain at least one item")
        );
    }

    #[test]
    fn product_report_validation_failure_carries_diagnostic_signal() {
        let inputs = inputs_fixture();
        let product = core_product_fixture();
        let mut body = observation_plan_report_body_fixture(&inputs, &product);
        body.result = None;

        let passed_errors = body
            .validate_semantics(ReportOutcome::Passed)
            .expect_err("passed report without result rejects");
        let failed_errors = body
            .validate_semantics(ReportOutcome::Failed)
            .expect_err("failed report without hard diagnostic rejects");

        assert!(passed_errors.iter().any(|diagnostic| {
            matches!(
                &diagnostic.code,
                ValidationCode::ReportSemanticInvariantViolated { field }
                    if field == &FieldPath::from("result")
            )
        }));
        assert!(failed_errors.iter().any(|diagnostic| {
            matches!(
                &diagnostic.code,
                ValidationCode::ReportSemanticInvariantViolated { field }
                    if field == &FieldPath::from("diagnostics")
            )
        }));
    }

    #[test]
    fn build_active_checkpoint_schema_hash_deterministic() {
        let first = build_active_checkpoint_schema_hash(&build_active_checkpoint_schema_fixture())
            .expect("first checkpoint schema hash");
        let second = build_active_checkpoint_schema_hash(&build_active_checkpoint_schema_fixture())
            .expect("second checkpoint schema hash");

        assert_eq!(first, second);
    }

    #[test]
    fn operational_probe_schema_hash_deterministic() {
        let first = operational_probe_schema_hash(&operational_probe_schema_fixture())
            .expect("first operational schema hash");
        let second = operational_probe_schema_hash(&operational_probe_schema_fixture())
            .expect("second operational schema hash");

        assert_eq!(first, second);
    }

    #[test]
    fn no_canonical_bytes_hash_field_on_core_product() {
        let value = serde_json::to_value(core_product_fixture()).expect("product serializes");
        let object = value.as_object().expect("product is object");

        assert!(
            object
                .get("observation_plan_canonical_bytes_hash")
                .is_none()
        );
        assert!(
            object
                .get("build_active_checkpoint_schema_canonical_bytes_hash")
                .is_none()
        );
        assert!(
            object
                .get("operational_probe_schema_canonical_bytes_hash")
                .is_none()
        );
    }

    #[test]
    fn legacy_observation_plan_product_alias_compiles() {
        fn accepts_legacy_alias(_: &ObservationPlanProduct) {}

        let product = core_product_fixture();
        accepts_legacy_alias(&product);
    }

    #[test]
    fn observation_plan_identity_carries_three_registry_hashes() {
        let identity = plan_fixture().identity;
        let value = serde_json::to_value(&identity).expect("identity serializes");

        assert_eq!(identity.probe_registry_hash, hash(0x14));
        assert_eq!(identity.metric_registry_hash, hash(0x15));
        assert_eq!(identity.trace_event_layout_registry_hash, hash(0x16));
        assert!(value.get("probe_registry_hash").is_some());
        assert!(value.get("metric_registry_hash").is_some());
        assert!(value.get("trace_event_layout_registry_hash").is_some());
    }

    #[test]
    fn semantic_observation_per_v1_checkpoint_id() {
        let kinds = [
            SemanticCheckpointKind::PostEmbedding {
                layer: LayerId::new(0),
            },
            SemanticCheckpointKind::PostRouter {
                layer: LayerId::new(2),
            },
            SemanticCheckpointKind::PostExpertDowncast {
                layer: LayerId::new(2),
                expert: ExpertId::new(3),
            },
            SemanticCheckpointKind::PostLogits,
            SemanticCheckpointKind::PostDecode,
        ];

        for (index, kind) in kinds.into_iter().enumerate() {
            let observation = semantic_observation(kind, index as u16 + 1);
            let encoded = serde_json::to_vec(&observation).expect("observation serializes");
            let decoded: SemanticObservation =
                serde_json::from_slice(&encoded).expect("observation decodes");

            assert_eq!(decoded, observation);
            assert_eq!(
                try_parse_semantic_checkpoint_kind(&observation.checkpoint),
                Some(kind)
            );
        }
    }

    #[test]
    fn operational_probe_two_instances_same_probe_id_different_source() {
        let probe_id = TraceProbeId(9);
        let first_source = probe_source(1);
        let second_source = probe_source(2);
        let first = ProbeInstanceId {
            probe_id,
            source_fingerprint: probe_instance_source_fingerprint(probe_id, &first_source)
                .expect("first source hashes"),
        };
        let second = ProbeInstanceId {
            probe_id,
            source_fingerprint: probe_instance_source_fingerprint(probe_id, &second_source)
                .expect("second source hashes"),
        };

        assert_ne!(first, second);
        assert_eq!(first.probe_id, second.probe_id);
    }

    #[test]
    fn probe_instance_id_source_fingerprint_deterministic() {
        let probe_id = TraceProbeId(10);
        let source = ProbeSource::Anchor {
            anchor: anchor(0x50),
        };

        assert_eq!(
            probe_instance_source_fingerprint(probe_id, &source).expect("first hash"),
            probe_instance_source_fingerprint(probe_id, &source).expect("second hash")
        );
    }

    #[test]
    fn probe_instance_source_fingerprint_uses_codegen_domain_prefix() {
        let probe_id = TraceProbeId(10);
        let source = ProbeSource::Anchor {
            anchor: anchor(0x50),
        };

        #[derive(Serialize)]
        struct FingerprintMaterial<'a> {
            probe_id: TraceProbeId,
            source: &'a ProbeSource,
        }

        let material = FingerprintMaterial {
            probe_id,
            source: &source,
        };
        let actual =
            probe_instance_source_fingerprint(probe_id, &source).expect("fingerprint hashes");

        assert_eq!(
            actual,
            expected_codegen_domain_hash(
                "ProbeInstanceSource",
                OPERATIONAL_PROBE_SCHEMA_VERSION,
                &material,
            )
        );
        assert_ne!(
            actual,
            legacy_nul_tuple_domain_hash(
                "ProbeInstanceSource",
                OPERATIONAL_PROBE_SCHEMA_VERSION,
                &material,
            )
        );
    }

    #[test]
    fn probe_instance_maps_serialize_in_canonical_key_order() {
        let id_2 = ProbeInstanceId {
            probe_id: TraceProbeId(2),
            source_fingerprint: hash(0x2),
        };
        let id_10 = ProbeInstanceId {
            probe_id: TraceProbeId(10),
            source_fingerprint: hash(0x10),
        };
        let table = AnchorAttachmentTable {
            semantic: BTreeMap::new(),
            probes: BTreeMap::from([(id_2, probe_source(2)), (id_10, probe_source(10))]),
            metrics: BTreeMap::new(),
        };
        let provenance = ObservationProvenance {
            semantic_provenance: BTreeMap::new(),
            probe_provenance: BTreeMap::from([
                (id_2, evidence("probe.2", 0x2)),
                (id_10, evidence("probe.10", 0x10)),
            ]),
            metric_provenance: BTreeMap::new(),
        };
        let table_value = serde_json::to_value(&table).expect("table serializes");
        let provenance_value = serde_json::to_value(&provenance).expect("provenance serializes");

        assert_eq!(table_value["probes"][0]["key"]["probe_id"], 10);
        assert_eq!(table_value["probes"][1]["key"]["probe_id"], 2);
        assert_eq!(
            provenance_value["probe_provenance"][0]["key"]["probe_id"],
            10
        );
        assert_eq!(
            provenance_value["probe_provenance"][1]["key"]["probe_id"],
            2
        );
    }

    #[test]
    fn anchor_attachment_semantic_serializes_as_string_keyed_object() {
        let table = AnchorAttachmentTable {
            semantic: BTreeMap::from([
                (
                    SemanticCheckpointId::from_static("z.checkpoint")
                        .expect("semantic id is valid"),
                    SemanticAttachment {
                        anchor: anchor(0x2),
                        source: observation_source(),
                    },
                ),
                (
                    SemanticCheckpointId::from_static("a.checkpoint")
                        .expect("semantic id is valid"),
                    SemanticAttachment {
                        anchor: anchor(0x1),
                        source: observation_source(),
                    },
                ),
            ]),
            probes: BTreeMap::new(),
            metrics: BTreeMap::new(),
        };

        let value = serde_json::to_value(&table).expect("table serializes");

        assert!(value["semantic"].is_object());
        assert_eq!(value["semantic"].as_object().expect("object").len(), 2);
        assert!(value["semantic"].get("a.checkpoint").is_some());
        assert!(value["semantic"].get("z.checkpoint").is_some());
        assert!(value["probes"].is_array());
    }

    #[test]
    fn effect_edge_uses_infer_ir_effect_class() {
        let source = ProbeSource::EffectEdge {
            effect: EffectId::new(7),
            class: EffectClass::Rng {
                slot: RngSlot::Decode,
            },
        };
        let value = serde_json::to_value(&source).expect("source serializes");

        assert_eq!(value["class"]["kind"], "Rng");
        assert_eq!(value["class"]["slot"]["kind"], "Decode");
    }

    #[test]
    fn metric_probe_has_weight_field() {
        let metric = MetricProbe {
            metric: MetricId::from_static("bank.switches").expect("metric id"),
            source: MetricSource::PerBankSwitch,
            aggregation: MetricAggregation::Sum,
            importance: ProbeImportanceClass::Diagnostic,
            weight: 17,
        };
        let value = serde_json::to_value(&metric).expect("metric serializes");

        assert_eq!(metric.weight, 17);
        assert_eq!(value["weight"], serde_json::json!(17));
    }

    #[test]
    fn probe_registry_instantiation_per_selector_kind() {
        let selection =
            build_probe_metric_selection_v1(&probe_metric_inputs_fixture()).expect("selection ok");

        assert_eq!(selection.probes.len(), 4);
        assert!(matches!(
            probe_by_id(&selection.probes, 10).source,
            ProbeSource::Anchor { .. }
        ));
        assert!(matches!(
            probe_by_id(&selection.probes, 11).source,
            ProbeSource::NodePreEntry { node } if node == NodeId::new(1)
        ));
        assert!(matches!(
            probe_by_id(&selection.probes, 12).source,
            ProbeSource::EffectEdge { effect, class: EffectClass::Rng { slot: RngSlot::Decode } }
                if effect == EffectId::new(0)
        ));
        assert!(matches!(
            probe_by_id(&selection.probes, 13).source,
            ProbeSource::ValueEdge { value } if value == ValueId::new(5)
        ));

        assert_eq!(
            (
                probe_by_id(&selection.probes, 10).importance,
                probe_by_id(&selection.probes, 10).level,
            ),
            (ProbeImportanceClass::Required, ProbeLevel::Always)
        );
        assert_eq!(
            (
                probe_by_id(&selection.probes, 11).importance,
                probe_by_id(&selection.probes, 11).level,
            ),
            (ProbeImportanceClass::Important, ProbeLevel::Verbose)
        );
        assert_eq!(
            (
                probe_by_id(&selection.probes, 12).importance,
                probe_by_id(&selection.probes, 12).level,
            ),
            (ProbeImportanceClass::Diagnostic, ProbeLevel::OnError)
        );
        assert_eq!(
            (
                probe_by_id(&selection.probes, 13).importance,
                probe_by_id(&selection.probes, 13).level,
            ),
            (ProbeImportanceClass::BestEffort, ProbeLevel::Always)
        );
        assert_eq!(selection.metrics.len(), 4);
        for class in ProbeImportanceClass::ALL {
            assert!(
                selection
                    .probes
                    .iter()
                    .any(|probe| probe.importance == class),
                "missing probe class {class:?}"
            );
            assert!(
                selection
                    .metrics
                    .iter()
                    .any(|metric| metric.importance == class),
                "missing metric class {class:?}"
            );
        }
    }

    #[test]
    fn probe_instance_id_unique_when_source_differs() {
        let mut inputs = probe_metric_inputs_fixture();
        inputs.probe_registry = ProbeRegistrySnapshot::new(vec![probe_registry_entry(
            20,
            ProbeSourceSelector::ByValueRole {
                role: PolicyValueRole::LogitVector,
            },
            ProbeLevel::Verbose,
            ProbeImportanceClass::BestEffort,
            1,
        )])
        .expect("single probe registry builds");
        inputs.infer_ir_product.infer_ir.values.push(ValueDecl {
            value_id: ValueId::new(90),
            kind: ValueKind::LogitVector,
            format: ValueFormat::ExactAccumulator,
            layout: ValueLayout::scalar(),
        });
        inputs.infer_ir_product.infer_ir.provenance.values.insert(
            ValueId::new(90),
            ValueProducerRef::Node {
                node: NodeId::new(3),
            },
        );

        let selection = build_probe_metric_selection_v1(&inputs).expect("selection ok");
        let matching = selection
            .probes
            .iter()
            .filter(|probe| probe.probe_id == TraceProbeId(20))
            .collect::<Vec<_>>();

        assert_eq!(matching.len(), 2);
        assert_eq!(matching[0].probe_id, matching[1].probe_id);
        assert_ne!(matching[0].instance_id, matching[1].instance_id);
    }

    #[test]
    fn probe_instance_id_collision_rejected_at_canonical_sort() {
        let probe = probe_for_ordering(7, hash(0x77), 1);
        let diagnostics = canonical_order_operational_probes_v1(vec![probe.clone(), probe])
            .expect_err("duplicate probe instance id rejects")
            .into_vec();

        assert!(diagnostics.iter().any(|diagnostic| matches!(
            diagnostic.code,
            ValidationCode::ObservationProbeSourceInvalid { probe_id }
                if probe_id == TraceProbeId(7)
        )));
    }

    #[test]
    fn probe_budget_governance_floor_filters() {
        let mut inputs = probe_metric_inputs_fixture();
        inputs.op_policy_projection.optional_probe_floor = ProbeImportanceClass::Important;

        let selection = build_probe_metric_selection_v1(&inputs).expect("selection ok");

        assert_eq!(
            selection
                .probes
                .iter()
                .map(|probe| probe.importance)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([
                ProbeImportanceClass::Required,
                ProbeImportanceClass::Important,
            ])
        );
        assert_eq!(
            selection
                .metrics
                .iter()
                .map(|metric| metric.importance)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([
                ProbeImportanceClass::Required,
                ProbeImportanceClass::Important,
            ])
        );
    }

    #[test]
    fn probe_budget_governance_demotion_filters() {
        let mut inputs = probe_metric_inputs_fixture();
        inputs.op_policy_projection.trace_demotion =
            TraceDemotionLevel::DropDiagnosticAndBestEffort;

        let selection = build_probe_metric_selection_v1(&inputs).expect("selection ok");

        assert!(selection.probes.iter().all(|probe| matches!(
            probe.importance,
            ProbeImportanceClass::Required | ProbeImportanceClass::Important
        )));
        assert!(selection.metrics.iter().all(|metric| matches!(
            metric.importance,
            ProbeImportanceClass::Required | ProbeImportanceClass::Important
        )));
    }

    #[test]
    fn probe_budget_governance_disabled_optional_filters() {
        let mut inputs = probe_metric_inputs_fixture();
        inputs
            .op_policy_projection
            .disabled_optional_probes
            .insert(TraceProbeId(11));

        let selection = build_probe_metric_selection_v1(&inputs).expect("selection ok");

        assert!(
            !selection
                .probes
                .iter()
                .any(|probe| probe.probe_id == TraceProbeId(11))
        );
        assert_eq!(selection.probes.len(), 3);
    }

    #[test]
    fn disabled_required_probe_rejected() {
        let mut inputs = probe_metric_inputs_fixture();
        inputs
            .op_policy_projection
            .disabled_optional_probes
            .insert(TraceProbeId(10));

        let diagnostics = build_probe_metric_selection_v1(&inputs)
            .expect_err("required probe disable rejects")
            .into_vec();

        assert!(diagnostics.iter().any(|diagnostic| matches!(
            diagnostic.code,
            ValidationCode::ObservationRequiredProbeDisabled { probe_id }
                if probe_id == TraceProbeId(10)
        )));
    }

    #[test]
    fn disabled_unknown_probe_rejected() {
        let mut inputs = probe_metric_inputs_fixture();
        inputs
            .op_policy_projection
            .disabled_optional_probes
            .insert(TraceProbeId(999));

        let diagnostics = build_probe_metric_selection_v1(&inputs)
            .expect_err("unknown disabled probe rejects")
            .into_vec();

        assert!(diagnostics.iter().any(|diagnostic| matches!(
            diagnostic.code,
            ValidationCode::ObservationProbeIdUnknown { probe_id }
                if probe_id == TraceProbeId(999)
        )));
    }

    #[test]
    fn probe_class_cap_exceeded_for_non_required_classes() {
        for class in [
            ProbeImportanceClass::Important,
            ProbeImportanceClass::Diagnostic,
            ProbeImportanceClass::BestEffort,
        ] {
            let mut inputs = probe_metric_inputs_fixture();
            match class {
                ProbeImportanceClass::Important => {
                    inputs
                        .op_policy_projection
                        .profile_observation_caps
                        .important_max = 1;
                }
                ProbeImportanceClass::Diagnostic => {
                    inputs
                        .op_policy_projection
                        .profile_observation_caps
                        .diagnostic_max = 1;
                }
                ProbeImportanceClass::BestEffort => {
                    inputs
                        .op_policy_projection
                        .profile_observation_caps
                        .best_effort_max = 1;
                }
                ProbeImportanceClass::Required => unreachable!("required is uncapped"),
            }

            let diagnostics = build_probe_metric_selection_v1(&inputs)
                .expect_err("class cap exceeded rejects")
                .into_vec();

            assert!(diagnostics.iter().any(|diagnostic| matches!(
                diagnostic.code,
                ValidationCode::ObservationProbeClassCapExceeded {
                    class: observed_class,
                    ..
                } if observed_class == class
            )));
        }
    }

    #[test]
    fn metric_registry_filter_per_slice_reserved_rejected() {
        let mut inputs = probe_metric_inputs_fixture();
        inputs.metric_registry = MetricRegistrySnapshot {
            entries: vec![metric_registry_entry(
                "slice.reserved",
                MetricSource::PerSliceReserved,
                MetricAggregation::Sum,
                ProbeImportanceClass::Diagnostic,
                1,
            )],
        };

        let diagnostics = build_probe_metric_selection_v1(&inputs)
            .expect_err("per-slice reserved metric rejects")
            .into_vec();

        assert!(diagnostics.iter().any(|diagnostic| matches!(
            &diagnostic.code,
            ValidationCode::ObservationMetricSourceReservedV1 { metric }
                if metric.as_str() == "slice.reserved"
        )));
    }

    #[test]
    fn metric_aggregation_histogram_bucket_count_zero_rejected() {
        let mut inputs = probe_metric_inputs_fixture();
        inputs.metric_registry = MetricRegistrySnapshot {
            entries: vec![metric_registry_entry(
                "hist.zero",
                MetricSource::PerPass,
                MetricAggregation::Histogram { bucket_count: 0 },
                ProbeImportanceClass::Diagnostic,
                1,
            )],
        };

        let diagnostics = build_probe_metric_selection_v1(&inputs)
            .expect_err("zero-bucket histogram rejects")
            .into_vec();

        assert!(diagnostics.iter().any(|diagnostic| matches!(
            &diagnostic.code,
            ValidationCode::ObservationMetricHistogramBucketCountZero { metric }
                if metric.as_str() == "hist.zero"
        )));
    }

    #[test]
    fn per_class_total_weight_includes_metrics() {
        let mut inputs = probe_metric_inputs_fixture();
        inputs
            .op_policy_projection
            .profile_observation_caps
            .important_max = 6;

        let diagnostics = build_probe_metric_selection_v1(&inputs)
            .expect_err("combined probe and metric cap rejects")
            .into_vec();

        assert!(diagnostics.iter().any(|diagnostic| matches!(
            diagnostic.code,
            ValidationCode::ObservationProbeClassCapExceeded {
                class: ProbeImportanceClass::Important,
                observed: 7,
                cap: 6,
            }
        )));
    }

    #[test]
    fn per_class_total_weight_arithmetic_saturates_on_overflow() {
        let mut total = PerClassWeightTotal {
            required: u32::MAX - 1,
            important: u32::MAX - 2,
            diagnostic: u32::MAX - 3,
            best_effort: u32::MAX - 4,
        };
        add_importance_weight(&mut total, ProbeImportanceClass::Required, 10);
        add_importance_weight(&mut total, ProbeImportanceClass::Important, 10);
        add_importance_weight(&mut total, ProbeImportanceClass::Diagnostic, 10);
        add_importance_weight(&mut total, ProbeImportanceClass::BestEffort, 10);

        assert_eq!(
            total,
            PerClassWeightTotal {
                required: u32::MAX,
                important: u32::MAX,
                diagnostic: u32::MAX,
                best_effort: u32::MAX,
            }
        );

        assert_eq!(
            combine_weight_totals(
                PerClassWeightTotal {
                    required: u32::MAX,
                    important: u32::MAX - 1,
                    diagnostic: 1,
                    best_effort: 2,
                },
                PerClassWeightTotal {
                    required: 1,
                    important: 2,
                    diagnostic: u32::MAX,
                    best_effort: u32::MAX,
                },
            ),
            PerClassWeightTotal {
                required: u32::MAX,
                important: u32::MAX,
                diagnostic: u32::MAX,
                best_effort: u32::MAX,
            }
        );
    }

    #[test]
    fn required_class_uncapped_in_v2_profile_default() {
        let mut inputs = probe_metric_inputs_fixture();
        inputs.probe_registry = ProbeRegistrySnapshot::new(vec![probe_registry_entry(
            10,
            ProbeSourceSelector::ByAnchorCheckpoint {
                checkpoint: semantic_checkpoint_kind_to_id(SemanticCheckpointKind::PostEmbedding {
                    layer: LayerId::new(0),
                }),
                timing: ProbeTiming::PostEntry,
            },
            ProbeLevel::Always,
            ProbeImportanceClass::Required,
            u16::MAX,
        )])
        .expect("required-only registry builds");

        let selection = build_probe_metric_selection_v1(&inputs).expect("required is uncapped");

        assert_eq!(
            inputs
                .op_policy_projection
                .profile_observation_caps
                .required_max,
            None
        );
        assert!(selection.per_class_total_weight.required > u32::from(u16::MAX));
    }

    #[test]
    fn probe_ordering_canonical_lex_on_pair() {
        let low_hash = hash(0x01);
        let high_hash = hash(0x02);
        let probes = canonical_order_operational_probes_v1(vec![
            probe_for_ordering(9, high_hash, 9),
            probe_for_ordering(8, high_hash, 8),
            probe_for_ordering(9, low_hash, 7),
        ])
        .expect("ordering succeeds");

        assert_eq!(
            probes
                .iter()
                .map(|probe| (probe.probe_id.0, probe.instance_id.source_fingerprint))
                .collect::<Vec<_>>(),
            vec![(8, high_hash), (9, low_hash), (9, high_hash)]
        );
    }

    #[test]
    fn metric_selection_orders_by_metric_id() {
        let mut inputs = probe_metric_inputs_fixture();
        inputs.metric_registry = MetricRegistrySnapshot {
            entries: vec![
                metric_registry_entry(
                    "z.last",
                    MetricSource::PerPass,
                    MetricAggregation::Sum,
                    ProbeImportanceClass::Required,
                    1,
                ),
                metric_registry_entry(
                    "a.first",
                    MetricSource::PerPass,
                    MetricAggregation::Sum,
                    ProbeImportanceClass::Required,
                    1,
                ),
            ],
        };

        let selection = build_probe_metric_selection_v1(&inputs).expect("selection ok");

        assert_eq!(metric_ids(&selection.metrics), vec!["a.first", "z.last"]);
    }

    #[test]
    fn effect_class_diagnostic_precedence() {
        let entry = probe_registry_entry(
            44,
            ProbeSourceSelector::ByEffectClass {
                class: PolicyEffectClass::SequenceState,
            },
            ProbeLevel::OnError,
            ProbeImportanceClass::Diagnostic,
            1,
        );
        assert!(matches!(
            reserved_effect_probe_diagnostic(
                &entry,
                EffectClass::SequenceState {
                    slot: StateSlotId::new(0),
                },
            )
            .code,
            ValidationCode::ObservationSequenceStateProbeReserved { .. }
        ));
        assert!(matches!(
            reserved_effect_probe_diagnostic(&entry, EffectClass::FaultBoundary).code,
            ValidationCode::ObservationFaultBoundaryProbeReserved { .. }
        ));
        assert!(matches!(
            reserved_effect_probe_diagnostic(
                &entry,
                EffectClass::Rng {
                    slot: RngSlot::Decode,
                },
            )
            .code,
            ValidationCode::ObservationReservedEffectProbe { .. }
        ));
    }

    #[test]
    fn sequence_state_and_fault_boundary_effect_probe_rejections() {
        for (policy_class, effect_class, expected) in [
            (
                PolicyEffectClass::SequenceState,
                EffectClass::SequenceState {
                    slot: StateSlotId::new(0),
                },
                "sequence",
            ),
            (
                PolicyEffectClass::FaultBoundary,
                EffectClass::FaultBoundary,
                "fault",
            ),
        ] {
            let mut inputs = probe_metric_inputs_fixture();
            inputs.probe_registry = ProbeRegistrySnapshot::new(vec![probe_registry_entry(
                30,
                ProbeSourceSelector::ByEffectClass {
                    class: policy_class,
                },
                ProbeLevel::OnError,
                ProbeImportanceClass::Diagnostic,
                1,
            )])
            .expect("effect registry builds");
            inputs.infer_ir_product.infer_ir.effects.clear();
            inputs.infer_ir_product.infer_ir.provenance.effects.clear();
            add_effect_to_inputs(&mut inputs, EffectId::new(30), effect_class);

            let diagnostics = build_probe_metric_selection_v1(&inputs)
                .expect_err("reserved effect rejects")
                .into_vec();

            match expected {
                "sequence" => assert!(diagnostics.iter().any(|diagnostic| matches!(
                    diagnostic.code,
                    ValidationCode::ObservationSequenceStateProbeReserved { .. }
                ))),
                "fault" => assert!(diagnostics.iter().any(|diagnostic| matches!(
                    diagnostic.code,
                    ValidationCode::ObservationFaultBoundaryProbeReserved { .. }
                ))),
                _ => unreachable!("closed fixture case"),
            }
        }
    }

    #[test]
    fn trace_probe_id_converts_at_trace_install_boundary() {
        let policy_id = TraceProbeId(77);
        let abi_id: gbf_abi::trace::TraceProbeId = policy_id.into();
        let round_trip: TraceProbeId = abi_id.into();

        assert_eq!(abi_id, gbf_abi::trace::TraceProbeId(77));
        assert_eq!(round_trip, policy_id);
    }

    #[test]
    fn trace_event_shape_payload_bytes_within_abi_slot() {
        let selection =
            build_probe_metric_selection_v1(&probe_metric_inputs_fixture()).expect("selection ok");

        for probe in &selection.probes {
            assert!(probe.event_shape.max_payload_bytes <= ABI_TRACE_EVENT_PAYLOAD_BYTES);
        }
        assert!(
            TraceEventShape::new(
                TraceEventPayloadLayout::Tuple {
                    spec: TraceEventTupleSpecId("tuple.large".to_owned()),
                },
                ABI_TRACE_EVENT_PAYLOAD_BYTES + 1,
                TraceEventTupleSpecId("tuple.large".to_owned()),
            )
            .is_err()
        );
    }

    #[test]
    fn probe_metric_selection_emits_construction_events() {
        let _ = take_recorded_finalization_events();
        build_probe_metric_selection_v1(&probe_metric_inputs_fixture()).expect("selection ok");

        let names = take_recorded_finalization_events();
        for expected in [
            OBSERVATION_PROBE_REGISTRY_INSTANTIATION_EVENT,
            OBSERVATION_PROBE_BUDGET_GOVERNANCE_EVENT,
            OBSERVATION_PROBE_ORDERING_EVENT,
            OBSERVATION_METRIC_REGISTRY_FILTER_EVENT,
            OBSERVATION_METRIC_SELECTION_EVENT,
            OBSERVATION_METRIC_ORDERING_EVENT,
        ] {
            assert!(
                names.iter().any(|name| *name == expected),
                "missing event {expected}; observed {names:?}"
            );
        }
    }

    #[test]
    fn probe_metric_construction_tracing_field_shapes_are_pinned() {
        let inputs = probe_metric_inputs_fixture();
        let capture = CapturedTracingLayer::default();
        let subscriber = tracing_subscriber::registry().with(capture.clone());
        tracing::subscriber::with_default(subscriber, || {
            build_probe_metric_selection_v1(&inputs).expect("selection ok");
        });
        let events = capture.events();

        assert_event_fields(
            &events,
            OBSERVATION_PROBE_REGISTRY_INSTANTIATION_EVENT,
            &[
                "event",
                "instantiated_count",
                "selector_kind",
                "importance_class",
            ],
        );
        assert_event_fields(
            &events,
            OBSERVATION_PROBE_BUDGET_GOVERNANCE_EVENT,
            &[
                "event",
                "surviving_count",
                "dropped_floor",
                "dropped_demotion",
                "dropped_disabled",
                "dropped_required_rejected_pre",
                "dropped_unknown_rejected_pre",
            ],
        );
        assert_event_fields(
            &events,
            OBSERVATION_PROBE_ORDERING_EVENT,
            &["event", "final_count"],
        );
        assert_event_fields(
            &events,
            OBSERVATION_METRIC_REGISTRY_FILTER_EVENT,
            &["event", "surviving_count", "dropped_per_slice_reserved"],
        );
        assert_event_fields(
            &events,
            OBSERVATION_METRIC_SELECTION_EVENT,
            &[
                "event",
                "surviving_count",
                "dropped_floor",
                "dropped_demotion",
            ],
        );
        assert_event_fields(
            &events,
            OBSERVATION_METRIC_ORDERING_EVENT,
            &["event", "final_count"],
        );
    }

    #[test]
    fn precheck_failure_governance_telemetry_distinguishes_unknown_and_required() {
        let mut inputs = probe_metric_inputs_fixture();
        inputs
            .op_policy_projection
            .disabled_optional_probes
            .insert(TraceProbeId(10));
        inputs
            .op_policy_projection
            .disabled_optional_probes
            .insert(TraceProbeId(999));

        let capture = CapturedTracingLayer::default();
        let subscriber = tracing_subscriber::registry().with(capture.clone());
        let diagnostics = tracing::subscriber::with_default(subscriber, || {
            build_probe_metric_selection_v1(&inputs)
                .expect_err("disabled required and unknown probes reject")
                .into_vec()
        });
        let events = capture.events();

        assert!(diagnostics.iter().any(|diagnostic| matches!(
            diagnostic.code,
            ValidationCode::ObservationRequiredProbeDisabled { probe_id }
                if probe_id == TraceProbeId(10)
        )));
        assert!(diagnostics.iter().any(|diagnostic| matches!(
            diagnostic.code,
            ValidationCode::ObservationProbeIdUnknown { probe_id }
                if probe_id == TraceProbeId(999)
        )));
        let governance = event_by_name(&events, OBSERVATION_PROBE_BUDGET_GOVERNANCE_EVENT);
        assert_eq!(
            governance.fields.get("dropped_required_rejected_pre"),
            Some(&"1".to_owned())
        );
        assert_eq!(
            governance.fields.get("dropped_unknown_rejected_pre"),
            Some(&"1".to_owned())
        );
    }

    #[test]
    fn precheck_failure_emits_governance_without_instantiation_by_contract() {
        let mut inputs = probe_metric_inputs_fixture();
        inputs
            .op_policy_projection
            .disabled_optional_probes
            .insert(TraceProbeId(999));

        let capture = CapturedTracingLayer::default();
        let subscriber = tracing_subscriber::registry().with(capture.clone());
        tracing::subscriber::with_default(subscriber, || {
            let _ = build_probe_metric_selection_v1(&inputs)
                .expect_err("precheck failure skips instantiation");
        });
        let event_names = capture
            .events()
            .into_iter()
            .map(|event| event.name)
            .collect::<Vec<_>>();

        assert_eq!(
            event_names,
            vec![OBSERVATION_PROBE_BUDGET_GOVERNANCE_EVENT.to_owned()],
            "precheck failures intentionally emit only governance telemetry"
        );
    }

    fn assert_event_fields(
        events: &[CapturedEvent],
        event_name: &'static str,
        expected_fields: &[&str],
    ) {
        let event = event_by_name(events, event_name);
        let actual = event
            .fields
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let expected = expected_fields.iter().copied().collect::<BTreeSet<_>>();
        assert_eq!(actual, expected, "field shape for {event_name}");
    }

    fn event_by_name<'a>(
        events: &'a [CapturedEvent],
        event_name: &'static str,
    ) -> &'a CapturedEvent {
        events
            .iter()
            .find(|event| event.name == event_name)
            .unwrap_or_else(|| panic!("missing tracing event {event_name}; observed {events:?}"))
    }

    #[test]
    fn anchor_attachment_table_consistent_with_vectors() {
        let plan = plan_fixture();
        let observation = &plan.semantic[0];
        let attachment = plan
            .anchor_table
            .semantic
            .get(&observation.checkpoint)
            .expect("semantic attachment exists");

        assert_eq!(attachment.anchor, observation.anchor);
        assert_eq!(attachment.source, observation.source);
    }

    #[test]
    fn observation_provenance_keyed_by_probe_instance_id_not_trace_probe_id() {
        fn assert_probe_provenance_map(_: &BTreeMap<ProbeInstanceId, EvidenceRef>) {}

        let plan = plan_fixture();
        assert_probe_provenance_map(&plan.provenance.probe_provenance);
    }

    #[test]
    fn trace_budget_projection_round_trip() {
        let projection = TraceBudgetProjection {
            projected_max_events_per_slice: 5,
            projected_max_bytes_per_frame: 64,
            fits_declared_budget: true,
        };
        let encoded = serde_json::to_vec(&projection).expect("projection serializes");
        let decoded: TraceBudgetProjection =
            serde_json::from_slice(&encoded).expect("projection decodes");

        assert_eq!(decoded, projection);
    }

    fn build_core_success_fixture() -> (ObservationPlanInputs, ObservationPlanCoreSuccess) {
        let inputs = probe_metric_inputs_fixture();
        let success = build_observation_plan_core(&inputs).expect("observation plan builds");
        (inputs, success)
    }

    fn valid_run_stage4_inputs() -> ObservationPlanInputs {
        let mut inputs = probe_metric_inputs_fixture();
        let infer_ir_self_hash =
            crate::s3::infer_ir::infer_ir_self_hash(&inputs.infer_ir_product.infer_ir)
                .expect("mutated fixture infer_ir hashes");
        inputs.infer_ir_product.infer_ir_self_hash = infer_ir_self_hash;
        inputs.infer_ir_self_hash = infer_ir_self_hash;
        inputs.audit_parents.static_budget_self_hash = inputs
            .infer_ir_product
            .infer_ir
            .identity
            .static_budget_self_hash;
        inputs
    }

    fn stage4_store() -> (tempfile::TempDir, BlobStore) {
        let dir = tempfile::tempdir().expect("cache tempdir");
        let store = BlobStore::open(dir.path().to_path_buf()).expect("blob store");
        (dir, store)
    }

    fn stage4_env_for<'a>(inputs: &ObservationPlanInputs) -> Stage4PassEnvironment<'a> {
        Stage4PassEnvironment::new(inputs.op_policy_projection.observability_mode)
    }

    fn stage4_core_failure_inputs() -> ObservationPlanInputs {
        let mut inputs = valid_run_stage4_inputs();
        inputs
            .op_policy_projection
            .disabled_optional_probes
            .insert(TraceProbeId(10));
        inputs
    }

    fn expect_stage4_failure(
        result: Result<ObservationPlanStageOutput, RunStage4Error>,
        context: &'static str,
    ) -> ObservationPlanStageFailure {
        let err = result.unwrap_err();
        err.stage_failure()
            .unwrap_or_else(|| panic!("{context} did not return a stage failure: {err:?}"))
            .clone()
    }

    #[test]
    fn run_stage4_dense_default_emits_three_reports() {
        let inputs = valid_run_stage4_inputs();
        let report_dir = tempfile::tempdir().expect("report tempdir");
        let (_cache_dir, store) = stage4_store();
        let cache = StageCache::new(&store);
        let env = stage4_env_for(&inputs)
            .with_report_dir(report_dir.path())
            .with_stage_cache(&cache);

        let output = run_stage4(inputs, env).expect("Stage 4 succeeds");

        assert_eq!(output.report.outcome, ReportOutcome::Passed);
        assert_eq!(output.sc_re_emit_report.outcome, ReportOutcome::Passed);
        assert_eq!(
            output.operational_probe_report.outcome,
            ReportOutcome::Passed
        );

        for file_name in [
            "observation_plan.json",
            "semantic_checkpoint_schema.json",
            "operational_probe_schema.json",
        ] {
            assert!(
                report_dir.path().join(file_name).is_file(),
                "missing {file_name}"
            );
        }

        let observation_bytes =
            std::fs::read(report_dir.path().join("observation_plan.json")).expect("report reads");
        let decoded: ReportEnvelope<ObservationPlanReportBody> =
            serde_json::from_slice(&observation_bytes).expect("observation report decodes");
        assert_eq!(
            canonicalize_report(&decoded).expect("decoded report canonicalizes"),
            observation_bytes
        );
        assert!(decoded.body.result.is_some());
    }

    #[test]
    fn run_stage4_moe_trace_emits_three_reports() {
        run_stage4_named_accept_fixture_emits_three_reports("moe_trace");
    }

    #[test]
    fn run_stage4_bringup_minimum_works() {
        run_stage4_named_accept_fixture_emits_three_reports("bringup_minimum");
    }

    #[test]
    fn run_stage4_sequence_state_works() {
        run_stage4_named_accept_fixture_emits_three_reports("sequence_state");
    }

    fn run_stage4_named_accept_fixture_emits_three_reports(_fixture_name: &'static str) {
        let inputs = valid_run_stage4_inputs();
        let report_dir = tempfile::tempdir().expect("report tempdir");
        let env = stage4_env_for(&inputs).with_report_dir(report_dir.path());

        let output = run_stage4(inputs, env).expect("Stage 4 succeeds");

        assert_eq!(output.report.outcome, ReportOutcome::Passed);
        assert_stage4_report_files(report_dir.path());
    }

    #[test]
    fn success_emits_three_reports_atomically() {
        let inputs = valid_run_stage4_inputs();
        let report_dir = tempfile::tempdir().expect("report tempdir");
        let env = stage4_env_for(&inputs).with_report_dir(report_dir.path());

        run_stage4(inputs, env).expect("Stage 4 succeeds");

        assert_stage4_report_files(report_dir.path());
        let mut report_files = std::fs::read_dir(report_dir.path())
            .expect("report dir reads")
            .map(|entry| {
                entry
                    .expect("report dir entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect::<Vec<_>>();
        report_files.sort();
        assert_eq!(
            report_files,
            vec![
                "observation_plan.json",
                "operational_probe_schema.json",
                "semantic_checkpoint_schema.json",
            ]
        );
    }

    #[test]
    fn failure_emits_observation_plan_json_failed_and_optional_ancillaries() {
        let inputs = stage4_core_failure_inputs();
        let report_dir = tempfile::tempdir().expect("report tempdir");
        let env = stage4_env_for(&inputs).with_report_dir(report_dir.path());

        let failure = expect_stage4_failure(run_stage4(inputs, env), "Stage 4 core fails");

        assert_eq!(failure.report.outcome, ReportOutcome::Failed);
        let observation_bytes =
            std::fs::read(report_dir.path().join("observation_plan.json")).expect("report reads");
        let decoded: ReportEnvelope<ObservationPlanReportBody> =
            serde_json::from_slice(&observation_bytes).expect("observation report decodes");
        assert_eq!(decoded.outcome, ReportOutcome::Failed);
        assert!(decoded.body.result.is_none());
        assert_eq!(
            canonicalize_report(&decoded).expect("decoded report canonicalizes"),
            observation_bytes
        );
    }

    #[test]
    fn r_nopartialproduct_op_failed_observation_plan_body_result_is_none() {
        let inputs = stage4_core_failure_inputs();
        let env = stage4_env_for(&inputs);

        let failure = expect_stage4_failure(run_stage4(inputs, env), "Stage 4 core fails");

        assert!(failure.report.body.result.is_none());
    }

    #[test]
    fn op_pre_2_failure_with_report_dir_emits_failed_envelope_and_canonical_code() {
        let mut inputs = valid_run_stage4_inputs();
        inputs.semantic_checkpoint_schema_hash = hash(0xe1);
        let report_dir = tempfile::tempdir().expect("report tempdir");
        let env = stage4_env_for(&inputs).with_report_dir(report_dir.path());

        let failure = expect_stage4_failure(run_stage4(inputs, env), "OP-Pre-2 fails");

        assert!(failure.diagnostics.as_slice().iter().any(|diagnostic| {
            matches!(
                diagnostic.code,
                ValidationCode::ObservationScHashMismatch { .. }
            ) && diagnostic.origin == ValidationOrigin::ObservationPlanConstruction
        }));
        let observation_bytes =
            std::fs::read(report_dir.path().join("observation_plan.json")).expect("report reads");
        let decoded: ReportEnvelope<ObservationPlanReportBody> =
            serde_json::from_slice(&observation_bytes).expect("observation report decodes");
        assert_eq!(decoded.outcome, ReportOutcome::Failed);
        assert!(decoded.body.result.is_none());
        assert!(decoded.body.diagnostics.iter().any(|diagnostic| {
            matches!(
                diagnostic.code,
                ValidationCode::ObservationScHashMismatch { .. }
            ) && diagnostic.origin == ValidationOrigin::ObservationPlanConstruction
        }));
    }

    fn assert_stage4_report_files(report_dir: &std::path::Path) {
        for file_name in [
            "observation_plan.json",
            "semantic_checkpoint_schema.json",
            "operational_probe_schema.json",
        ] {
            assert!(report_dir.join(file_name).is_file(), "missing {file_name}");
        }
    }

    #[test]
    fn run_stage4_second_run_same_inputs_cache_hit() {
        let inputs = valid_run_stage4_inputs();
        let (_cache_dir, store) = stage4_store();
        let cache = StageCache::new(&store);
        let first_report_dir = tempfile::tempdir().expect("first report tempdir");
        let second_report_dir = tempfile::tempdir().expect("second report tempdir");

        let first_env = stage4_env_for(&inputs)
            .with_report_dir(first_report_dir.path())
            .with_stage_cache(&cache);
        let first = run_stage4(inputs.clone(), first_env).expect("first Stage 4 succeeds");
        let second_env = stage4_env_for(&inputs)
            .with_report_dir(second_report_dir.path())
            .with_stage_cache(&cache);
        let second = run_stage4(inputs, second_env).expect("second Stage 4 succeeds from cache");

        assert_eq!(
            canonical_json_bytes(&first.product).expect("first product canonicalizes"),
            canonical_json_bytes(&second.product).expect("second product canonicalizes")
        );
        assert_eq!(first.report, second.report);
    }

    #[test]
    fn run_stage4_audit_parent_drift_cache_hit_with_rewrap() {
        let first_inputs = valid_run_stage4_inputs();
        let mut second_inputs = first_inputs.clone();
        second_inputs.audit_parents.compile_request_hash = hash(0xf1);
        second_inputs.audit_parents.policy_resolution_self_hash = hash(0xf2);
        let (_cache_dir, store) = stage4_store();
        let cache = StageCache::new(&store);

        let first_env = stage4_env_for(&first_inputs).with_stage_cache(&cache);
        let first = run_stage4(first_inputs, first_env).expect("first Stage 4 succeeds");
        let second_env = stage4_env_for(&second_inputs).with_stage_cache(&cache);
        let second = run_stage4(second_inputs.clone(), second_env)
            .expect("second Stage 4 succeeds from cache");

        assert_eq!(
            canonical_json_bytes(&first.product).expect("first product canonicalizes"),
            canonical_json_bytes(&second.product).expect("second product canonicalizes")
        );
        assert_eq!(
            second.report.body.input_identity.compile_request_hash,
            second_inputs.audit_parents.compile_request_hash
        );
        assert_ne!(
            first.report.body.input_identity.compile_request_hash,
            second.report.body.input_identity.compile_request_hash
        );
        assert_ne!(
            first.report.report_self_hash,
            second.report.report_self_hash
        );
    }

    #[test]
    fn run_stage4_failed_memo_replay_refreshes_audit_parents() {
        let first_inputs = stage4_core_failure_inputs();
        let mut second_inputs = first_inputs.clone();
        second_inputs.audit_parents.compile_request_hash = hash(0xe2);
        let (_cache_dir, store) = stage4_store();
        let cache = StageCache::new(&store);

        let first_env = stage4_env_for(&first_inputs).with_stage_cache(&cache);
        let first = expect_stage4_failure(
            run_stage4(first_inputs, first_env),
            "first Stage 4 fails and writes memo",
        );
        let second_env = stage4_env_for(&second_inputs).with_stage_cache(&cache);
        let second = expect_stage4_failure(
            run_stage4(second_inputs.clone(), second_env),
            "second Stage 4 replays failure memo",
        );

        assert_eq!(first.report.outcome, ReportOutcome::Failed);
        assert_eq!(second.report.outcome, ReportOutcome::Failed);
        assert_eq!(
            second.report.body.input_identity.compile_request_hash,
            second_inputs.audit_parents.compile_request_hash
        );
        assert_ne!(
            first.report.body.input_identity.compile_request_hash,
            second.report.body.input_identity.compile_request_hash
        );
    }

    #[test]
    fn stage4_driver_failure_memo_event_fires_on_new_memo_write_not_replay() {
        let inputs = stage4_core_failure_inputs();
        let (_cache_dir, store) = stage4_store();
        let cache = StageCache::new(&store);
        let capture = CapturedTracingLayer::default();
        let subscriber = tracing_subscriber::registry().with(capture.clone());

        tracing::subscriber::with_default(subscriber, || {
            tracing_core::callsite::rebuild_interest_cache();
            let first_env = stage4_env_for(&inputs).with_stage_cache(&cache);
            let _ = run_stage4(inputs.clone(), first_env).expect_err("first failure writes memo");
            let second_env = stage4_env_for(&inputs).with_stage_cache(&cache);
            let _ = run_stage4(inputs, second_env).expect_err("second failure replays memo");
        });
        tracing_core::callsite::rebuild_interest_cache();

        let events = capture.events();
        let count = events
            .iter()
            .filter(|event| event.name == STAGE4_DRIVER_FAILURE_MEMO_EVENT)
            .count();
        assert_eq!(
            count, 1,
            "stage4.driver.failure_memo is emitted only for new memo writes; observed {events:?}"
        );
    }

    #[test]
    fn op_pre_3_runs_before_cache_hit_replay() {
        let inputs = valid_run_stage4_inputs();
        let (_cache_dir, store) = stage4_store();
        let cache = StageCache::new(&store);
        let first_env = stage4_env_for(&inputs).with_stage_cache(&cache);
        run_stage4(inputs.clone(), first_env).expect("first run populates cache");
        let resolved_mode = match inputs.op_policy_projection.observability_mode {
            ObservabilityMode::Invariant => ObservabilityMode::Flexible,
            ObservabilityMode::Flexible => ObservabilityMode::Invariant,
        };
        let second_env = Stage4PassEnvironment::new(resolved_mode).with_stage_cache(&cache);

        let failure = expect_stage4_failure(
            run_stage4(inputs, second_env),
            "OP-Pre-3 fails before cache lookup",
        );

        assert!(failure.diagnostics.as_slice().iter().any(|diagnostic| {
            matches!(
                &diagnostic.code,
                ValidationCode::ReportSemanticInvariantViolated { field }
                    if field == &FieldPath::from("op_policy_projection.observability_mode")
            )
        }));
    }

    #[test]
    fn op_pre_1_infer_ir_self_hash_mismatch() {
        let mut inputs = valid_run_stage4_inputs();
        inputs.infer_ir_self_hash = hash(0xd1);
        let env = stage4_env_for(&inputs);

        let failure = run_stage4(inputs, env)
            .expect_err("OP-Pre-1 fails")
            .stage_failure()
            .expect("stage failure")
            .clone();

        assert!(failure.report.body.result.is_none());
        assert!(failure.diagnostics.as_slice().iter().any(|diagnostic| {
            matches!(diagnostic.code, ValidationCode::SemanticCoreHashMismatch)
        }));
    }

    #[test]
    fn op_pre_3_observability_mode_mismatch() {
        let inputs = valid_run_stage4_inputs();
        let resolved_mode = match inputs.op_policy_projection.observability_mode {
            ObservabilityMode::Invariant => ObservabilityMode::Flexible,
            ObservabilityMode::Flexible => ObservabilityMode::Invariant,
        };
        let env = Stage4PassEnvironment::new(resolved_mode);

        let failure = run_stage4(inputs, env)
            .expect_err("OP-Pre-3 fails")
            .stage_failure()
            .expect("stage failure")
            .clone();

        assert!(failure.diagnostics.as_slice().iter().any(|diagnostic| {
            matches!(
                &diagnostic.code,
                ValidationCode::ReportSemanticInvariantViolated { field }
                    if field == &FieldPath::from("op_policy_projection.observability_mode")
            )
        }));
    }

    #[test]
    fn op_pre_3a_driver_determinism_mismatch_uses_canonical_diagnostic() {
        let mut inputs = valid_run_stage4_inputs();
        inputs.op_policy_projection.determinism_class = DeterminismClass::Deterministic;
        let env = stage4_env_for(&inputs);

        let failure = expect_stage4_failure(run_stage4(inputs, env), "OP-Pre-3a fails");

        assert!(failure.diagnostics.as_slice().iter().any(|diagnostic| {
            matches!(
                &diagnostic.code,
                ValidationCode::ObservationDeterminismMismatch { field }
                    if field == &FieldPath::from("op_policy_projection.determinism_class")
            ) && diagnostic.origin == ValidationOrigin::ObservationPlanConstruction
        }));
    }

    #[test]
    fn op_pre_4_static_budget_not_passing() {
        let mut inputs = valid_run_stage4_inputs();
        inputs.audit_parents.static_budget_self_hash = hash(0xd4);
        let env = stage4_env_for(&inputs);

        let failure = expect_stage4_failure(run_stage4(inputs, env), "OP-Pre-4 fails");

        assert!(failure.diagnostics.as_slice().iter().any(|diagnostic| {
            matches!(
                &diagnostic.code,
                ValidationCode::ReportSemanticInvariantViolated { field }
                    if field == &FieldPath::from("audit_parents.static_budget_self_hash")
            )
        }));
    }

    #[test]
    fn anchor_attachment_table_bind_consistency() {
        let (_inputs, success) = build_core_success_fixture();
        let plan = &success.product.observation_plan;

        assert_eq!(plan.anchor_table.semantic.len(), plan.semantic.len());
        assert_eq!(plan.anchor_table.probes.len(), plan.probes.len());
        assert_eq!(plan.anchor_table.metrics.len(), plan.metrics.len());
        for semantic in &plan.semantic {
            let attachment = plan
                .anchor_table
                .semantic
                .get(&semantic.checkpoint)
                .expect("semantic attachment exists");
            assert_eq!(attachment.anchor, semantic.anchor);
            assert_eq!(attachment.source, semantic.source);
        }
        for probe in &plan.probes {
            assert_eq!(
                plan.anchor_table.probes.get(&probe.instance_id),
                Some(&probe.source)
            );
        }
        for metric in &plan.metrics {
            assert_eq!(
                plan.anchor_table.metrics.get(&metric.metric),
                Some(&metric.source)
            );
        }
    }

    #[test]
    fn provenance_bind_keyed_by_probe_instance_id_and_total() {
        fn assert_probe_provenance_map(_: &BTreeMap<ProbeInstanceId, EvidenceRef>) {}

        let (_inputs, success) = build_core_success_fixture();
        let plan = &success.product.observation_plan;
        assert_probe_provenance_map(&plan.provenance.probe_provenance);

        assert_eq!(
            plan.provenance
                .semantic_provenance
                .keys()
                .cloned()
                .collect::<BTreeSet<_>>(),
            plan.semantic
                .iter()
                .map(|entry| entry.checkpoint.clone())
                .collect::<BTreeSet<_>>()
        );
        assert_eq!(
            plan.provenance
                .probe_provenance
                .keys()
                .copied()
                .collect::<BTreeSet<_>>(),
            plan.probes
                .iter()
                .map(|entry| entry.instance_id)
                .collect::<BTreeSet<_>>()
        );
        assert_eq!(
            plan.provenance
                .metric_provenance
                .keys()
                .cloned()
                .collect::<BTreeSet<_>>(),
            plan.metrics
                .iter()
                .map(|entry| entry.metric.clone())
                .collect::<BTreeSet<_>>()
        );
    }

    #[test]
    fn provenance_bind_unknown_metric_uses_typed_metric_id_diagnostic() {
        let inputs = probe_metric_inputs_fixture();
        let bindings = bind_semantic_observations_v1(&inputs).expect("semantic binds");
        let selection = build_probe_metric_selection_v1(&inputs).expect("selection builds");
        let mut metrics = selection.metrics.clone();
        metrics.push(MetricProbe {
            metric: MetricId::from_static("unknown.metric").expect("metric id"),
            source: MetricSource::PerPass,
            aggregation: MetricAggregation::Sum,
            importance: ProbeImportanceClass::Diagnostic,
            weight: 1,
        });

        let diagnostics =
            bind_observation_provenance(&inputs, &bindings.selected, &selection.probes, &metrics)
                .expect_err("unknown metric provenance rejects")
                .into_vec();

        assert!(diagnostics.iter().any(|diagnostic| matches!(
            &diagnostic.code,
            ValidationCode::ObservationMetricIdUnknown { metric }
                if metric.as_str() == "unknown.metric"
        )));
    }

    #[test]
    fn schema_re_emit_includes_source_metadata_and_is_schema_subset() {
        let (inputs, success) = build_core_success_fixture();
        let schema = &success.product.build_active_checkpoint_schema;
        let original = inputs
            .semantic_checkpoint_schema
            .checkpoints
            .iter()
            .map(|entry| (entry.semantic.clone(), entry))
            .collect::<BTreeMap<_, _>>();
        let plan_semantic = success
            .product
            .observation_plan
            .semantic
            .iter()
            .map(|entry| (entry.checkpoint.clone(), entry))
            .collect::<BTreeMap<_, _>>();

        assert_eq!(
            usize::from(schema.build_active_count),
            schema.checkpoints.len()
        );
        assert_eq!(
            u32::from(schema.mandatory_count) + u32::from(schema.optional_count),
            u32::from(schema.build_active_count)
        );
        for entry in &schema.checkpoints {
            let original = original
                .get(&entry.id)
                .expect("re-emitted checkpoint came from artifact schema");
            let semantic = plan_semantic
                .get(&entry.id)
                .expect("re-emitted checkpoint came from selected semantic plan");
            assert_eq!(entry.kind, semantic.kind);
            assert_eq!(entry.original_checkpoint_metadata.compact, original.compact);
            assert_eq!(entry.original_checkpoint_metadata.stratum, original.stratum);
            assert_eq!(
                entry.original_checkpoint_metadata.source_op.as_deref(),
                original.source_op.as_deref()
            );
            assert_eq!(entry.source, semantic.source);
            assert_eq!(entry.encoding, semantic.encoding);
            assert_eq!(entry.attachment_anchor, semantic.anchor);
            assert!(
                serde_json::to_value(entry)
                    .expect("entry serializes")
                    .get("canonical_provenance_tuple")
                    .is_some()
            );
        }

        let first = canonical_json_bytes(schema).expect("first re-emit canonicalizes");
        let second =
            canonical_json_bytes(&success.product.build_active_checkpoint_schema).expect("second");
        assert_eq!(first, second);
    }

    #[test]
    fn operational_probe_schema_emit_per_class_totals() {
        let (_inputs, success) = build_core_success_fixture();
        let schema = &success.product.operational_probe_schema;

        assert_eq!(usize::from(schema.probe_count), schema.probes.len());
        assert_eq!(usize::from(schema.metric_count), schema.metrics.len());
        assert_eq!(
            combine_weight_totals(
                schema.per_class_probe_weight_total,
                schema.per_class_metric_weight_total,
            ),
            schema.per_class_total_weight
        );
        assert_eq!(
            schema.per_class_probe_weight_total,
            per_class_probe_weight_total(&success.product.observation_plan.probes)
        );
        assert_eq!(
            schema.per_class_metric_weight_total,
            per_class_metric_weight_total(&success.product.observation_plan.metrics)
        );
    }

    #[test]
    fn op_sc_checklist_labels_all_nineteen_invariants() {
        assert_eq!(OP_SC_CHECKS.len(), 19);
        for (index, check) in OP_SC_CHECKS.iter().enumerate() {
            assert_eq!(check.id, format!("OP-SC-{}", index + 1));
            assert!(!check.field.is_empty(), "missing field for {}", check.id);
            assert!(
                !check.description.is_empty(),
                "missing description for {}",
                check.id
            );
        }
    }

    #[test]
    fn self_consistency_duplicate_metric_localizes_to_op_sc_3() {
        let inputs = probe_metric_inputs_fixture();
        let bindings = bind_semantic_observations_v1(&inputs).expect("semantic binds");
        let selection = build_probe_metric_selection_v1(&inputs).expect("selection builds");
        let success = build_observation_plan_core(&inputs).expect("core builds");
        let mut plan = success.product.observation_plan.clone();
        plan.metrics
            .push(plan.metrics.first().expect("fixture has a metric").clone());

        let diagnostics = validate_observation_plan_self_consistency(
            &inputs,
            &bindings,
            &selection,
            &plan,
            &success.product.build_active_checkpoint_schema,
            &success.product.operational_probe_schema,
            success.product.observation_plan_self_hash,
        )
        .expect_err("duplicate metric violates OP-SC-3")
        .into_vec();

        assert!(diagnostics.iter().any(|diagnostic| matches!(
            &diagnostic.detail,
            ValidationDetail::Field { field } if field.as_str().starts_with("OP-SC-3.")
        )));
    }

    #[test]
    fn invariant_budget_check_under_invariant_fails_when_over() {
        let mut inputs = probe_metric_inputs_fixture();
        inputs.op_policy_projection.observability_mode = ObservabilityMode::Invariant;
        inputs.op_policy_projection.trace_budget = PolicyTraceBudget {
            max_events_per_slice: 1,
            max_bytes_per_frame: 1,
            drop_policy: PolicyTraceDropPolicy::HaltAndFault,
        };

        let failure = build_observation_plan_core(&inputs).expect_err("invariant bust rejects");

        assert!(
            failure
                .diagnostics
                .as_slice()
                .iter()
                .any(|diagnostic| matches!(
                    diagnostic.code,
                    ValidationCode::ObservationInvariantModeBudgetBusted { .. }
                ))
        );
    }

    #[test]
    fn invariant_budget_check_under_flexible_records_but_passes() {
        let mut inputs = probe_metric_inputs_fixture();
        inputs.op_policy_projection.observability_mode = ObservabilityMode::Flexible;
        inputs.op_policy_projection.trace_budget = PolicyTraceBudget {
            max_events_per_slice: 1,
            max_bytes_per_frame: 1,
            drop_policy: PolicyTraceDropPolicy::DropOldest,
        };

        let success = build_observation_plan_core(&inputs).expect("flexible bust records only");

        assert!(
            !success
                .product
                .observation_plan
                .trace_budget_projection
                .fits_declared_budget
        );
        assert!(success.observation_plan_body.diagnostics.is_empty());
    }

    #[test]
    fn trace_budget_projection_uses_frequency_bound() {
        fn probe_with_frequency(
            id: u16,
            frequency_bound: TraceFrequencyBound,
            payload_bytes: u16,
        ) -> OperationalProbe {
            let probe_id = TraceProbeId(id);
            let source = probe_source(u32::from(id));
            OperationalProbe {
                instance_id: ProbeInstanceId {
                    probe_id,
                    source_fingerprint: probe_instance_source_fingerprint(probe_id, &source)
                        .expect("fingerprint hashes"),
                },
                probe_id,
                source,
                level: ProbeLevel::Always,
                importance: ProbeImportanceClass::Required,
                event_shape: trace_shape_with_layout(
                    "budget.probe",
                    TraceEventPayloadLayout::Tuple {
                        spec: TraceEventTupleSpecId(format!("budget.probe.{id}")),
                    },
                    payload_bytes,
                ),
                frequency_bound,
                weight: 1,
            }
        }

        let budget = TraceBudget::new(100, 1000, TraceDropPolicy::DropOldest).expect("budget");
        let probes = vec![
            probe_with_frequency(1, TraceFrequencyBound::PerPass { max_events: 2 }, 3),
            probe_with_frequency(
                2,
                TraceFrequencyBound::PerToken {
                    max_events_per_token: 3,
                },
                5,
            ),
            probe_with_frequency(
                3,
                TraceFrequencyBound::PerNodeExecution {
                    max_events_per_execution: 4,
                },
                7,
            ),
            probe_with_frequency(
                4,
                TraceFrequencyBound::PerFrame {
                    max_events_per_frame: 5,
                },
                11,
            ),
            probe_with_frequency(
                5,
                TraceFrequencyBound::FaultOnly {
                    max_events_per_frame: 6,
                },
                13,
            ),
        ];

        let projection = project_trace_budget(&probes, budget);

        assert_eq!(projection.projected_max_events_per_slice, 20);
        assert_eq!(
            projection.projected_max_bytes_per_frame,
            2 * 3 + 3 * 5 + 4 * 7 + 5 * 11 + 6 * 13
        );
        assert!(projection.fits_declared_budget);
    }

    #[test]
    fn self_consistency_op_sc_1_through_19_representative_fixture() {
        let (inputs, success) = build_core_success_fixture();
        let plan = &success.product.observation_plan;
        let probe_registry = inputs
            .probe_registry
            .entries
            .iter()
            .map(|entry| (entry.probe_id, entry))
            .collect::<BTreeMap<_, _>>();

        assert_eq!(
            observation_plan_self_hash(plan).expect("plan hashes"),
            success.product.observation_plan_self_hash
        );
        assert!(plan.trace_budget_projection.fits_declared_budget);
        for probe in &plan.probes {
            let registry = probe_registry
                .get(&probe.probe_id)
                .expect("active probe has registry entry");
            assert_eq!(probe.level, registry.level);
            assert_eq!(probe.importance, registry.importance);
            assert_eq!(probe.event_shape, registry.event_shape);
            assert_eq!(probe.frequency_bound, registry.frequency_bound);
            assert_eq!(probe.weight, registry.weight);
        }
        assert_eq!(
            success
                .product
                .build_active_checkpoint_schema
                .checkpoints
                .len(),
            plan.semantic.len()
        );
        assert_eq!(
            success.product.operational_probe_schema.probes.len(),
            plan.probes.len()
        );
        assert_eq!(
            success.product.operational_probe_schema.metrics.len(),
            plan.metrics.len()
        );
    }

    #[test]
    fn canonical_sort_deterministic_for_core_product() {
        let first = build_observation_plan_core(&probe_metric_inputs_fixture())
            .expect("first observation plan builds");
        let second = build_observation_plan_core(&perturbed_order_inputs_fixture())
            .expect("second observation plan builds");

        assert_eq!(
            canonical_json_bytes(&first.product).expect("first product canonicalizes"),
            canonical_json_bytes(&second.product).expect("second product canonicalizes")
        );
    }

    fn perturbed_order_inputs_fixture() -> ObservationPlanInputs {
        let mut inputs = probe_metric_inputs_fixture();
        inputs.semantic_checkpoint_schema.checkpoints.reverse();
        inputs.probe_registry.entries.reverse();
        inputs.metric_registry.entries.reverse();
        inputs.trace_event_layout_registry.entries.reverse();
        inputs.infer_ir_product.infer_ir.nodes.reverse();
        inputs.infer_ir_product.infer_ir.values.reverse();
        inputs.infer_ir_product.infer_ir.effects.reverse();
        inputs
    }

    #[test]
    fn finalization_events_fire_once_per_core_build() {
        let _ = take_recorded_finalization_events();
        build_observation_plan_core(&probe_metric_inputs_fixture()).expect("core builds");

        let names = take_recorded_finalization_events();
        for expected in [
            OBSERVATION_ANCHOR_TABLE_BIND_EVENT,
            OBSERVATION_PROVENANCE_BIND_EVENT,
            OBSERVATION_SCHEMA_RE_EMIT_EVENT,
            OBSERVATION_OPERATIONAL_PROBE_SCHEMA_EMIT_EVENT,
            OBSERVATION_INVARIANT_BUDGET_CHECK_EVENT,
            OBSERVATION_SELF_CONSISTENCY_EVENT,
            OBSERVATION_CANONICAL_SORT_EVENT,
        ] {
            let count = names.iter().filter(|name| **name == expected).count();
            assert_eq!(count, 1, "event {expected} count in {names:?}");
        }
    }

    #[test]
    fn no_report_self_hash_cycles_in_report_bodies() {
        let (_inputs, success) = build_core_success_fixture();
        let observation_report =
            ReportEnvelope::new(ReportOutcome::Passed, success.observation_plan_body.clone())
                .expect("observation report envelope")
                .with_computed_self_hash()
                .expect("observation report hashes");
        let sc_report = ReportEnvelope::new(ReportOutcome::Passed, success.sc_re_emit_body.clone())
            .expect("sc re-emit envelope")
            .with_computed_self_hash()
            .expect("sc re-emit hashes");
        let op_report = ReportEnvelope::new(
            ReportOutcome::Passed,
            success.operational_probe_body.clone(),
        )
        .expect("operational probe envelope")
        .with_computed_self_hash()
        .expect("operational probe hashes");

        for (own_hash, body) in [
            (
                observation_report.report_self_hash,
                serde_json::to_string(&observation_report.body).expect("body serializes"),
            ),
            (
                sc_report.report_self_hash,
                serde_json::to_string(&sc_report.body).expect("body serializes"),
            ),
            (
                op_report.report_self_hash,
                serde_json::to_string(&op_report.body).expect("body serializes"),
            ),
        ] {
            assert!(
                !body.contains(&own_hash.to_string()),
                "report body references its own report_self_hash"
            );
        }
    }

    #[test]
    fn semantic_anchor_uses_landed_f_b5_shape() {
        let value = serde_json::to_value(anchor(0x77)).expect("anchor serializes");
        let object = value.as_object().expect("anchor is an object");

        assert_eq!(object.len(), 1);
        assert!(object.get("anchor_id").is_some());
    }

    #[test]
    fn semantic_checkpoint_kind_closed_round_trip() {
        let kinds = [
            SemanticCheckpointKind::PostEmbedding {
                layer: LayerId::new(0),
            },
            SemanticCheckpointKind::PostRouter {
                layer: LayerId::new(1),
            },
            SemanticCheckpointKind::PostExpertDowncast {
                layer: LayerId::new(1),
                expert: ExpertId::new(2),
            },
            SemanticCheckpointKind::PostLogits,
            SemanticCheckpointKind::PostDecode,
        ];

        for kind in kinds {
            let id = semantic_checkpoint_kind_to_id(kind);
            assert_eq!(try_parse_semantic_checkpoint_kind(&id), Some(kind));
        }
    }

    #[test]
    fn semantic_checkpoint_kind_parse_to_id_uses_canonical_numeric_segments() {
        for raw in [
            "layer.0.post_embedding",
            "layer.12.post_router",
            "layer.12.expert.3.post_downcast",
            "post_logits",
            "post_decode",
        ] {
            let id = SemanticCheckpointId::from_static(raw).expect("semantic id is valid");
            let kind =
                try_parse_semantic_checkpoint_kind(&id).expect("canonical id parses to kind");

            assert_eq!(semantic_checkpoint_kind_to_id(kind), id);
        }
    }

    #[test]
    fn semantic_checkpoint_kind_rejects_non_canonical_numeric_segments() {
        for raw in [
            "layer.00.post_embedding",
            "layer.01.post_embedding",
            "layer.001.post_router",
            "layer.1.expert.02.post_downcast",
        ] {
            let id = SemanticCheckpointId::from_static(raw).expect("semantic id is valid");

            assert_eq!(try_parse_semantic_checkpoint_kind(&id), None);
        }
    }

    #[test]
    fn semantic_checkpoint_role_from_stratum() {
        assert_eq!(
            SemanticCheckpointRole::from(SemanticStratum::Denotation),
            SemanticCheckpointRole::Mandatory
        );
        assert_eq!(
            SemanticCheckpointRole::from(SemanticStratum::Artifact),
            SemanticCheckpointRole::Mandatory
        );
        assert_eq!(
            SemanticCheckpointRole::from(SemanticStratum::Operational),
            SemanticCheckpointRole::Optional
        );
    }

    #[test]
    fn semantic_selection_v1_mandatory_and_optional_intersect_feasible() {
        let bindings =
            bind_semantic_observations_v1(&semantic_binding_inputs_fixture()).expect("binds");
        let selected = bindings
            .selected
            .iter()
            .map(|entry| entry.observation.checkpoint.as_str().to_owned())
            .collect::<BTreeSet<_>>();

        assert_eq!(bindings.selected.len(), 5);
        assert_eq!(bindings.mandatory_count, 3);
        assert_eq!(bindings.optional_count, 2);
        assert_eq!(
            selected,
            BTreeSet::from([
                "layer.0.expert.1.post_downcast".to_owned(),
                "layer.0.post_embedding".to_owned(),
                "layer.0.post_router".to_owned(),
                "post_decode".to_owned(),
                "post_logits".to_owned(),
            ])
        );
    }

    #[test]
    fn semantic_selection_optional_not_feasible_silently_dropped() {
        let schema = semantic_checkpoint_schema_from_entries(vec![
            (
                SemanticCheckpointKind::PostEmbedding {
                    layer: LayerId::new(0),
                },
                1,
                SemanticStratum::Denotation,
                "embedding",
            ),
            (
                SemanticCheckpointKind::PostRouter {
                    layer: LayerId::new(9),
                },
                2,
                SemanticStratum::Operational,
                "route_top1",
            ),
        ]);
        let bindings =
            bind_semantic_observations_v1(&with_schema(semantic_binding_inputs_fixture(), schema))
                .expect("optional infeasible checkpoint drops without a diagnostic");

        assert_eq!(bindings.selected.len(), 1);
        assert_eq!(bindings.mandatory_count, 1);
        assert_eq!(bindings.optional_count, 0);
        assert_eq!(
            bindings.selected[0].observation.checkpoint.as_str(),
            "layer.0.post_embedding"
        );
    }

    #[test]
    fn semantic_selection_mandatory_not_feasible_fails() {
        let schema = semantic_checkpoint_schema_from_entries(vec![(
            SemanticCheckpointKind::PostLogits,
            4,
            SemanticStratum::Artifact,
            "classify",
        )]);
        let diagnostics = bind_semantic_observations_v1(&with_schema(inputs_fixture(), schema))
            .expect_err("mandatory infeasible checkpoint rejects")
            .into_vec();

        assert!(diagnostics.iter().any(|diagnostic| matches!(
            &diagnostic.code,
            ValidationCode::ObservationMandatoryCheckpointNotFeasible { checkpoint }
                if checkpoint.as_str() == "post_logits"
        )));
    }

    #[test]
    fn semantic_anchor_binding_missing_anchor_fails() {
        let mut inputs = semantic_binding_inputs_fixture();
        inputs
            .infer_ir_product
            .infer_ir
            .anchors
            .remove(&NodeId::new(4));
        let diagnostics = bind_semantic_observations_v1(&inputs)
            .expect_err("missing selected anchor rejects")
            .into_vec();

        assert!(diagnostics.iter().any(|diagnostic| matches!(
            &diagnostic.code,
            ValidationCode::ObservationCheckpointNotAttachable { checkpoint }
                if checkpoint.as_str() == "post_decode"
        )));
    }

    #[test]
    fn semantic_encoding_binding_follows_generated_bytes_compare_domain() {
        let inputs = with_workload_compare_domain(
            semantic_binding_inputs_fixture(),
            WorkloadCompareDomain::GeneratedBytes,
        );
        let bindings = bind_semantic_observations_v1(&inputs).expect("binds");
        let encodings = bindings
            .selected
            .iter()
            .map(|entry| {
                (
                    entry.observation.checkpoint.as_str().to_owned(),
                    entry.observation.encoding,
                )
            })
            .collect::<BTreeMap<_, _>>();

        assert_eq!(encodings["post_decode"], ObservationEncoding::TokenId);
        assert_eq!(encodings["post_logits"], ObservationEncoding::TokenId);
        assert_eq!(
            encodings["layer.0.post_router"],
            ObservationEncoding::ExpertId
        );
        assert_eq!(
            encodings["layer.0.post_embedding"],
            ObservationEncoding::Canonical
        );
    }

    #[test]
    fn anchor_to_checkpoint_per_inferoptag_variant() {
        assert_eq!(
            anchor_to_checkpoint(CanonicalProvenanceTuple {
                op_tag: InferOpTag::Embedding,
                occurrence_index: 0,
                ..CanonicalProvenanceTuple::new(InferOpTag::Embedding, 0)
            }),
            Some(SemanticCheckpointKind::PostEmbedding {
                layer: LayerId::new(0)
            })
        );
        assert_eq!(
            anchor_to_checkpoint(CanonicalProvenanceTuple {
                op_tag: InferOpTag::RouteTop1,
                layer: Some(LayerId::new(2)),
                occurrence_index: 0,
                ..CanonicalProvenanceTuple::new(InferOpTag::RouteTop1, 0)
            }),
            Some(SemanticCheckpointKind::PostRouter {
                layer: LayerId::new(2)
            })
        );
        assert_eq!(
            anchor_to_checkpoint(CanonicalProvenanceTuple {
                op_tag: InferOpTag::ExpertMatVec,
                layer: Some(LayerId::new(2)),
                expert: Some(ExpertId::new(3)),
                expert_weight_slot: Some(ExpertWeightSlot::FfnDown),
                occurrence_index: 0,
                ..CanonicalProvenanceTuple::new(InferOpTag::ExpertMatVec, 0)
            }),
            Some(SemanticCheckpointKind::PostExpertDowncast {
                layer: LayerId::new(2),
                expert: ExpertId::new(3)
            })
        );
        assert_eq!(
            anchor_to_checkpoint(CanonicalProvenanceTuple::new(InferOpTag::Classify, 0)),
            Some(SemanticCheckpointKind::PostLogits)
        );
        assert_eq!(
            anchor_to_checkpoint(CanonicalProvenanceTuple::new(InferOpTag::DecodeToken, 0)),
            Some(SemanticCheckpointKind::PostDecode)
        );
        assert_eq!(
            anchor_to_checkpoint(CanonicalProvenanceTuple {
                op_tag: InferOpTag::CombineResidual,
                layer: Some(LayerId::new(1)),
                residual_site: Some(ResidualSite::PostSequence),
                occurrence_index: 0,
                ..CanonicalProvenanceTuple::new(InferOpTag::CombineResidual, 0)
            }),
            None
        );
    }

    #[test]
    fn anchor_to_checkpoint_unmapped_or_noncanonical_tuple_returns_none() {
        assert_eq!(
            anchor_to_checkpoint(CanonicalProvenanceTuple::new(InferOpTag::RouterMatVec, 0)),
            None
        );
        assert_eq!(
            anchor_to_checkpoint(CanonicalProvenanceTuple {
                op_tag: InferOpTag::ExpertMatVec,
                layer: Some(LayerId::new(2)),
                expert: Some(ExpertId::new(3)),
                expert_weight_slot: Some(ExpertWeightSlot::FfnUp),
                occurrence_index: 0,
                ..CanonicalProvenanceTuple::new(InferOpTag::ExpertMatVec, 0)
            }),
            None
        );
        assert_eq!(
            anchor_to_checkpoint(CanonicalProvenanceTuple {
                op_tag: InferOpTag::DecodeToken,
                occurrence_index: 1,
                ..CanonicalProvenanceTuple::new(InferOpTag::DecodeToken, 1)
            }),
            None
        );
    }

    #[test]
    fn encoding_for_closed_mapping_reachable_v1_cases() {
        let embedding = SemanticCheckpointKind::PostEmbedding {
            layer: LayerId::new(0),
        };
        let router = SemanticCheckpointKind::PostRouter {
            layer: LayerId::new(0),
        };
        let expert = SemanticCheckpointKind::PostExpertDowncast {
            layer: LayerId::new(0),
            expert: ExpertId::new(1),
        };

        assert_eq!(
            encoding_for(
                SemanticCheckpointKind::PostDecode,
                CompareDomain::CanonicalValue,
                DeterminismClass::BitExact,
            ),
            ObservationEncoding::TokenId
        );
        assert_eq!(
            encoding_for(
                SemanticCheckpointKind::PostDecode,
                CompareDomain::TokenIdOnly,
                DeterminismClass::BitExact,
            ),
            ObservationEncoding::TokenId
        );
        assert_eq!(
            encoding_for(
                router,
                CompareDomain::TokenIdOnly,
                DeterminismClass::BitExact
            ),
            ObservationEncoding::ExpertId
        );
        assert_eq!(
            encoding_for(
                router,
                CompareDomain::CanonicalValue,
                DeterminismClass::BitExact
            ),
            ObservationEncoding::Canonical
        );
        assert_eq!(
            encoding_for(
                embedding,
                CompareDomain::TokenIdOnly,
                DeterminismClass::BitExact
            ),
            ObservationEncoding::Canonical
        );
        assert_eq!(
            encoding_for(
                SemanticCheckpointKind::PostLogits,
                CompareDomain::CanonicalValue,
                DeterminismClass::BitExact,
            ),
            ObservationEncoding::Canonical
        );
        assert_eq!(
            encoding_for(
                SemanticCheckpointKind::PostLogits,
                CompareDomain::CanonicalValue,
                DeterminismClass::Deterministic,
            ),
            ObservationEncoding::QuantizedQ8_8
        );
        assert_eq!(
            encoding_for(
                SemanticCheckpointKind::PostLogits,
                CompareDomain::CanonicalValue,
                DeterminismClass::Nondeterministic,
            ),
            ObservationEncoding::QuantizedQ16_16
        );
        assert_eq!(
            encoding_for(
                SemanticCheckpointKind::PostLogits,
                CompareDomain::TokenIdOnly,
                DeterminismClass::BitExact,
            ),
            ObservationEncoding::TokenId
        );
        assert_eq!(
            encoding_for(
                expert,
                CompareDomain::TokenIdOnly,
                DeterminismClass::BitExact
            ),
            ObservationEncoding::Canonical
        );
    }

    #[test]
    fn encoding_for_reserved_v1_domains_panic_in_debug() {
        #[cfg(debug_assertions)]
        {
            for domain in [
                CompareDomain::ExpertIdOnly,
                CompareDomain::EnvelopeQ8_8,
                CompareDomain::EnvelopeQ16_16,
            ] {
                assert!(
                    std::panic::catch_unwind(|| {
                        encoding_for(
                            SemanticCheckpointKind::PostLogits,
                            domain,
                            DeterminismClass::BitExact,
                        );
                    })
                    .is_err(),
                    "reserved v1 compare domain {domain:?} should panic in debug"
                );
            }
        }
    }

    #[test]
    fn encoding_for_invalid_override_fails_without_panicking() {
        let diagnostic = try_encoding_for(
            SemanticCheckpointKind::PostLogits,
            CompareDomain::ExpertIdOnly,
            DeterminismClass::BitExact,
        )
        .expect_err("reserved v1 compare domain reports encoding diagnostic");

        assert!(matches!(
            diagnostic.code,
            ValidationCode::ObservationEncodingInvalidForCheckpoint { .. }
        ));
        assert_eq!(diagnostic.severity, DiagnosticSeverity::Hard);
    }

    #[test]
    fn semantic_binding_emits_construction_events() {
        let _ = take_recorded_finalization_events();
        bind_semantic_observations_v1(&semantic_binding_inputs_fixture())
            .expect("semantic binding succeeds");

        let names = take_recorded_finalization_events();
        for expected in [
            OBSERVATION_IDENTITY_BIND_EVENT,
            OBSERVATION_SCHEMA_INGEST_EVENT,
            OBSERVATION_BUILD_FEASIBILITY_FILTER_EVENT,
            OBSERVATION_SEMANTIC_SELECTION_EVENT,
            OBSERVATION_SEMANTIC_ANCHOR_BINDING_EVENT,
            OBSERVATION_ENCODING_BINDING_EVENT,
        ] {
            assert!(
                names.iter().any(|name| *name == expected),
                "missing event {expected}; observed {names:?}"
            );
        }
    }

    #[test]
    fn re_emitted_checkpoint_entry_carries_compact_checkpoint_id() {
        let entry = ReEmittedCheckpointEntry {
            id: semantic_checkpoint_kind_to_id(SemanticCheckpointKind::PostDecode),
            kind: SemanticCheckpointKind::PostDecode,
            artifact_role: SemanticCheckpointRole::Mandatory,
            original_checkpoint_metadata: SemanticCheckpointMetadata {
                compact: CompactCheckpointId(42),
                stratum: SemanticStratum::Denotation,
                source_op: Some("decode".to_owned()),
            },
            encoding: ObservationEncoding::TokenId,
            source: ObservationSource::DecodedToken {
                node: NodeId::new(8),
                value: ValueId::new(9),
            },
            attachment_node_id: NodeId::new(8),
            attachment_anchor: anchor(0x88),
            canonical_provenance_tuple: CanonicalProvenanceTuple {
                op_tag: InferOpTag::DecodeToken,
                layer: None,
                expert: None,
                expert_weight_slot: None,
                norm_site: None,
                state_slot: None,
                residual_site: None,
                occurrence_index: 0,
            },
        };
        let value = serde_json::to_value(&entry).expect("entry serializes");

        assert_eq!(
            entry.original_checkpoint_metadata.compact,
            CompactCheckpointId(42)
        );
        assert!(
            value["original_checkpoint_metadata"]
                .get("compact")
                .is_some()
        );
    }
}
