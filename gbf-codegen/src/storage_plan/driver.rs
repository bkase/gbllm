//! Pure Stage 6 construction-order driver over normalized storage facts.

use std::collections::{BTreeMap, BTreeSet};

use gbf_foundation::{CanonicalJson, CanonicalJsonError};
use gbf_policy::StoragePlanDiagnosticCode;
use serde::Serialize;

use crate::s3::infer_ir::ValueId;
use crate::storage_plan::alias_engine::{
    AliasCandidateEdge, AliasSeedBinding, build_alias_classes,
};
use crate::storage_plan::persist::{PersistBindingInput, resolve_persist_bindings};
use crate::storage_plan::predicates::{PredicateEnv, ValueRole, recompute_allowed};
use crate::storage_plan::types::{
    AbstractLiveRange, AliasClass, AliasClassFingerprint, AliasClassId, AliasIntent,
    BindingJustification, CommitGroupDecl, CommitGroupId, CommitGroupReason, DecisionRuleId,
    Materialization, PersistKind, PersistPageDecl, PersistPageId, StorageBinding,
    StoragePlanInputIdentity,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoragePlanCoreInput {
    pub input_identity: StoragePlanInputIdentity,
    pub predicate_env: PredicateEnv,
    pub values: Vec<StoragePlanCoreValue>,
    pub alias_edges: Vec<AliasCandidateEdge>,
    pub alias_forced_recompute_values: BTreeSet<ValueId>,
    pub fail_before_result: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
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
                return failed_output(
                    input.input_identity.clone(),
                    StoragePlanDiagnosticCode::StorageForcedRecomputeNotAllowed,
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

    let result = StoragePlanCoreResult {
        bindings,
        alias_classes,
        persist_pages: persist_resolution.persist_pages,
        commit_groups: persist_resolution.commit_groups,
    };
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
    let order = input
        .values
        .iter()
        .flat_map(|value| {
            [
                value.live_range.def_node,
                value
                    .live_range
                    .last_use_node
                    .unwrap_or(value.live_range.def_node),
            ]
        })
        .collect::<Vec<_>>();
    build_alias_classes(&seeds, &input.alias_edges, &order)
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

#[derive(Serialize)]
struct SerializableOutput {
    input_identity: StoragePlanInputIdentity,
    outcome: StoragePlanCoreOutcome,
    result: Option<SerializableResult>,
    summary: Option<StoragePlanCoreSummary>,
    diagnostics: Vec<String>,
}

#[derive(Serialize)]
struct SerializableResult {
    bindings: BTreeMap<String, SerializableBinding>,
    alias_classes: BTreeMap<String, SerializableAliasClass>,
    persist_pages: BTreeMap<String, PersistPageDecl>,
    commit_groups: BTreeMap<String, CommitGroupDecl>,
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
        }
    }
}

#[cfg(test)]
mod tests {
    use gbf_foundation::{Hash256, SemVer};
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
    fn failure_path_has_no_partial_result_or_summary() {
        let output = build_storage_plan_core(&fixture_input(BTreeSet::new(), true));

        assert_eq!(output.outcome, StoragePlanCoreOutcome::Failed);
        assert!(output.result.is_none());
        assert!(output.summary.is_none());
    }

    fn fixture_input(forced: BTreeSet<ValueId>, fail_before_result: bool) -> StoragePlanCoreInput {
        let mut env = PredicateEnv::new();
        for value in [1, 2, 3] {
            env = env.with_value(ValueId::new(value), activation_facts());
        }
        StoragePlanCoreInput {
            input_identity: identity(),
            predicate_env: env,
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
}
