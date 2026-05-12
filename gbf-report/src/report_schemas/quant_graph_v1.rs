//! `quant_graph.v1` Stage 1 report schema.

use std::collections::{BTreeMap, BTreeSet};

use gbf_foundation::{FieldPath, Hash256, LayerId};
use gbf_policy::{
    DiagnosticSeverity, ValidationCode, ValidationDetail, ValidationDiagnostic, ValidationOrigin,
};
use serde::{Deserialize, Serialize};

use crate::{ReportBody, ReportOutcome};

pub const SCHEMA_ID: &str = "quant_graph.v1";
pub const SCHEMA_VERSION: &str = "1.0.0";

pub type ValidationDiagnosticRecord = ValidationDiagnostic;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuantGraphReportBody<P = QuantGraphProduct> {
    pub input_identity: QuantGraphInputIdentity,
    pub result: Option<QuantGraphResult<P>>,
    pub diagnostics: Vec<ValidationDiagnosticRecord>,
}

impl<P> QuantGraphReportBody<P> {
    #[must_use]
    pub fn new(
        input_identity: QuantGraphInputIdentity,
        result: Option<QuantGraphResult<P>>,
        diagnostics: Vec<ValidationDiagnosticRecord>,
    ) -> Self {
        tracing::info!(schema = SCHEMA_ID, "stage1.envelope.bind");
        Self {
            input_identity,
            result,
            diagnostics,
        }
    }
}

impl<P> ReportBody for QuantGraphReportBody<P>
where
    P: Serialize + for<'de> Deserialize<'de> + Clone + PartialEq,
{
    const REPORT_TYPE: &'static str = "quant_graph";
    const SCHEMA_ID: &'static str = SCHEMA_ID;
    const SCHEMA_VERSION: &'static str = SCHEMA_VERSION;

    fn validate_semantics(&self, outcome: ReportOutcome) -> Result<(), Vec<ValidationDiagnostic>> {
        let mut errors = Vec::new();
        let has_hard = self
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Hard);

        for diagnostic in &self.diagnostics {
            if diagnostic.severity == DiagnosticSeverity::Soft {
                errors.push(semantic_error("diagnostics.soft"));
            }
        }

        match outcome {
            ReportOutcome::Passed => {
                if self.result.is_none() {
                    log_outcome_mismatch(outcome, false);
                    errors.push(semantic_error("result"));
                }
                if has_hard {
                    errors.push(semantic_error("diagnostics.hard"));
                }
            }
            ReportOutcome::Failed => {
                if self.result.is_some() {
                    log_outcome_mismatch(outcome, true);
                    errors.push(semantic_error("result"));
                }
                if !has_hard {
                    errors.push(semantic_error("diagnostics.hard"));
                }
            }
        }
        if let Some(result) = &self.result {
            validate_result_semantics(result, &self.input_identity, &mut errors);
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

fn validate_result_semantics<P>(
    result: &QuantGraphResult<P>,
    input_identity: &QuantGraphInputIdentity,
    errors: &mut Vec<ValidationDiagnostic>,
) {
    if result.tensor_count != result.tensor_summary.len() as u32 {
        errors.push(semantic_error("result.tensor_count"));
    }
    if !is_sorted_unique_by_key(&result.tensor_summary, |entry| entry.tensor_id) {
        errors.push(semantic_error("result.tensor_summary"));
    }
    if !is_sorted_unique_by_key(&result.provenance_summary, |entry| entry.tensor_id) {
        errors.push(semantic_error("result.provenance_summary"));
    }
    let tensor_ids = result
        .tensor_summary
        .iter()
        .map(|entry| entry.tensor_id)
        .collect::<BTreeSet<_>>();
    let mut export_ids = BTreeSet::new();
    for entry in &result.provenance_summary {
        if !tensor_ids.contains(&entry.tensor_id) {
            errors.push(semantic_error("result.provenance_summary.tensor_id"));
        }
        if !export_ids.insert(entry.export_tensor_id.as_str()) {
            errors.push(semantic_error("result.provenance_summary.export_tensor_id"));
        }
    }

    match input_identity.ffn_topology_kind {
        FfnTopologyKindTag::Dense if result.routing_layers_count != 0 => {
            errors.push(semantic_error("result.routing_layers_count"));
        }
        FfnTopologyKindTag::Routed if result.routing_layers_count == 0 => {
            errors.push(semantic_error("result.routing_layers_count"));
        }
        FfnTopologyKindTag::Mixed => {
            let has_dense = input_identity
                .model_spec_summary
                .ffn_kind
                .values()
                .any(|kind| matches!(kind, FfnKindTag::Dense));
            let has_routed = input_identity
                .model_spec_summary
                .ffn_kind
                .values()
                .any(|kind| matches!(kind, FfnKindTag::Routed));
            if result.routing_layers_count == 0 || !has_dense || !has_routed {
                errors.push(semantic_error("input_identity.ffn_topology_kind"));
            }
        }
        _ => {}
    }
}

fn is_sorted_unique_by_key<T, K: Ord>(values: &[T], key: impl Fn(&T) -> K) -> bool {
    values.windows(2).all(|pair| key(&pair[0]) < key(&pair[1]))
}

fn log_outcome_mismatch(outcome: ReportOutcome, result_present: bool) {
    tracing::error!(
        schema = SCHEMA_ID,
        code = "ReportSemanticInvariantViolated",
        semantic_invariant = "ReportOutcomeMismatch",
        ?outcome,
        result_present,
        "stage1.envelope.outcome_mismatch"
    );
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuantGraphInputIdentity {
    pub artifact_core_hash: Hash256,
    pub artifact_validation_self_hash: Hash256,
    pub policy_resolution_self_hash: Hash256,
    pub semantic_core_hash: Hash256,
    pub lowering_manifest_hash: Hash256,
    pub resolved_blob_index_hash: Hash256,
    pub determinism: DeterminismClassTag,
    pub model_spec_summary: ModelSpecSummary,
    pub sequence_semantics_kind: SequenceSemanticsKindTag,
    pub ffn_topology_kind: FfnTopologyKindTag,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum DeterminismClassTag {
    BitExact,
    Deterministic,
    Nondeterministic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum SequenceSemanticsKindTag {
    Identity,
    LinearState,
    BoundedKv,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum FfnTopologyKindTag {
    Dense,
    Routed,
    Mixed,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuantGraphResult<P = QuantGraphProduct> {
    pub product: P,
    pub tensor_count: u32,
    pub norm_plan_count: u16,
    pub layer_norm_count: u16,
    pub routing_layers_count: u16,
    pub expert_section_count: u32,
    pub classify_head_kind: ClassifyHeadKind,
    pub tensor_summary: Vec<TensorSummaryEntry>,
    pub provenance_summary: Vec<ProvenanceSummaryEntry>,
    pub decode_spec_summary: DecodeSpecSummary,
    pub sequence_semantics_summary: SequenceSemanticsSummary,
    pub classify_head_summary: ClassifyHeadSummary,
    pub quant_graph_self_hash: Hash256,
    pub quant_graph_canonical_bytes_hash: Hash256,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuantGraphProduct {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum FfnKindTag {
    Dense,
    Routed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ClassifyHeadKind {
    TiedEmbedding,
    Untied,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TensorSummaryEntry {
    pub tensor_id: u32,
    pub role: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProvenanceSummaryEntry {
    pub tensor_id: u32,
    pub export_tensor_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecodeSpecSummary {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SequenceSemanticsSummary {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClassifyHeadSummary {}

fn semantic_error(field: &'static str) -> ValidationDiagnostic {
    let field = FieldPath::from(field);
    ValidationDiagnostic {
        severity: DiagnosticSeverity::Hard,
        origin: ValidationOrigin::Schema,
        code: ValidationCode::ReportSemanticInvariantViolated {
            field: field.clone(),
        },
        detail: ValidationDetail::Field { field },
        provenance: Vec::new(),
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
