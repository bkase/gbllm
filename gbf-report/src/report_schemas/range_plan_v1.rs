//! Stage 5 range-plan and range certificate report schemas.

use std::collections::BTreeMap;

use gbf_foundation::{CompileProfileId, ExpertId, FieldPath, Hash256, LayerId};
use gbf_policy::{
    DiagnosticSeverity, PolicySource, RangeCapsSpec, ReductionPlanCeiling, ReductionSiteId,
    ValidationCode, ValidationDetail, ValidationDiagnostic, ValidationOrigin,
};
use serde::{Deserialize, Serialize};

use crate::report_schemas::f_b6_f_b7_common::{
    AccumulatorDomain, ExpertWeightSlot, NodeId, NormSite, QuantGraphEntityRef, ResidualSite,
};
use crate::report_schemas::quant_graph_v1::DeterminismClassTag;
use crate::{CanonicalJsonError, ReportBody, ReportOutcome, domain_hash};
use crate::{canonical_map, string_key_map};

pub const SCHEMA_ID: &str = "range_plan.v1";
pub const RANGE_CERT_SCHEMA_ID: &str = "range.cert.v1";
pub const SCHEMA_VERSION: &str = "1.0.0";

pub type ValidationDiagnosticRecord = ValidationDiagnostic;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RangePolicyProjection {
    pub profile_id: CompileProfileId,
    pub range_caps: RangeCapsSpec,
    pub reduction_ceiling: ReductionPlanCeiling,
    /// `ReductionSelector` is a transparent string newtype, so this uses the
    /// same duplicate-checking JSON object convention as `ReductionSiteId`
    /// maps below rather than CanonicalMap sorted-list encoding.
    #[serde(with = "string_key_map")]
    pub reduction_ceiling_overrides: BTreeMap<ReductionSelector, ReductionPlanCeiling>,
    pub determinism_class: DeterminismClassTag,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ReductionSelector(pub String);

impl ReductionSelector {
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

impl From<&str> for ReductionSelector {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for ReductionSelector {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RangePlanReportBody {
    pub input_identity: RangePlanReportInputIdentity,
    pub result: Option<RangePlanReportResult>,
    pub diagnostics: Vec<ValidationDiagnosticRecord>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RangePlanReportInputIdentity {
    pub infer_ir_self_hash: Hash256,
    pub quant_graph_self_hash: Hash256,
    pub static_budget_self_hash: Hash256,
    pub range_policy_projection_hash: Hash256,
    pub policy_resolution_self_hash: Hash256,
    pub compile_request_hash: Hash256,
    pub artifact_aux_hash: Hash256,
    pub determinism: DeterminismClassTag,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RangePlanReportResult {
    pub product: RangePlan,
    pub entry_count: u32,
    pub single_i16_count: u32,
    pub chunked_i16_count: u32,
    pub renorm_loop_count: u32,
    #[serde(with = "canonical_map")]
    pub effective_ceiling_histogram: BTreeMap<ReductionPlanCeiling, u32>,
    #[serde(with = "canonical_map")]
    pub ceiling_provenance_histogram: BTreeMap<ReductionCeilingProvenanceTag, u32>,
    pub range_cert_report_self_hash: Hash256,
    pub range_plan_self_hash: Hash256,
}

impl ReportBody for RangePlanReportBody {
    const REPORT_TYPE: &'static str = "RangePlanReport";
    const SCHEMA_ID: &'static str = SCHEMA_ID;
    const SCHEMA_VERSION: &'static str = SCHEMA_VERSION;

    fn validate_semantics(&self, outcome: ReportOutcome) -> Result<(), Vec<ValidationDiagnostic>> {
        validate_product_report_body(outcome, self.result.is_some(), &self.diagnostics)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RangePlan {
    pub identity: RangePlanIdentity,
    pub entries: Vec<RangePlanEntry>,
    pub provenance: RangePlanProvenance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RangePlanIdentity {
    pub infer_ir_self_hash: Hash256,
    pub quant_graph_self_hash: Hash256,
    pub static_budget_self_hash: Hash256,
    pub range_policy_projection_hash: Hash256,
    pub determinism: DeterminismClassTag,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RangePlanEntry {
    pub site: ReductionSiteId,
    pub plan: ReductionPlan,
    pub site_facts: ReductionSiteFacts,
    pub effective_ceiling: ReductionPlanCeiling,
    pub ceiling_provenance: ReductionCeilingProvenance,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReductionSiteFacts {
    pub site: ReductionSiteId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layer: Option<LayerId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expert: Option<ExpertId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slot: Option<ExpertWeightSlot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub norm_site: Option<NormSite>,
    pub term_count: u32,
    pub input_max_abs_q: u32,
    pub weight_max_abs_q: u32,
    pub per_term_abs_max_q: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bias_max_abs_q: Option<u32>,
    pub accumulator_domain: AccumulatorDomain,
    pub op_tag: gbf_policy::InferOpTag,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ReductionPlan {
    SingleI16,
    ChunkedI16 { chunk_len: u16 },
    RenormLoop { tile_len: u16, renorm: RenormSpec },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenormSpec {
    pub strategy: RenormStrategy,
    pub recurrence: RenormRecurrence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum RenormStrategy {
    ExactPostBoundary,
    DynamicMargin { margin_q16_16: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenormRecurrence {
    pub input_scale_q16_16: u32,
    pub output_scale_q16_16: u32,
    pub rounding: RenormRounding,
    pub saturation: RenormSaturationPolicy,
    pub max_rounding_error_q16_16: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum RenormRounding {
    TowardZero,
    NearestEven,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum RenormSaturationPolicy {
    Forbidden,
    AtNamedNumericBoundary { boundary: NamedNumericBoundary },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum NamedNumericBoundary {
    ResidualCombine {
        #[serde(skip_serializing_if = "Option::is_none")]
        layer: Option<LayerId>,
        site: ResidualSite,
    },
    ClassifyLogit,
    FfnActivationOutput {
        layer: LayerId,
        expert: ExpertId,
    },
    FinalClamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ReductionCeilingProvenance {
    Global {
        source: PolicySource,
    },
    LayerOverride {
        layer: LayerId,
        source: PolicySource,
    },
    SiteOverride {
        site: ReductionSiteId,
        source: PolicySource,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ReductionCeilingProvenanceTag {
    Global,
    LayerOverride,
    SiteOverride,
}

impl From<&ReductionCeilingProvenance> for ReductionCeilingProvenanceTag {
    fn from(value: &ReductionCeilingProvenance) -> Self {
        match value {
            ReductionCeilingProvenance::Global { .. } => Self::Global,
            ReductionCeilingProvenance::LayerOverride { .. } => Self::LayerOverride,
            ReductionCeilingProvenance::SiteOverride { .. } => Self::SiteOverride,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RangePlanProvenance {
    /// `ReductionSiteId` is a transparent string newtype, so these use native
    /// JSON object-key maps with duplicate-key rejection rather than
    /// CanonicalMap sorted-list encoding.
    #[serde(with = "string_key_map")]
    pub site_to_node: BTreeMap<ReductionSiteId, NodeId>,
    #[serde(with = "string_key_map")]
    pub site_to_qg: BTreeMap<ReductionSiteId, QuantGraphEntityRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RangeCertBody {
    pub identity: RangeCertIdentity,
    pub cert_outcome: CertOutcome,
    pub certificates: Vec<CertifiedReduction>,
    /// `ReductionSiteId` is a transparent string newtype; this uses the same
    /// duplicate-checking JSON object convention as other reduction-site maps.
    #[serde(with = "string_key_map")]
    pub site_to_certificate_index: BTreeMap<ReductionSiteId, u32>,
    pub diagnostics: Vec<ValidationDiagnosticRecord>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RangeCertIdentity {
    pub range_plan_self_hash: Option<Hash256>,
    pub infer_ir_self_hash: Hash256,
    pub quant_graph_self_hash: Hash256,
    pub static_budget_self_hash: Hash256,
    pub determinism: DeterminismClassTag,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum CertOutcome {
    Verified,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CertifiedReduction {
    pub site: ReductionSiteId,
    pub plan: ReductionPlan,
    pub facts: ReductionSiteFacts,
    pub proof: AccumulatorCertificate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum AccumulatorCertificate {
    SingleI16Proof {
        site: ReductionSiteId,
        term_count: u64,
        per_term_abs_max: u64,
        sum_bound: u64,
        bias_abs_max: u64,
        total_abs_max: u64,
        i16_envelope: u64,
        slack: u64,
    },
    ChunkedI16Proof {
        site: ReductionSiteId,
        chunk_len: u16,
        chunk_count: u64,
        per_term_abs_max: u64,
        per_chunk_sum_bound: u64,
        per_chunk_i16_slack: u64,
        cross_chunk_sum_bound: u64,
        bias_abs_max: u64,
        total_abs_max: u64,
        i32_envelope: u64,
        slack: u64,
    },
    RenormLoopProof {
        site: ReductionSiteId,
        tile_len: u16,
        tile_count: u64,
        per_term_abs_max: u64,
        per_tile_sum_bound: u64,
        per_tile_i16_slack: u64,
        renorm: RenormSpec,
        bias_abs_max: u64,
        total_abs_max: u64,
        slack: u64,
    },
    Failed {
        site: ReductionSiteId,
        attempted_plan: ReductionPlan,
        proof_state: AccumulatorProofState,
        witness: AccumulatorFailureWitness,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum AccumulatorProofState {
    SumExceedsI16Envelope {
        sum_bound: u64,
        envelope: u64,
    },
    PerChunkExceedsI16Envelope {
        per_chunk_sum_bound: u64,
        envelope: u64,
    },
    CrossChunkExceedsI32Envelope {
        cross_chunk_sum_bound: u64,
        envelope: u64,
    },
    PerTileExceedsI16Envelope {
        per_tile_sum_bound: u64,
        envelope: u64,
    },
    LengthZero {
        length_field: LengthField,
    },
    ChunkLenExceedsProfileMax {
        chunk_len: u16,
        profile_chunk_max: u16,
    },
    TileLenBelowProfileMin {
        tile_len: u16,
        profile_tile_min: u16,
    },
    TileLenExceedsProfileMax {
        tile_len: u16,
        profile_tile_max: u16,
    },
    BitExactRequiresChunkDivides {
        term_count: u32,
        chunk_len: u16,
    },
    TileLenExceedsU16 {
        term_count: u32,
    },
    DeterminismRequiresEnforcedRenorm,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum AccumulatorFailureWitness {
    BoundCalculation {
        input_max_abs_q: u32,
        weight_max_abs_q: u32,
        term_count: u32,
        bias: u32,
    },
    BitExactSaturationForbidden,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum LengthField {
    ChunkLen,
    TileLen,
}

impl ReportBody for RangeCertBody {
    const REPORT_TYPE: &'static str = "RangeCertBody";
    const SCHEMA_ID: &'static str = RANGE_CERT_SCHEMA_ID;
    const SCHEMA_VERSION: &'static str = SCHEMA_VERSION;

    fn validate_semantics(&self, outcome: ReportOutcome) -> Result<(), Vec<ValidationDiagnostic>> {
        let has_hard = has_hard_diagnostic(&self.diagnostics);
        let has_failed_certificate = self
            .certificates
            .iter()
            .any(|cert| matches!(cert.proof, AccumulatorCertificate::Failed { .. }));

        match outcome {
            ReportOutcome::Passed
                if self.identity.range_plan_self_hash.is_some()
                    && self.cert_outcome == CertOutcome::Verified
                    && !has_failed_certificate
                    && !has_hard =>
            {
                Ok(())
            }
            ReportOutcome::Failed
                if self.cert_outcome == CertOutcome::Failed
                    && (has_failed_certificate || has_hard) =>
            {
                Ok(())
            }
            _ => Err(vec![product_report_invariant_diagnostic("cert_outcome")]),
        }
    }
}

pub fn range_plan_self_hash(plan: &RangePlan) -> Result<Hash256, CanonicalJsonError> {
    domain_hash("RangePlan", SCHEMA_ID, plan)
}

pub fn range_cert_body_hash(body: &RangeCertBody) -> Result<Hash256, CanonicalJsonError> {
    domain_hash("RangeCertBody", RANGE_CERT_SCHEMA_ID, body)
}

fn validate_product_report_body(
    outcome: ReportOutcome,
    has_result: bool,
    diagnostics: &[ValidationDiagnostic],
) -> Result<(), Vec<ValidationDiagnostic>> {
    let has_hard = has_hard_diagnostic(diagnostics);

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

fn has_hard_diagnostic(diagnostics: &[ValidationDiagnostic]) -> bool {
    diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Hard)
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

    use crate::{ReportEnvelope, canonicalize, round_trip_self_hash};

    fn hash(byte: u8) -> Hash256 {
        Hash256::from_bytes([byte; 32])
    }

    fn hard_diagnostic() -> ValidationDiagnostic {
        product_report_invariant_diagnostic("result")
    }

    fn site(raw: &str) -> ReductionSiteId {
        ReductionSiteId(raw.to_owned())
    }

    fn facts(site: ReductionSiteId) -> ReductionSiteFacts {
        ReductionSiteFacts {
            site,
            layer: Some(LayerId::new(1)),
            expert: Some(ExpertId::new(2)),
            slot: Some(ExpertWeightSlot::FfnDown),
            norm_site: Some(NormSite::LayerFfn {
                layer: LayerId::new(1),
            }),
            term_count: 64,
            input_max_abs_q: 31,
            weight_max_abs_q: 17,
            per_term_abs_max_q: 527,
            bias_max_abs_q: Some(3),
            accumulator_domain: AccumulatorDomain::RawIntegerProducts,
            op_tag: gbf_policy::InferOpTag::ExpertMatVec,
        }
    }

    fn recurrence() -> RenormRecurrence {
        RenormRecurrence {
            input_scale_q16_16: 0x0001_0000,
            output_scale_q16_16: 0x0000_8000,
            rounding: RenormRounding::NearestEven,
            saturation: RenormSaturationPolicy::AtNamedNumericBoundary {
                boundary: NamedNumericBoundary::ResidualCombine {
                    layer: Some(LayerId::new(1)),
                    site: ResidualSite::PostFfn,
                },
            },
            max_rounding_error_q16_16: 1,
        }
    }

    fn entry(site_id: &str, plan: ReductionPlan) -> RangePlanEntry {
        let id = site(site_id);
        RangePlanEntry {
            site: id.clone(),
            plan,
            site_facts: facts(id.clone()),
            effective_ceiling: ReductionPlanCeiling::Adaptive,
            ceiling_provenance: ReductionCeilingProvenance::SiteOverride {
                site: id,
                source: PolicySource::CompileRequestOverride,
            },
        }
    }

    fn plan_fixture() -> RangePlan {
        RangePlan {
            identity: RangePlanIdentity {
                infer_ir_self_hash: hash(1),
                quant_graph_self_hash: hash(2),
                static_budget_self_hash: hash(3),
                range_policy_projection_hash: hash(4),
                determinism: DeterminismClassTag::BitExact,
            },
            entries: vec![
                entry("classify", ReductionPlan::SingleI16),
                entry(
                    "expert.1.2.down",
                    ReductionPlan::RenormLoop {
                        tile_len: 16,
                        renorm: RenormSpec {
                            strategy: RenormStrategy::ExactPostBoundary,
                            recurrence: recurrence(),
                        },
                    },
                ),
            ],
            provenance: RangePlanProvenance {
                site_to_node: BTreeMap::from([
                    (site("classify"), NodeId::new(7)),
                    (site("expert.1.2.down"), NodeId::new(5)),
                ]),
                site_to_qg: BTreeMap::from([
                    (site("classify"), QuantGraphEntityRef::ClassifyHead),
                    (
                        site("expert.1.2.down"),
                        QuantGraphEntityRef::ExpertTensor {
                            layer: LayerId::new(1),
                            expert: ExpertId::new(2),
                            slot: ExpertWeightSlot::FfnDown,
                            tensor: crate::report_schemas::f_b6_f_b7_common::TensorId::new(9),
                        },
                    ),
                ]),
            },
        }
    }

    fn cert_body_fixture() -> RangeCertBody {
        RangeCertBody {
            identity: RangeCertIdentity {
                range_plan_self_hash: Some(range_plan_self_hash(&plan_fixture()).expect("hash")),
                infer_ir_self_hash: hash(1),
                quant_graph_self_hash: hash(2),
                static_budget_self_hash: hash(3),
                determinism: DeterminismClassTag::BitExact,
            },
            cert_outcome: CertOutcome::Verified,
            certificates: vec![CertifiedReduction {
                site: site("classify"),
                plan: ReductionPlan::SingleI16,
                facts: facts(site("classify")),
                proof: AccumulatorCertificate::SingleI16Proof {
                    site: site("classify"),
                    term_count: 8,
                    per_term_abs_max: 256,
                    sum_bound: 2048,
                    bias_abs_max: 4,
                    total_abs_max: 2052,
                    i16_envelope: 32767,
                    slack: 30715,
                },
            }],
            site_to_certificate_index: BTreeMap::from([(site("classify"), 0)]),
            diagnostics: Vec::new(),
        }
    }

    fn failed_cert_body_fixture() -> RangeCertBody {
        RangeCertBody {
            identity: RangeCertIdentity {
                range_plan_self_hash: None,
                infer_ir_self_hash: hash(1),
                quant_graph_self_hash: hash(2),
                static_budget_self_hash: hash(3),
                determinism: DeterminismClassTag::BitExact,
            },
            cert_outcome: CertOutcome::Failed,
            certificates: vec![CertifiedReduction {
                site: site("classify"),
                plan: ReductionPlan::SingleI16,
                facts: facts(site("classify")),
                proof: AccumulatorCertificate::Failed {
                    site: site("classify"),
                    attempted_plan: ReductionPlan::SingleI16,
                    proof_state: AccumulatorProofState::SumExceedsI16Envelope {
                        sum_bound: 40_000,
                        envelope: 32_767,
                    },
                    witness: AccumulatorFailureWitness::BoundCalculation {
                        input_max_abs_q: 31,
                        weight_max_abs_q: 17,
                        term_count: 64,
                        bias: 3,
                    },
                },
            }],
            site_to_certificate_index: BTreeMap::from([(site("classify"), 0)]),
            diagnostics: Vec::new(),
        }
    }

    fn range_plan_body() -> RangePlanReportBody {
        let product = plan_fixture();
        RangePlanReportBody {
            input_identity: RangePlanReportInputIdentity {
                infer_ir_self_hash: hash(1),
                quant_graph_self_hash: hash(2),
                static_budget_self_hash: hash(3),
                range_policy_projection_hash: hash(4),
                policy_resolution_self_hash: hash(5),
                compile_request_hash: hash(6),
                artifact_aux_hash: hash(7),
                determinism: DeterminismClassTag::BitExact,
            },
            result: Some(RangePlanReportResult {
                entry_count: product.entries.len() as u32,
                single_i16_count: 1,
                chunked_i16_count: 0,
                renorm_loop_count: 1,
                effective_ceiling_histogram: BTreeMap::from([
                    (ReductionPlanCeiling::Adaptive, 1),
                    (ReductionPlanCeiling::Conservative, 1),
                    (ReductionPlanCeiling::ExactOnly, 0),
                ]),
                ceiling_provenance_histogram: BTreeMap::from([
                    (ReductionCeilingProvenanceTag::Global, 0),
                    (ReductionCeilingProvenanceTag::LayerOverride, 0),
                    (ReductionCeilingProvenanceTag::SiteOverride, 2),
                ]),
                range_cert_report_self_hash: hash(8),
                range_plan_self_hash: range_plan_self_hash(&product).expect("hash"),
                product,
            }),
            diagnostics: Vec::new(),
        }
    }

    fn assert_round_trip<T>(value: &T)
    where
        T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug,
    {
        let encoded = serde_json::to_vec(value).expect("serializes");
        let decoded: T = serde_json::from_slice(&encoded).expect("decodes");
        assert_eq!(&decoded, value);
    }

    #[test]
    fn range_plan_report_body_serde_round_trip() {
        assert_round_trip(&range_plan_body());
    }

    #[test]
    fn range_cert_body_serde_round_trip() {
        assert_round_trip(&cert_body_fixture());
        assert_round_trip(&failed_cert_body_fixture());
    }

    #[test]
    fn range_plan_hash_deterministic_across_runs() {
        let plan = plan_fixture();
        assert_eq!(
            range_plan_self_hash(&plan).expect("hash"),
            range_plan_self_hash(&plan).expect("hash")
        );
    }

    #[test]
    fn range_cert_body_hash_deterministic_across_runs() {
        let body = cert_body_fixture();
        assert_eq!(
            range_cert_body_hash(&body).expect("hash"),
            range_cert_body_hash(&body).expect("hash")
        );
    }

    #[test]
    fn range_plan_self_hash_round_trips() {
        let env = ReportEnvelope::new(ReportOutcome::Passed, range_plan_body())
            .expect("envelope")
            .with_computed_self_hash()
            .expect("self hash");
        round_trip_self_hash(&env).expect("round trip");
    }

    #[test]
    fn range_cert_body_self_hash_round_trips() {
        let env = ReportEnvelope::new(ReportOutcome::Passed, cert_body_fixture())
            .expect("envelope")
            .with_computed_self_hash()
            .expect("self hash");
        round_trip_self_hash(&env).expect("round trip");
    }

    #[test]
    fn report_envelope_outcome_failed_with_optional_hash_identity() {
        let env = ReportEnvelope::new(ReportOutcome::Failed, failed_cert_body_fixture())
            .expect("envelope")
            .with_computed_self_hash()
            .expect("self hash");
        canonicalize(&env).expect("failed cert with null range_plan_self_hash canonicalizes");
        round_trip_self_hash(&env).expect("round trip");
    }

    #[test]
    fn option_hash_field_serializes_as_null_when_none() {
        let value = serde_json::to_value(RangeCertIdentity {
            range_plan_self_hash: None,
            infer_ir_self_hash: hash(1),
            quant_graph_self_hash: hash(2),
            static_budget_self_hash: hash(3),
            determinism: DeterminismClassTag::BitExact,
        })
        .expect("identity serializes");
        assert_eq!(value["range_plan_self_hash"], serde_json::Value::Null);
    }

    #[test]
    fn option_hash_field_serializes_as_hex_when_some() {
        let value = serde_json::to_value(RangeCertIdentity {
            range_plan_self_hash: Some(hash(4)),
            infer_ir_self_hash: hash(1),
            quant_graph_self_hash: hash(2),
            static_budget_self_hash: hash(3),
            determinism: DeterminismClassTag::BitExact,
        })
        .expect("identity serializes");
        assert_eq!(value["range_plan_self_hash"], serde_json::json!(hash(4)));
    }

    #[test]
    fn range_cert_identity_range_plan_self_hash_is_optional() {
        let env = ReportEnvelope::new(ReportOutcome::Failed, failed_cert_body_fixture())
            .expect("envelope");
        canonicalize(&env).expect("null range_plan_self_hash is allowed");
    }

    #[test]
    fn canonical_map_for_each_range_complex_key_type() {
        let projection = RangePolicyProjection {
            profile_id: CompileProfileId::from("Bringup"),
            range_caps: RangeCapsSpec::default_v2(),
            reduction_ceiling: ReductionPlanCeiling::Adaptive,
            reduction_ceiling_overrides: BTreeMap::from([(
                ReductionSelector::from("site:classify"),
                ReductionPlanCeiling::Conservative,
            )]),
            determinism_class: DeterminismClassTag::BitExact,
        };
        let projection_value = serde_json::to_value(&projection).expect("projection serializes");
        assert!(projection_value["reduction_ceiling_overrides"].is_object());
        assert_eq!(
            projection_value["reduction_ceiling_overrides"]["site:classify"],
            serde_json::json!({"kind": "Conservative"})
        );

        let body_value = serde_json::to_value(range_plan_body()).expect("body serializes");
        assert!(body_value["result"]["effective_ceiling_histogram"].is_array());
        assert!(body_value["result"]["ceiling_provenance_histogram"].is_array());
        assert!(body_value["result"]["product"]["provenance"]["site_to_node"].is_object());
        assert!(body_value["result"]["product"]["provenance"]["site_to_qg"].is_object());

        let cert_value = serde_json::to_value(cert_body_fixture()).expect("cert serializes");
        assert!(cert_value["site_to_certificate_index"].is_object());
    }

    #[test]
    fn transparent_string_maps_reject_duplicate_json_object_keys() {
        #[derive(Debug, Deserialize)]
        #[allow(dead_code)]
        struct SelectorMapFixture {
            #[serde(with = "string_key_map")]
            map: BTreeMap<ReductionSelector, u32>,
        }

        let error = serde_json::from_str::<SelectorMapFixture>(
            r#"{"map":{"site:classify":1,"site:classify":2}}"#,
        )
        .expect_err("duplicate selector key must reject");
        assert!(
            error
                .to_string()
                .contains("duplicate string-key map key in JSON object"),
            "{error}"
        );
    }

    #[test]
    fn r_no_report_hash_cycles_range_cert_body_does_not_reference_own_envelope_hash() {
        let value = serde_json::to_value(cert_body_fixture()).expect("cert serializes");
        assert!(value.get("report_self_hash").is_none());
        assert!(value.to_string().contains("range_plan_self_hash"));
        assert!(!value.to_string().contains("range_cert_report_self_hash"));
    }

    #[test]
    fn r_no_partial_product_failed_range_plan_rejects_result() {
        let mut body = range_plan_body();
        body.diagnostics = vec![hard_diagnostic()];
        let env = ReportEnvelope::new(ReportOutcome::Failed, body).expect("envelope");
        assert!(canonicalize(&env).is_err());
    }
}
