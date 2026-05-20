//! Shared test helpers for gbf-codegen integration and downstream F-B8 beads.

pub use gbf_codegen::assert_storage_plan_traced;
pub use gbf_codegen::storage_plan_test_infra::{
    NdjsonTraceSink, TraceCapture, cache_key_prefix, debug_harness, sc_violations, synth,
    timestamp_string, trace_catalog, with_trace_capture,
};

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    use gbf_codegen::s1::quant_graph::{NormPlanId, ReductionSiteKey, reduction_site_id};
    use gbf_codegen::s3::infer_ir::{NodeId, ValueId};
    use gbf_codegen::storage_plan::{
        AbstractLiveRange, AliasCandidateEdge, AliasIntent, BindingJustification, CommitGroupId,
        DecisionRuleId, LifetimeClass, Materialization, PersistPageId, PredicateValueFacts,
        QuantFormatId, ReductionSiteRef, StorageClass, StoragePlanCoreOutcome,
        StoragePlanCoreSummary, StoragePlanCoreValue, ValueFormat, ValueRole,
        build_storage_plan_core, storage_plan_core_output_canonical_bytes,
    };

    #[test]
    fn public_synth_surface_builds_canonical_storage_plan_inputs() {
        let inputs = synth::tiny_routed_ffn_inputs();

        gbf_codegen::storage_plan::canonicalize_inputs(&inputs)
            .expect("public synth surface emits canonical inputs");
    }

    #[test]
    fn public_trace_assertion_macro_is_reexported() {
        let (_, events) = with_trace_capture(|| {
            tracing_event_for_reexport_test();
        });

        assert_storage_plan_traced!(
            &events,
            [
                {
                    event: "f_b8.rule.fired",
                    rule_name: "DR-6"
                }
            ]
        );
    }

    #[test]
    fn routed_ffn_dense_fixture_examples_a_through_d_match_rfc_9_7() {
        let input = routed_examples_core_input();
        let first = build_storage_plan_core(&input);
        let second = build_storage_plan_core(&input);

        assert_eq!(first.outcome, StoragePlanCoreOutcome::Succeeded);
        assert!(first.diagnostics.is_empty());
        assert_eq!(
            storage_plan_core_output_canonical_bytes(&first).expect("first replay canonicalizes"),
            storage_plan_core_output_canonical_bytes(&second).expect("second replay canonicalizes"),
            "routed FFN storage fixture must replay byte-identically"
        );

        let result = first.result.as_ref().expect("successful plan has result");
        let summary = first.summary.as_ref().expect("summary");
        assert_eq!(summary.total_bindings, input.values.len() as u32);
        assert_rule_firings(
            summary,
            [
                (DecisionRuleId(4), 1),
                (DecisionRuleId(8), 8),
                (DecisionRuleId(12), 2),
                (DecisionRuleId(14), 1),
                (DecisionRuleId(15), 2),
            ],
        );

        assert_binding(
            result,
            ValueId::new(13),
            DecisionRuleId(8),
            Materialization::Materialize {
                class: StorageClass::RomConst,
                lifetime: LifetimeClass::Persistent,
            },
        );
        assert_singleton_alias(result, ValueId::new(13), AliasIntent::NoAlias);

        assert_binding(
            result,
            ValueId::new(18),
            DecisionRuleId(14),
            Materialization::Materialize {
                class: StorageClass::HramHot,
                lifetime: LifetimeClass::Slice,
            },
        );
        assert_singleton_alias(result, ValueId::new(18), AliasIntent::NoAlias);
        assert_alias_intent_is_not(result, ValueId::new(18), AliasIntent::PingPong);

        for accumulator in [ValueId::new(19), ValueId::new(20)] {
            assert_binding(
                result,
                accumulator,
                DecisionRuleId(15),
                Materialization::Materialize {
                    class: StorageClass::WramHot,
                    lifetime: LifetimeClass::Slice,
                },
            );
            assert_alias_intent_is_not(result, accumulator, AliasIntent::PingPong);
        }
        assert_alias_members(
            result,
            ValueId::new(19),
            AliasIntent::ScratchReuse,
            [ValueId::new(19), ValueId::new(20)],
        );

        for scratch in [ValueId::new(21), ValueId::new(22)] {
            assert_binding(
                result,
                scratch,
                DecisionRuleId(12),
                Materialization::Materialize {
                    class: StorageClass::WramHot,
                    lifetime: LifetimeClass::Slice,
                },
            );
            assert_not_recompute(result, scratch);
        }
        assert_alias_members(
            result,
            ValueId::new(21),
            AliasIntent::ScratchReuse,
            [ValueId::new(21), ValueId::new(22)],
        );

        // RFC §9.7 Example D narrates the ResumeWindow activation as DR-13.
        // The registry binds resume continuations first via DR-3a, so this pins
        // the actual registry order while accumulators above keep DR-13 covered.
        assert_binding(
            result,
            ValueId::new(1),
            DecisionRuleId(4),
            Materialization::Materialize {
                class: StorageClass::WramHot,
                lifetime: LifetimeClass::ResumeWindow,
            },
        );
        assert_singleton_alias(result, ValueId::new(1), AliasIntent::NoAlias);
    }

    #[test]
    fn routed_ffn_router_decision_stays_hram_hot_when_recompute_promotion_is_enabled() {
        let mut input = synth::InputBuilder::with_expert_weights(8)
            .with_router_decision_value()
            .with_promotion_level(synth::RecomputePromotionLevel::RecomputePureValues)
            .build_core();
        set_router_example_facts(&mut input, ValueId::new(18));

        let output = build_storage_plan_core(&input);
        assert_eq!(output.outcome, StoragePlanCoreOutcome::Succeeded);
        let result = output.result.as_ref().expect("successful plan has result");

        assert_binding(
            result,
            ValueId::new(18),
            DecisionRuleId(14),
            Materialization::Materialize {
                class: StorageClass::HramHot,
                lifetime: LifetimeClass::Slice,
            },
        );
        assert_singleton_alias(result, ValueId::new(18), AliasIntent::NoAlias);
    }

    #[test]
    fn routed_ffn_renorm_scratch_stays_materialized_when_recompute_promotion_is_enabled() {
        let mut input = synth::InputBuilder::with_expert_weights(8)
            .with_router_decision_value()
            .with_renorm_loop_scratch(16)
            .with_promotion_level(synth::RecomputePromotionLevel::RecomputePureValues)
            .build_core();
        set_router_example_facts(&mut input, ValueId::new(18));
        mark_renorm_loop_scratch(&mut input, ValueId::new(19), 38, 0);
        mark_renorm_loop_scratch(&mut input, ValueId::new(20), 40, 1);

        let output = build_storage_plan_core(&input);
        assert_eq!(output.outcome, StoragePlanCoreOutcome::Succeeded);
        let result = output.result.as_ref().expect("successful plan has result");

        for scratch in [ValueId::new(19), ValueId::new(20)] {
            assert_binding(
                result,
                scratch,
                DecisionRuleId(12),
                Materialization::Materialize {
                    class: StorageClass::WramHot,
                    lifetime: LifetimeClass::Slice,
                },
            );
            assert_not_recompute(result, scratch);
        }
    }

    #[test]
    fn routed_ffn_dense_fixture_example_e_rejects_sequence_state_slot() {
        let mut input = synth::minimal_singleton_core_input();
        let sequence_state = ValueId::new(90);
        input.values.push(StoragePlanCoreValue {
            value: sequence_state,
            materialization: Materialization::Persist {
                page: PersistPageId(90),
                commit_group: CommitGroupId(90),
            },
            live_range: live_range(190, 191, LifetimeClass::Persistent),
            role: ValueRole::SequenceStateSlot,
            persist_kind: None,
            commit_group_reason: None,
        });
        input
            .topological_order
            .extend([NodeId::new(190), NodeId::new(191)]);
        let env = std::mem::take(&mut input.predicate_env);
        input.predicate_env = env.with_value(
            sequence_state,
            PredicateValueFacts::new(
                ValueRole::SequenceStateSlot,
                ValueFormat::TokenIdDomain { vocab_size: 1 },
            )
            .with_sequence_state_slot(0, 0),
        );

        let output = build_storage_plan_core(&input);

        assert_eq!(output.outcome, StoragePlanCoreOutcome::Failed);
        assert!(
            output.result.is_none(),
            "rejected Example E must emit no binding result"
        );
        assert!(
            output.summary.is_none(),
            "rejected Example E must emit no summary"
        );
        assert_eq!(
            output
                .diagnostics
                .iter()
                .map(|code| format!("{code:?}"))
                .collect::<Vec<_>>(),
            vec!["StoragePersistSequenceStateUnsupportedV1"]
        );
    }

    fn tracing_event_for_reexport_test() {
        tracing::info!(event = "f_b8.rule.fired", rule_name = "DR-6");
    }

    fn routed_examples_core_input() -> gbf_codegen::storage_plan::StoragePlanCoreInput {
        let mut input = synth::InputBuilder::with_expert_weights(8)
            .with_router_decision_value()
            .build_core();

        let resume_activation = ValueId::new(1);
        let left_accumulator = ValueId::new(19);
        let right_accumulator = ValueId::new(20);
        let left_renorm_scratch = ValueId::new(21);
        let right_renorm_scratch = ValueId::new(22);

        for value in &mut input.values {
            if value.value == resume_activation {
                value.materialization = Materialization::Materialize {
                    class: StorageClass::WramHot,
                    lifetime: LifetimeClass::ResumeWindow,
                };
                value.live_range = live_range(2, 3, LifetimeClass::ResumeWindow);
            }
        }

        input.values.extend([
            wram_hot_slice_value(left_accumulator, 38, 39, ValueRole::Accumulator),
            wram_hot_slice_value(right_accumulator, 40, 41, ValueRole::Accumulator),
            wram_hot_slice_value(left_renorm_scratch, 42, 43, ValueRole::Scratch),
            wram_hot_slice_value(right_renorm_scratch, 44, 45, ValueRole::Scratch),
        ]);
        input.topological_order.extend([
            NodeId::new(38),
            NodeId::new(39),
            NodeId::new(40),
            NodeId::new(41),
            NodeId::new(42),
            NodeId::new(43),
            NodeId::new(44),
            NodeId::new(45),
        ]);
        input.alias_edges.extend([
            AliasCandidateEdge {
                left: left_accumulator,
                right: right_accumulator,
                intent: AliasIntent::ScratchReuse,
            },
            AliasCandidateEdge {
                left: left_renorm_scratch,
                right: right_renorm_scratch,
                intent: AliasIntent::ScratchReuse,
            },
        ]);

        let env = std::mem::take(&mut input.predicate_env)
            .with_value(
                resume_activation,
                activation_facts(LifetimeClass::ResumeWindow),
            )
            .with_resume_across_yield_value(resume_activation)
            .with_value(left_accumulator, accumulator_facts(256))
            .with_value(right_accumulator, accumulator_facts(256))
            .with_value(left_renorm_scratch, renorm_scratch_facts(16))
            .with_value(right_renorm_scratch, renorm_scratch_facts(16));
        input.predicate_env = env;
        mark_renorm_loop_scratch(&mut input, left_renorm_scratch, 42, 0);
        mark_renorm_loop_scratch(&mut input, right_renorm_scratch, 44, 1);
        set_router_example_facts(&mut input, ValueId::new(18));
        input
    }

    fn set_router_example_facts(
        input: &mut gbf_codegen::storage_plan::StoragePlanCoreInput,
        router: ValueId,
    ) {
        let mut facts = PredicateValueFacts::new(
            ValueRole::RouterDecision,
            ValueFormat::TokenIdDomain { vocab_size: 8 },
        );
        facts.logical_size = Some(1);
        let env = std::mem::take(&mut input.predicate_env).with_value(router, facts);
        input.predicate_env = env;
    }

    fn activation_facts(lifetime: LifetimeClass) -> PredicateValueFacts {
        let mut facts = PredicateValueFacts::new(
            ValueRole::Activation,
            ValueFormat::QuantInt {
                quant_format_id: QuantFormatId(1),
            },
        );
        facts.logical_size = Some(4);
        facts.lifetime_estimate = Some(lifetime);
        facts
    }

    fn accumulator_facts(logical_size: u32) -> PredicateValueFacts {
        let mut facts = PredicateValueFacts::new(
            ValueRole::Accumulator,
            ValueFormat::IntAccum { width_bits: 16 },
        );
        facts.logical_size = Some(logical_size);
        facts
    }

    fn renorm_scratch_facts(tile_len: u32) -> PredicateValueFacts {
        let mut facts =
            PredicateValueFacts::new(ValueRole::Scratch, ValueFormat::IntAccum { width_bits: 16 });
        facts.logical_size = Some(tile_len);
        facts
    }

    fn wram_hot_slice_value(
        value: ValueId,
        def: u32,
        last: u32,
        role: ValueRole,
    ) -> StoragePlanCoreValue {
        StoragePlanCoreValue {
            value,
            materialization: Materialization::Materialize {
                class: StorageClass::WramHot,
                lifetime: LifetimeClass::Slice,
            },
            live_range: live_range(def, last, LifetimeClass::Slice),
            role,
            persist_kind: None,
            commit_group_reason: None,
        }
    }

    fn mark_renorm_loop_scratch(
        input: &mut gbf_codegen::storage_plan::StoragePlanCoreInput,
        scratch: ValueId,
        def_node: u32,
        site_ordinal: u32,
    ) {
        let site = renorm_site(def_node, site_ordinal);
        let env = std::mem::take(&mut input.predicate_env)
            .with_value_reduction_site(scratch, site.clone())
            .with_renorm_loop_site(site);
        input.predicate_env = env;
    }

    fn renorm_site(node: u32, site_ordinal: u32) -> ReductionSiteRef {
        ReductionSiteRef {
            node: NodeId::new(node),
            site: reduction_site_id(ReductionSiteKey::Norm {
                norm_plan_id: NormPlanId::new(site_ordinal),
            }),
        }
    }

    fn live_range(def: u32, last: u32, lifetime: LifetimeClass) -> AbstractLiveRange {
        AbstractLiveRange {
            def_node: NodeId::new(def),
            first_use_node: Some(NodeId::new(last)),
            last_use_node: Some(NodeId::new(last)),
            lifetime_class: lifetime,
            checkpoint_stable: false,
        }
    }

    fn assert_binding(
        result: &gbf_codegen::storage_plan::StoragePlanCoreResult,
        value: ValueId,
        rule: DecisionRuleId,
        materialization: Materialization,
    ) {
        let binding = result.bindings.get(&value).expect("binding exists");
        assert_eq!(binding.materialization, materialization);
        assert_eq!(
            binding.justification,
            BindingJustification::DecisionRule(rule)
        );
        let provenance = result
            .provenance
            .bindings
            .get(&value)
            .expect("binding provenance exists");
        assert_eq!(provenance.decision_rule, rule);
    }

    fn assert_rule_firings<const N: usize>(
        summary: &StoragePlanCoreSummary,
        expected: [(DecisionRuleId, u32); N],
    ) {
        let expected_rules = expected
            .iter()
            .map(|(rule, _)| *rule)
            .collect::<BTreeSet<_>>();
        for (rule, expected_count) in expected {
            assert_eq!(
                rule_firing_count(summary, rule),
                expected_count,
                "unexpected firing count for {rule:?}"
            );
        }
        for entry in summary.rule_firings {
            if !expected_rules.contains(&entry.rule) {
                assert_eq!(
                    entry.count, 0,
                    "unlisted rule should not fire in routed FFN fixture: {:?}",
                    entry.rule
                );
            }
        }
        assert_eq!(
            summary
                .rule_firings
                .iter()
                .map(|entry| entry.count)
                .sum::<u32>(),
            summary.bindings
        );
    }

    fn rule_firing_count(summary: &StoragePlanCoreSummary, rule: DecisionRuleId) -> u32 {
        summary
            .rule_firings
            .iter()
            .find(|entry| entry.rule == rule)
            .map_or(0, |entry| entry.count)
    }

    fn assert_singleton_alias(
        result: &gbf_codegen::storage_plan::StoragePlanCoreResult,
        value: ValueId,
        intent: AliasIntent,
    ) {
        assert_alias_members(result, value, intent, [value]);
    }

    fn assert_alias_intent_is_not(
        result: &gbf_codegen::storage_plan::StoragePlanCoreResult,
        value: ValueId,
        forbidden: AliasIntent,
    ) {
        let binding = result.bindings.get(&value).expect("binding exists");
        let class = result
            .alias_classes
            .get(&binding.alias_class)
            .expect("alias class exists");
        assert_ne!(
            class.intent(),
            forbidden,
            "{value:?} must not bind alias intent {forbidden:?}"
        );
    }

    fn assert_not_recompute(
        result: &gbf_codegen::storage_plan::StoragePlanCoreResult,
        value: ValueId,
    ) {
        let binding = result.bindings.get(&value).expect("binding exists");
        assert_ne!(
            binding.materialization,
            Materialization::Recompute,
            "{value:?} must not bind Recompute"
        );
    }

    fn assert_alias_members<const N: usize>(
        result: &gbf_codegen::storage_plan::StoragePlanCoreResult,
        value: ValueId,
        intent: AliasIntent,
        expected: [ValueId; N],
    ) {
        let binding = result.bindings.get(&value).expect("binding exists");
        let class = result
            .alias_classes
            .get(&binding.alias_class)
            .expect("alias class exists");
        assert_eq!(class.intent(), intent);
        assert_eq!(
            class.members().iter().copied().collect::<BTreeSet<_>>(),
            expected.into_iter().collect::<BTreeSet<_>>()
        );
    }
}
