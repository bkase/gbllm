//! Stage 4 observation-plan report schemas.

use std::collections::BTreeMap;

use gbf_abi::{
    CompactCheckpointId, ProbeLevel, SemanticCheckpointId, SemanticStratum, TraceBudget,
};
use gbf_foundation::{
    CompileProfileId, EvidenceRef, ExpertId, FieldPath, Hash256, LayerId, WorkloadId,
};
use gbf_policy::{
    DiagnosticSeverity, MetricAggregation, MetricId, MetricSource, ObservabilityMode,
    ProbeImportanceClass, TraceEventShape, TraceFrequencyBound, TraceProbeId, ValidationCode,
    ValidationDetail, ValidationDiagnostic, ValidationOrigin,
};
use serde::{Deserialize, Serialize};

use crate::report_schemas::f_b6_f_b7_common::{
    CanonicalProvenanceTuple, EffectId, NodeId, SemanticAnchor, ValueId,
};
use crate::report_schemas::quant_graph_v1::DeterminismClassTag;
use crate::{CanonicalJsonError, ReportBody, ReportOutcome, domain_hash};
use crate::{canonical_map, string_key_map};

pub const SCHEMA_ID: &str = "observation_plan.v1";
pub const BUILD_ACTIVE_SEMANTIC_CHECKPOINT_SCHEMA_ID: &str =
    "build_active_semantic_checkpoint_schema.v1";
pub const OPERATIONAL_PROBE_SCHEMA_ID: &str = "operational_probe_schema.v1";
pub const SCHEMA_VERSION: &str = "1.0.0";

pub type ValidationDiagnosticRecord = ValidationDiagnostic;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationPlanReportBody {
    pub input_identity: ObservationPlanReportInputIdentity,
    pub result: Option<ObservationPlanReportResult>,
    pub diagnostics: Vec<ValidationDiagnosticRecord>,
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
    pub determinism: DeterminismClassTag,
    pub observability_mode: ObservabilityMode,
    pub trace_budget: TraceBudget,
    pub profile_id: CompileProfileId,
    pub workload_id: WorkloadId,
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

impl ReportBody for ObservationPlanReportBody {
    const REPORT_TYPE: &'static str = "ObservationPlanReport";
    const SCHEMA_ID: &'static str = SCHEMA_ID;
    const SCHEMA_VERSION: &'static str = SCHEMA_VERSION;

    fn validate_semantics(&self, outcome: ReportOutcome) -> Result<(), Vec<ValidationDiagnostic>> {
        validate_product_report_body(outcome, self.result.is_some(), &self.diagnostics)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticCheckpointSchemaReEmitBody {
    pub input_identity: SemanticCheckpointSchemaReEmitInputIdentity,
    pub result: Option<SemanticCheckpointSchemaReEmitResult>,
    pub diagnostics: Vec<ValidationDiagnosticRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticCheckpointSchemaReEmitInputIdentity {
    pub observation_plan_self_hash: Option<Hash256>,
    pub original_schema_hash: Hash256,
    pub infer_ir_self_hash: Hash256,
    pub quant_graph_self_hash: Hash256,
    pub artifact_aux_hash: Hash256,
    pub determinism: DeterminismClassTag,
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

impl ReportBody for SemanticCheckpointSchemaReEmitBody {
    const REPORT_TYPE: &'static str = "SemanticCheckpointSchemaReEmit";
    const SCHEMA_ID: &'static str = BUILD_ACTIVE_SEMANTIC_CHECKPOINT_SCHEMA_ID;
    const SCHEMA_VERSION: &'static str = SCHEMA_VERSION;

    fn validate_semantics(&self, outcome: ReportOutcome) -> Result<(), Vec<ValidationDiagnostic>> {
        validate_product_report_body_with_observation_hash(
            outcome,
            self.result.is_some(),
            &self.diagnostics,
            self.input_identity.observation_plan_self_hash,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationalProbeSchemaBody {
    pub input_identity: OperationalProbeSchemaInputIdentity,
    pub result: Option<OperationalProbeSchemaResult>,
    pub diagnostics: Vec<ValidationDiagnosticRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationalProbeSchemaInputIdentity {
    pub observation_plan_self_hash: Option<Hash256>,
    pub infer_ir_self_hash: Hash256,
    pub quant_graph_self_hash: Hash256,
    pub determinism: DeterminismClassTag,
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

impl ReportBody for OperationalProbeSchemaBody {
    const REPORT_TYPE: &'static str = "OperationalProbeSchema";
    const SCHEMA_ID: &'static str = OPERATIONAL_PROBE_SCHEMA_ID;
    const SCHEMA_VERSION: &'static str = SCHEMA_VERSION;

    fn validate_semantics(&self, outcome: ReportOutcome) -> Result<(), Vec<ValidationDiagnostic>> {
        validate_product_report_body_with_observation_hash(
            outcome,
            self.result.is_some(),
            &self.diagnostics,
            self.input_identity.observation_plan_self_hash,
        )
    }
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
    pub determinism: DeterminismClassTag,
    pub observability_mode: ObservabilityMode,
    pub trace_budget: TraceBudget,
    pub workload_id: WorkloadId,
    pub probe_registry_hash: Hash256,
    pub metric_registry_hash: Hash256,
    pub trace_event_layout_registry_hash: Hash256,
}

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
    pub source: MetricSource,
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
        class: gbf_policy::EffectClass,
    },
    Anchor {
        anchor: SemanticAnchor,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetricProbe {
    pub metric: MetricId,
    pub source: MetricSource,
    pub aggregation: MetricAggregation,
    pub importance: ProbeImportanceClass,
    pub weight: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnchorAttachmentTable {
    #[serde(with = "string_key_map")]
    pub semantic: BTreeMap<SemanticCheckpointId, SemanticAttachment>,
    #[serde(with = "canonical_map")]
    pub probes: BTreeMap<ProbeInstanceId, ProbeSource>,
    #[serde(with = "canonical_map")]
    pub metrics: BTreeMap<MetricId, MetricSource>,
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
    #[serde(with = "canonical_map")]
    pub semantic_provenance: BTreeMap<SemanticCheckpointId, EvidenceRef>,
    #[serde(with = "canonical_map")]
    pub probe_provenance: BTreeMap<ProbeInstanceId, EvidenceRef>,
    #[serde(with = "canonical_map")]
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

pub fn observation_plan_self_hash(plan: &ObservationPlan) -> Result<Hash256, CanonicalJsonError> {
    domain_hash("ObservationPlan", SCHEMA_ID, plan)
}

pub fn build_active_checkpoint_schema_hash(
    schema: &BuildActiveCheckpointSchema,
) -> Result<Hash256, CanonicalJsonError> {
    domain_hash(
        "BuildActiveCheckpointSchema",
        BUILD_ACTIVE_SEMANTIC_CHECKPOINT_SCHEMA_ID,
        schema.checkpoints.as_slice(),
    )
}

pub fn operational_probe_schema_hash(
    schema: &OperationalProbeSchema,
) -> Result<Hash256, CanonicalJsonError> {
    #[derive(Serialize)]
    struct OperationalProbeSchemaHashProjection<'a> {
        probes: &'a [ProbeSchemaEntry],
        metrics: &'a [MetricSchemaEntry],
    }

    domain_hash(
        "OperationalProbeSchema",
        OPERATIONAL_PROBE_SCHEMA_ID,
        &OperationalProbeSchemaHashProjection {
            probes: schema.probes.as_slice(),
            metrics: schema.metrics.as_slice(),
        },
    )
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
            let mut errors = Vec::new();
            if !has_result {
                errors.push(product_report_invariant_diagnostic("result"));
            }
            if has_hard {
                errors.push(product_report_invariant_diagnostic("diagnostics"));
            }
            Err(errors)
        }
        ReportOutcome::Failed => {
            let mut errors = Vec::new();
            if has_result {
                errors.push(product_report_invariant_diagnostic("result"));
            }
            if !has_hard {
                errors.push(product_report_invariant_diagnostic("diagnostics"));
            }
            Err(errors)
        }
    }
}

fn validate_product_report_body_with_observation_hash(
    outcome: ReportOutcome,
    has_result: bool,
    diagnostics: &[ValidationDiagnostic],
    observation_plan_self_hash: Option<Hash256>,
) -> Result<(), Vec<ValidationDiagnostic>> {
    let mut errors = match validate_product_report_body(outcome, has_result, diagnostics) {
        Ok(()) => Vec::new(),
        Err(errors) => errors,
    };

    if outcome == ReportOutcome::Passed && observation_plan_self_hash.is_none() {
        errors.push(product_report_invariant_diagnostic(
            "input_identity.observation_plan_self_hash",
        ));
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
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

#[cfg(test)]
mod tests {
    use super::*;
    use gbf_abi::TraceDropPolicy;
    use gbf_report_test_support::{assert_body_round_trips, hard_diagnostic, hash};

    use crate::{ReportEnvelope, canonicalize, round_trip_self_hash};

    mod gbf_report_test_support {
        use super::*;

        pub fn hash(byte: u8) -> Hash256 {
            Hash256::from_bytes([byte; 32])
        }

        pub fn hard_diagnostic() -> ValidationDiagnostic {
            ValidationDiagnostic::hard(
                ValidationOrigin::SemanticCore,
                ValidationCode::ReportSemanticInvariantViolated {
                    field: FieldPath::from("result"),
                },
                ValidationDetail::Field {
                    field: FieldPath::from("result"),
                },
                Vec::new(),
            )
        }

        pub fn assert_body_round_trips<T>(body: &T)
        where
            T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug,
        {
            let encoded = serde_json::to_vec(body).expect("body serializes");
            let decoded: T = serde_json::from_slice(&encoded).expect("body decodes");
            assert_eq!(&decoded, body);
        }
    }

    fn trace_budget() -> TraceBudget {
        TraceBudget {
            max_events_per_slice: 4,
            max_bytes_per_frame: 128,
            drop_policy: TraceDropPolicy::HaltAndFault,
        }
    }

    fn checkpoint(raw: &'static str) -> SemanticCheckpointId {
        SemanticCheckpointId::from_static(raw).expect("checkpoint id")
    }

    fn metric(raw: &'static str) -> MetricId {
        MetricId::from_static(raw).expect("metric id")
    }

    fn anchor(byte: u8) -> SemanticAnchor {
        SemanticAnchor::new(hash(byte))
    }

    fn evidence(byte: u8) -> EvidenceRef {
        EvidenceRef {
            kind: "fixture".to_owned(),
            reference: format!("fixture-{byte}"),
            hash: Some(hash(byte)),
        }
    }

    fn source(node: u32) -> ObservationSource {
        ObservationSource::NodeOutput {
            node: NodeId::new(node),
            value: ValueId::new(node + 10),
        }
    }

    fn semantic_attachment(node: u32) -> SemanticAttachment {
        SemanticAttachment {
            anchor: anchor(node as u8),
            source: source(node),
        }
    }

    fn probe_instance(byte: u8) -> ProbeInstanceId {
        ProbeInstanceId {
            probe_id: TraceProbeId(byte as u16),
            source_fingerprint: hash(byte),
        }
    }

    fn plan_fixture() -> ObservationPlan {
        let semantic_id = checkpoint("layer.0.post_embedding");
        let metric_id = metric("token.latency");
        let probe_id = probe_instance(7);
        ObservationPlan {
            identity: ObservationPlanIdentity {
                infer_ir_self_hash: hash(1),
                quant_graph_self_hash: hash(2),
                semantic_checkpoint_schema_hash: hash(3),
                observation_policy_projection_hash: hash(4),
                determinism: DeterminismClassTag::BitExact,
                observability_mode: ObservabilityMode::Invariant,
                trace_budget: trace_budget(),
                workload_id: WorkloadId::from("tiny"),
                probe_registry_hash: hash(5),
                metric_registry_hash: hash(6),
                trace_event_layout_registry_hash: hash(7),
            },
            semantic: vec![SemanticObservation {
                checkpoint: semantic_id.clone(),
                kind: SemanticCheckpointKind::PostEmbedding {
                    layer: LayerId::new(0),
                },
                compact: CompactCheckpointId(1),
                stratum: SemanticStratum::Denotation,
                source: source(1),
                encoding: ObservationEncoding::Canonical,
                anchor: anchor(8),
                artifact_role: SemanticCheckpointRole::Mandatory,
            }],
            probes: Vec::new(),
            metrics: Vec::new(),
            anchor_table: AnchorAttachmentTable {
                semantic: BTreeMap::from([(semantic_id.clone(), semantic_attachment(1))]),
                probes: BTreeMap::from([(
                    probe_id,
                    ProbeSource::NodePostEntry { node: NodeId(2) },
                )]),
                metrics: BTreeMap::from([(metric_id.clone(), MetricSource::PerToken)]),
            },
            provenance: ObservationProvenance {
                semantic_provenance: BTreeMap::from([(semantic_id, evidence(1))]),
                probe_provenance: BTreeMap::from([(probe_id, evidence(2))]),
                metric_provenance: BTreeMap::from([(metric_id, evidence(3))]),
            },
            trace_budget_projection: TraceBudgetProjection {
                projected_max_events_per_slice: 4,
                projected_max_bytes_per_frame: 64,
                fits_declared_budget: true,
            },
        }
    }

    fn input_identity() -> ObservationPlanReportInputIdentity {
        ObservationPlanReportInputIdentity {
            infer_ir_self_hash: hash(1),
            quant_graph_self_hash: hash(2),
            semantic_checkpoint_schema_hash: hash(3),
            observation_policy_projection_hash: hash(4),
            static_budget_self_hash: hash(5),
            policy_resolution_self_hash: hash(6),
            compile_request_hash: hash(7),
            artifact_aux_hash: hash(8),
            determinism: DeterminismClassTag::BitExact,
            observability_mode: ObservabilityMode::Invariant,
            trace_budget: trace_budget(),
            profile_id: CompileProfileId::from("Bringup"),
            workload_id: WorkloadId::from("tiny"),
        }
    }

    fn re_emit_entry() -> ReEmittedCheckpointEntry {
        ReEmittedCheckpointEntry {
            id: checkpoint("layer.0.post_embedding"),
            kind: SemanticCheckpointKind::PostEmbedding {
                layer: LayerId::new(0),
            },
            artifact_role: SemanticCheckpointRole::Mandatory,
            original_checkpoint_metadata: SemanticCheckpointMetadata {
                compact: CompactCheckpointId(1),
                stratum: SemanticStratum::Denotation,
                source_op: None,
            },
            encoding: ObservationEncoding::Canonical,
            source: source(3),
            attachment_node_id: NodeId::new(3),
            attachment_anchor: anchor(9),
            canonical_provenance_tuple: CanonicalProvenanceTuple::new(
                gbf_policy::InferOpTag::Embedding,
                0,
            ),
        }
    }

    fn observation_plan_body() -> ObservationPlanReportBody {
        let product = plan_fixture();
        ObservationPlanReportBody {
            input_identity: input_identity(),
            result: Some(ObservationPlanReportResult {
                semantic_count: 1,
                probe_count: 0,
                metric_count: 0,
                mandatory_semantic_count: 1,
                optional_semantic_count: 0,
                per_class_probe_count: PerClassCount::default(),
                per_class_metric_count: PerClassCount::default(),
                sc_re_emit_report_self_hash: hash(10),
                operational_probe_schema_report_self_hash: hash(11),
                observation_plan_self_hash: observation_plan_self_hash(&product).expect("hash"),
                product,
            }),
            diagnostics: Vec::new(),
        }
    }

    fn re_emit_body(observation_hash: Option<Hash256>) -> SemanticCheckpointSchemaReEmitBody {
        let checkpoints = vec![re_emit_entry()];
        let schema = BuildActiveCheckpointSchema {
            checkpoints: checkpoints.clone(),
            build_active_count: 1,
            mandatory_count: 1,
            optional_count: 0,
        };
        SemanticCheckpointSchemaReEmitBody {
            input_identity: SemanticCheckpointSchemaReEmitInputIdentity {
                observation_plan_self_hash: observation_hash,
                original_schema_hash: hash(3),
                infer_ir_self_hash: hash(1),
                quant_graph_self_hash: hash(2),
                artifact_aux_hash: hash(8),
                determinism: DeterminismClassTag::BitExact,
                workload_id: WorkloadId::from("tiny"),
            },
            result: Some(SemanticCheckpointSchemaReEmitResult {
                schema_hash: build_active_checkpoint_schema_hash(&schema).expect("schema hash"),
                checkpoints,
                build_active_count: 1,
                mandatory_count: 1,
                optional_count: 0,
            }),
            diagnostics: Vec::new(),
        }
    }

    fn failed_re_emit_body(
        observation_hash: Option<Hash256>,
    ) -> SemanticCheckpointSchemaReEmitBody {
        SemanticCheckpointSchemaReEmitBody {
            input_identity: SemanticCheckpointSchemaReEmitInputIdentity {
                observation_plan_self_hash: observation_hash,
                original_schema_hash: hash(3),
                infer_ir_self_hash: hash(1),
                quant_graph_self_hash: hash(2),
                artifact_aux_hash: hash(8),
                determinism: DeterminismClassTag::BitExact,
                workload_id: WorkloadId::from("tiny"),
            },
            result: None,
            diagnostics: vec![hard_diagnostic()],
        }
    }

    fn operational_probe_body(observation_hash: Option<Hash256>) -> OperationalProbeSchemaBody {
        let schema = OperationalProbeSchema {
            probes: Vec::new(),
            metrics: Vec::new(),
            probe_count: 0,
            metric_count: 0,
            per_class_probe_weight_total: PerClassWeightTotal::default(),
            per_class_metric_weight_total: PerClassWeightTotal::default(),
            per_class_total_weight: PerClassWeightTotal::default(),
        };
        OperationalProbeSchemaBody {
            input_identity: OperationalProbeSchemaInputIdentity {
                observation_plan_self_hash: observation_hash,
                infer_ir_self_hash: hash(1),
                quant_graph_self_hash: hash(2),
                determinism: DeterminismClassTag::BitExact,
                observability_mode: ObservabilityMode::Invariant,
                trace_budget: trace_budget(),
                profile_id: CompileProfileId::from("Bringup"),
                workload_id: WorkloadId::from("tiny"),
            },
            result: Some(OperationalProbeSchemaResult {
                schema_hash: operational_probe_schema_hash(&schema).expect("schema hash"),
                probes: Vec::new(),
                metrics: Vec::new(),
                probe_count: 0,
                metric_count: 0,
                per_class_probe_weight_total: PerClassWeightTotal::default(),
                per_class_metric_weight_total: PerClassWeightTotal::default(),
                per_class_total_weight: PerClassWeightTotal::default(),
            }),
            diagnostics: Vec::new(),
        }
    }

    fn failed_operational_probe_body(
        observation_hash: Option<Hash256>,
    ) -> OperationalProbeSchemaBody {
        OperationalProbeSchemaBody {
            input_identity: OperationalProbeSchemaInputIdentity {
                observation_plan_self_hash: observation_hash,
                infer_ir_self_hash: hash(1),
                quant_graph_self_hash: hash(2),
                determinism: DeterminismClassTag::BitExact,
                observability_mode: ObservabilityMode::Invariant,
                trace_budget: trace_budget(),
                profile_id: CompileProfileId::from("Bringup"),
                workload_id: WorkloadId::from("tiny"),
            },
            result: None,
            diagnostics: vec![hard_diagnostic()],
        }
    }

    #[test]
    fn observation_plan_report_body_serde_round_trip() {
        assert_body_round_trips(&observation_plan_body());
    }

    #[test]
    fn build_active_semantic_checkpoint_schema_re_emit_body_serde_round_trip() {
        assert_body_round_trips(&re_emit_body(Some(hash(20))));
    }

    #[test]
    fn operational_probe_schema_body_serde_round_trip() {
        assert_body_round_trips(&operational_probe_body(Some(hash(20))));
    }

    #[test]
    fn observation_plan_hash_deterministic_across_runs() {
        let product = plan_fixture();
        assert_eq!(
            observation_plan_self_hash(&product).expect("hash"),
            observation_plan_self_hash(&product).expect("hash")
        );
    }

    #[test]
    fn build_active_schema_hash_deterministic_across_runs() {
        let schema = BuildActiveCheckpointSchema {
            checkpoints: vec![re_emit_entry()],
            build_active_count: 1,
            mandatory_count: 1,
            optional_count: 0,
        };
        assert_eq!(
            build_active_checkpoint_schema_hash(&schema).expect("hash"),
            build_active_checkpoint_schema_hash(&schema).expect("hash")
        );
    }

    #[test]
    fn build_active_schema_hash_uses_checkpoint_projection_only() {
        let first = BuildActiveCheckpointSchema {
            checkpoints: vec![re_emit_entry()],
            build_active_count: 1,
            mandatory_count: 1,
            optional_count: 0,
        };
        let second = BuildActiveCheckpointSchema {
            checkpoints: first.checkpoints.clone(),
            build_active_count: 99,
            mandatory_count: 42,
            optional_count: 57,
        };
        assert_eq!(
            build_active_checkpoint_schema_hash(&first).expect("first hash"),
            build_active_checkpoint_schema_hash(&second).expect("second hash"),
            "SCRE-8 hashes only the public checkpoints projection"
        );
        assert_eq!(
            build_active_checkpoint_schema_hash(&first).expect("hash"),
            domain_hash(
                "BuildActiveCheckpointSchema",
                BUILD_ACTIVE_SEMANTIC_CHECKPOINT_SCHEMA_ID,
                first.checkpoints.as_slice(),
            )
            .expect("projection hash")
        );
    }

    #[test]
    fn operational_probe_schema_hash_deterministic_across_runs() {
        let schema = OperationalProbeSchema {
            probes: Vec::new(),
            metrics: Vec::new(),
            probe_count: 0,
            metric_count: 0,
            per_class_probe_weight_total: PerClassWeightTotal::default(),
            per_class_metric_weight_total: PerClassWeightTotal::default(),
            per_class_total_weight: PerClassWeightTotal::default(),
        };
        assert_eq!(
            operational_probe_schema_hash(&schema).expect("hash"),
            operational_probe_schema_hash(&schema).expect("hash")
        );
    }

    #[test]
    fn operational_probe_schema_hash_uses_probe_metric_projection_only() {
        let first = OperationalProbeSchema {
            probes: Vec::new(),
            metrics: Vec::new(),
            probe_count: 0,
            metric_count: 0,
            per_class_probe_weight_total: PerClassWeightTotal::default(),
            per_class_metric_weight_total: PerClassWeightTotal::default(),
            per_class_total_weight: PerClassWeightTotal::default(),
        };
        let second = OperationalProbeSchema {
            probes: first.probes.clone(),
            metrics: first.metrics.clone(),
            probe_count: 7,
            metric_count: 11,
            per_class_probe_weight_total: PerClassWeightTotal {
                required: 1,
                important: 2,
                diagnostic: 3,
                best_effort: 4,
            },
            per_class_metric_weight_total: PerClassWeightTotal {
                required: 5,
                important: 6,
                diagnostic: 7,
                best_effort: 8,
            },
            per_class_total_weight: PerClassWeightTotal {
                required: 6,
                important: 8,
                diagnostic: 10,
                best_effort: 12,
            },
        };
        assert_eq!(
            operational_probe_schema_hash(&first).expect("first hash"),
            operational_probe_schema_hash(&second).expect("second hash"),
            "OPS-8 hashes only the public probes/metrics projection"
        );
    }

    #[test]
    fn observation_plan_self_hash_round_trips() {
        let env = ReportEnvelope::new(ReportOutcome::Passed, observation_plan_body())
            .expect("envelope")
            .with_computed_self_hash()
            .expect("self hash");
        round_trip_self_hash(&env).expect("round trip");
    }

    #[test]
    fn build_active_schema_self_hash_round_trips() {
        let env = ReportEnvelope::new(ReportOutcome::Passed, re_emit_body(Some(hash(20))))
            .expect("envelope")
            .with_computed_self_hash()
            .expect("self hash");
        round_trip_self_hash(&env).expect("round trip");
    }

    #[test]
    fn operational_probe_schema_self_hash_round_trips() {
        let env = ReportEnvelope::new(
            ReportOutcome::Passed,
            operational_probe_body(Some(hash(20))),
        )
        .expect("envelope")
        .with_computed_self_hash()
        .expect("self hash");
        round_trip_self_hash(&env).expect("round trip");
    }

    #[test]
    fn semantic_anchor_table_uses_native_string_keyed_object_encoding() {
        let table = AnchorAttachmentTable {
            semantic: BTreeMap::from([
                (checkpoint("z.checkpoint"), semantic_attachment(2)),
                (checkpoint("a.checkpoint"), semantic_attachment(1)),
            ]),
            probes: BTreeMap::new(),
            metrics: BTreeMap::new(),
        };

        let value = serde_json::to_value(&table).expect("table serializes");
        assert!(value["semantic"].is_object());
        assert_eq!(
            value["semantic"]["a.checkpoint"],
            serde_json::json!(semantic_attachment(1))
        );
        assert_eq!(
            value["semantic"]["z.checkpoint"],
            serde_json::json!(semantic_attachment(2))
        );
    }

    #[test]
    fn canonical_map_for_each_observation_complex_key_type() {
        let table = plan_fixture().anchor_table;
        let value = serde_json::to_value(&table).expect("table serializes");
        assert!(value["semantic"].is_object());
        assert!(value["probes"].is_array());
        assert!(value["metrics"].is_array());
        let decoded: AnchorAttachmentTable = serde_json::from_value(value).expect("table decodes");
        assert_eq!(decoded, table);
    }

    #[test]
    fn sc_re_emit_passed_requires_observation_plan_self_hash() {
        let body = re_emit_body(None);
        let env = ReportEnvelope::new(ReportOutcome::Passed, body).expect("envelope");
        assert!(canonicalize(&env).is_err());
    }

    #[test]
    fn sc_re_emit_failed_allows_null_observation_plan_self_hash() {
        let body = failed_re_emit_body(None);
        let env = ReportEnvelope::new(ReportOutcome::Failed, body)
            .expect("envelope")
            .with_computed_self_hash()
            .expect("hash");
        canonicalize(&env).expect("failed null observation_plan_self_hash is allowed");
    }

    #[test]
    fn operational_probe_schema_passed_requires_observation_plan_self_hash() {
        let body = operational_probe_body(None);
        let env = ReportEnvelope::new(ReportOutcome::Passed, body).expect("envelope");
        assert!(canonicalize(&env).is_err());
    }

    #[test]
    fn operational_probe_schema_failed_allows_null_observation_plan_self_hash() {
        let body = failed_operational_probe_body(None);
        let env = ReportEnvelope::new(ReportOutcome::Failed, body)
            .expect("envelope")
            .with_computed_self_hash()
            .expect("hash");
        canonicalize(&env).expect("failed null observation_plan_self_hash is allowed");
    }

    #[test]
    fn r_no_report_hash_cycles_observation_plan_body_does_not_reference_own_envelope_hash() {
        let body = observation_plan_body();
        let value = serde_json::to_value(&body).expect("body serializes");
        assert!(value.get("report_self_hash").is_none());
        assert!(value.to_string().contains("observation_plan_self_hash"));
        assert!(
            !value
                .to_string()
                .contains("observation_plan_report_self_hash")
        );
    }

    #[test]
    fn cross_report_only_product_hashes_referenced() {
        let body = re_emit_body(Some(hash(20)));
        let value = serde_json::to_value(&body).expect("body serializes");
        assert_eq!(
            value["input_identity"].get("observation_plan_self_hash"),
            Some(&serde_json::json!(hash(20)))
        );
        assert!(
            value["input_identity"]
                .get("observation_plan_report_self_hash")
                .is_none()
        );
    }

    #[test]
    fn operational_probe_schema_references_product_hash_not_sibling_envelope_hash() {
        let body = operational_probe_body(Some(hash(20)));
        let value = serde_json::to_value(&body).expect("body serializes");
        assert_eq!(
            value["input_identity"].get("observation_plan_self_hash"),
            Some(&serde_json::json!(hash(20)))
        );
        assert!(
            value["input_identity"]
                .get("observation_plan_report_self_hash")
                .is_none()
        );
        assert!(value.get("sc_re_emit_report_self_hash").is_none());
        assert!(
            value
                .get("operational_probe_schema_report_self_hash")
                .is_none()
        );
    }

    #[test]
    fn r_no_partial_product_failed_observation_plan_rejects_result() {
        let mut body = observation_plan_body();
        body.diagnostics = vec![hard_diagnostic()];
        let env = ReportEnvelope::new(ReportOutcome::Failed, body).expect("envelope");
        assert!(canonicalize(&env).is_err());
    }
}
