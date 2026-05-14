//! `infer_ir.v1` Stage 3 report schema.

use std::collections::{BTreeMap, BTreeSet};

use gbf_foundation::{FieldPath, Hash256};
use gbf_policy::{
    DiagnosticSeverity, RuntimeMode, ValidationCode, ValidationDetail, ValidationDiagnostic,
    ValidationOrigin,
};
use serde::{Deserialize, Serialize};

use crate::report_schemas::quant_graph_v1::DeterminismClassTag;
use crate::{ReportBody, ReportOutcome};

pub const SCHEMA_ID: &str = "infer_ir.v1";
pub const SCHEMA_VERSION: &str = "1.0.0";

pub type ValidationDiagnosticRecord = ValidationDiagnostic;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InferIrReportBody<P = GbInferIr> {
    pub input_identity: InferIrInputIdentity,
    pub result: Option<InferIrResult<P>>,
    pub diagnostics: Vec<ValidationDiagnosticRecord>,
}

impl<P> InferIrReportBody<P> {
    #[must_use]
    pub fn new(
        input_identity: InferIrInputIdentity,
        result: Option<InferIrResult<P>>,
        diagnostics: Vec<ValidationDiagnosticRecord>,
    ) -> Self {
        tracing::info!(schema = SCHEMA_ID, "stage3.envelope.bind");
        tracing::debug!(
            schema = SCHEMA_ID,
            policy_resolution_self_hash = %input_identity.policy_resolution_self_hash,
            compile_request_hash = %input_identity.compile_request_hash,
            "stage3.envelope.audit_parents"
        );
        if let Some(result) = &result {
            tracing::debug!(
                schema = SCHEMA_ID,
                infer_ir_self_hash = %result.infer_ir_self_hash,
                "stage3.envelope.embedded_product_hash"
            );
        }
        Self {
            input_identity,
            result,
            diagnostics,
        }
    }
}

impl<P> ReportBody for InferIrReportBody<P>
where
    P: Serialize + for<'de> Deserialize<'de> + Clone + PartialEq,
{
    const REPORT_TYPE: &'static str = "infer_ir";
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
            validate_result_semantics(result, &mut errors);
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

fn validate_result_semantics<P>(result: &InferIrResult<P>, errors: &mut Vec<ValidationDiagnostic>) {
    if result.op_histogram.len() != INFER_OP_TAG_CANONICAL_ORDER.len()
        || !INFER_OP_TAG_CANONICAL_ORDER
            .iter()
            .all(|tag| result.op_histogram.contains_key(tag))
        || result.op_histogram.values().copied().sum::<u32>() != result.node_count
    {
        errors.push(semantic_error("result.op_histogram"));
    }
    if result.effect_class_histogram.len() != EFFECT_CLASS_TAG_CANONICAL_ORDER.len()
        || !EFFECT_CLASS_TAG_CANONICAL_ORDER
            .iter()
            .all(|tag| result.effect_class_histogram.contains_key(tag))
        || result.effect_class_histogram.values().copied().sum::<u16>() != result.effect_count
    {
        errors.push(semantic_error("result.effect_class_histogram"));
    }
    if result.value_kind_histogram.len() != VALUE_KIND_TAG_CANONICAL_ORDER.len()
        || !VALUE_KIND_TAG_CANONICAL_ORDER
            .iter()
            .all(|tag| result.value_kind_histogram.contains_key(tag))
        || result.value_kind_histogram.values().copied().sum::<u32>() != result.value_count
    {
        errors.push(semantic_error("result.value_kind_histogram"));
    }
}

fn log_outcome_mismatch(outcome: ReportOutcome, result_present: bool) {
    tracing::error!(
        schema = SCHEMA_ID,
        code = "ReportSemanticInvariantViolated",
        semantic_invariant = "ReportOutcomeMismatch",
        ?outcome,
        result_present,
        "stage3.envelope.outcome_mismatch"
    );
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InferIrInputIdentity {
    pub quant_graph_self_hash: Hash256,
    pub policy_resolution_self_hash: Hash256,
    pub compile_request_hash: Hash256,
    pub static_budget_self_hash: Hash256,
    pub requested_runtime_modes_hash: Hash256,
    pub determinism: DeterminismClassTag,
    pub requested_runtime_modes: BTreeSet<RuntimeMode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InferIrResult<P = GbInferIr> {
    pub product: P,
    pub node_count: u32,
    pub value_count: u32,
    pub effect_count: u16,
    pub token_input_count: u8,
    pub topological_order_hash: Hash256,
    pub op_histogram: BTreeMap<InferOpTag, u32>,
    pub effect_class_histogram: BTreeMap<EffectClassTag, u16>,
    pub value_kind_histogram: BTreeMap<ValueKindTag, u32>,
    pub anchor_count: u32,
    pub fixture_equivalence: FixtureEquivalenceTag,
    pub infer_ir_self_hash: Hash256,
    pub infer_ir_canonical_bytes_hash: Hash256,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GbInferIr {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ValueKindTag {
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

pub const VALUE_KIND_TAG_CANONICAL_ORDER: [ValueKindTag; 15] = [
    ValueKindTag::Activation,
    ValueKindTag::DecodedToken,
    ValueKindTag::EmbeddingOutput,
    ValueKindTag::ExpertCandidate,
    ValueKindTag::ExpertIntermediate,
    ValueKindTag::ExpertOutput,
    ValueKindTag::GateWeight,
    ValueKindTag::InputToken,
    ValueKindTag::LogitVector,
    ValueKindTag::NormalizedActivation,
    ValueKindTag::RouterDecision,
    ValueKindTag::RouterScore,
    ValueKindTag::SequenceBlockOutput,
    ValueKindTag::SequenceStateNext,
    ValueKindTag::SequenceStateRead,
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum FixtureEquivalenceTag {
    VerifiedFixtureBitExact,
    Skipped {
        reason: FixtureEquivalenceSkippedReason,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum FixtureEquivalenceSkippedReason {
    NonFixtureBuild,
    FeatureFlagDisabled,
    NonBitExactDeterminism,
}

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
