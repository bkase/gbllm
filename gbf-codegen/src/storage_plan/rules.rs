//! Decision-rule registry and execution engine for Stage 6 storage planning.

use gbf_foundation::{CanonicalJson, CanonicalJsonError};
use gbf_policy::StoragePlanDiagnosticCode;
use serde::{Deserialize, Serialize};

use crate::s3::infer_ir::ValueId;
use crate::storage_plan::predicates::{
    PredicateEnv, ValueRole, effective_lifetime_estimate, exceeds_transcript_inline_ceiling,
    format_known, is_const_tensor_ref_value, is_continuation_live_value, is_expert_weight,
    is_forced_recompute_value, is_hot_scalar, is_observed_checkpoint_backing_value,
    is_precomputed_hram_admitted, is_pure_value, is_renorm_loop_scratch, is_router_table,
    is_sequence_state_slot, is_trace_probe_attached_value, logical_size_known, logical_size_of,
    must_survive_power_loss, participates_in_harness_io, participates_in_resume_across_yield,
    precomputed_hram_admission_exceeds_budget, predicate_wram_hot_per_value_eligibility_ceiling,
    recompute_allowed, recompute_cost_within_cycle_ceiling,
    recompute_promotion_admits_pure_slice_values, role_known,
    storage_predicate_transcript_capture_enabled, trace_capture_admits_value, value_role_of,
};
use crate::storage_plan::types::{
    BindingJustification, CommitGroupId, DecisionRuleId, LifetimeClass, Materialization,
    PersistPageId, StorageClass,
};

pub const DECISION_RULE_FIRED_EVENT: &str = "f_b8.rule.fired";
pub const DECISION_RULE_REJECTED_EVENT: &str = "f_b8.rule.rejected";
pub const DECISION_RULE_SET_MANIFEST_SCHEMA: &str = "storage_plan.decision_rule_set.v1";
pub const DECISION_RULE_SET_RFC_REVISION: &str = "F-B8-v2";
pub type DecisionRuleDiagnosticCode = StoragePlanDiagnosticCode;

#[derive(Clone, Copy)]
pub struct DecisionRule {
    pub id: DecisionRuleId,
    pub name: &'static str,
    pub predicate: fn(&PredicateEnv, ValueId) -> bool,
    pub outcome: fn(&PredicateEnv, ValueId) -> DecisionRuleOutcome,
    pub priority: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum DecisionRuleOutcome {
    Bind(Materialization),
    Reject(DecisionRuleDiagnosticCode),
}

impl DecisionRuleOutcome {
    #[must_use]
    pub const fn trace_kind(&self) -> &'static str {
        match self {
            Self::Bind(_) => "Bind",
            Self::Reject(_) => "Reject",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecisionRuleEvaluation {
    Fired(DecisionRuleFiring),
    NoAdmittingRule {
        value: ValueId,
        code: DecisionRuleDiagnosticCode,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionRuleFiring {
    pub value: ValueId,
    pub rule_id: DecisionRuleId,
    pub rule_name: &'static str,
    pub priority: u32,
    pub outcome: DecisionRuleOutcome,
}

impl DecisionRuleFiring {
    #[must_use]
    pub fn binding_justification(&self) -> Option<BindingJustification> {
        match &self.outcome {
            DecisionRuleOutcome::Bind(_) if self.rule_id == DecisionRuleId(1) => {
                Some(BindingJustification::ForcedRecompute)
            }
            DecisionRuleOutcome::Bind(_) => Some(BindingJustification::DecisionRule(self.rule_id)),
            DecisionRuleOutcome::Reject(_) => None,
        }
    }

    #[must_use]
    pub fn provenance(&self, env: &PredicateEnv) -> DecisionRuleFiringProvenance {
        DecisionRuleFiringProvenance {
            decision_rule: self.rule_id,
            op_output_role: value_role_of(env, self.value),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionRuleFiringProvenance {
    pub decision_rule: DecisionRuleId,
    pub op_output_role: Option<ValueRole>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionRuleSetManifest {
    pub schema: String,
    pub rfc_revision: String,
    pub rules: Vec<DecisionRuleManifestEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionRuleManifestEntry {
    pub id: DecisionRuleId,
    pub name: String,
    pub priority: u32,
    pub semantic_predicate_id: String,
    pub semantic_outcome_id: String,
    pub rfc_revision: String,
}

pub static DECISION_RULES: &[DecisionRule] = &[
    DecisionRule {
        id: DecisionRuleId(1),
        name: "DR-1 ForcedRecomputeValueOverride",
        predicate: predicate_forced_recompute_allowed,
        outcome: outcome_recompute,
        priority: 10,
    },
    DecisionRule {
        id: DecisionRuleId(2),
        name: "DR-1b IllegalForcedRecomputeValueOverride",
        predicate: predicate_forced_recompute_disallowed,
        outcome: outcome_forced_recompute_reject,
        priority: 20,
    },
    DecisionRule {
        id: DecisionRuleId(3),
        name: "DR-2 PersistSequenceStateSlot",
        predicate: predicate_sequence_state_slot,
        outcome: outcome_sequence_state_reserved_v1,
        priority: 30,
    },
    DecisionRule {
        id: DecisionRuleId(4),
        name: "DR-3a MaterializeResumeContinuation",
        predicate: predicate_materialize_resume_continuation,
        outcome: outcome_wram_resume_window,
        priority: 40,
    },
    DecisionRule {
        id: DecisionRuleId(5),
        name: "DR-3 PersistContinuationRecord",
        predicate: predicate_persist_continuation_record,
        outcome: outcome_persist_continuation_record,
        priority: 50,
    },
    DecisionRule {
        id: DecisionRuleId(6),
        name: "DR-4 PersistTranscriptPage",
        predicate: predicate_persist_transcript_page,
        outcome: outcome_persist_transcript_page,
        priority: 60,
    },
    DecisionRule {
        id: DecisionRuleId(7),
        name: "DR-5 PersistHarnessOrTracePage",
        predicate: predicate_persist_harness_or_trace_page,
        outcome: outcome_persist_harness_or_trace_page,
        priority: 70,
    },
    DecisionRule {
        id: DecisionRuleId(8),
        name: "DR-6 RomConstExpertWeight",
        predicate: predicate_rom_const_expert_weight,
        outcome: outcome_rom_const_persistent,
        priority: 80,
    },
    DecisionRule {
        id: DecisionRuleId(9),
        name: "DR-7 RomConstRouterTable",
        predicate: predicate_rom_const_router_table,
        outcome: outcome_rom_const_persistent,
        priority: 90,
    },
    DecisionRule {
        id: DecisionRuleId(10),
        name: "DR-8 RomConstLut",
        predicate: predicate_rom_const_lut,
        outcome: outcome_rom_const_persistent,
        priority: 100,
    },
    DecisionRule {
        id: DecisionRuleId(11),
        name: "DR-9 RomConstEmbeddingOrLogitProj",
        predicate: predicate_rom_const_embedding_or_logit_proj,
        outcome: outcome_rom_const_persistent,
        priority: 110,
    },
    DecisionRule {
        id: DecisionRuleId(12),
        name: "DR-10 RenormLoopScratch",
        predicate: predicate_renorm_loop_scratch,
        outcome: outcome_wram_slice,
        priority: 120,
    },
    DecisionRule {
        id: DecisionRuleId(13),
        name: "DR-11 RecomputeForPureSliceValue",
        predicate: predicate_recompute_for_pure_slice_value,
        outcome: outcome_recompute,
        priority: 130,
    },
    DecisionRule {
        id: DecisionRuleId(14),
        name: "DR-12 HotScalarHram",
        predicate: predicate_hot_scalar_hram,
        outcome: outcome_hram_slice,
        priority: 140,
    },
    DecisionRule {
        id: DecisionRuleId(15),
        name: "DR-13 DefaultMaterializeKnownIntermediate",
        predicate: predicate_default_materialize_known_intermediate,
        outcome: outcome_default_materialize_known_intermediate,
        priority: 150,
    },
];

#[must_use]
pub fn evaluate_registry(env: &PredicateEnv, value: ValueId) -> DecisionRuleEvaluation {
    evaluate_decision_rules(DECISION_RULES, env, value)
}

#[must_use]
pub fn evaluate_decision_rules(
    rules: &[DecisionRule],
    env: &PredicateEnv,
    value: ValueId,
) -> DecisionRuleEvaluation {
    for rule in rules {
        if (rule.predicate)(env, value) {
            let outcome = (rule.outcome)(env, value);
            emit_rule_event(rule, value, &outcome);
            return DecisionRuleEvaluation::Fired(DecisionRuleFiring {
                value,
                rule_id: rule.id,
                rule_name: rule.name,
                priority: rule.priority,
                outcome,
            });
        }
    }

    let code = DecisionRuleDiagnosticCode::StorageNoAdmittingDecisionRule;
    tracing::error!(
        target: "gbf_codegen::storage_plan",
        event = DECISION_RULE_REJECTED_EVENT,
        value_id = value.get() as u64,
        diagnostic_code = code.as_str(),
        "storage decision rejected"
    );
    DecisionRuleEvaluation::NoAdmittingRule { value, code }
}

#[must_use]
pub fn decision_rule_set_manifest() -> DecisionRuleSetManifest {
    DecisionRuleSetManifest {
        schema: DECISION_RULE_SET_MANIFEST_SCHEMA.to_owned(),
        rfc_revision: DECISION_RULE_SET_RFC_REVISION.to_owned(),
        rules: DECISION_RULES
            .iter()
            .map(decision_rule_manifest_entry)
            .collect(),
    }
}

pub fn decision_rule_set_manifest_canonical_json() -> Result<Vec<u8>, CanonicalJsonError> {
    CanonicalJson::to_vec(&decision_rule_set_manifest())
}

fn decision_rule_manifest_entry(rule: &DecisionRule) -> DecisionRuleManifestEntry {
    DecisionRuleManifestEntry {
        id: rule.id,
        name: rule.name.to_owned(),
        priority: rule.priority,
        semantic_predicate_id: semantic_predicate_id(rule.id).to_owned(),
        semantic_outcome_id: semantic_outcome_id(rule.id).to_owned(),
        rfc_revision: DECISION_RULE_SET_RFC_REVISION.to_owned(),
    }
}

fn emit_rule_event(rule: &DecisionRule, value: ValueId, outcome: &DecisionRuleOutcome) {
    match outcome {
        DecisionRuleOutcome::Bind(_) => {
            tracing::info!(
                target: "gbf_codegen::storage_plan",
                event = DECISION_RULE_FIRED_EVENT,
                rule_id = rule.id.0 as u64,
                rule_name = rule.name,
                priority = rule.priority as u64,
                value_id = value.get() as u64,
                outcome = outcome.trace_kind(),
                "storage decision rule fired"
            );
        }
        DecisionRuleOutcome::Reject(code) => {
            tracing::error!(
                target: "gbf_codegen::storage_plan",
                event = DECISION_RULE_REJECTED_EVENT,
                rule_id = rule.id.0 as u64,
                rule_name = rule.name,
                priority = rule.priority as u64,
                value_id = value.get() as u64,
                outcome = outcome.trace_kind(),
                diagnostic_code = code.as_str(),
                "storage decision rejected"
            );
        }
    }
}

fn semantic_predicate_id(id: DecisionRuleId) -> &'static str {
    match id.0 {
        1 => "ForcedRecomputeValueOverride",
        2 => "IllegalForcedRecomputeValueOverride",
        3 => "PersistSequenceStateSlot",
        4 => "MaterializeResumeContinuation",
        5 => "PersistContinuationRecord",
        6 => "PersistTranscriptPage",
        7 => "PersistHarnessOrTracePage",
        8 => "RomConstExpertWeight",
        9 => "RomConstRouterTable",
        10 => "RomConstLut",
        11 => "RomConstEmbeddingOrLogitProj",
        12 => "RenormLoopScratch",
        13 => "RecomputeForPureSliceValue",
        14 => "HotScalarHramAdmitted",
        15 => "DefaultMaterializeKnownIntermediate",
        _ => "UnknownRule",
    }
}

fn semantic_outcome_id(id: DecisionRuleId) -> &'static str {
    match id.0 {
        1 | 13 => "BindRecompute",
        2 => "RejectStore006",
        3 => "RejectStore007",
        4 => "BindWramHotResumeWindow",
        5..=7 => "BindPersist",
        8..=11 => "BindRomConstPersistent",
        12 => "BindWramHotSlice",
        14 => "BindHramHotSlice",
        15 => "BindDefaultMaterializeKnownIntermediate",
        _ => "UnknownOutcome",
    }
}

fn predicate_forced_recompute_allowed(env: &PredicateEnv, value: ValueId) -> bool {
    is_forced_recompute_value(env, value) && recompute_allowed(env, value)
}

fn predicate_forced_recompute_disallowed(env: &PredicateEnv, value: ValueId) -> bool {
    is_forced_recompute_value(env, value) && !recompute_allowed(env, value)
}

fn predicate_sequence_state_slot(env: &PredicateEnv, value: ValueId) -> bool {
    is_sequence_state_slot(env, value)
}

fn predicate_materialize_resume_continuation(env: &PredicateEnv, value: ValueId) -> bool {
    value_role_of(env, value) == Some(ValueRole::Activation)
        && participates_in_resume_across_yield(env, value)
        && !must_survive_power_loss(env, value)
}

fn predicate_persist_continuation_record(env: &PredicateEnv, value: ValueId) -> bool {
    value_role_of(env, value) == Some(ValueRole::Activation)
        && is_continuation_live_value(env, value)
        && participates_in_resume_across_yield(env, value)
        && must_survive_power_loss(env, value)
}

fn predicate_persist_transcript_page(env: &PredicateEnv, value: ValueId) -> bool {
    value_role_of(env, value) == Some(ValueRole::OutputToken)
        && storage_predicate_transcript_capture_enabled(env)
        && exceeds_transcript_inline_ceiling(env, value)
}

fn predicate_persist_harness_or_trace_page(env: &PredicateEnv, value: ValueId) -> bool {
    let harness_clause = matches!(
        value_role_of(env, value),
        Some(ValueRole::InputToken | ValueRole::OutputToken)
    ) && participates_in_harness_io(env, value);
    let trace_clause =
        is_trace_probe_attached_value(env, value) && trace_capture_admits_value(env, value);

    harness_clause || trace_clause
}

fn predicate_rom_const_expert_weight(env: &PredicateEnv, value: ValueId) -> bool {
    is_expert_weight(env, value)
}

fn predicate_rom_const_router_table(env: &PredicateEnv, value: ValueId) -> bool {
    is_router_table(env, value) && value_role_of(env, value) == Some(ValueRole::RouterWeight)
}

fn predicate_rom_const_lut(env: &PredicateEnv, value: ValueId) -> bool {
    value_role_of(env, value) == Some(ValueRole::LutFragment)
}

fn predicate_rom_const_embedding_or_logit_proj(env: &PredicateEnv, value: ValueId) -> bool {
    matches!(
        value_role_of(env, value),
        Some(
            ValueRole::EmbeddingTable
                | ValueRole::LogitProj
                | ValueRole::NormParam
                | ValueRole::DecodeConst
        )
    )
}

fn predicate_renorm_loop_scratch(env: &PredicateEnv, value: ValueId) -> bool {
    is_renorm_loop_scratch(env, value)
}

fn predicate_recompute_for_pure_slice_value(env: &PredicateEnv, value: ValueId) -> bool {
    is_pure_value(env, value)
        && !is_renorm_loop_scratch(env, value)
        && recompute_allowed(env, value)
        && recompute_promotion_admits_pure_slice_values(env)
        && recompute_cost_within_cycle_ceiling(env, value)
}

fn predicate_hot_scalar_hram(env: &PredicateEnv, value: ValueId) -> bool {
    is_hot_scalar(env, value) && is_precomputed_hram_admitted(env, value)
}

fn predicate_default_materialize_known_intermediate(env: &PredicateEnv, value: ValueId) -> bool {
    role_known(env, value)
        && format_known(env, value)
        && logical_size_known(env, value)
        && predicate_wram_hot_per_value_eligibility_ceiling(env).is_some()
        && !matches!(
            value_role_of(env, value),
            Some(
                ValueRole::SequenceStateSlot
                    | ValueRole::RouterWeight
                    | ValueRole::ExpertWeight
                    | ValueRole::EmbeddingTable
                    | ValueRole::LogitProj
                    | ValueRole::NormParam
                    | ValueRole::DecodeConst
                    | ValueRole::LutFragment
            )
        )
}

fn outcome_recompute(_env: &PredicateEnv, _value: ValueId) -> DecisionRuleOutcome {
    DecisionRuleOutcome::Bind(Materialization::Recompute)
}

fn outcome_forced_recompute_reject(env: &PredicateEnv, value: ValueId) -> DecisionRuleOutcome {
    if is_observed_checkpoint_backing_value(env, value) {
        outcome_reject_store_006(env, value)
    } else {
        DecisionRuleOutcome::Reject(DecisionRuleDiagnosticCode::StorageForcedRecomputeNotAllowed)
    }
}

fn outcome_reject_store_006(_env: &PredicateEnv, _value: ValueId) -> DecisionRuleOutcome {
    DecisionRuleOutcome::Reject(
        DecisionRuleDiagnosticCode::StorageRecomputeForbiddenForObservedValue,
    )
}

fn outcome_reject_store_007(_env: &PredicateEnv, _value: ValueId) -> DecisionRuleOutcome {
    DecisionRuleOutcome::Reject(
        DecisionRuleDiagnosticCode::StoragePersistSequenceStateUnsupportedV1,
    )
}

fn outcome_sequence_state_reserved_v1(env: &PredicateEnv, value: ValueId) -> DecisionRuleOutcome {
    outcome_reject_store_007(env, value)
}

fn outcome_wram_resume_window(_env: &PredicateEnv, _value: ValueId) -> DecisionRuleOutcome {
    DecisionRuleOutcome::Bind(Materialization::Materialize {
        class: StorageClass::WramHot,
        lifetime: LifetimeClass::ResumeWindow,
    })
}

fn outcome_persist_continuation_record(_env: &PredicateEnv, value: ValueId) -> DecisionRuleOutcome {
    DecisionRuleOutcome::Bind(Materialization::Persist {
        page: persist_page(0x2000_0000, value),
        commit_group: commit_group(0x2000_0000, value),
    })
}

fn outcome_persist_transcript_page(_env: &PredicateEnv, value: ValueId) -> DecisionRuleOutcome {
    DecisionRuleOutcome::Bind(Materialization::Persist {
        page: persist_page(0x3000_0000, value),
        commit_group: commit_group(0x3000_0000, value),
    })
}

fn outcome_persist_harness_or_trace_page(
    env: &PredicateEnv,
    value: ValueId,
) -> DecisionRuleOutcome {
    let base = if matches!(
        value_role_of(env, value),
        Some(ValueRole::InputToken | ValueRole::OutputToken)
    ) && participates_in_harness_io(env, value)
    {
        0x4000_0000
    } else {
        0x5000_0000
    };

    DecisionRuleOutcome::Bind(Materialization::Persist {
        page: persist_page(base, value),
        commit_group: commit_group(base, value),
    })
}

fn outcome_rom_const_persistent(env: &PredicateEnv, value: ValueId) -> DecisionRuleOutcome {
    if !is_const_tensor_ref_value(env, value) {
        return DecisionRuleOutcome::Reject(
            DecisionRuleDiagnosticCode::StorageRomConstWriteViolation,
        );
    }

    DecisionRuleOutcome::Bind(Materialization::Materialize {
        class: StorageClass::RomConst,
        lifetime: LifetimeClass::Persistent,
    })
}

fn outcome_wram_slice(_env: &PredicateEnv, _value: ValueId) -> DecisionRuleOutcome {
    DecisionRuleOutcome::Bind(Materialization::Materialize {
        class: StorageClass::WramHot,
        lifetime: LifetimeClass::Slice,
    })
}

fn outcome_hram_slice(env: &PredicateEnv, _value: ValueId) -> DecisionRuleOutcome {
    if precomputed_hram_admission_exceeds_budget(env) {
        return DecisionRuleOutcome::Reject(
            DecisionRuleDiagnosticCode::StorageHramAdmissionInvariantViolation,
        );
    }

    DecisionRuleOutcome::Bind(Materialization::Materialize {
        class: StorageClass::HramHot,
        lifetime: LifetimeClass::Slice,
    })
}

fn outcome_default_materialize_known_intermediate(
    env: &PredicateEnv,
    value: ValueId,
) -> DecisionRuleOutcome {
    let logical_size = logical_size_of(env, value).expect("DR-13 predicate proves size");
    let ceiling = predicate_wram_hot_per_value_eligibility_ceiling(env)
        .expect("DR-13 predicate proves ceiling");
    let class = if logical_size <= ceiling {
        StorageClass::WramHot
    } else {
        StorageClass::SramPaged
    };
    let lifetime = effective_lifetime_estimate(env, value);

    DecisionRuleOutcome::Bind(Materialization::Materialize { class, lifetime })
}

fn persist_page(base: u32, value: ValueId) -> PersistPageId {
    PersistPageId(base.wrapping_add(value.get()))
}

fn commit_group(base: u32, value: ValueId) -> CommitGroupId {
    CommitGroupId(base.wrapping_add(value.get()))
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use crate::storage_plan::predicates::{
        PrecomputedHramAdmittedSet, PredicateValueFacts, RecomputeAllowedFailurePredicate,
        ValueFormat, ValueRole, precompute_hram_admitted_set, recompute_allowed_failure_predicates,
    };
    use gbf_policy::{ReductionSiteId, StorageMaterialization};
    use tracing::field::{Field, Visit};
    use tracing::{Event, Subscriber};
    use tracing_subscriber::Layer;
    use tracing_subscriber::layer::Context;
    use tracing_subscriber::prelude::*;

    use super::*;

    #[test]
    fn registry_order_is_deterministic() {
        let names: Vec<_> = DECISION_RULES.iter().map(|rule| rule.name).collect();
        assert_eq!(
            names,
            vec![
                "DR-1 ForcedRecomputeValueOverride",
                "DR-1b IllegalForcedRecomputeValueOverride",
                "DR-2 PersistSequenceStateSlot",
                "DR-3a MaterializeResumeContinuation",
                "DR-3 PersistContinuationRecord",
                "DR-4 PersistTranscriptPage",
                "DR-5 PersistHarnessOrTracePage",
                "DR-6 RomConstExpertWeight",
                "DR-7 RomConstRouterTable",
                "DR-8 RomConstLut",
                "DR-9 RomConstEmbeddingOrLogitProj",
                "DR-10 RenormLoopScratch",
                "DR-11 RecomputeForPureSliceValue",
                "DR-12 HotScalarHram",
                "DR-13 DefaultMaterializeKnownIntermediate",
            ]
        );
        assert!(
            DECISION_RULES
                .windows(2)
                .all(|pair| pair[0].priority < pair[1].priority)
        );
    }

    #[test]
    fn engine_uses_first_matching_rule() {
        let env = PredicateEnv::new();
        let value = ValueId::new(3);
        let rules = [
            test_rule(DecisionRuleId(100), "first", 1, true, outcome_wram_slice),
            test_rule(DecisionRuleId(101), "second", 2, true, outcome_hram_slice),
        ];

        let decision = evaluate_decision_rules(&rules, &env, value);

        assert!(matches!(
            decision,
            DecisionRuleEvaluation::Fired(DecisionRuleFiring {
                rule_id: DecisionRuleId(100),
                outcome: DecisionRuleOutcome::Bind(Materialization::Materialize {
                    class: StorageClass::WramHot,
                    lifetime: LifetimeClass::Slice
                }),
                ..
            })
        ));
    }

    #[test]
    fn bind_and_reject_outcomes_round_trip_through_engine_and_tracing() {
        let env = PredicateEnv::new();
        let value = ValueId::new(7);
        let capture = CapturedTracingLayer::default();
        let subscriber = tracing_subscriber::registry().with(capture.clone());

        tracing::subscriber::with_default(subscriber, || {
            let bind_rules = [test_rule(
                DecisionRuleId(200),
                "bind",
                1,
                true,
                outcome_wram_slice,
            )];
            let reject_rules = [test_rule(
                DecisionRuleId(201),
                "reject",
                1,
                true,
                outcome_reject_store_006,
            )];

            assert!(matches!(
                evaluate_decision_rules(&bind_rules, &env, value),
                DecisionRuleEvaluation::Fired(DecisionRuleFiring {
                    outcome: DecisionRuleOutcome::Bind(Materialization::Materialize {
                        class: StorageClass::WramHot,
                        lifetime: LifetimeClass::Slice
                    }),
                    ..
                })
            ));
            assert!(matches!(
                evaluate_decision_rules(&reject_rules, &env, value),
                DecisionRuleEvaluation::Fired(DecisionRuleFiring {
                    outcome: DecisionRuleOutcome::Reject(
                        DecisionRuleDiagnosticCode::StorageRecomputeForbiddenForObservedValue
                    ),
                    ..
                })
            ));
        });

        let events = capture.events();
        assert_event(&events, DECISION_RULE_FIRED_EVENT, "rule_name", "bind");
        assert_event(
            &events,
            DECISION_RULE_REJECTED_EVENT,
            "diagnostic_code",
            "STORE-006",
        );
    }

    #[test]
    fn no_matching_rule_reports_store_001() {
        let env = PredicateEnv::new();
        let value = ValueId::new(9);
        let rules = [test_rule(
            DecisionRuleId(300),
            "never",
            1,
            false,
            outcome_wram_slice,
        )];

        assert_eq!(
            evaluate_decision_rules(&rules, &env, value),
            DecisionRuleEvaluation::NoAdmittingRule {
                value,
                code: DecisionRuleDiagnosticCode::StorageNoAdmittingDecisionRule
            }
        );
    }

    #[test]
    fn dr_1_forced_recompute_override_binds_recompute() {
        let value = ValueId::new(1);
        let env = PredicateEnv::new()
            .with_forced_recompute_value(value)
            .with_value(value, activation_facts());

        let decision = evaluate_registry(&env, value);

        match decision {
            DecisionRuleEvaluation::Fired(firing) => {
                assert_eq!(firing.rule_id, DecisionRuleId(1));
                assert_eq!(
                    firing.outcome,
                    DecisionRuleOutcome::Bind(Materialization::Recompute)
                );
                assert_eq!(
                    firing.binding_justification(),
                    Some(BindingJustification::ForcedRecompute)
                );
            }
            other => panic!("expected DR-1 firing, got {other:?}"),
        }
    }

    #[test]
    fn dr_1b_forced_recompute_observed_value_rejects_store_006() {
        let value = ValueId::new(1);
        let env = PredicateEnv::new()
            .with_forced_recompute_value(value)
            .with_observed_checkpoint_backing_value(value)
            .with_value(value, activation_facts());

        let decision = evaluate_registry(&env, value);

        assert!(matches!(
            decision,
            DecisionRuleEvaluation::Fired(DecisionRuleFiring {
                rule_id: DecisionRuleId(2),
                outcome: DecisionRuleOutcome::Reject(
                    DecisionRuleDiagnosticCode::StorageRecomputeForbiddenForObservedValue
                ),
                ..
            })
        ));
    }

    #[test]
    fn dr_1b_forced_recompute_router_decision_rejects_store_033() {
        let value = ValueId::new(1);
        let env = PredicateEnv::new()
            .with_forced_recompute_value(value)
            .with_value(
                value,
                PredicateValueFacts::new(
                    ValueRole::RouterDecision,
                    ValueFormat::TokenIdDomain { vocab_size: 8 },
                ),
            );

        let decision = evaluate_registry(&env, value);
        let failures = recompute_allowed_failure_predicates(&env, value);

        assert!(matches!(
            decision,
            DecisionRuleEvaluation::Fired(DecisionRuleFiring {
                rule_id: DecisionRuleId(2),
                outcome: DecisionRuleOutcome::Reject(
                    DecisionRuleDiagnosticCode::StorageForcedRecomputeNotAllowed
                ),
                ..
            })
        ));
        assert!(failures.contains(&RecomputeAllowedFailurePredicate::ValueRoleOf));
    }

    #[test]
    fn po_22_forced_recompute_precedes_default_rules_or_rejects() {
        let legal = ValueId::new(1);
        let illegal = ValueId::new(2);
        let legal_env = PredicateEnv::new()
            .with_forced_recompute_value(legal)
            .with_value(legal, activation_facts());
        let illegal_env = PredicateEnv::new()
            .with_forced_recompute_value(illegal)
            .with_value(
                illegal,
                PredicateValueFacts::new(
                    ValueRole::RouterDecision,
                    ValueFormat::TokenIdDomain { vocab_size: 8 },
                ),
            );
        let rules = [
            DECISION_RULES[0],
            DECISION_RULES[1],
            test_rule(
                DecisionRuleId(999),
                "default",
                999,
                true,
                outcome_wram_slice,
            ),
        ];

        assert!(matches!(
            evaluate_decision_rules(&rules, &legal_env, legal),
            DecisionRuleEvaluation::Fired(DecisionRuleFiring {
                rule_id: DecisionRuleId(1),
                outcome: DecisionRuleOutcome::Bind(Materialization::Recompute),
                ..
            })
        ));
        assert!(matches!(
            evaluate_decision_rules(&rules, &illegal_env, illegal),
            DecisionRuleEvaluation::Fired(DecisionRuleFiring {
                rule_id: DecisionRuleId(2),
                outcome: DecisionRuleOutcome::Reject(
                    DecisionRuleDiagnosticCode::StorageForcedRecomputeNotAllowed
                ),
                ..
            })
        ));
    }

    #[test]
    fn dr_2_sequence_state_slot_is_a_reserved_v1_rejection() {
        let sequence_state = ValueId::new(2);
        let activation = ValueId::new(3);
        let env = PredicateEnv::new()
            .with_value(sequence_state, facts(ValueRole::SequenceStateSlot))
            .with_value(activation, activation_facts());
        let dr_2 = [DECISION_RULES[2]];

        assert!(matches!(
            evaluate_decision_rules(&dr_2, &env, sequence_state),
            DecisionRuleEvaluation::Fired(DecisionRuleFiring {
                rule_id: DecisionRuleId(3),
                outcome: DecisionRuleOutcome::Reject(
                    DecisionRuleDiagnosticCode::StoragePersistSequenceStateUnsupportedV1
                ),
                ..
            })
        ));
        assert_eq!(
            evaluate_decision_rules(&dr_2, &env, activation),
            DecisionRuleEvaluation::NoAdmittingRule {
                value: activation,
                code: DecisionRuleDiagnosticCode::StorageNoAdmittingDecisionRule
            }
        );
    }

    #[test]
    fn dr_3a_and_dr_3_split_resume_window_from_power_loss_persistence() {
        let yield_only = ValueId::new(10);
        let durable = ValueId::new(11);
        let env = PredicateEnv::new()
            .with_value(yield_only, activation_facts())
            .with_resume_across_yield_value(yield_only)
            .with_value(durable, activation_facts())
            .with_resume_across_yield_value(durable)
            .with_continuation_live_value(durable)
            .with_power_loss_resume_value(durable);

        assert!(matches!(
            evaluate_registry(&env, yield_only),
            DecisionRuleEvaluation::Fired(DecisionRuleFiring {
                rule_id: DecisionRuleId(4),
                outcome: DecisionRuleOutcome::Bind(Materialization::Materialize {
                    class: StorageClass::WramHot,
                    lifetime: LifetimeClass::ResumeWindow
                }),
                ..
            })
        ));
        assert_eq!(
            evaluate_registry(&env, durable),
            DecisionRuleEvaluation::Fired(DecisionRuleFiring {
                value: durable,
                rule_id: DecisionRuleId(5),
                rule_name: "DR-3 PersistContinuationRecord",
                priority: 50,
                outcome: DecisionRuleOutcome::Bind(Materialization::Persist {
                    page: PersistPageId(0x2000_0000 + 11),
                    commit_group: CommitGroupId(0x2000_0000 + 11),
                }),
            })
        );
    }

    #[test]
    fn dr_4_uses_normalized_transcript_capture_enabled_flag() {
        let transcript = ValueId::new(12);
        let dr_4 = [DECISION_RULES[5]];
        let disabled = PredicateEnv::new()
            .with_value(transcript, sized_token_facts(ValueRole::OutputToken, 17))
            .with_transcript_inline_ceiling(16)
            .with_transcript_capture_enabled(false);
        let enabled = disabled.clone().with_transcript_capture_enabled(true);

        assert_eq!(
            evaluate_decision_rules(&dr_4, &disabled, transcript),
            DecisionRuleEvaluation::NoAdmittingRule {
                value: transcript,
                code: DecisionRuleDiagnosticCode::StorageNoAdmittingDecisionRule
            }
        );
        assert_eq!(
            evaluate_decision_rules(&dr_4, &enabled, transcript),
            DecisionRuleEvaluation::Fired(DecisionRuleFiring {
                value: transcript,
                rule_id: DecisionRuleId(6),
                rule_name: "DR-4 PersistTranscriptPage",
                priority: 60,
                outcome: DecisionRuleOutcome::Bind(Materialization::Persist {
                    page: PersistPageId(0x3000_0000 + 12),
                    commit_group: CommitGroupId(0x3000_0000 + 12),
                }),
            })
        );
    }

    #[test]
    fn dr_5_parenthesized_harness_or_trace_truth_table() {
        let input_harness = ValueId::new(20);
        let output_without_harness = ValueId::new(21);
        let activation_harness = ValueId::new(22);
        let trace_admitted = ValueId::new(23);
        let trace_not_admitted = ValueId::new(24);
        let admitted_not_attached = ValueId::new(25);
        let dr_5 = [DECISION_RULES[6]];
        let env = PredicateEnv::new()
            .with_value(input_harness, token_facts(ValueRole::InputToken))
            .with_harness_io_value(input_harness)
            .with_value(output_without_harness, token_facts(ValueRole::OutputToken))
            .with_value(activation_harness, activation_facts())
            .with_harness_io_value(activation_harness)
            .with_value(trace_admitted, activation_facts())
            .with_trace_probe_attached_value(trace_admitted)
            .with_trace_capture_admitted_value(trace_admitted)
            .with_value(trace_not_admitted, activation_facts())
            .with_trace_probe_attached_value(trace_not_admitted)
            .with_value(admitted_not_attached, activation_facts())
            .with_trace_capture_admitted_value(admitted_not_attached);

        assert_eq!(
            evaluate_decision_rules(&dr_5, &env, input_harness),
            DecisionRuleEvaluation::Fired(DecisionRuleFiring {
                value: input_harness,
                rule_id: DecisionRuleId(7),
                rule_name: "DR-5 PersistHarnessOrTracePage",
                priority: 70,
                outcome: DecisionRuleOutcome::Bind(Materialization::Persist {
                    page: PersistPageId(0x4000_0000 + 20),
                    commit_group: CommitGroupId(0x4000_0000 + 20),
                }),
            })
        );
        assert_eq!(
            evaluate_decision_rules(&dr_5, &env, trace_admitted),
            DecisionRuleEvaluation::Fired(DecisionRuleFiring {
                value: trace_admitted,
                rule_id: DecisionRuleId(7),
                rule_name: "DR-5 PersistHarnessOrTracePage",
                priority: 70,
                outcome: DecisionRuleOutcome::Bind(Materialization::Persist {
                    page: PersistPageId(0x5000_0000 + 23),
                    commit_group: CommitGroupId(0x5000_0000 + 23),
                }),
            })
        );
        for value in [
            output_without_harness,
            activation_harness,
            trace_not_admitted,
            admitted_not_attached,
        ] {
            assert_eq!(
                evaluate_decision_rules(&dr_5, &env, value),
                DecisionRuleEvaluation::NoAdmittingRule {
                    value,
                    code: DecisionRuleDiagnosticCode::StorageNoAdmittingDecisionRule
                }
            );
        }
    }

    #[test]
    fn dr_6_binds_synthetic_routed_ffn_expert_weights() {
        let expert_a = ValueId::new(30);
        let expert_b = ValueId::new(31);
        let env = PredicateEnv::new()
            .with_value(expert_a, const_tensor_facts(ValueRole::ExpertWeight, 100))
            .with_value(expert_b, const_tensor_facts(ValueRole::ExpertWeight, 101));

        for expert in [expert_a, expert_b] {
            assert_rom_const_persistent(&env, expert, DecisionRuleId(8), ValueRole::ExpertWeight);
        }
    }

    #[test]
    fn dr_7_router_table_requires_router_weight_role() {
        let router = ValueId::new(32);
        let activation = ValueId::new(33);
        let env = PredicateEnv::new()
            .with_value(router, const_tensor_facts(ValueRole::RouterWeight, 102))
            .with_value(activation, const_tensor_facts(ValueRole::Activation, 103));
        let dr_7 = [DECISION_RULES[8]];

        assert_rom_const_persistent(&env, router, DecisionRuleId(9), ValueRole::RouterWeight);
        assert_eq!(
            evaluate_decision_rules(&dr_7, &env, activation),
            DecisionRuleEvaluation::NoAdmittingRule {
                value: activation,
                code: DecisionRuleDiagnosticCode::StorageNoAdmittingDecisionRule
            }
        );
    }

    #[test]
    fn dr_8_and_dr_9_bind_lut_and_common_const_tensor_roles() {
        let lut = ValueId::new(34);
        let embedding = ValueId::new(35);
        let logit = ValueId::new(36);
        let norm = ValueId::new(37);
        let decode = ValueId::new(38);
        let env = PredicateEnv::new()
            .with_value(lut, const_tensor_facts(ValueRole::LutFragment, 104))
            .with_value(
                embedding,
                const_tensor_facts(ValueRole::EmbeddingTable, 105),
            )
            .with_value(logit, const_tensor_facts(ValueRole::LogitProj, 106))
            .with_value(norm, const_tensor_facts(ValueRole::NormParam, 107))
            .with_value(decode, const_tensor_facts(ValueRole::DecodeConst, 108));

        assert_rom_const_persistent(&env, lut, DecisionRuleId(10), ValueRole::LutFragment);
        for (value, role) in [
            (embedding, ValueRole::EmbeddingTable),
            (logit, ValueRole::LogitProj),
            (norm, ValueRole::NormParam),
            (decode, ValueRole::DecodeConst),
        ] {
            assert_rom_const_persistent(&env, value, DecisionRuleId(11), role);
        }
    }

    #[test]
    fn dr_6_reports_store_004_when_role_points_at_non_const_producer() {
        let expert = ValueId::new(39);
        let env = PredicateEnv::new().with_value(expert, facts(ValueRole::ExpertWeight));
        let dr_6 = [DECISION_RULES[7]];

        assert!(matches!(
            evaluate_decision_rules(&dr_6, &env, expert),
            DecisionRuleEvaluation::Fired(DecisionRuleFiring {
                rule_id: DecisionRuleId(8),
                outcome: DecisionRuleOutcome::Reject(
                    DecisionRuleDiagnosticCode::StorageRomConstWriteViolation
                ),
                ..
            })
        ));
    }

    #[test]
    fn dr_8_static_scan_rejects_removed_lut_policy_surface() {
        let source = include_str!("rules.rs");
        let forbidden = [
            ["staged_", "lut_", "fragments"].concat(),
            ["kernel_", "residency"].concat(),
            ["Kernel", "Resid", "ency"].concat(),
        ];

        for key in forbidden {
            assert!(
                !source.contains(&key),
                "rom-constant rules reference unavailable policy surface {key:?}"
            );
        }
    }

    #[test]
    fn dr_10_renorm_loop_scratch_precedes_recompute_even_when_policy_admits_recompute() {
        let scratch = ValueId::new(40);
        let site = reduction_site(7, "renorm");
        let env = PredicateEnv::new()
            .with_value(scratch, facts(ValueRole::Scratch))
            .with_value_reduction_site(scratch, site.clone())
            .with_renorm_loop_site(site)
            .with_recompute_promotion(StorageMaterialization::RecomputePureValues)
            .with_recompute_cycle_ceiling(1)
            .with_recompute_cost_estimate(scratch, 1);

        assert_eq!(
            evaluate_registry(&env, scratch),
            DecisionRuleEvaluation::Fired(DecisionRuleFiring {
                value: scratch,
                rule_id: DecisionRuleId(12),
                rule_name: "DR-10 RenormLoopScratch",
                priority: 120,
                outcome: DecisionRuleOutcome::Bind(Materialization::Materialize {
                    class: StorageClass::WramHot,
                    lifetime: LifetimeClass::Slice,
                }),
            })
        );
    }

    #[test]
    fn dr_11_honors_recompute_promotion_policy() {
        let value = ValueId::new(41);
        let base = PredicateEnv::new()
            .with_value(value, activation_facts())
            .with_recompute_cycle_ceiling(8)
            .with_recompute_cost_estimate(value, 8);
        let dr_11 = [DECISION_RULES[12]];
        let disabled = base
            .clone()
            .with_recompute_promotion(StorageMaterialization::PreserveAll);
        let enabled = base.with_recompute_promotion(StorageMaterialization::RecomputePureValues);

        assert_eq!(
            evaluate_decision_rules(&dr_11, &disabled, value),
            DecisionRuleEvaluation::NoAdmittingRule {
                value,
                code: DecisionRuleDiagnosticCode::StorageNoAdmittingDecisionRule
            }
        );
        assert_eq!(
            evaluate_decision_rules(&dr_11, &enabled, value),
            DecisionRuleEvaluation::Fired(DecisionRuleFiring {
                value,
                rule_id: DecisionRuleId(13),
                rule_name: "DR-11 RecomputeForPureSliceValue",
                priority: 130,
                outcome: DecisionRuleOutcome::Bind(Materialization::Recompute),
            })
        );
    }

    #[test]
    fn hram_admission_prepass_is_deterministic_and_uses_documented_sort_key() {
        let decision_large = ValueId::new(50);
        let decision_small = ValueId::new(51);
        let score = ValueId::new(52);
        let accumulator = ValueId::new(53);
        let env = PredicateEnv::new()
            .with_value(
                decision_large,
                hot_scalar_facts(ValueRole::RouterDecision, 3),
            )
            .with_value(
                decision_small,
                hot_scalar_facts(ValueRole::RouterDecision, 1),
            )
            .with_value(score, hot_scalar_facts(ValueRole::RouterScore, 1))
            .with_value(accumulator, hot_scalar_facts(ValueRole::Accumulator, 2));

        let first = precompute_hram_admitted_set(&env, 4);
        let second = precompute_hram_admitted_set(&env, 4);

        assert_eq!(first, second);
        assert_eq!(first.admission_order, vec![decision_small, decision_large]);
        assert_eq!(
            first.admitted_values,
            BTreeSet::from([decision_small, decision_large])
        );

        let admitted_env = env.with_precomputed_hram_admitted_set(first);
        assert_eq!(
            evaluate_registry(&admitted_env, decision_small),
            DecisionRuleEvaluation::Fired(DecisionRuleFiring {
                value: decision_small,
                rule_id: DecisionRuleId(14),
                rule_name: "DR-12 HotScalarHram",
                priority: 140,
                outcome: DecisionRuleOutcome::Bind(Materialization::Materialize {
                    class: StorageClass::HramHot,
                    lifetime: LifetimeClass::Slice,
                }),
            })
        );
        assert_eq!(
            evaluate_registry(&admitted_env, score),
            DecisionRuleEvaluation::NoAdmittingRule {
                value: score,
                code: DecisionRuleDiagnosticCode::StorageNoAdmittingDecisionRule
            }
        );
    }

    #[test]
    fn dr_12_reports_store_005_only_for_invalid_precomputed_set() {
        let scalar = ValueId::new(54);
        let invalid = PrecomputedHramAdmittedSet {
            admitted_values: BTreeSet::from([scalar]),
            admission_order: vec![scalar],
            cumulative_logical_size: 2,
            allocatable_budget: 1,
        };
        let env = PredicateEnv::new()
            .with_value(scalar, hot_scalar_facts(ValueRole::RouterScore, 2))
            .with_precomputed_hram_admitted_set(invalid);
        let dr_12 = [DECISION_RULES[13]];

        assert!(matches!(
            evaluate_decision_rules(&dr_12, &env, scalar),
            DecisionRuleEvaluation::Fired(DecisionRuleFiring {
                rule_id: DecisionRuleId(14),
                outcome: DecisionRuleOutcome::Reject(
                    DecisionRuleDiagnosticCode::StorageHramAdmissionInvariantViolation
                ),
                ..
            })
        ));
    }

    #[test]
    fn dr_13_excludes_rom_const_roles_and_reports_store_001() {
        let expert = ValueId::new(60);
        let env = PredicateEnv::new()
            .with_wram_hot_per_value_eligibility_ceiling(64)
            .with_value(
                expert,
                with_logical_size(const_tensor_facts(ValueRole::ExpertWeight, 109), 4),
            );
        let dr_13 = [DECISION_RULES[14]];

        assert_eq!(
            evaluate_decision_rules(&dr_13, &env, expert),
            DecisionRuleEvaluation::NoAdmittingRule {
                value: expert,
                code: DecisionRuleDiagnosticCode::StorageNoAdmittingDecisionRule
            }
        );
    }

    #[test]
    fn dr_13_uses_per_value_wram_ceiling_and_lifetime_estimate() {
        let small = ValueId::new(61);
        let large = ValueId::new(62);
        let mut large_facts = with_logical_size(facts(ValueRole::Activation), 65);
        large_facts.lifetime_estimate = Some(LifetimeClass::Token);
        let env = PredicateEnv::new()
            .with_wram_hot_per_value_eligibility_ceiling(64)
            .with_value(small, with_logical_size(facts(ValueRole::Activation), 64))
            .with_value(large, large_facts);

        assert_eq!(
            evaluate_registry(&env, small),
            DecisionRuleEvaluation::Fired(DecisionRuleFiring {
                value: small,
                rule_id: DecisionRuleId(15),
                rule_name: "DR-13 DefaultMaterializeKnownIntermediate",
                priority: 150,
                outcome: DecisionRuleOutcome::Bind(Materialization::Materialize {
                    class: StorageClass::WramHot,
                    lifetime: LifetimeClass::Slice,
                }),
            })
        );
        assert_eq!(
            evaluate_registry(&env, large),
            DecisionRuleEvaluation::Fired(DecisionRuleFiring {
                value: large,
                rule_id: DecisionRuleId(15),
                rule_name: "DR-13 DefaultMaterializeKnownIntermediate",
                priority: 150,
                outcome: DecisionRuleOutcome::Bind(Materialization::Materialize {
                    class: StorageClass::SramPaged,
                    lifetime: LifetimeClass::Token,
                }),
            })
        );
    }

    #[test]
    fn manifest_canonical_json_is_stable() {
        let first = decision_rule_set_manifest_canonical_json().expect("manifest canonicalizes");
        let second = decision_rule_set_manifest_canonical_json().expect("manifest canonicalizes");
        let manifest = decision_rule_set_manifest();

        assert_eq!(first, second);
        assert_eq!(manifest.rules.len(), DECISION_RULES.len());
        assert_eq!(manifest.rules[11].name, "DR-10 RenormLoopScratch");
        assert!(
            String::from_utf8(first)
                .expect("canonical JSON is utf8")
                .contains("\"semantic_predicate_id\":\"RenormLoopScratch\"")
        );
    }

    fn test_rule(
        id: DecisionRuleId,
        name: &'static str,
        priority: u32,
        matches_value: bool,
        outcome: fn(&PredicateEnv, ValueId) -> DecisionRuleOutcome,
    ) -> DecisionRule {
        DecisionRule {
            id,
            name,
            predicate: if matches_value {
                predicate_true
            } else {
                predicate_false
            },
            outcome,
            priority,
        }
    }

    fn predicate_true(_env: &PredicateEnv, _value: ValueId) -> bool {
        true
    }

    fn predicate_false(_env: &PredicateEnv, _value: ValueId) -> bool {
        false
    }

    fn activation_facts() -> PredicateValueFacts {
        facts(ValueRole::Activation)
    }

    fn facts(role: ValueRole) -> PredicateValueFacts {
        PredicateValueFacts::new(
            role,
            ValueFormat::QuantInt {
                quant_format_id: crate::storage_plan::predicates::QuantFormatId(1),
            },
        )
    }

    fn const_tensor_facts(role: ValueRole, tensor: u32) -> PredicateValueFacts {
        PredicateValueFacts::new(
            role,
            ValueFormat::ConstTensorRef {
                tensor_id: crate::s1::quant_graph::TensorId::new(tensor),
            },
        )
    }

    fn hot_scalar_facts(role: ValueRole, logical_size: u32) -> PredicateValueFacts {
        let format = match role {
            ValueRole::RouterDecision => ValueFormat::TokenIdDomain { vocab_size: 4 },
            ValueRole::RouterScore | ValueRole::Accumulator => {
                ValueFormat::IntAccum { width_bits: 16 }
            }
            _ => ValueFormat::Flag,
        };
        with_logical_size(PredicateValueFacts::new(role, format), logical_size)
    }

    fn with_logical_size(mut facts: PredicateValueFacts, logical_size: u32) -> PredicateValueFacts {
        facts.logical_size = Some(logical_size);
        facts
    }

    fn token_facts(role: ValueRole) -> PredicateValueFacts {
        PredicateValueFacts::new(role, ValueFormat::TokenIdDomain { vocab_size: 256 })
    }

    fn sized_token_facts(role: ValueRole, logical_size: u32) -> PredicateValueFacts {
        let mut facts = token_facts(role);
        facts.logical_size = Some(logical_size);
        facts
    }

    fn reduction_site(node: u32, site: &str) -> crate::storage_plan::predicates::ReductionSiteRef {
        crate::storage_plan::predicates::ReductionSiteRef {
            node: crate::s3::infer_ir::NodeId::new(node),
            site: ReductionSiteId(site.to_owned()),
        }
    }

    fn assert_rom_const_persistent(
        env: &PredicateEnv,
        value: ValueId,
        rule_id: DecisionRuleId,
        role: ValueRole,
    ) {
        match evaluate_registry(env, value) {
            DecisionRuleEvaluation::Fired(firing) => {
                assert_eq!(firing.rule_id, rule_id);
                assert_eq!(
                    firing.outcome,
                    DecisionRuleOutcome::Bind(Materialization::Materialize {
                        class: StorageClass::RomConst,
                        lifetime: LifetimeClass::Persistent,
                    })
                );
                assert_eq!(
                    firing.provenance(env),
                    DecisionRuleFiringProvenance {
                        decision_rule: rule_id,
                        op_output_role: Some(role),
                    }
                );
            }
            other => panic!("expected RomConst firing for {value:?}, got {other:?}"),
        }
    }

    fn assert_event(events: &[CapturedEvent], name: &str, field: &str, expected_value: &str) {
        assert!(
            events.iter().any(|event| {
                event.name == name
                    && event
                        .fields
                        .get(field)
                        .is_some_and(|value| value == expected_value)
            }),
            "missing event {name} with {field}={expected_value}; events={events:?}"
        );
    }

    #[derive(Clone, Default)]
    struct CapturedTracingLayer {
        events: std::sync::Arc<std::sync::Mutex<Vec<CapturedEvent>>>,
    }

    impl CapturedTracingLayer {
        fn events(&self) -> Vec<CapturedEvent> {
            self.events.lock().expect("events lock").clone()
        }
    }

    #[derive(Debug, Clone)]
    struct CapturedEvent {
        name: String,
        fields: BTreeMap<String, String>,
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
    }
}
