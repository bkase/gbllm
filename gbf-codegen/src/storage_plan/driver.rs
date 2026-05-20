//! Pure Stage 6 construction-order driver over normalized storage facts.

use std::collections::{BTreeMap, BTreeSet};

use gbf_foundation::{CanonicalJson, CanonicalJsonError, EvidenceRef, Hash256};
use gbf_policy::{
    StorageMaterialization, StoragePlanDiagnosticCode, StoragePlanDiagnosticProvenance,
};
use serde::{Deserialize, Serialize};

use crate::s3::infer_ir::{NodeId, ValueId};
use crate::storage_plan::alias_engine::{
    AliasCandidateEdge, AliasEngineDiagnostic, AliasSeedBinding, build_alias_classes,
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
    PredicateEnv, RecomputeAllowedFailurePredicate, ValueRole, logical_size_of, recompute_allowed,
    recompute_allowed_failure_predicates, recompute_cost_estimate,
    recompute_cost_within_cycle_ceiling, sequence_state_slot_ref, value_format_of,
};
use crate::storage_plan::rules::{
    DecisionRuleEvaluation, DecisionRuleFiring, DecisionRuleOutcome, evaluate_registry,
};
use crate::storage_plan::types::{
    AbstractLiveRange, AdmittingPredicateId, AliasClass, AliasClassFingerprint, AliasClassId,
    AliasClassProvenance, AliasIntent, BindingJustification, BindingProvenance, CommitGroupDecl,
    CommitGroupId, CommitGroupProvenance, CommitGroupReason, DecisionRuleId, LifetimeClass,
    Materialization, PersistKind, PersistPageDecl, PersistPageId, PersistPageProvenance,
    PersistPageSource, STORAGE_PLAN_INPUT_HASH_MISMATCH_SPECS, StorageBinding, StorageClass,
    StoragePlanInputHashes, StoragePlanInputIdentity, StorageProvenance,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoragePlanCoreInput {
    pub input_identity: StoragePlanInputIdentity,
    pub expected_input_hashes: StoragePlanInputHashes,
    pub repair_policy: StoragePlanRepairPolicy,
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
    pub repair_proposals: Vec<RepairProposal>,
    pub provenance: StorageProvenance,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StoragePlanCoreDiagnosticDetail {
    pub code: StoragePlanDiagnosticCode,
    pub provenance: StoragePlanDiagnosticProvenance,
    pub evidence: Vec<EvidenceRef>,
}

type StoragePlanCoreDiagnosticResult<T> = Result<T, Box<StoragePlanCoreDiagnosticDetail>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoragePlanCoreSummary {
    pub total_bindings: u32,
    pub bindings: u32,
    pub rule_firings: [StoragePlanRuleFiringCount; STORAGE_PLAN_RULE_FIRING_SUMMARY_LEN],
    pub alias_classes_no_alias: u32,
    pub alias_classes: u32,
    pub persist_pages: u32,
    pub commit_groups: u32,
    pub abstract_pressure: StoragePlanAbstractPressure,
}

pub const STORAGE_PLAN_RULE_FIRING_SUMMARY_LEN: usize = 15;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoragePlanRuleFiringCount {
    pub rule: DecisionRuleId,
    pub count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoragePlanAbstractPressure {
    pub wram_hot_bytes: u32,
    pub hram_hot_bytes: u32,
    pub sram_paged_bytes: u32,
}

impl StoragePlanAbstractPressure {
    #[must_use]
    pub const fn exceeds_soft_threshold(self, soft_threshold_bytes: u32) -> bool {
        self.wram_hot_bytes > soft_threshold_bytes
            || self.hram_hot_bytes > soft_threshold_bytes
            || self.sram_paged_bytes > soft_threshold_bytes
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoragePlanRepairPolicy {
    pub soft_pressure_threshold_bytes: Option<u32>,
    pub recompute_promotion: StorageMaterialization,
    pub max_recompute_promotion: StorageMaterialization,
    pub storage_recompute_promotion_locked: bool,
}

impl Default for StoragePlanRepairPolicy {
    fn default() -> Self {
        Self {
            soft_pressure_threshold_bytes: None,
            recompute_promotion: StorageMaterialization::PreserveAll,
            max_recompute_promotion: StorageMaterialization::RecomputePureValues,
            storage_recompute_promotion_locked: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepairProposal {
    pub source: PlanningStage,
    pub reason: RepairReason,
    pub tighten: ConstraintDelta,
    pub estimated_cost: EstimatedCostDelta,
    pub proposal_id: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum PlanningStage {
    StoragePlan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum RepairReason {
    PromoteRecompute,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConstraintDelta {
    pub changes: Vec<KnobDelta>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum KnobDelta {
    PromoteRecomputeLevel { to: StorageMaterialization },
    ForceRecompute { values: Vec<ValueId> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EstimatedCostDelta {
    pub cycles: Option<u64>,
    pub bytes: Option<u64>,
}

pub fn build_storage_plan_core(input: &StoragePlanCoreInput) -> StoragePlanCoreOutput {
    let hash_mismatch_details = input_hash_mismatch_details(input);
    if !hash_mismatch_details.is_empty() {
        return failed_output_with_details(input.input_identity.clone(), hash_mismatch_details);
    }

    if input.fail_before_result {
        return failed_output(
            input.input_identity.clone(),
            StoragePlanDiagnosticCode::StorageNoAdmittingDecisionRule,
        );
    }

    let mut bindings = match tentative_bindings(input) {
        Ok(bindings) => bindings,
        Err(detail) => return failed_output_with_detail(input.input_identity.clone(), *detail),
    };
    let persist_inputs = persist_inputs(&input.values);
    let persist_resolution = match resolve_persist_bindings(&persist_inputs) {
        Ok(resolution) => resolution,
        Err(error) => return failed_output(input.input_identity.clone(), error.code),
    };

    let mut alias_classes = match construct_alias_classes(input, &bindings) {
        Ok(classes) => classes,
        Err(error) => {
            return failed_output_with_detail(
                input.input_identity.clone(),
                alias_engine_diagnostic_detail(error),
            );
        }
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
            Err(error) => {
                return failed_output_with_detail(
                    input.input_identity.clone(),
                    alias_engine_diagnostic_detail(error),
                );
            }
        };
        assign_alias_class_ids(&mut bindings, &alias_classes);
    }

    let mut result = StoragePlanCoreResult {
        bindings,
        alias_classes,
        persist_pages: persist_resolution.persist_pages,
        commit_groups: persist_resolution.commit_groups,
        repair_proposals: Vec::new(),
        provenance: StorageProvenance {
            bindings: BTreeMap::new(),
            alias_classes: BTreeMap::new(),
            persist_pages: BTreeMap::new(),
            commit_groups: BTreeMap::new(),
        },
    };
    result.provenance = build_storage_provenance(input, &result);
    let abstract_pressure = abstract_pressure(input, &result);

    match repair_proposals(input, &result, abstract_pressure) {
        Ok(proposals) => result.repair_proposals = proposals,
        Err(detail) => return failed_output_with_detail(input.input_identity.clone(), *detail),
    }

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
    let consistency_details =
        storage_plan_diagnostic_details_from_validation(&consistency_diagnostics);
    if !consistency_details.is_empty() {
        return failed_output_with_details(input.input_identity.clone(), consistency_details);
    }

    let summary = StoragePlanCoreSummary {
        total_bindings: result.bindings.len() as u32,
        bindings: result.bindings.len() as u32,
        rule_firings: rule_firings_from_bindings(&result.bindings),
        alias_classes_no_alias: result
            .alias_classes
            .values()
            .filter(|class| class.intent() == AliasIntent::NoAlias)
            .count() as u32,
        alias_classes: result.alias_classes.len() as u32,
        persist_pages: result.persist_pages.len() as u32,
        commit_groups: result.commit_groups.len() as u32,
        abstract_pressure,
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
    failed_output_with_details(input_identity, vec![detail])
}

fn failed_output_with_details(
    input_identity: StoragePlanInputIdentity,
    details: Vec<StoragePlanCoreDiagnosticDetail>,
) -> StoragePlanCoreOutput {
    StoragePlanCoreOutput {
        input_identity,
        outcome: StoragePlanCoreOutcome::Failed,
        result: None,
        summary: None,
        diagnostics: details.iter().map(|detail| detail.code).collect(),
        diagnostic_details: details,
    }
}

fn input_hash_mismatch_details(
    input: &StoragePlanCoreInput,
) -> Vec<StoragePlanCoreDiagnosticDetail> {
    STORAGE_PLAN_INPUT_HASH_MISMATCH_SPECS
        .iter()
        .filter_map(|spec| {
            let recorded = input.input_identity.hash_for_product(spec.product);
            let computed = input.expected_input_hashes.hash_for_product(spec.product);
            (recorded != computed).then(|| StoragePlanCoreDiagnosticDetail {
                code: spec.storage_code,
                provenance: StoragePlanDiagnosticProvenance::HashMismatch {
                    product: spec.identity_field.to_owned(),
                    recorded,
                    computed,
                },
                evidence: vec![EvidenceRef {
                    kind: "StoragePlanInputHash".to_owned(),
                    reference: format!("{}.recorded_vs_computed", spec.identity_field),
                    hash: Some(Hash256::ZERO),
                }],
            })
        })
        .collect()
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

fn tentative_bindings(
    input: &StoragePlanCoreInput,
) -> StoragePlanCoreDiagnosticResult<BTreeMap<ValueId, StorageBinding>> {
    input
        .values
        .iter()
        .map(|value| {
            let decision_rule = decision_rule_for_value(input, value)?;
            Ok((
                value.value,
                StorageBinding {
                    value: value.value,
                    materialization: value.materialization.clone(),
                    alias_class: AliasClassId(0),
                    live_range: value.live_range.clone(),
                    justification: BindingJustification::DecisionRule(decision_rule),
                },
            ))
        })
        .collect()
}

fn decision_rule_for_value(
    input: &StoragePlanCoreInput,
    value: &StoragePlanCoreValue,
) -> StoragePlanCoreDiagnosticResult<DecisionRuleId> {
    if input.alias_forced_recompute_values.contains(&value.value) {
        return Ok(DecisionRuleId(1));
    }

    match evaluate_registry(&input.predicate_env, value.value) {
        DecisionRuleEvaluation::Fired(firing) => match firing.outcome {
            DecisionRuleOutcome::Bind(_) => Ok(firing.rule_id),
            DecisionRuleOutcome::Reject(_) if value.persist_kind.is_some() => {
                Ok(persist_decision_rule(value))
            }
            DecisionRuleOutcome::Reject(code) => Err(Box::new(decision_rule_rejection_detail(
                input,
                value.value,
                code,
                Some(&firing),
            ))),
        },
        DecisionRuleEvaluation::NoAdmittingRule { code: _, .. } if value.persist_kind.is_some() => {
            Ok(persist_decision_rule(value))
        }
        DecisionRuleEvaluation::NoAdmittingRule { code, .. } => Err(Box::new(
            decision_rule_rejection_detail(input, value.value, code, None),
        )),
    }
}

fn persist_decision_rule(value: &StoragePlanCoreValue) -> DecisionRuleId {
    match value.persist_kind {
        Some(PersistKind::SequenceState) => DecisionRuleId(3),
        Some(PersistKind::Continuation) => DecisionRuleId(5),
        Some(PersistKind::Transcript) => DecisionRuleId(6),
        Some(PersistKind::Harness | PersistKind::Trace) => DecisionRuleId(7),
        None => DecisionRuleId(15),
    }
}

fn decision_rule_rejection_detail(
    input: &StoragePlanCoreInput,
    value: ValueId,
    code: StoragePlanDiagnosticCode,
    firing: Option<&DecisionRuleFiring>,
) -> StoragePlanCoreDiagnosticDetail {
    if code == StoragePlanDiagnosticCode::StoragePersistSequenceStateUnsupportedV1 {
        let slot = sequence_state_slot_ref(&input.predicate_env, value);
        return StoragePlanCoreDiagnosticDetail {
            code,
            provenance: StoragePlanDiagnosticProvenance::SequenceState {
                value_id: value.get(),
                state_slot_id: slot.map_or(0, |slot| slot.slot),
                layer_id: slot.map_or(0, |slot| slot.layer),
            },
            evidence: vec![EvidenceRef {
                kind: "StoragePlanDecisionRule".to_owned(),
                reference: firing.map_or_else(
                    || "registry.no_admitting_rule".to_owned(),
                    |firing| format!("{}.reject", firing.rule_name),
                ),
                hash: Some(Hash256::ZERO),
            }],
        };
    }

    StoragePlanCoreDiagnosticDetail {
        code,
        provenance: StoragePlanDiagnosticProvenance::ValueClassification {
            value_id: value.get(),
            producer_node: None,
            value_role: input
                .predicate_env
                .facts(value)
                .map(|facts| format!("{:?}", facts.role)),
            value_format: input
                .predicate_env
                .facts(value)
                .and_then(|facts| facts.format.as_ref())
                .map(|format| format!("{format:?}")),
        },
        evidence: vec![EvidenceRef {
            kind: "StoragePlanDecisionRule".to_owned(),
            reference: firing.map_or_else(
                || "registry.no_admitting_rule".to_owned(),
                |firing| format!("{}.reject", firing.rule_name),
            ),
            hash: Some(Hash256::ZERO),
        }],
    }
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
) -> Result<BTreeMap<AliasClassId, AliasClass>, AliasEngineDiagnostic> {
    construct_alias_classes_with_edges(input, bindings, &input.alias_edges)
}

fn construct_alias_classes_with_edges(
    input: &StoragePlanCoreInput,
    bindings: &BTreeMap<ValueId, StorageBinding>,
    alias_edges: &[AliasCandidateEdge],
) -> Result<BTreeMap<AliasClassId, AliasClass>, AliasEngineDiagnostic> {
    let seeds = alias_seed_bindings(input, bindings);
    build_alias_classes(&seeds, alias_edges, &input.topological_order).map(classes_by_id)
}

fn construct_alias_classes_after_forced_recompute(
    input: &StoragePlanCoreInput,
    bindings: &BTreeMap<ValueId, StorageBinding>,
) -> Result<BTreeMap<AliasClassId, AliasClass>, AliasEngineDiagnostic> {
    let filtered_edges: Vec<_> = input
        .alias_edges
        .iter()
        .copied()
        .filter(|edge| {
            !input.alias_forced_recompute_values.contains(&edge.left)
                && !input.alias_forced_recompute_values.contains(&edge.right)
        })
        .collect();
    construct_alias_classes_with_edges(input, bindings, &filtered_edges)
}

fn alias_engine_diagnostic_detail(error: AliasEngineDiagnostic) -> StoragePlanCoreDiagnosticDetail {
    let members: Vec<_> = error.members.iter().map(|member| member.get()).collect();
    let provenance = match error.code {
        StoragePlanDiagnosticCode::StorageAliasClassOverlapWithoutIntent => {
            StoragePlanDiagnosticProvenance::AliasOverlap {
                alias_class_id: 0,
                members,
            }
        }
        StoragePlanDiagnosticCode::StorageAliasMixedIntentComponent => {
            StoragePlanDiagnosticProvenance::AliasMixedIntent {
                members,
                edge_count: error.intents.len() as u32,
                intents: error
                    .intents
                    .iter()
                    .map(|intent| format!("{intent:?}"))
                    .collect(),
            }
        }
        StoragePlanDiagnosticCode::StorageAliasIntentCardinalityViolation => {
            StoragePlanDiagnosticProvenance::AliasCardinality {
                alias_class_id: 0,
                intent: error
                    .intent
                    .map_or_else(|| "Unknown".to_owned(), |intent| format!("{intent:?}")),
                members,
            }
        }
        StoragePlanDiagnosticCode::StorageAliasClassFingerprintCollision => {
            StoragePlanDiagnosticProvenance::FingerprintCollision {
                first_payload_hash: Hash256::from_bytes([0xaa; 32]),
                second_payload_hash: Hash256::from_bytes([0xbb; 32]),
            }
        }
        _ => StoragePlanDiagnosticProvenance::ValueClassification {
            value_id: members.first().copied().unwrap_or_default(),
            producer_node: None,
            value_role: None,
            value_format: None,
        },
    };
    StoragePlanCoreDiagnosticDetail {
        code: error.code,
        provenance,
        evidence: vec![EvidenceRef {
            kind: "StoragePlanAliasEngine".to_owned(),
            reference: format!("alias_engine.{}", error.code.name()),
            hash: Some(Hash256::ZERO),
        }],
    }
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

fn rule_firings_from_bindings(
    bindings: &BTreeMap<ValueId, StorageBinding>,
) -> [StoragePlanRuleFiringCount; STORAGE_PLAN_RULE_FIRING_SUMMARY_LEN] {
    let mut counts = storage_plan_rule_firing_zero_counts();
    for binding in bindings.values() {
        let rule = match binding.justification {
            BindingJustification::DecisionRule(rule) => rule,
            BindingJustification::ForcedRecompute => DecisionRuleId(1),
        };
        if let Some(entry) = counts.iter_mut().find(|entry| entry.rule == rule) {
            entry.count = entry.count.saturating_add(1);
        }
    }
    counts
}

const fn storage_plan_rule_firing_zero_counts()
-> [StoragePlanRuleFiringCount; STORAGE_PLAN_RULE_FIRING_SUMMARY_LEN] {
    [
        StoragePlanRuleFiringCount {
            rule: DecisionRuleId(1),
            count: 0,
        },
        StoragePlanRuleFiringCount {
            rule: DecisionRuleId(2),
            count: 0,
        },
        StoragePlanRuleFiringCount {
            rule: DecisionRuleId(3),
            count: 0,
        },
        StoragePlanRuleFiringCount {
            rule: DecisionRuleId(4),
            count: 0,
        },
        StoragePlanRuleFiringCount {
            rule: DecisionRuleId(5),
            count: 0,
        },
        StoragePlanRuleFiringCount {
            rule: DecisionRuleId(6),
            count: 0,
        },
        StoragePlanRuleFiringCount {
            rule: DecisionRuleId(7),
            count: 0,
        },
        StoragePlanRuleFiringCount {
            rule: DecisionRuleId(8),
            count: 0,
        },
        StoragePlanRuleFiringCount {
            rule: DecisionRuleId(9),
            count: 0,
        },
        StoragePlanRuleFiringCount {
            rule: DecisionRuleId(10),
            count: 0,
        },
        StoragePlanRuleFiringCount {
            rule: DecisionRuleId(11),
            count: 0,
        },
        StoragePlanRuleFiringCount {
            rule: DecisionRuleId(12),
            count: 0,
        },
        StoragePlanRuleFiringCount {
            rule: DecisionRuleId(13),
            count: 0,
        },
        StoragePlanRuleFiringCount {
            rule: DecisionRuleId(14),
            count: 0,
        },
        StoragePlanRuleFiringCount {
            rule: DecisionRuleId(15),
            count: 0,
        },
    ]
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
                    BindingJustification::DecisionRule(rule) => {
                        decision_rule_success_evidence(rule)
                    }
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

fn abstract_pressure(
    input: &StoragePlanCoreInput,
    result: &StoragePlanCoreResult,
) -> StoragePlanAbstractPressure {
    let mut pressure = StoragePlanAbstractPressure {
        wram_hot_bytes: 0,
        hram_hot_bytes: 0,
        sram_paged_bytes: 0,
    };

    for (value, binding) in &result.bindings {
        let bytes = logical_size_of(&input.predicate_env, *value).unwrap_or_default();
        match binding.materialization {
            Materialization::Materialize {
                class: StorageClass::WramHot,
                ..
            } => pressure.wram_hot_bytes = pressure.wram_hot_bytes.saturating_add(bytes),
            Materialization::Materialize {
                class: StorageClass::HramHot,
                ..
            } => pressure.hram_hot_bytes = pressure.hram_hot_bytes.saturating_add(bytes),
            Materialization::Materialize {
                class: StorageClass::SramPaged,
                ..
            } => pressure.sram_paged_bytes = pressure.sram_paged_bytes.saturating_add(bytes),
            Materialization::Recompute
            | Materialization::Persist { .. }
            | Materialization::Materialize { .. } => {}
        }
    }

    pressure
}

fn repair_proposals(
    input: &StoragePlanCoreInput,
    result: &StoragePlanCoreResult,
    pressure: StoragePlanAbstractPressure,
) -> StoragePlanCoreDiagnosticResult<Vec<RepairProposal>> {
    let Some(soft_threshold_bytes) = input.repair_policy.soft_pressure_threshold_bytes else {
        return Ok(Vec::new());
    };
    if input.repair_policy.storage_recompute_promotion_locked
        || !pressure.exceeds_soft_threshold(soft_threshold_bytes)
        || input.repair_policy.recompute_promotion >= StorageMaterialization::RecomputePureValues
    {
        return Ok(Vec::new());
    }

    let candidates = recompute_repair_candidates(input, result);
    if candidates.is_empty() {
        return Ok(Vec::new());
    }

    if input.repair_policy.max_recompute_promotion < StorageMaterialization::RecomputePureValues {
        return Err(Box::new(repair_proposal_illegal_detail(
            "proposal-1",
            "PromoteRecomputeLevel",
            "max_recompute_promotion",
        )));
    }

    let estimated_cost = estimated_recompute_cost(input, &candidates);
    let change = if candidates.len() == 1 {
        KnobDelta::ForceRecompute { values: candidates }
    } else {
        KnobDelta::PromoteRecomputeLevel {
            to: StorageMaterialization::RecomputePureValues,
        }
    };

    Ok(vec![RepairProposal {
        source: PlanningStage::StoragePlan,
        reason: RepairReason::PromoteRecompute,
        tighten: ConstraintDelta {
            changes: vec![change],
        },
        estimated_cost,
        proposal_id: 1,
    }])
}

fn recompute_repair_candidates(
    input: &StoragePlanCoreInput,
    result: &StoragePlanCoreResult,
) -> Vec<ValueId> {
    result
        .bindings
        .iter()
        .filter_map(|(value, binding)| {
            (binding.justification == BindingJustification::DecisionRule(DecisionRuleId(15))
                && matches!(
                    binding.materialization,
                    Materialization::Materialize {
                        class: StorageClass::WramHot | StorageClass::SramPaged,
                        lifetime: LifetimeClass::Slice,
                    }
                )
                && recompute_allowed(&input.predicate_env, *value)
                && recompute_cost_within_cycle_ceiling(&input.predicate_env, *value))
            .then_some(*value)
        })
        .collect()
}

fn estimated_recompute_cost(
    input: &StoragePlanCoreInput,
    values: &[ValueId],
) -> EstimatedCostDelta {
    EstimatedCostDelta {
        cycles: values
            .iter()
            .map(|value| recompute_cost_estimate(&input.predicate_env, *value).map(u64::from))
            .sum(),
        bytes: Some(
            values
                .iter()
                .map(|value| {
                    u64::from(logical_size_of(&input.predicate_env, *value).unwrap_or_default())
                })
                .sum(),
        ),
    }
}

fn repair_proposal_illegal_detail(
    proposal_id: &str,
    delta: &str,
    locks_bounds: &str,
) -> StoragePlanCoreDiagnosticDetail {
    StoragePlanCoreDiagnosticDetail {
        code: StoragePlanDiagnosticCode::StorageRepairProposalIllegal,
        provenance: StoragePlanDiagnosticProvenance::RepairProposal {
            proposal_id: proposal_id.to_owned(),
            delta: delta.to_owned(),
            locks_bounds: locks_bounds.to_owned(),
        },
        evidence: vec![EvidenceRef {
            kind: "StoragePlanRepairProposal".to_owned(),
            reference: format!("{proposal_id}.{delta}.{locks_bounds}"),
            hash: Some(Hash256::ZERO),
        }],
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
        expected_input_hashes: input.expected_input_hashes,
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

fn decision_rule_success_evidence(rule: DecisionRuleId) -> Vec<EvidenceRef> {
    vec![EvidenceRef {
        kind: "StoragePlanDecisionRule".to_owned(),
        reference: format!("DR-{}.fired", rule.0),
        hash: Some(Hash256::ZERO),
    }]
}

fn storage_plan_diagnostic_details_from_validation(
    diagnostics: &[gbf_policy::ValidationDiagnostic],
) -> Vec<StoragePlanCoreDiagnosticDetail> {
    diagnostics
        .iter()
        .filter_map(|diagnostic| match &diagnostic.code {
            gbf_policy::ValidationCode::StoragePlan { code, provenance } => {
                Some(StoragePlanCoreDiagnosticDetail {
                    code: *code,
                    provenance: provenance.clone(),
                    evidence: diagnostic.provenance.clone(),
                })
            }
            _ => None,
        })
        .collect()
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
    repair_proposals: Vec<RepairProposal>,
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
            repair_proposals: result.repair_proposals.clone(),
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
    use gbf_foundation::Hash256;
    use gbf_policy::{StoragePlanDiagnosticProvenance, ValidationCode};

    use super::*;
    use crate::s1::quant_graph::DeterminismClass;
    use crate::storage_plan::diagnostics::storage_plan_diagnostic;
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
    fn boundary_hash_mismatch_reports_all_drift_in_spec_order() {
        let mut input = fixture_input(BTreeSet::new(), false);
        input.input_identity.range_plan_hash = Hash256::from_bytes([0x44; 32]);
        input.input_identity.policy_hash = Hash256::from_bytes([0x55; 32]);

        let output = build_storage_plan_core(&input);

        assert_eq!(output.outcome, StoragePlanCoreOutcome::Failed);
        assert!(output.result.is_none());
        assert!(output.summary.is_none());
        assert_eq!(
            output.diagnostics,
            vec![
                StoragePlanDiagnosticCode::StorageRangePlanHashMismatch,
                StoragePlanDiagnosticCode::StoragePolicyHashMismatch,
            ]
        );
        assert_eq!(output.diagnostic_details.len(), 2);
        match &output.diagnostic_details[0].provenance {
            StoragePlanDiagnosticProvenance::HashMismatch {
                product,
                recorded,
                computed,
            } => {
                assert_eq!(product, "range_plan");
                assert_eq!(*recorded, Hash256::from_bytes([0x44; 32]));
                assert_eq!(*computed, input.expected_input_hashes.range_plan_hash);
            }
            other => panic!("expected range hash mismatch, got {other:?}"),
        }
        match &output.diagnostic_details[1].provenance {
            StoragePlanDiagnosticProvenance::HashMismatch {
                product,
                recorded,
                computed,
            } => {
                assert_eq!(product, "policy");
                assert_eq!(*recorded, Hash256::from_bytes([0x55; 32]));
                assert_eq!(*computed, input.expected_input_hashes.policy_hash);
            }
            other => panic!("expected policy hash mismatch, got {other:?}"),
        }
    }

    #[test]
    fn registry_provenance_uses_non_stub_rule_for_expert_weights() {
        let mut input = fixture_input(BTreeSet::new(), false);
        let expert = ValueId::new(9);
        input.values = vec![StoragePlanCoreValue {
            value: expert,
            materialization: Materialization::Materialize {
                class: StorageClass::RomConst,
                lifetime: LifetimeClass::Persistent,
            },
            live_range: live_range(2, 3),
            role: ValueRole::ExpertWeight,
            persist_kind: None,
            commit_group_reason: None,
        }];
        input.alias_edges.clear();
        input.predicate_env = PredicateEnv::new().with_value(
            expert,
            PredicateValueFacts::new(
                ValueRole::ExpertWeight,
                ValueFormat::ConstTensorRef {
                    tensor_id: crate::s1::quant_graph::TensorId::new(10),
                },
            ),
        );

        let output = build_storage_plan_core(&input);
        let result = output.result.expect("driver succeeds");
        let provenance = &result.provenance.bindings[&expert];

        assert_eq!(provenance.decision_rule, DecisionRuleId(8));
        assert_eq!(provenance.admitting_predicate, AdmittingPredicateId(8));
        assert_ne!(provenance.decision_rule, DecisionRuleId(15));
        assert!(
            provenance
                .evidence
                .iter()
                .any(|evidence| evidence.reference == "DR-8.fired")
        );
    }

    #[test]
    fn validation_diagnostic_details_preserve_validator_order() {
        let diagnostics = vec![
            storage_plan_diagnostic(
                StoragePlanDiagnosticCode::StoragePolicyHashMismatch,
                StoragePlanDiagnosticProvenance::HashMismatch {
                    product: "policy".to_owned(),
                    recorded: Hash256::from_bytes([0x55; 32]),
                    computed: Hash256::from_bytes([0x05; 32]),
                },
                vec![EvidenceRef {
                    kind: "StoragePlanInputHash".to_owned(),
                    reference: "policy.first".to_owned(),
                    hash: None,
                }],
            )
            .expect("policy diagnostic"),
            storage_plan_diagnostic(
                StoragePlanDiagnosticCode::StorageRangePlanHashMismatch,
                StoragePlanDiagnosticProvenance::HashMismatch {
                    product: "range_plan".to_owned(),
                    recorded: Hash256::from_bytes([0x44; 32]),
                    computed: Hash256::from_bytes([0x04; 32]),
                },
                vec![EvidenceRef {
                    kind: "StoragePlanInputHash".to_owned(),
                    reference: "range.second".to_owned(),
                    hash: None,
                }],
            )
            .expect("range diagnostic"),
        ];

        let details = storage_plan_diagnostic_details_from_validation(&diagnostics);

        assert_eq!(
            details.iter().map(|detail| detail.code).collect::<Vec<_>>(),
            vec![
                StoragePlanDiagnosticCode::StoragePolicyHashMismatch,
                StoragePlanDiagnosticCode::StorageRangePlanHashMismatch,
            ]
        );
        assert_eq!(details[0].evidence[0].reference, "policy.first");
        assert_eq!(details[1].evidence[0].reference, "range.second");
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

    #[test]
    fn repair_proposal_promotes_recompute_when_soft_pressure_exceeds_threshold() {
        let input = repair_proposal_input(false, StorageMaterialization::RecomputePureValues);

        let output = build_storage_plan_core(&input);

        assert_eq!(output.outcome, StoragePlanCoreOutcome::Succeeded);
        let summary = output.summary.expect("summary");
        assert_eq!(summary.abstract_pressure.wram_hot_bytes, 12);
        let result = output.result.expect("driver succeeds");
        assert_eq!(result.repair_proposals.len(), 1);
        let proposal = &result.repair_proposals[0];
        assert_eq!(proposal.source, PlanningStage::StoragePlan);
        assert_eq!(proposal.reason, RepairReason::PromoteRecompute);
        assert_eq!(
            proposal.tighten.changes,
            vec![KnobDelta::PromoteRecomputeLevel {
                to: StorageMaterialization::RecomputePureValues
            }]
        );
        assert_eq!(
            proposal.estimated_cost,
            EstimatedCostDelta {
                cycles: Some(21),
                bytes: Some(12),
            }
        );
    }

    #[test]
    fn repair_proposal_force_recompute_single_candidate_keeps_cost_axes_separate() {
        let mut input = repair_proposal_input(false, StorageMaterialization::RecomputePureValues);
        let env = std::mem::take(&mut input.predicate_env)
            .with_recompute_cost_estimate(ValueId::new(2), 17)
            .with_recompute_cost_estimate(ValueId::new(3), 19);
        input.predicate_env = env;

        let output = build_storage_plan_core(&input);

        assert_eq!(output.outcome, StoragePlanCoreOutcome::Succeeded);
        let result = output.result.expect("driver succeeds");
        assert_eq!(result.repair_proposals.len(), 1);
        let proposal = &result.repair_proposals[0];
        assert_eq!(proposal.source, PlanningStage::StoragePlan);
        assert!(proposal.tighten.changes.iter().all(|change| matches!(
            change,
            KnobDelta::ForceRecompute { .. } | KnobDelta::PromoteRecomputeLevel { .. }
        )));
        assert_eq!(
            proposal.tighten.changes,
            vec![KnobDelta::ForceRecompute {
                values: vec![ValueId::new(1)]
            }]
        );
        assert_eq!(proposal.estimated_cost.cycles, Some(3));
        assert_eq!(proposal.estimated_cost.bytes, Some(4));
    }

    #[test]
    fn locked_recompute_promotion_emits_no_repair_proposal() {
        let input = repair_proposal_input(true, StorageMaterialization::RecomputePureValues);

        let output = build_storage_plan_core(&input);

        assert_eq!(output.outcome, StoragePlanCoreOutcome::Succeeded);
        assert!(
            output
                .result
                .expect("driver succeeds")
                .repair_proposals
                .is_empty()
        );
    }

    #[test]
    fn repair_proposal_bound_violation_fails_store_027() {
        let input = repair_proposal_input(false, StorageMaterialization::PreserveAll);

        let output = build_storage_plan_core(&input);

        assert_eq!(output.outcome, StoragePlanCoreOutcome::Failed);
        assert_eq!(
            output.diagnostics,
            vec![StoragePlanDiagnosticCode::StorageRepairProposalIllegal]
        );
        match &output.diagnostic_details[0].provenance {
            StoragePlanDiagnosticProvenance::RepairProposal {
                proposal_id,
                delta,
                locks_bounds,
            } => {
                assert_eq!(proposal_id, "proposal-1");
                assert_eq!(delta, "PromoteRecomputeLevel");
                assert_eq!(locks_bounds, "max_recompute_promotion");
            }
            other => panic!("expected repair proposal provenance, got {other:?}"),
        }
    }

    fn repair_proposal_input(
        locked: bool,
        max_recompute_promotion: StorageMaterialization,
    ) -> StoragePlanCoreInput {
        let mut input = fixture_input(BTreeSet::new(), false);
        input.repair_policy = StoragePlanRepairPolicy {
            soft_pressure_threshold_bytes: Some(1),
            recompute_promotion: StorageMaterialization::PreserveAll,
            max_recompute_promotion,
            storage_recompute_promotion_locked: locked,
        };
        let env = std::mem::take(&mut input.predicate_env)
            .with_recompute_cycle_ceiling(16)
            .with_recompute_cost_estimate(ValueId::new(1), 3)
            .with_recompute_cost_estimate(ValueId::new(2), 7)
            .with_recompute_cost_estimate(ValueId::new(3), 11);
        input.predicate_env = env;
        input
    }

    fn fixture_input(forced: BTreeSet<ValueId>, fail_before_result: bool) -> StoragePlanCoreInput {
        let mut env = PredicateEnv::new().with_wram_hot_per_value_eligibility_ceiling(32);
        for value in [1, 2, 3] {
            env = env.with_value(ValueId::new(value), activation_facts());
        }
        StoragePlanCoreInput {
            input_identity: identity(),
            expected_input_hashes: input_hashes(),
            repair_policy: Default::default(),
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
        let mut facts = PredicateValueFacts::new(
            ValueRole::Activation,
            ValueFormat::QuantInt {
                quant_format_id: QuantFormatId(1),
            },
        );
        facts.logical_size = Some(4);
        facts
    }

    fn identity() -> StoragePlanInputIdentity {
        StoragePlanInputIdentity::new(input_hashes(), DeterminismClass::Deterministic)
    }

    fn input_hashes() -> StoragePlanInputHashes {
        StoragePlanInputHashes {
            quant_graph_hash: Hash256::from_bytes([1; 32]),
            infer_ir_hash: Hash256::from_bytes([2; 32]),
            observation_plan_hash: Hash256::from_bytes([3; 32]),
            range_plan_hash: Hash256::from_bytes([4; 32]),
            policy_hash: Hash256::from_bytes([5; 32]),
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
