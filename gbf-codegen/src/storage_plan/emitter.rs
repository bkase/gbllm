//! `storage_plan.json` report body, emitter, and parser.

use std::collections::{BTreeMap, BTreeSet};
use std::{error::Error, fmt};

use gbf_policy::{
    DiagnosticSeverity, StoragePlanDiagnosticCode, StoragePlanDiagnosticProvenance, ValidationCode,
    ValidationDetail, ValidationDiagnostic, ValidationOrigin,
};
use gbf_report::{
    CanonicalJsonError as ReportCanonicalJsonError, ReportBody, ReportEnvelope,
    ReportEnvelopeError, ReportOutcome, ReportSelfHashError, canonicalize, round_trip_self_hash,
};
use serde::{Deserialize, Serialize};

use crate::s3::infer_ir::{NodeId, ValueId};
use crate::storage_plan::diagnostics::storage_plan_diagnostic;
use crate::storage_plan::driver::RepairProposal;
use crate::storage_plan::driver::{
    StoragePlanCoreDiagnosticDetail, StoragePlanCoreOutcome, StoragePlanCoreOutput,
    StoragePlanCoreResult, StoragePlanCoreSummary,
};
use crate::storage_plan::invariants::closed_spatial_surface_diagnostics;
use crate::storage_plan::types::{
    AbstractLiveRange, AliasClass, AliasClassFingerprint, AliasClassId, AliasIntent,
    BindingJustification, CommitAtomicityClass, CommitGroupDecl, CommitGroupId,
    CommitGroupProvenance, Materialization, PersistKind, PersistPageDecl, PersistPageId,
    PersistPageProvenance, STORAGE_PLAN_SCHEMA_ID, StorageBinding, StoragePlanInputIdentity,
    StorageProvenance,
};

pub type StoragePlanReportEnvelope = ReportEnvelope<StoragePlanReportEnvelopeBody>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoragePlanReportEnvelopeBody {
    pub body: StoragePlanReportBody,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoragePlanReportBody {
    pub outcome: ReportOutcome,
    pub result: Option<StoragePlanReportResult>,
    pub diagnostics: Vec<ValidationDiagnostic>,
    pub input_identity: StoragePlanInputIdentity,
    pub summary: Option<StoragePlanCoreSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoragePlanReportResult {
    pub input_identity: StoragePlanInputIdentity,
    pub bindings: Vec<SortedEntry<ValueId, StorageBindingJson>>,
    pub alias_classes: Vec<SortedEntry<AliasClassId, AliasClassJson>>,
    pub persist_pages: Vec<SortedEntry<PersistPageId, PersistPageDeclJson>>,
    pub commit_groups: Vec<SortedEntry<CommitGroupId, CommitGroupDeclJson>>,
    pub repair_proposals: Vec<RepairProposal>,
    pub provenance: StorageProvenanceJson,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SortedEntry<K, V> {
    pub key: K,
    pub value: V,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StorageBindingJson {
    pub value: ValueId,
    pub materialization: Materialization,
    pub alias_class: AliasClassId,
    pub live_range: AbstractLiveRangeJson,
    pub justification: BindingJustification,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AbstractLiveRangeJson {
    pub def_node: NodeId,
    pub first_use_node: Option<NodeId>,
    pub last_use_node: Option<NodeId>,
    pub lifetime_class: crate::storage_plan::types::LifetimeClass,
    pub checkpoint_stable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AliasClassJson {
    pub id: AliasClassId,
    pub fingerprint: AliasClassFingerprint,
    pub members: Vec<ValueId>,
    pub intent: AliasIntent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersistPageDeclJson {
    pub id: PersistPageId,
    pub kind: PersistKind,
    pub durability: crate::storage_plan::types::DurabilityClass,
    pub schema_pin: crate::storage_plan::types::PersistSchemaPin,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommitGroupDeclJson {
    pub id: CommitGroupId,
    pub members: Vec<PersistPageId>,
    pub kind_set: Vec<PersistKind>,
    pub atomicity: CommitAtomicityClass,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StorageProvenanceJson {
    pub bindings: Vec<SortedEntry<ValueId, crate::storage_plan::types::BindingProvenance>>,
    pub alias_classes:
        Vec<SortedEntry<AliasClassId, crate::storage_plan::types::AliasClassProvenance>>,
    pub persist_pages: Vec<SortedEntry<PersistPageId, PersistPageProvenance>>,
    pub commit_groups: Vec<SortedEntry<CommitGroupId, CommitGroupProvenance>>,
}

impl ReportBody for StoragePlanReportEnvelopeBody {
    const REPORT_TYPE: &'static str = "StoragePlan";
    const SCHEMA_ID: &'static str = STORAGE_PLAN_SCHEMA_ID;
    const SCHEMA_VERSION: &'static str = "1.0.0";

    fn validate_semantics(&self, outcome: ReportOutcome) -> Result<(), Vec<ValidationDiagnostic>> {
        validate_storage_plan_report_body(&self.body, outcome)
    }
}

pub fn emit_storage_plan_report(
    output: &StoragePlanCoreOutput,
) -> Result<StoragePlanReportEnvelope, StoragePlanEmitError> {
    let outcome = match output.outcome {
        StoragePlanCoreOutcome::Succeeded => ReportOutcome::Passed,
        StoragePlanCoreOutcome::Failed => ReportOutcome::Failed,
    };
    let body = StoragePlanReportEnvelopeBody {
        body: StoragePlanReportBody::from_core_output(output),
    };
    let envelope = ReportEnvelope::new(outcome, body)?;
    reject_forbidden_surface(&envelope)?;
    let envelope = envelope.with_computed_self_hash()?;
    reject_forbidden_surface(&envelope)?;
    round_trip_self_hash(&envelope)?;
    Ok(envelope)
}

pub fn emit_storage_plan_json_bytes(
    output: &StoragePlanCoreOutput,
) -> Result<Vec<u8>, StoragePlanEmitError> {
    let envelope = emit_storage_plan_report(output)?;
    Ok(canonicalize(&envelope)?)
}

pub fn parse_storage_plan_report_bytes(
    bytes: &[u8],
) -> Result<StoragePlanReportEnvelope, ReportSelfHashError> {
    serde_json::from_slice(bytes).map_err(|error| ReportSelfHashError::Json {
        reason: error.to_string(),
    })
}

impl StoragePlanReportBody {
    pub fn from_core_output(output: &StoragePlanCoreOutput) -> Self {
        let outcome = match output.outcome {
            StoragePlanCoreOutcome::Succeeded => ReportOutcome::Passed,
            StoragePlanCoreOutcome::Failed => ReportOutcome::Failed,
        };
        Self {
            outcome,
            result: output.result.as_ref().map(|result| {
                StoragePlanReportResult::from_core_result(&output.input_identity, result)
            }),
            diagnostics: diagnostics_from_core_output(output),
            input_identity: output.input_identity.clone(),
            summary: output.summary,
        }
    }
}

fn diagnostics_from_core_output(output: &StoragePlanCoreOutput) -> Vec<ValidationDiagnostic> {
    if output.diagnostic_details.is_empty() {
        output
            .diagnostics
            .iter()
            .copied()
            .map(diagnostic_for_code)
            .collect()
    } else {
        output
            .diagnostic_details
            .iter()
            .map(diagnostic_for_detail)
            .collect()
    }
}

fn diagnostic_for_detail(detail: &StoragePlanCoreDiagnosticDetail) -> ValidationDiagnostic {
    storage_plan_diagnostic(
        detail.code,
        detail.provenance.clone(),
        detail.evidence.clone(),
    )
    .unwrap_or_else(|_| report_invariant("diagnostics"))
}

impl StoragePlanReportResult {
    pub fn from_core_result(
        input_identity: &StoragePlanInputIdentity,
        result: &StoragePlanCoreResult,
    ) -> Self {
        Self {
            input_identity: input_identity.clone(),
            bindings: sorted_entries(&result.bindings, StorageBindingJson::from_binding),
            alias_classes: sorted_entries(&result.alias_classes, AliasClassJson::from_alias_class),
            persist_pages: sorted_entries(&result.persist_pages, PersistPageDeclJson::from_decl),
            commit_groups: sorted_entries(&result.commit_groups, CommitGroupDeclJson::from_decl),
            repair_proposals: result.repair_proposals.clone(),
            provenance: StorageProvenanceJson::from_provenance(&result.provenance),
        }
    }
}

impl StorageBindingJson {
    fn from_binding(binding: &StorageBinding) -> Self {
        Self {
            value: binding.value,
            materialization: binding.materialization.clone(),
            alias_class: binding.alias_class,
            live_range: AbstractLiveRangeJson::from_live_range(&binding.live_range),
            justification: binding.justification.clone(),
        }
    }
}

impl AbstractLiveRangeJson {
    fn from_live_range(range: &AbstractLiveRange) -> Self {
        Self {
            def_node: range.def_node,
            first_use_node: range.first_use_node,
            last_use_node: range.last_use_node,
            lifetime_class: range.lifetime_class.clone(),
            checkpoint_stable: range.checkpoint_stable,
        }
    }
}

impl AliasClassJson {
    fn from_alias_class(class: &AliasClass) -> Self {
        Self {
            id: *class.id(),
            fingerprint: class.fingerprint(),
            members: class.members().iter().copied().collect(),
            intent: class.intent(),
        }
    }
}

impl PersistPageDeclJson {
    fn from_decl(page: &PersistPageDecl) -> Self {
        Self {
            id: page.id,
            kind: page.kind,
            durability: page.durability,
            schema_pin: page.schema_pin,
        }
    }
}

impl CommitGroupDeclJson {
    fn from_decl(group: &CommitGroupDecl) -> Self {
        Self {
            id: group.id,
            members: sorted_set_values(group.members.as_btree_set()),
            kind_set: sorted_set_values(&group.kind_set),
            atomicity: group.atomicity.clone(),
        }
    }
}

impl StorageProvenanceJson {
    fn from_provenance(provenance: &StorageProvenance) -> Self {
        Self {
            bindings: sorted_entries(&provenance.bindings, Clone::clone),
            alias_classes: sorted_entries(&provenance.alias_classes, Clone::clone),
            persist_pages: sorted_entries(&provenance.persist_pages, Clone::clone),
            commit_groups: sorted_entries(&provenance.commit_groups, Clone::clone),
        }
    }
}

fn sorted_entries<K, V, J>(
    map: &BTreeMap<K, V>,
    convert: impl Fn(&V) -> J,
) -> Vec<SortedEntry<K, J>>
where
    K: Copy + Ord,
{
    map.iter()
        .map(|(key, value)| SortedEntry {
            key: *key,
            value: convert(value),
        })
        .collect()
}

fn sorted_set_values<T>(set: &BTreeSet<T>) -> Vec<T>
where
    T: Copy + Ord,
{
    set.iter().copied().collect()
}

fn validate_storage_plan_report_body(
    body: &StoragePlanReportBody,
    envelope_outcome: ReportOutcome,
) -> Result<(), Vec<ValidationDiagnostic>> {
    let mut diagnostics = Vec::new();
    if body.outcome != envelope_outcome {
        diagnostics.push(report_invariant("body.outcome"));
    }

    match envelope_outcome {
        ReportOutcome::Passed => {
            if body.result.is_none() {
                diagnostics.push(report_invariant("body.result"));
            }
            if !body.diagnostics.is_empty() {
                diagnostics.push(report_invariant("body.diagnostics"));
            }
            if body.summary.is_none() {
                diagnostics.push(report_invariant("body.summary"));
            }
            if let Some(result) = &body.result
                && result.input_identity != body.input_identity
            {
                diagnostics.push(report_invariant("body.result.input_identity"));
            }
        }
        ReportOutcome::Failed => {
            if body.result.is_some() {
                diagnostics.push(report_invariant("body.result"));
            }
            if body.summary.is_some() {
                diagnostics.push(report_invariant("body.summary"));
            }
            if body.diagnostics.is_empty()
                || body
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.severity != DiagnosticSeverity::Hard)
            {
                diagnostics.push(report_invariant("body.diagnostics"));
            }
        }
    }

    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(diagnostics)
    }
}

fn diagnostic_for_code(code: StoragePlanDiagnosticCode) -> ValidationDiagnostic {
    let provenance = match code {
        StoragePlanDiagnosticCode::StorageNoAdmittingDecisionRule => {
            StoragePlanDiagnosticProvenance::ValueClassification {
                value_id: 0,
                producer_node: Some(0),
                value_role: Some("Unknown".to_owned()),
                value_format: Some("Unknown".to_owned()),
            }
        }
        StoragePlanDiagnosticCode::StorageBindingCoverageGap => {
            StoragePlanDiagnosticProvenance::ValueProducer {
                value_id: 0,
                producer_node: 0,
            }
        }
        StoragePlanDiagnosticCode::StorageForbiddenSpatialEnumLeak
        | StoragePlanDiagnosticCode::StorageReservedShapeEmitted => {
            StoragePlanDiagnosticProvenance::JsonPath {
                json_path: "$".to_owned(),
                field_or_tag: code.name().to_owned(),
            }
        }
        StoragePlanDiagnosticCode::StoragePersistBindingKindMismatch => {
            StoragePlanDiagnosticProvenance::PersistBinding {
                value_id: 0,
                persist_page_id: 0,
                commit_group_id: 0,
                persist_kind: "Unknown".to_owned(),
                expected: "consistent persist page kind".to_owned(),
            }
        }
        StoragePlanDiagnosticCode::StorageCommitGroupKindMix => {
            StoragePlanDiagnosticProvenance::CommitGroupKind {
                commit_group_id: 0,
                kinds: vec!["Unknown".to_owned()],
                allowed_table: "persist_commit_group_kind_sets.v1".to_owned(),
            }
        }
        StoragePlanDiagnosticCode::StorageAliasMixedIntentComponent => {
            StoragePlanDiagnosticProvenance::AliasMixedIntent {
                members: Vec::new(),
                edge_count: 0,
                intents: Vec::new(),
            }
        }
        StoragePlanDiagnosticCode::StorageAliasIntentCardinalityViolation => {
            StoragePlanDiagnosticProvenance::AliasCardinality {
                alias_class_id: 0,
                intent: "Unknown".to_owned(),
                members: Vec::new(),
            }
        }
        _ => return report_invariant(code.as_str()),
    };
    storage_plan_diagnostic(code, provenance, vec![])
        .unwrap_or_else(|_| report_invariant("diagnostics"))
}

fn reject_forbidden_surface(
    envelope: &StoragePlanReportEnvelope,
) -> Result<(), StoragePlanEmitError> {
    let mut value = serde_json::to_value(envelope)?;
    // SC11 guards the public StoragePlan product surface. Diagnostics must be
    // allowed to name the forbidden key or tag that caused STORE-018.
    if let Some(body) = value
        .get_mut("body")
        .and_then(serde_json::Value::as_object_mut)
    {
        body.insert(
            "diagnostics".to_owned(),
            serde_json::Value::Array(Vec::new()),
        );
        if let Some(nested_body) = body
            .get_mut("body")
            .and_then(serde_json::Value::as_object_mut)
        {
            nested_body.insert(
                "diagnostics".to_owned(),
                serde_json::Value::Array(Vec::new()),
            );
        }
    }
    let diagnostics = closed_spatial_surface_diagnostics(&value);
    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(StoragePlanEmitError::ForbiddenSurface { diagnostics })
    }
}

fn report_invariant(field: &'static str) -> ValidationDiagnostic {
    ValidationDiagnostic::hard(
        ValidationOrigin::StoragePlanConstruction,
        ValidationCode::ReportSemanticInvariantViolated {
            field: field.into(),
        },
        ValidationDetail::Field {
            field: field.into(),
        },
        vec![],
    )
}

#[derive(Debug)]
pub enum StoragePlanEmitError {
    Envelope(ReportEnvelopeError),
    SelfHash(ReportSelfHashError),
    CanonicalJson(ReportCanonicalJsonError),
    Json(serde_json::Error),
    ForbiddenSurface {
        diagnostics: Vec<ValidationDiagnostic>,
    },
}

impl fmt::Display for StoragePlanEmitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Envelope(error) => write!(f, "{error}"),
            Self::SelfHash(error) => write!(f, "{error}"),
            Self::CanonicalJson(error) => write!(f, "{error}"),
            Self::Json(error) => write!(f, "{error}"),
            Self::ForbiddenSurface { diagnostics } => {
                write!(f, "storage_plan.json forbidden surface: {diagnostics:?}")
            }
        }
    }
}

impl Error for StoragePlanEmitError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Envelope(error) => Some(error),
            Self::SelfHash(error) => Some(error),
            Self::CanonicalJson(error) => Some(error),
            Self::Json(error) => Some(error),
            Self::ForbiddenSurface { .. } => None,
        }
    }
}

impl From<ReportEnvelopeError> for StoragePlanEmitError {
    fn from(value: ReportEnvelopeError) -> Self {
        Self::Envelope(value)
    }
}

impl From<ReportSelfHashError> for StoragePlanEmitError {
    fn from(value: ReportSelfHashError) -> Self {
        Self::SelfHash(value)
    }
}

impl From<ReportCanonicalJsonError> for StoragePlanEmitError {
    fn from(value: ReportCanonicalJsonError) -> Self {
        Self::CanonicalJson(value)
    }
}

impl From<serde_json::Error> for StoragePlanEmitError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use gbf_foundation::{Hash256, SemVer};
    use gbf_policy::StorageMaterialization;
    use gbf_report::ReportSchemaId;
    use serde_json::Value;

    use super::*;
    use crate::s1::quant_graph::DeterminismClass;
    use crate::storage_plan::driver::{
        KnobDelta, PlanningStage, RepairReason, StoragePlanRepairPolicy, build_storage_plan_core,
    };
    use crate::storage_plan::predicates::{
        PredicateEnv, PredicateValueFacts, QuantFormatId, ValueFormat, ValueRole,
    };
    use crate::storage_plan::types::{
        LifetimeClass, PersistKind, StorageClass, StoragePlanInputHashes,
    };
    use crate::storage_plan::{CommitGroupReason, StoragePlanCoreInput, StoragePlanCoreValue};

    #[test]
    fn storage_plan_json_round_trips_byte_identically() {
        let output = build_storage_plan_core(&fixture_input(false));
        let bytes = emit_storage_plan_json_bytes(&output).expect("emit succeeds");
        let parsed = parse_storage_plan_report_bytes(&bytes).expect("parse succeeds");
        let reparsed_bytes = canonicalize(&parsed).expect("canonicalizes");

        assert_eq!(bytes, reparsed_bytes);
        assert_eq!(
            parsed.body.body.result,
            emit_storage_plan_report(&output).unwrap().body.body.result
        );
    }

    #[test]
    fn result_input_identity_matches_body_input_identity() {
        let output = build_storage_plan_core(&fixture_input(false));
        let report = emit_storage_plan_report(&output).expect("emit succeeds");
        let body = &report.body.body;

        assert_eq!(
            body.result.as_ref().expect("result").input_identity,
            body.input_identity
        );
    }

    #[test]
    fn failure_report_has_null_result_null_summary_nonempty_diagnostics_and_hash() {
        let output = build_storage_plan_core(&fixture_input(true));
        let bytes = emit_storage_plan_json_bytes(&output).expect("failure emits");
        let parsed = parse_storage_plan_report_bytes(&bytes).expect("failure parses");
        let body = &parsed.body.body;

        assert_eq!(parsed.outcome, ReportOutcome::Failed);
        assert!(body.result.is_none());
        assert!(body.summary.is_none());
        assert!(!body.diagnostics.is_empty());
        round_trip_self_hash(&parsed).expect("self hash round trip");
    }

    #[test]
    fn repair_proposals_round_trip_when_non_empty() {
        let mut input = fixture_input(false);
        input.repair_policy = StoragePlanRepairPolicy {
            soft_pressure_threshold_bytes: Some(1),
            recompute_promotion: StorageMaterialization::PreserveAll,
            max_recompute_promotion: StorageMaterialization::RecomputePureValues,
            storage_recompute_promotion_locked: false,
        };
        let env = std::mem::take(&mut input.predicate_env)
            .with_recompute_cycle_ceiling(16)
            .with_recompute_cost_estimate(ValueId::new(1), 3)
            .with_recompute_cost_estimate(ValueId::new(4), 5);
        input.predicate_env = env;

        let output = build_storage_plan_core(&input);
        let bytes = emit_storage_plan_json_bytes(&output).expect("emit succeeds");
        let parsed = parse_storage_plan_report_bytes(&bytes).expect("parse succeeds");
        let proposal = &parsed
            .body
            .body
            .result
            .as_ref()
            .expect("result")
            .repair_proposals[0];

        assert_eq!(proposal.source, PlanningStage::StoragePlan);
        assert_eq!(proposal.reason, RepairReason::PromoteRecompute);
        assert!(matches!(
            proposal.tighten.changes.as_slice(),
            [KnobDelta::PromoteRecomputeLevel {
                to: StorageMaterialization::RecomputePureValues
            }]
        ));
        assert_eq!(canonicalize(&parsed).expect("canonicalizes"), bytes);
    }

    #[test]
    fn sc11_emit_scan_allows_legal_storage_tags_and_forbids_closed_keys() {
        let output = build_storage_plan_core(&fixture_input(false));
        let report = emit_storage_plan_report(&output).expect("emit succeeds");
        let value = serde_json::to_value(&report).expect("report serializes");

        assert!(closed_spatial_surface_diagnostics(&value).is_empty());
        assert!(value.to_string().contains("WramHot"));
        assert!(value.to_string().contains("SramPaged"));
        assert!(value.to_string().contains("RomConst"));
        let mut illegal = serde_json::json!({"body": {}});
        illegal["body"][closed_key(&["byte_", "offset"])] = serde_json::json!(0);
        assert!(!closed_spatial_surface_diagnostics(&illegal).is_empty());
    }

    #[test]
    fn parser_rejects_unknown_fields() {
        let output = build_storage_plan_core(&fixture_input(false));
        let bytes = emit_storage_plan_json_bytes(&output).expect("emit succeeds");
        let mut value: Value = serde_json::from_slice(&bytes).expect("json value");
        value["body"]["unknown_field"] = serde_json::json!(true);

        assert!(serde_json::from_value::<StoragePlanReportEnvelope>(value).is_err());
    }

    #[test]
    fn report_uses_sorted_entry_vectors_for_keyed_collections() {
        let output = build_storage_plan_core(&fixture_input(false));
        let report = emit_storage_plan_report(&output).expect("emit succeeds");
        let value = serde_json::to_value(&report).expect("report serializes");

        assert!(value["body"]["result"]["bindings"].is_array());
        assert!(value["body"]["result"]["provenance"]["bindings"].is_array());
        assert!(value["body"]["result"]["bindings"][0].get("key").is_some());
        assert!(
            value["body"]["result"]["bindings"][0]
                .get("value")
                .is_some()
        );
    }

    fn fixture_input(fail_before_result: bool) -> StoragePlanCoreInput {
        let mut env = PredicateEnv::new().with_wram_hot_per_value_eligibility_ceiling(32);
        for (value, facts) in [
            (1, facts(ValueRole::Activation)),
            (2, facts(ValueRole::ExpertWeight)),
            (3, facts(ValueRole::OutputToken)),
            (4, facts(ValueRole::Activation)),
        ] {
            env = env.with_value(ValueId::new(value), facts);
        }
        StoragePlanCoreInput {
            input_identity: identity(),
            expected_input_hashes: input_hashes(),
            repair_policy: Default::default(),
            predicate_env: env,
            topological_order: (1..=4).map(NodeId::new).collect(),
            values: vec![
                StoragePlanCoreValue {
                    value: ValueId::new(1),
                    materialization: Materialization::Materialize {
                        class: StorageClass::WramHot,
                        lifetime: LifetimeClass::Slice,
                    },
                    live_range: live_range(1),
                    role: ValueRole::Activation,
                    persist_kind: None,
                    commit_group_reason: None,
                },
                StoragePlanCoreValue {
                    value: ValueId::new(2),
                    materialization: Materialization::Materialize {
                        class: StorageClass::RomConst,
                        lifetime: LifetimeClass::Persistent,
                    },
                    live_range: live_range(2),
                    role: ValueRole::ExpertWeight,
                    persist_kind: None,
                    commit_group_reason: None,
                },
                StoragePlanCoreValue {
                    value: ValueId::new(3),
                    materialization: Materialization::Persist {
                        page: PersistPageId(1),
                        commit_group: CommitGroupId(1),
                    },
                    live_range: live_range(3),
                    role: ValueRole::OutputToken,
                    persist_kind: Some(PersistKind::Trace),
                    commit_group_reason: Some(CommitGroupReason::Independent),
                },
                StoragePlanCoreValue {
                    value: ValueId::new(4),
                    materialization: Materialization::Materialize {
                        class: StorageClass::SramPaged,
                        lifetime: LifetimeClass::Slice,
                    },
                    live_range: live_range(4),
                    role: ValueRole::Activation,
                    persist_kind: None,
                    commit_group_reason: None,
                },
            ],
            alias_edges: vec![],
            alias_forced_recompute_values: BTreeSet::new(),
            fail_before_result,
        }
    }

    fn facts(role: ValueRole) -> PredicateValueFacts {
        let format = match role {
            ValueRole::ExpertWeight => ValueFormat::ConstTensorRef {
                tensor_id: crate::s1::quant_graph::TensorId::new(1),
            },
            _ => ValueFormat::QuantInt {
                quant_format_id: QuantFormatId(1),
            },
        };
        let mut facts = PredicateValueFacts::new(role, format);
        facts.logical_size = Some(4);
        facts
    }

    fn live_range(value: u32) -> AbstractLiveRange {
        AbstractLiveRange {
            def_node: NodeId::new(value),
            first_use_node: Some(NodeId::new(value + 10)),
            last_use_node: Some(NodeId::new(value + 10)),
            lifetime_class: LifetimeClass::Slice,
            checkpoint_stable: false,
        }
    }

    fn identity() -> StoragePlanInputIdentity {
        StoragePlanInputIdentity {
            quant_graph_hash: hash(1),
            infer_ir_hash: hash(2),
            observation_plan_hash: hash(3),
            range_plan_hash: hash(4),
            policy_hash: hash(5),
            determinism: DeterminismClass::Deterministic,
            schema: ReportSchemaId::from(STORAGE_PLAN_SCHEMA_ID),
            schema_version: SemVer::new(1, 0, 0),
        }
    }

    fn input_hashes() -> StoragePlanInputHashes {
        StoragePlanInputHashes {
            quant_graph_hash: hash(1),
            infer_ir_hash: hash(2),
            observation_plan_hash: hash(3),
            range_plan_hash: hash(4),
            policy_hash: hash(5),
        }
    }

    fn hash(byte: u8) -> Hash256 {
        Hash256::from_bytes([byte; 32])
    }

    fn closed_key(parts: &[&str]) -> String {
        parts.concat()
    }
}
