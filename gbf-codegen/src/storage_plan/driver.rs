//! Pure Stage 6 construction-order driver over normalized storage facts.

use std::collections::{BTreeMap, BTreeSet};

use gbf_foundation::{CanonicalJson, CanonicalJsonError, EvidenceRef, Hash256};
use gbf_policy::{StoragePlanDiagnosticCode, StoragePlanDiagnosticProvenance};
use serde::{Deserialize, Serialize};

use crate::s3::infer_ir::{NodeId, ValueId};
use crate::storage_plan::alias_engine::{
    AliasCandidateEdge, AliasSeedBinding, build_alias_classes,
};
use crate::storage_plan::invariants::{
    StoragePlanConsistencyContext, StoragePlanConsistencyView,
    validate_storage_plan_self_consistency,
};
use crate::storage_plan::lifetime::{
    LifetimeBound, LifetimeBoundSource, LifetimeBounds, lifetime_bounds,
};
use crate::storage_plan::persist::{PersistBindingInput, resolve_persist_bindings};
use crate::storage_plan::predicates::{
    PredicateEnv, RecomputeAllowedFailurePredicate, ValueRole, recompute_allowed,
    recompute_allowed_failure_predicates, sequence_state_slot_ref, value_format_of,
};
use crate::storage_plan::types::{
    AbstractLiveRange, AdmittingPredicateId, AliasClass, AliasClassFingerprint, AliasClassId,
    AliasClassProvenance, AliasIntent, BindingJustification, BindingProvenance, CommitGroupDecl,
    CommitGroupId, CommitGroupProvenance, CommitGroupReason, DecisionRuleId, LifetimeClass,
    Materialization, PersistKind, PersistPageDecl, PersistPageId, PersistPageProvenance,
    PersistPageSource, StorageBinding, StoragePlanInputHashes, StoragePlanInputIdentity,
    StorageProvenance,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoragePlanCoreInput {
    pub input_identity: StoragePlanInputIdentity,
    pub predicate_env: PredicateEnv,
    pub topological_order: Vec<NodeId>,
    pub values: Vec<StoragePlanCoreValue>,
    pub alias_edges: Vec<AliasCandidateEdge>,
    pub alias_forced_recompute_values: BTreeSet<ValueId>,
    pub fail_before_result: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoragePlanCoreValue {
    pub value: ValueId,
    pub materialization: Materialization,
    pub live_range: AbstractLiveRange,
    pub role: ValueRole,
    pub persist_kind: Option<PersistKind>,
    pub commit_group_reason: Option<CommitGroupReason>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoragePlanCoreOutput {
    pub input_identity: StoragePlanInputIdentity,
    pub outcome: StoragePlanCoreOutcome,
    pub result: Option<StoragePlanCoreResult>,
    pub summary: Option<StoragePlanCoreSummary>,
    pub diagnostics: Vec<StoragePlanDiagnosticCode>,
    pub diagnostic_details: Vec<StoragePlanCoreDiagnosticDetail>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum StoragePlanCoreOutcome {
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoragePlanCoreResult {
    pub bindings: BTreeMap<ValueId, StorageBinding>,
    pub alias_classes: BTreeMap<AliasClassId, AliasClass>,
    pub persist_pages: BTreeMap<PersistPageId, PersistPageDecl>,
    pub commit_groups: BTreeMap<CommitGroupId, CommitGroupDecl>,
    pub provenance: StorageProvenance,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StoragePlanCoreDiagnosticDetail {
    pub code: StoragePlanDiagnosticCode,
    pub provenance: StoragePlanDiagnosticProvenance,
    pub evidence: Vec<EvidenceRef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoragePlanCoreSummary {
    pub bindings: u32,
    pub alias_classes: u32,
    pub persist_pages: u32,
    pub commit_groups: u32,
}

pub fn build_storage_plan_core(input: &StoragePlanCoreInput) -> StoragePlanCoreOutput {
    if input.fail_before_result {
        return failed_output(
            input.input_identity.clone(),
            StoragePlanDiagnosticCode::StorageNoAdmittingDecisionRule,
        );
    }

    let mut bindings = tentative_bindings(&input.values);
    let persist_inputs = persist_inputs(&input.values);
    let persist_resolution = match resolve_persist_bindings(&persist_inputs) {
        Ok(resolution) => resolution,
        Err(error) => return failed_output(input.input_identity.clone(), error.code),
    };

    let mut alias_classes = match construct_alias_classes(input, &bindings) {
        Ok(classes) => classes,
        Err(code) => return failed_output(input.input_identity.clone(), code),
    };
    assign_alias_class_ids(&mut bindings, &alias_classes);

    if !input.alias_forced_recompute_values.is_empty() {
        for value in &input.alias_forced_recompute_values {
            if !recompute_allowed(&input.predicate_env, *value) {
                return failed_output_with_detail(
                    input.input_identity.clone(),
                    forced_recompute_rejection_detail(input, *value),
                );
            }
            if let Some(binding) = bindings.get_mut(value) {
                binding.materialization = Materialization::Recompute;
                binding.justification = BindingJustification::ForcedRecompute;
            }
        }
        alias_classes = match construct_alias_classes_after_forced_recompute(input, &bindings) {
            Ok(classes) => classes,
            Err(code) => return failed_output(input.input_identity.clone(), code),
        };
        assign_alias_class_ids(&mut bindings, &alias_classes);
    }

    let mut result = StoragePlanCoreResult {
        bindings,
        alias_classes,
        persist_pages: persist_resolution.persist_pages,
        commit_groups: persist_resolution.commit_groups,
        provenance: StorageProvenance {
            bindings: BTreeMap::new(),
            alias_classes: BTreeMap::new(),
            persist_pages: BTreeMap::new(),
            commit_groups: BTreeMap::new(),
        },
    };
    result.provenance = build_storage_provenance(input, &result);

    let consistency_context = consistency_context(input, &result);
    let consistency_bindings: Vec<_> = result.bindings.values().cloned().collect();
    let consistency_json = serde_json::to_value(SerializableResult::from(&result))
        .expect("storage plan result serializes for self-consistency");
    let consistency_diagnostics = validate_storage_plan_self_consistency(
        &consistency_context,
        StoragePlanConsistencyView {
            input_identity: &input.input_identity,
            bindings: &consistency_bindings,
            alias_classes: &result.alias_classes,
            persist_pages: &result.persist_pages,
            commit_groups: &result.commit_groups,
            json_value: Some(&consistency_json),
        },
    );
    if let Some(code) = first_storage_plan_code(&consistency_diagnostics) {
        return failed_output(input.input_identity.clone(), code);
    }

    let summary = StoragePlanCoreSummary {
        bindings: result.bindings.len() as u32,
        alias_classes: result.alias_classes.len() as u32,
        persist_pages: result.persist_pages.len() as u32,
        commit_groups: result.commit_groups.len() as u32,
    };

    StoragePlanCoreOutput {
        input_identity: input.input_identity.clone(),
        outcome: StoragePlanCoreOutcome::Succeeded,
        result: Some(result),
        summary: Some(summary),
        diagnostics: vec![],
        diagnostic_details: vec![],
    }
}

pub fn storage_plan_core_output_canonical_bytes(
    output: &StoragePlanCoreOutput,
) -> Result<Vec<u8>, CanonicalJsonError> {
    CanonicalJson::to_vec(&SerializableOutput::from(output))
}

fn failed_output(
    input_identity: StoragePlanInputIdentity,
    code: StoragePlanDiagnosticCode,
) -> StoragePlanCoreOutput {
    StoragePlanCoreOutput {
        input_identity,
        outcome: StoragePlanCoreOutcome::Failed,
        result: None,
        summary: None,
        diagnostics: vec![code],
        diagnostic_details: vec![],
    }
}

fn failed_output_with_detail(
    input_identity: StoragePlanInputIdentity,
    detail: StoragePlanCoreDiagnosticDetail,
) -> StoragePlanCoreOutput {
    StoragePlanCoreOutput {
        input_identity,
        outcome: StoragePlanCoreOutcome::Failed,
        result: None,
        summary: None,
        diagnostics: vec![detail.code],
        diagnostic_details: vec![detail],
    }
}

fn forced_recompute_rejection_detail(
    input: &StoragePlanCoreInput,
    value: ValueId,
) -> StoragePlanCoreDiagnosticDetail {
    let failed_predicates = recompute_allowed_failure_predicates(&input.predicate_env, value);
    let evidence = recompute_allowed_failure_evidence(&failed_predicates);
    if failed_predicates
        .contains(&RecomputeAllowedFailurePredicate::IsObservedCheckpointBackingValue)
    {
        StoragePlanCoreDiagnosticDetail {
            code: StoragePlanDiagnosticCode::StorageRecomputeForbiddenForObservedValue,
            provenance: StoragePlanDiagnosticProvenance::ObservationCheckpoint {
                value_id: value.get(),
                semantic_anchor: recompute_allowed_failure_anchor(&failed_predicates),
                checkpoint_id: 0,
            },
            evidence,
        }
    } else {
        StoragePlanCoreDiagnosticDetail {
            code: StoragePlanDiagnosticCode::StorageForcedRecomputeNotAllowed,
            provenance: StoragePlanDiagnosticProvenance::ForcedRecompute {
                value_id: value.get(),
                failed_predicates: failed_predicates
                    .iter()
                    .map(|predicate| recompute_allowed_failure_predicate_id(*predicate).to_owned())
                    .collect(),
            },
            evidence,
        }
    }
}

fn recompute_allowed_failure_anchor(
    failed_predicates: &BTreeSet<RecomputeAllowedFailurePredicate>,
) -> String {
    let joined = failed_predicates
        .iter()
        .map(|predicate| recompute_allowed_failure_predicate_id(*predicate))
        .collect::<Vec<_>>()
        .join("+");
    format!("DR-1b.RecomputeAllowed.{joined}")
}

fn recompute_allowed_failure_evidence(
    failed_predicates: &BTreeSet<RecomputeAllowedFailurePredicate>,
) -> Vec<EvidenceRef> {
    failed_predicates
        .iter()
        .map(|predicate| EvidenceRef {
            kind: "StoragePlanPredicateEvidence".to_owned(),
            reference: format!(
                "DR-1b.RecomputeAllowed.{}=failed",
                recompute_allowed_failure_predicate_id(*predicate)
            ),
            hash: Some(Hash256::ZERO),
        })
        .collect()
}

fn recompute_allowed_success_evidence() -> Vec<EvidenceRef> {
    [
        "ValueRoleOf",
        "IsPureValue",
        "IsObservedCheckpointBackingValue",
        "IsSequenceStateSlot",
        "EffectiveLifetimeEstimate",
    ]
    .into_iter()
    .map(|predicate| EvidenceRef {
        kind: "StoragePlanPredicateEvidence".to_owned(),
        reference: format!("DR-1.RecomputeAllowed.{predicate}=passed"),
        hash: Some(Hash256::ZERO),
    })
    .collect()
}

const fn recompute_allowed_failure_predicate_id(
    predicate: RecomputeAllowedFailurePredicate,
) -> &'static str {
    match predicate {
        RecomputeAllowedFailurePredicate::ValueRoleOf => "ValueRoleOf",
        RecomputeAllowedFailurePredicate::IsPureValue => "IsPureValue",
        RecomputeAllowedFailurePredicate::IsObservedCheckpointBackingValue => {
            "IsObservedCheckpointBackingValue"
        }
        RecomputeAllowedFailurePredicate::IsSequenceStateSlot => "IsSequenceStateSlot",
        RecomputeAllowedFailurePredicate::EffectiveLifetimeEstimate => "EffectiveLifetimeEstimate",
    }
}

fn tentative_bindings(values: &[StoragePlanCoreValue]) -> BTreeMap<ValueId, StorageBinding> {
    values
        .iter()
        .map(|value| {
            (
                value.value,
                StorageBinding {
                    value: value.value,
                    materialization: value.materialization.clone(),
                    alias_class: AliasClassId(0),
                    live_range: value.live_range.clone(),
                    justification: BindingJustification::DecisionRule(DecisionRuleId(15)),
                },
            )
        })
        .collect()
}

fn persist_inputs(values: &[StoragePlanCoreValue]) -> Vec<PersistBindingInput> {
    values
        .iter()
        .filter_map(|value| match &value.materialization {
            Materialization::Persist { page, commit_group } => Some(PersistBindingInput {
                value: value.value,
                page: *page,
                commit_group: *commit_group,
                kind: value.persist_kind?,
                reason: value.commit_group_reason?,
            }),
            _ => None,
        })
        .collect()
}

fn construct_alias_classes(
    input: &StoragePlanCoreInput,
    bindings: &BTreeMap<ValueId, StorageBinding>,
) -> Result<BTreeMap<AliasClassId, AliasClass>, StoragePlanDiagnosticCode> {
    let seeds = alias_seed_bindings(input, bindings);
    build_alias_classes(&seeds, &input.alias_edges, &input.topological_order)
        .map(classes_by_id)
        .map_err(|error| error.code)
}

fn construct_alias_classes_after_forced_recompute(
    input: &StoragePlanCoreInput,
    bindings: &BTreeMap<ValueId, StorageBinding>,
) -> Result<BTreeMap<AliasClassId, AliasClass>, StoragePlanDiagnosticCode> {
    let filtered_edges: Vec<_> = input
        .alias_edges
        .iter()
        .copied()
        .filter(|edge| {
            !input.alias_forced_recompute_values.contains(&edge.left)
                && !input.alias_forced_recompute_values.contains(&edge.right)
        })
        .collect();
    let rewritten = StoragePlanCoreInput {
        alias_edges: filtered_edges,
        ..input.clone()
    };
    construct_alias_classes(&rewritten, bindings)
}

fn alias_seed_bindings(
    input: &StoragePlanCoreInput,
    bindings: &BTreeMap<ValueId, StorageBinding>,
) -> Vec<AliasSeedBinding> {
    input
        .values
        .iter()
        .filter_map(|value| {
            let binding = bindings.get(&value.value)?;
            Some(AliasSeedBinding {
                value: value.value,
                materialization: binding.materialization.clone(),
                live_range: value.live_range.clone(),
                role: value.role,
            })
        })
        .collect()
}

fn classes_by_id(classes: Vec<AliasClass>) -> BTreeMap<AliasClassId, AliasClass> {
    classes
        .into_iter()
        .map(|class| (*class.id(), class))
        .collect()
}

fn assign_alias_class_ids(
    bindings: &mut BTreeMap<ValueId, StorageBinding>,
    alias_classes: &BTreeMap<AliasClassId, AliasClass>,
) {
    for (id, class) in alias_classes {
        for value in class.members() {
            if let Some(binding) = bindings.get_mut(value) {
                binding.alias_class = *id;
            }
        }
    }
}

fn build_storage_provenance(
    input: &StoragePlanCoreInput,
    result: &StoragePlanCoreResult,
) -> StorageProvenance {
    let role_by_value: BTreeMap<_, _> = input
        .values
        .iter()
        .map(|value| (value.value, value.role))
        .collect();
    let reason_by_group: BTreeMap<_, _> = input
        .values
        .iter()
        .filter_map(|value| match value.materialization {
            Materialization::Persist { commit_group, .. } => {
                Some((commit_group, value.commit_group_reason?))
            }
            Materialization::Recompute | Materialization::Materialize { .. } => None,
        })
        .collect();
    let sequence_state_page_sources = sequence_state_page_sources(input);

    StorageProvenance {
        bindings: result
            .bindings
            .iter()
            .map(|(value, binding)| {
                let decision_rule = match binding.justification {
                    BindingJustification::DecisionRule(rule) => rule,
                    BindingJustification::ForcedRecompute => DecisionRuleId(1),
                };
                let evidence = match binding.justification {
                    BindingJustification::DecisionRule(_) => Vec::new(),
                    BindingJustification::ForcedRecompute => recompute_allowed_success_evidence(),
                };
                (
                    *value,
                    BindingProvenance::new(
                        AdmittingPredicateId(decision_rule.0),
                        decision_rule,
                        false,
                        evidence,
                        role_by_value.get(value).copied(),
                        value_format_of(&input.predicate_env, *value).cloned(),
                    ),
                )
            })
            .collect(),
        alias_classes: result
            .alias_classes
            .iter()
            .map(|(id, class)| (*id, AliasClassProvenance::new(class.intent(), Vec::new())))
            .collect(),
        persist_pages: result
            .persist_pages
            .iter()
            .map(|(id, page)| {
                (
                    *id,
                    PersistPageProvenance {
                        source: persist_page_source(*id, page.kind, &sequence_state_page_sources),
                    },
                )
            })
            .collect(),
        commit_groups: result
            .commit_groups
            .keys()
            .map(|id| {
                (
                    *id,
                    CommitGroupProvenance::new(
                        reason_by_group
                            .get(id)
                            .copied()
                            .unwrap_or(CommitGroupReason::Independent),
                        Vec::new(),
                    ),
                )
            })
            .collect(),
    }
}

fn sequence_state_page_sources(
    input: &StoragePlanCoreInput,
) -> BTreeMap<PersistPageId, PersistPageSource> {
    input
        .values
        .iter()
        .filter_map(|value| {
            let Materialization::Persist { page, .. } = value.materialization else {
                return None;
            };
            if value.persist_kind != Some(PersistKind::SequenceState) {
                return None;
            }
            let slot = sequence_state_slot_ref(&input.predicate_env, value.value)?;
            Some((
                page,
                PersistPageSource::SequenceStateSlot {
                    layer: slot.layer,
                    slot: slot.slot,
                },
            ))
        })
        .collect()
}

fn persist_page_source(
    id: PersistPageId,
    kind: PersistKind,
    sequence_state_page_sources: &BTreeMap<PersistPageId, PersistPageSource>,
) -> PersistPageSource {
    match kind {
        PersistKind::SequenceState => sequence_state_page_sources.get(&id).cloned().unwrap_or(
            PersistPageSource::SequenceStateSlot {
                layer: 0,
                slot: id.0,
            },
        ),
        PersistKind::Continuation => PersistPageSource::Continuation,
        PersistKind::Transcript => PersistPageSource::Transcript { family: id.0 },
        PersistKind::Harness => PersistPageSource::Harness { family: id.0 },
        PersistKind::Trace => PersistPageSource::Trace { family: id.0 },
    }
}

fn consistency_context(
    input: &StoragePlanCoreInput,
    result: &StoragePlanCoreResult,
) -> StoragePlanConsistencyContext {
    StoragePlanConsistencyContext {
        expected_values: input.values.iter().map(|value| value.value).collect(),
        lifetime_bounds: input
            .values
            .iter()
            .map(|value| {
                let bounds = match result
                    .bindings
                    .get(&value.value)
                    .map(|binding| &binding.materialization)
                {
                    Some(Materialization::Persist { .. }) => persistent_lifetime_bounds(),
                    _ => lifetime_bounds(&input.predicate_env, value.value),
                };
                (value.value, bounds)
            })
            .collect(),
        expected_input_hashes: input_hashes_from_identity(&input.input_identity),
    }
}

fn persistent_lifetime_bounds() -> LifetimeBounds {
    LifetimeBounds {
        min_required: LifetimeBound {
            lifetime: LifetimeClass::Persistent,
            source: LifetimeBoundSource::Persistence,
        },
        max_admissible: LifetimeBound {
            lifetime: LifetimeClass::Persistent,
            source: LifetimeBoundSource::Persistence,
        },
    }
}

fn input_hashes_from_identity(identity: &StoragePlanInputIdentity) -> StoragePlanInputHashes {
    StoragePlanInputHashes {
        quant_graph_hash: identity.quant_graph_hash,
        infer_ir_hash: identity.infer_ir_hash,
        observation_plan_hash: identity.observation_plan_hash,
        range_plan_hash: identity.range_plan_hash,
        policy_hash: identity.policy_hash,
    }
}

fn first_storage_plan_code(
    diagnostics: &[gbf_policy::ValidationDiagnostic],
) -> Option<StoragePlanDiagnosticCode> {
    diagnostics
        .iter()
        .find_map(|diagnostic| match &diagnostic.code {
            gbf_policy::ValidationCode::StoragePlan { code, .. } => Some(*code),
            _ => None,
        })
}

#[derive(Serialize)]
struct SerializableOutput {
    input_identity: StoragePlanInputIdentity,
    outcome: StoragePlanCoreOutcome,
    result: Option<SerializableResult>,
    summary: Option<StoragePlanCoreSummary>,
    diagnostics: Vec<String>,
    diagnostic_details: Vec<StoragePlanCoreDiagnosticDetail>,
}

#[derive(Serialize)]
struct SerializableResult {
    bindings: BTreeMap<String, SerializableBinding>,
    alias_classes: BTreeMap<String, SerializableAliasClass>,
    persist_pages: BTreeMap<String, PersistPageDecl>,
    commit_groups: BTreeMap<String, CommitGroupDecl>,
    provenance: SerializableProvenance,
}

#[derive(Serialize)]
struct SerializableBinding {
    materialization: Materialization,
    alias_class: AliasClassId,
}

#[derive(Serialize)]
struct SerializableAliasClass {
    fingerprint: AliasClassFingerprint,
    members: Vec<u32>,
    intent: AliasIntent,
}

#[derive(Serialize)]
struct SerializableProvenance {
    bindings: BTreeMap<String, BindingProvenance>,
    alias_classes: BTreeMap<String, AliasClassProvenance>,
    persist_pages: BTreeMap<String, PersistPageProvenance>,
    commit_groups: BTreeMap<String, CommitGroupProvenance>,
}

impl From<&StoragePlanCoreOutput> for SerializableOutput {
    fn from(output: &StoragePlanCoreOutput) -> Self {
        Self {
            input_identity: output.input_identity.clone(),
            outcome: output.outcome,
            result: output.result.as_ref().map(SerializableResult::from),
            summary: output.summary,
            diagnostics: output
                .diagnostics
                .iter()
                .map(|code| code.as_str().to_owned())
                .collect(),
            diagnostic_details: output.diagnostic_details.clone(),
        }
    }
}

impl From<&StoragePlanCoreResult> for SerializableResult {
    fn from(result: &StoragePlanCoreResult) -> Self {
        Self {
            bindings: result
                .bindings
                .iter()
                .map(|(value, binding)| {
                    (
                        value.get().to_string(),
                        SerializableBinding {
                            materialization: binding.materialization.clone(),
                            alias_class: binding.alias_class,
                        },
                    )
                })
                .collect(),
            alias_classes: result
                .alias_classes
                .iter()
                .map(|(id, class)| {
                    (
                        id.0.to_string(),
                        SerializableAliasClass {
                            fingerprint: class.fingerprint(),
                            members: class.members().iter().map(|value| value.get()).collect(),
                            intent: class.intent(),
                        },
                    )
                })
                .collect(),
            persist_pages: result
                .persist_pages
                .iter()
                .map(|(id, page)| (id.0.to_string(), page.clone()))
                .collect(),
            commit_groups: result
                .commit_groups
                .iter()
                .map(|(id, group)| (id.0.to_string(), group.clone()))
                .collect(),
            provenance: SerializableProvenance {
                bindings: result
                    .provenance
                    .bindings
                    .iter()
                    .map(|(value, provenance)| (value.get().to_string(), provenance.clone()))
                    .collect(),
                alias_classes: result
                    .provenance
                    .alias_classes
                    .iter()
                    .map(|(id, provenance)| (id.0.to_string(), provenance.clone()))
                    .collect(),
                persist_pages: result
                    .provenance
                    .persist_pages
                    .iter()
                    .map(|(id, provenance)| (id.0.to_string(), provenance.clone()))
                    .collect(),
                commit_groups: result
                    .provenance
                    .commit_groups
                    .iter()
                    .map(|(id, provenance)| (id.0.to_string(), provenance.clone()))
                    .collect(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use gbf_foundation::{Hash256, SemVer};
    use gbf_policy::{StoragePlanDiagnosticProvenance, ValidationCode};
    use gbf_report::ReportSchemaId;

    use super::*;
    use crate::s1::quant_graph::DeterminismClass;
    use crate::storage_plan::predicates::{PredicateValueFacts, QuantFormatId, ValueFormat};
    use crate::storage_plan::types::{LifetimeClass, StorageClass};

    #[test]
    fn build_storage_plan_core_is_deterministic_for_same_input() {
        let input = fixture_input(BTreeSet::new(), false);
        let first = build_storage_plan_core(&input);
        let second = build_storage_plan_core(&input);

        assert_eq!(
            storage_plan_core_output_canonical_bytes(&first).expect("first canonicalizes"),
            storage_plan_core_output_canonical_bytes(&second).expect("second canonicalizes")
        );
    }

    #[test]
    fn alias_overlap_uses_canonical_topology_not_values_iteration_order() {
        let mut input = fixture_input(BTreeSet::new(), false);
        let mut later_listed = hot_value(1);
        later_listed.live_range = live_range(2, 5);
        let mut earlier_listed = hot_value(2);
        earlier_listed.live_range = live_range(3, 4);
        input.values = vec![earlier_listed, later_listed];
        input.topological_order = (1..=6).map(NodeId::new).collect();
        input.alias_edges = vec![edge(1, 2, AliasIntent::ScratchReuse)];

        let output = build_storage_plan_core(&input);

        assert_eq!(output.outcome, StoragePlanCoreOutcome::Failed);
        assert_eq!(
            output.diagnostics,
            vec![StoragePlanDiagnosticCode::StorageAliasClassOverlapWithoutIntent]
        );
    }

    #[test]
    fn persist_rotation_pair_predicate_has_persist_pages_available_before_alias() {
        let mut input = fixture_input(BTreeSet::new(), false);
        input.values = vec![
            persist_value(1, PersistPageId(1), CommitGroupId(1), PersistKind::Trace),
            persist_value(2, PersistPageId(2), CommitGroupId(2), PersistKind::Trace),
        ];
        input.alias_edges = vec![AliasCandidateEdge {
            left: ValueId::new(1),
            right: ValueId::new(2),
            intent: AliasIntent::PersistRotation,
        }];

        let output = build_storage_plan_core(&input);
        let result = output.result.expect("driver succeeds");

        assert!(result.persist_pages.contains_key(&PersistPageId(1)));
        assert_eq!(
            result
                .alias_classes
                .values()
                .find(|class| class.intent() == AliasIntent::PersistRotation)
                .expect("persist rotation class exists")
                .members()
                .len(),
            2
        );
    }

    #[test]
    fn sequence_state_persist_page_provenance_uses_layer_slot_facts() {
        let value = ValueId::new(9);
        let page = PersistPageId(91);
        let mut input = fixture_input(BTreeSet::new(), false);
        input.values = vec![StoragePlanCoreValue {
            value,
            materialization: Materialization::Persist {
                page,
                commit_group: CommitGroupId(7),
            },
            live_range: live_range(2, 3),
            role: ValueRole::SequenceStateSlot,
            persist_kind: Some(PersistKind::SequenceState),
            commit_group_reason: Some(CommitGroupReason::PerSequenceStateSlot),
        }];
        input.alias_edges = vec![];
        input.predicate_env = PredicateEnv::new().with_value(
            value,
            PredicateValueFacts::new(ValueRole::SequenceStateSlot, ValueFormat::Flag)
                .with_sequence_state_slot(4, 99),
        );

        let output = build_storage_plan_core(&input);
        let result = output.result.expect("sequence-state page resolves");

        assert_eq!(
            result.provenance.persist_pages[&page].source,
            PersistPageSource::SequenceStateSlot { layer: 4, slot: 99 }
        );
    }

    #[test]
    fn alias_level_forced_recompute_splits_named_member_after_initial_partition() {
        let forced = BTreeSet::from([ValueId::new(2)]);
        let input = fixture_input(forced, false);
        let output = build_storage_plan_core(&input);
        let result = output.result.expect("driver succeeds");

        assert_eq!(
            result.bindings[&ValueId::new(2)].materialization,
            Materialization::Recompute
        );
        assert_eq!(
            result.alias_classes[&result.bindings[&ValueId::new(2)].alias_class].intent(),
            AliasIntent::NoAlias
        );
        let remaining = result
            .alias_classes
            .values()
            .find(|class| class.intent() == AliasIntent::ScratchReuse)
            .expect("remaining scratch reuse pair");
        assert_eq!(remaining.members().len(), 2);
        assert!(
            remaining
                .members()
                .as_btree_set()
                .contains(&ValueId::new(1))
        );
        assert!(
            remaining
                .members()
                .as_btree_set()
                .contains(&ValueId::new(3))
        );
    }

    #[test]
    fn forced_recompute_success_uses_dr_1_provenance_not_store_033() {
        let forced = BTreeSet::from([ValueId::new(2)]);
        let input = fixture_input(forced, false);
        let output = build_storage_plan_core(&input);
        let result = output.result.expect("driver succeeds");
        let provenance = &result.provenance.bindings[&ValueId::new(2)];

        assert_eq!(
            result.bindings[&ValueId::new(2)].justification,
            BindingJustification::ForcedRecompute
        );
        assert_eq!(provenance.decision_rule, DecisionRuleId(1));
        assert_eq!(provenance.admitting_predicate, AdmittingPredicateId(1));
        assert_ne!(provenance.decision_rule, DecisionRuleId(33));
        assert!(
            provenance.evidence.iter().any(|evidence| {
                evidence.reference == "DR-1.RecomputeAllowed.ValueRoleOf=passed"
            })
        );
        assert!(provenance.evidence.iter().any(|evidence| {
            evidence.reference == "DR-1.RecomputeAllowed.EffectiveLifetimeEstimate=passed"
        }));
    }

    #[test]
    fn forced_recompute_router_rejection_carries_failed_predicates_to_report() {
        let forced_value = ValueId::new(2);
        let mut input = fixture_input(BTreeSet::from([forced_value]), false);
        input.predicate_env = input.predicate_env.with_value(
            forced_value,
            PredicateValueFacts::new(
                ValueRole::RouterDecision,
                ValueFormat::TokenIdDomain { vocab_size: 8 },
            ),
        );
        input.values[1].role = ValueRole::RouterDecision;

        let output = build_storage_plan_core(&input);

        assert_eq!(output.outcome, StoragePlanCoreOutcome::Failed);
        assert_eq!(
            output.diagnostics,
            vec![StoragePlanDiagnosticCode::StorageForcedRecomputeNotAllowed]
        );
        assert_eq!(output.diagnostic_details.len(), 1);
        match &output.diagnostic_details[0].provenance {
            StoragePlanDiagnosticProvenance::ForcedRecompute {
                value_id,
                failed_predicates,
            } => {
                assert_eq!(*value_id, forced_value.get());
                assert_eq!(failed_predicates, &vec!["ValueRoleOf".to_owned()]);
            }
            other => panic!("expected forced recompute provenance, got {other:?}"),
        }

        let envelope =
            crate::storage_plan::emit_storage_plan_report(&output).expect("report emits");
        match &envelope.body.body.diagnostics[0].code {
            ValidationCode::StoragePlan { code, provenance } => {
                assert_eq!(
                    *code,
                    StoragePlanDiagnosticCode::StorageForcedRecomputeNotAllowed
                );
                assert!(matches!(
                    provenance,
                    StoragePlanDiagnosticProvenance::ForcedRecompute {
                        value_id: 2,
                        failed_predicates,
                    } if failed_predicates == &vec!["ValueRoleOf".to_owned()]
                ));
            }
            other => panic!("expected storage plan diagnostic, got {other:?}"),
        }
    }

    #[test]
    fn forced_recompute_observed_value_rejection_names_recompute_allowed_anchor() {
        let forced_value = ValueId::new(2);
        let mut input = fixture_input(BTreeSet::from([forced_value]), false);
        input.predicate_env = input
            .predicate_env
            .with_observed_checkpoint_backing_value(forced_value);

        let output = build_storage_plan_core(&input);

        assert_eq!(output.outcome, StoragePlanCoreOutcome::Failed);
        assert_eq!(
            output.diagnostics,
            vec![StoragePlanDiagnosticCode::StorageRecomputeForbiddenForObservedValue]
        );
        match &output.diagnostic_details[0].provenance {
            StoragePlanDiagnosticProvenance::ObservationCheckpoint {
                value_id,
                semantic_anchor,
                checkpoint_id,
            } => {
                assert_eq!(*value_id, forced_value.get());
                assert_eq!(*checkpoint_id, 0);
                assert!(semantic_anchor.contains("DR-1b.RecomputeAllowed"));
                assert!(semantic_anchor.contains("IsObservedCheckpointBackingValue"));
            }
            other => panic!("expected observation checkpoint provenance, got {other:?}"),
        }
        assert!(
            output.diagnostic_details[0]
                .evidence
                .iter()
                .any(|evidence| {
                    evidence.reference
                        == "DR-1b.RecomputeAllowed.IsObservedCheckpointBackingValue=failed"
                })
        );
    }

    #[test]
    fn failure_path_has_no_partial_result_or_summary() {
        let output = build_storage_plan_core(&fixture_input(BTreeSet::new(), true));

        assert_eq!(output.outcome, StoragePlanCoreOutcome::Failed);
        assert!(output.result.is_none());
        assert!(output.summary.is_none());
    }

    #[test]
    fn provenance_maps_are_source_of_truth_not_inline_on_plan_items() {
        let output = build_storage_plan_core(&fixture_input(BTreeSet::new(), false));
        let result = output.result.expect("driver succeeds");

        assert_eq!(
            result
                .provenance
                .bindings
                .keys()
                .copied()
                .collect::<Vec<_>>(),
            result.bindings.keys().copied().collect::<Vec<_>>()
        );
        assert_eq!(
            result
                .provenance
                .alias_classes
                .keys()
                .copied()
                .collect::<Vec<_>>(),
            result.alias_classes.keys().copied().collect::<Vec<_>>()
        );
        assert_eq!(
            result
                .provenance
                .persist_pages
                .keys()
                .copied()
                .collect::<Vec<_>>(),
            result.persist_pages.keys().copied().collect::<Vec<_>>()
        );
        assert_eq!(
            result
                .provenance
                .commit_groups
                .keys()
                .copied()
                .collect::<Vec<_>>(),
            result.commit_groups.keys().copied().collect::<Vec<_>>()
        );

        let binding_json = serde_json::to_value(result.bindings.values().next().expect("binding"))
            .expect("binding serializes");
        let class_json = serde_json::to_value(result.alias_classes.values().next().expect("class"))
            .expect("class serializes");
        let page_json = serde_json::to_value(result.persist_pages.values().next())
            .expect("optional page serializes");
        let group_json = serde_json::to_value(result.commit_groups.values().next())
            .expect("optional group serializes");
        let plan_json =
            serde_json::to_value(SerializableResult::from(&result)).expect("result serializes");

        assert!(!json_has_key(&binding_json, "provenance"));
        assert!(!json_has_key(&class_json, "provenance"));
        assert!(!json_has_key(&page_json, "provenance"));
        assert!(!json_has_key(&group_json, "provenance"));
        assert!(json_has_key(&plan_json, "provenance"));
    }

    fn fixture_input(forced: BTreeSet<ValueId>, fail_before_result: bool) -> StoragePlanCoreInput {
        let mut env = PredicateEnv::new();
        for value in [1, 2, 3] {
            env = env.with_value(ValueId::new(value), activation_facts());
        }
        StoragePlanCoreInput {
            input_identity: identity(),
            predicate_env: env,
            topological_order: (1..=8).map(NodeId::new).collect(),
            values: vec![hot_value(1), hot_value(2), hot_value(3)],
            alias_edges: vec![
                edge(1, 2, AliasIntent::ScratchReuse),
                edge(2, 3, AliasIntent::ScratchReuse),
                edge(1, 3, AliasIntent::ScratchReuse),
            ],
            alias_forced_recompute_values: forced,
            fail_before_result,
        }
    }

    fn hot_value(value: u32) -> StoragePlanCoreValue {
        StoragePlanCoreValue {
            value: ValueId::new(value),
            materialization: Materialization::Materialize {
                class: StorageClass::WramHot,
                lifetime: LifetimeClass::Slice,
            },
            live_range: live_range(value * 2, value * 2 + 1),
            role: ValueRole::Activation,
            persist_kind: None,
            commit_group_reason: None,
        }
    }

    fn persist_value(
        value: u32,
        page: PersistPageId,
        commit_group: CommitGroupId,
        kind: PersistKind,
    ) -> StoragePlanCoreValue {
        StoragePlanCoreValue {
            value: ValueId::new(value),
            materialization: Materialization::Persist { page, commit_group },
            live_range: live_range(value * 2, value * 2 + 1),
            role: ValueRole::Activation,
            persist_kind: Some(kind),
            commit_group_reason: Some(CommitGroupReason::Independent),
        }
    }

    fn live_range(def: u32, last: u32) -> AbstractLiveRange {
        AbstractLiveRange {
            def_node: crate::s3::infer_ir::NodeId::new(def),
            first_use_node: Some(crate::s3::infer_ir::NodeId::new(last)),
            last_use_node: Some(crate::s3::infer_ir::NodeId::new(last)),
            lifetime_class: LifetimeClass::Slice,
            checkpoint_stable: false,
        }
    }

    fn edge(left: u32, right: u32, intent: AliasIntent) -> AliasCandidateEdge {
        AliasCandidateEdge {
            left: ValueId::new(left),
            right: ValueId::new(right),
            intent,
        }
    }

    fn activation_facts() -> PredicateValueFacts {
        PredicateValueFacts::new(
            ValueRole::Activation,
            ValueFormat::QuantInt {
                quant_format_id: QuantFormatId(1),
            },
        )
    }

    fn identity() -> StoragePlanInputIdentity {
        StoragePlanInputIdentity {
            quant_graph_hash: Hash256::from_bytes([1; 32]),
            infer_ir_hash: Hash256::from_bytes([2; 32]),
            observation_plan_hash: Hash256::from_bytes([3; 32]),
            range_plan_hash: Hash256::from_bytes([4; 32]),
            policy_hash: Hash256::from_bytes([5; 32]),
            determinism: DeterminismClass::Deterministic,
            schema: ReportSchemaId::from("storage_plan.v1"),
            schema_version: SemVer::new(1, 0, 0),
        }
    }

    fn json_has_key(value: &serde_json::Value, key: &str) -> bool {
        match value {
            serde_json::Value::Object(map) => map
                .iter()
                .any(|(field, nested)| field == key || json_has_key(nested, key)),
            serde_json::Value::Array(values) => {
                values.iter().any(|nested| json_has_key(nested, key))
            }
            serde_json::Value::Null
            | serde_json::Value::Bool(_)
            | serde_json::Value::Number(_)
            | serde_json::Value::String(_) => false,
        }
    }
}
