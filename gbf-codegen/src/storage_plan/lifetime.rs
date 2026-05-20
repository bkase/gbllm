//! Abstract live-range and lifetime-bound helpers for Stage 6.

use std::collections::{BTreeMap, BTreeSet};

use gbf_foundation::EvidenceRef;
use gbf_policy::{
    StoragePlanDiagnosticCode, StoragePlanDiagnosticProvenance, ValidationDiagnostic,
};
use serde::{Deserialize, Serialize};

use crate::s3::infer_ir::{GbInferIR, NodeId, ValueId};
use crate::storage_plan::diagnostics::storage_plan_diagnostic;
use crate::storage_plan::predicates::{
    PredicateEnv, ValueRole, effective_lifetime_estimate, is_observed_checkpoint_backing_value,
    is_renorm_loop_scratch, value_role_of,
};
use crate::storage_plan::types::{AbstractLiveRange, LifetimeClass, Materialization};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AbstractDefUseNode {
    pub node: NodeId,
    pub inputs: Vec<ValueId>,
    pub outputs: Vec<ValueId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveRangeError {
    MissingDef { value: ValueId },
    MissingTopologicalNode { node: NodeId },
}

pub fn compute_abstract_live_range(
    iir: &GbInferIR,
    value: ValueId,
) -> Result<AbstractLiveRange, LiveRangeError> {
    compute_abstract_live_range_with_checkpoints(iir, &BTreeSet::new(), value)
}

pub fn compute_abstract_live_range_with_checkpoints(
    iir: &GbInferIR,
    checkpoint_stable_values: &BTreeSet<ValueId>,
    value: ValueId,
) -> Result<AbstractLiveRange, LiveRangeError> {
    compute_abstract_live_range_with_predicates(
        iir,
        checkpoint_stable_values,
        &PredicateEnv::new(),
        value,
    )
}

pub fn compute_abstract_live_range_with_predicates(
    iir: &GbInferIR,
    checkpoint_stable_values: &BTreeSet<ValueId>,
    env: &PredicateEnv,
    value: ValueId,
) -> Result<AbstractLiveRange, LiveRangeError> {
    let nodes: Vec<_> = iir
        .nodes
        .iter()
        .map(|node| AbstractDefUseNode {
            node: node.node_id,
            inputs: node.inputs.clone(),
            outputs: node.outputs.clone(),
        })
        .collect();

    compute_abstract_live_range_from_nodes_with_predicates(
        &nodes,
        checkpoint_stable_values,
        env,
        value,
    )
}

pub fn compute_abstract_live_range_from_nodes_with_predicates(
    nodes: &[AbstractDefUseNode],
    checkpoint_stable_values: &BTreeSet<ValueId>,
    env: &PredicateEnv,
    value: ValueId,
) -> Result<AbstractLiveRange, LiveRangeError> {
    compute_abstract_live_range_from_nodes(
        nodes,
        checkpoint_stable_values,
        &BTreeMap::from([(value, effective_lifetime_estimate(env, value))]),
        value,
    )
}

pub fn compute_abstract_live_range_from_nodes(
    nodes: &[AbstractDefUseNode],
    checkpoint_stable_values: &BTreeSet<ValueId>,
    lifetime_estimates: &BTreeMap<ValueId, LifetimeClass>,
    value: ValueId,
) -> Result<AbstractLiveRange, LiveRangeError> {
    let def_node = nodes
        .iter()
        .find_map(|node| node.outputs.contains(&value).then_some(node.node))
        .ok_or(LiveRangeError::MissingDef { value })?;
    let mut uses = nodes
        .iter()
        .enumerate()
        .filter_map(|(index, node)| node.inputs.contains(&value).then_some((index, node.node)));
    let first_use = uses.next();
    let last_use = first_use
        .into_iter()
        .chain(uses)
        .max_by_key(|(index, _)| *index);
    let checkpoint_stable = checkpoint_stable_values.contains(&value);
    let lifetime_class = lifetime_estimates
        .get(&value)
        .cloned()
        .unwrap_or_else(|| effective_lifetime_estimate(&PredicateEnv::new(), value));

    Ok(AbstractLiveRange {
        def_node,
        first_use_node: first_use.map(|(_, node)| node),
        last_use_node: last_use.map(|(_, node)| node),
        lifetime_class,
        checkpoint_stable,
    })
}

pub fn abstract_live_ranges_overlap(
    topological_order: &[NodeId],
    left: &AbstractLiveRange,
    right: &AbstractLiveRange,
) -> Result<bool, LiveRangeError> {
    let positions: BTreeMap<_, _> = topological_order
        .iter()
        .copied()
        .enumerate()
        .map(|(index, node)| (node, index))
        .collect();
    let (left_start, left_end) = interval_indexes(&positions, left)?;
    let (right_start, right_end) = interval_indexes(&positions, right)?;

    Ok(left_start <= right_end && right_start <= left_end)
}

pub fn abstract_live_ranges_disjoint(
    topological_order: &[NodeId],
    left: &AbstractLiveRange,
    right: &AbstractLiveRange,
) -> Result<bool, LiveRangeError> {
    abstract_live_ranges_overlap(topological_order, left, right).map(|overlap| !overlap)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LifetimeBound {
    pub lifetime: LifetimeClass,
    pub source: LifetimeBoundSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum LifetimeBoundSource {
    DefaultSlice,
    ObservationStability,
    ReductionScratch,
    Persistence,
    RoutingStability,
    ImmutableGraphEntity,
    DefUseWidth,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LifetimeBounds {
    pub min_required: LifetimeBound,
    pub max_admissible: LifetimeBound,
}

pub fn min_required_lifetime(env: &PredicateEnv, value: ValueId) -> LifetimeBound {
    let mut strictest = LifetimeBound {
        lifetime: LifetimeClass::Slice,
        source: LifetimeBoundSource::DefaultSlice,
    };

    if is_observed_checkpoint_backing_value(env, value) {
        strictest = stricter_lifetime_bound(
            strictest,
            LifetimeBound {
                lifetime: LifetimeClass::Token,
                source: LifetimeBoundSource::ObservationStability,
            },
        );
    }

    if is_immutable_graph_entity(env, value) {
        strictest = stricter_lifetime_bound(
            strictest,
            LifetimeBound {
                lifetime: LifetimeClass::Persistent,
                source: LifetimeBoundSource::ImmutableGraphEntity,
            },
        );
    }

    if is_persistent_role(env, value) {
        strictest = stricter_lifetime_bound(
            strictest,
            LifetimeBound {
                lifetime: LifetimeClass::Persistent,
                source: LifetimeBoundSource::Persistence,
            },
        );
    }

    if value_role_of(env, value) == Some(ValueRole::RouterDecision) {
        strictest = stricter_lifetime_bound(
            strictest,
            LifetimeBound {
                lifetime: LifetimeClass::Slice,
                source: LifetimeBoundSource::RoutingStability,
            },
        );
    }

    if is_renorm_loop_scratch(env, value) {
        strictest = stricter_lifetime_bound(
            strictest,
            LifetimeBound {
                lifetime: LifetimeClass::Slice,
                source: LifetimeBoundSource::ReductionScratch,
            },
        );
    }

    strictest
}

pub fn max_admissible_lifetime(env: &PredicateEnv, value: ValueId) -> LifetimeBound {
    if is_immutable_graph_entity(env, value) || is_persistent_role(env, value) {
        return LifetimeBound {
            lifetime: LifetimeClass::Persistent,
            source: LifetimeBoundSource::ImmutableGraphEntity,
        };
    }

    LifetimeBound {
        lifetime: effective_lifetime_estimate(env, value),
        source: LifetimeBoundSource::DefUseWidth,
    }
}

pub fn lifetime_bounds(env: &PredicateEnv, value: ValueId) -> LifetimeBounds {
    LifetimeBounds {
        min_required: min_required_lifetime(env, value),
        max_admissible: max_admissible_lifetime(env, value),
    }
}

pub fn lifetime_of_materialization(materialization: &Materialization) -> LifetimeClass {
    match materialization {
        Materialization::Recompute => LifetimeClass::Slice,
        Materialization::Materialize { lifetime, .. } => lifetime.clone(),
        Materialization::Persist { .. } => LifetimeClass::Persistent,
    }
}

pub fn validate_lifetime_bounds(
    value: ValueId,
    materialization: &Materialization,
    bounds: &LifetimeBounds,
) -> Result<(), Box<ValidationDiagnostic>> {
    let chosen = lifetime_of_materialization(materialization);
    if bounds.min_required.lifetime <= chosen && chosen <= bounds.max_admissible.lifetime {
        return Ok(());
    }

    let failing_source = if chosen < bounds.min_required.lifetime {
        bounds.min_required.source
    } else {
        bounds.max_admissible.source
    };

    Err(Box::new(
        storage_plan_diagnostic(
            StoragePlanDiagnosticCode::StorageLifetimeAdmissibilityViolation,
            StoragePlanDiagnosticProvenance::LifetimeAdmissibility {
                value_id: value.get(),
                computed_lifetime: lifetime_class_name(&chosen).to_owned(),
                min_lifetime: lifetime_class_name(&bounds.min_required.lifetime).to_owned(),
                max_lifetime: lifetime_class_name(&bounds.max_admissible.lifetime).to_owned(),
                source: lifetime_bound_source_name(failing_source).to_owned(),
            },
            Vec::<EvidenceRef>::new(),
        )
        .expect("STORE-017 provenance schema is fixed"),
    ))
}

pub const fn lifetime_class_name(lifetime: &LifetimeClass) -> &'static str {
    match lifetime {
        LifetimeClass::Slice => "Slice",
        LifetimeClass::ResumeWindow => "ResumeWindow",
        LifetimeClass::Token => "Token",
        LifetimeClass::Session => "Session",
        LifetimeClass::Persistent => "Persistent",
    }
}

pub const fn lifetime_bound_source_name(source: LifetimeBoundSource) -> &'static str {
    match source {
        LifetimeBoundSource::DefaultSlice => "DefaultSlice",
        LifetimeBoundSource::ObservationStability => "ObservationStability",
        LifetimeBoundSource::ReductionScratch => "ReductionScratch",
        LifetimeBoundSource::Persistence => "Persistence",
        LifetimeBoundSource::RoutingStability => "RoutingStability",
        LifetimeBoundSource::ImmutableGraphEntity => "ImmutableGraphEntity",
        LifetimeBoundSource::DefUseWidth => "DefUseWidth",
    }
}

fn stricter_lifetime_bound(left: LifetimeBound, right: LifetimeBound) -> LifetimeBound {
    if right.lifetime > left.lifetime {
        right
    } else {
        left
    }
}

fn interval_indexes(
    positions: &BTreeMap<NodeId, usize>,
    live_range: &AbstractLiveRange,
) -> Result<(usize, usize), LiveRangeError> {
    let start = node_position(positions, live_range.def_node)?;
    let end_node = live_range.last_use_node.unwrap_or(live_range.def_node);
    let end = node_position(positions, end_node)?;
    Ok((start, end))
}

fn node_position(
    positions: &BTreeMap<NodeId, usize>,
    node: NodeId,
) -> Result<usize, LiveRangeError> {
    positions
        .get(&node)
        .copied()
        .ok_or(LiveRangeError::MissingTopologicalNode { node })
}

fn is_persistent_role(env: &PredicateEnv, value: ValueId) -> bool {
    value_role_of(env, value) == Some(ValueRole::SequenceStateSlot)
}

fn is_immutable_graph_entity(env: &PredicateEnv, value: ValueId) -> bool {
    matches!(
        value_role_of(env, value),
        Some(
            ValueRole::ExpertWeight
                | ValueRole::RouterWeight
                | ValueRole::EmbeddingTable
                | ValueRole::LogitProj
                | ValueRole::NormParam
                | ValueRole::DecodeConst
                | ValueRole::LutFragment
        )
    )
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use gbf_policy::{StoragePlanDiagnosticCode, ValidationCode};

    use super::*;
    use crate::s1::quant_graph::TensorId;
    use crate::storage_plan::predicates::{
        PredicateValueFacts, QuantFormatId, ValueFormat, ValueRole, effective_lifetime_estimate,
    };
    use crate::storage_plan::types::StorageClass;

    #[test]
    fn abstract_live_ranges_track_disjoint_and_overlapping_intervals_in_topological_order() {
        let topological_order = vec![
            NodeId::new(10),
            NodeId::new(5),
            NodeId::new(20),
            NodeId::new(15),
        ];
        let nodes = vec![
            node(10, [], [ValueId::new(1)]),
            node(5, [ValueId::new(1)], [ValueId::new(2)]),
            node(20, [ValueId::new(2)], [ValueId::new(3)]),
            node(15, [ValueId::new(3)], []),
        ];
        let first = compute_abstract_live_range_from_nodes(
            &nodes,
            &BTreeSet::new(),
            &BTreeMap::new(),
            ValueId::new(1),
        )
        .expect("first range computes");
        let second = compute_abstract_live_range_from_nodes(
            &nodes,
            &BTreeSet::new(),
            &BTreeMap::new(),
            ValueId::new(3),
        )
        .expect("second range computes");
        let overlapping = compute_abstract_live_range_from_nodes(
            &nodes,
            &BTreeSet::new(),
            &BTreeMap::new(),
            ValueId::new(2),
        )
        .expect("overlap range computes");

        assert!(abstract_live_ranges_disjoint(&topological_order, &first, &second).unwrap());
        assert!(abstract_live_ranges_overlap(&topological_order, &first, &overlapping).unwrap());
    }

    #[test]
    fn abstract_live_range_uses_effective_lifetime_not_interval_topology() {
        let value = ValueId::new(12);
        let nodes = vec![node(1, [], [value]), node(2, [value], [])];
        let mut facts = activation_facts();
        facts.lifetime_estimate = Some(LifetimeClass::Slice);
        let env = PredicateEnv::new().with_value(value, facts);

        let range = compute_abstract_live_range_from_nodes_with_predicates(
            &nodes,
            &BTreeSet::new(),
            &env,
            value,
        )
        .expect("range computes");

        assert_eq!(range.first_use_node, Some(NodeId::new(2)));
        assert_eq!(range.last_use_node, Some(NodeId::new(2)));
        assert_eq!(range.lifetime_class, LifetimeClass::Slice);
        assert_eq!(
            effective_lifetime_estimate(&env, value),
            LifetimeClass::Slice
        );
    }

    #[test]
    fn observed_checkpoint_value_rejects_too_short_lifetime_with_store_017() {
        let value = ValueId::new(4);
        let env = PredicateEnv::new()
            .with_value(value, activation_facts())
            .with_observed_checkpoint_backing_value(value);
        let bounds = lifetime_bounds(&env, value);

        assert_eq!(bounds.min_required.lifetime, LifetimeClass::Token);
        let diagnostic = validate_lifetime_bounds(
            value,
            &Materialization::Materialize {
                class: StorageClass::WramHot,
                lifetime: LifetimeClass::Slice,
            },
            &bounds,
        )
        .expect_err("slice lifetime is below observed checkpoint floor");

        assert!(matches!(
            diagnostic.as_ref().code,
            ValidationCode::StoragePlan {
                code: StoragePlanDiagnosticCode::StorageLifetimeAdmissibilityViolation,
                ..
            }
        ));
    }

    #[test]
    fn expert_weight_has_persistent_lower_and_upper_bounds() {
        let value = ValueId::new(5);
        let env = PredicateEnv::new().with_value(value, const_facts(ValueRole::ExpertWeight));
        let bounds = lifetime_bounds(&env, value);

        assert_eq!(bounds.min_required.lifetime, LifetimeClass::Persistent);
        assert_eq!(bounds.max_admissible.lifetime, LifetimeClass::Persistent);
        validate_lifetime_bounds(
            value,
            &Materialization::Materialize {
                class: StorageClass::RomConst,
                lifetime: LifetimeClass::Persistent,
            },
            &bounds,
        )
        .expect("persistent expert weight is admissible");
    }

    #[test]
    fn min_required_lifetime_uses_strictest_applicable_floor() {
        let value = ValueId::new(55);
        let env = PredicateEnv::new()
            .with_value(value, const_facts(ValueRole::ExpertWeight))
            .with_observed_checkpoint_backing_value(value);

        let bound = min_required_lifetime(&env, value);

        assert_eq!(bound.lifetime, LifetimeClass::Persistent);
        assert_eq!(bound.source, LifetimeBoundSource::ImmutableGraphEntity);
    }

    #[test]
    fn pure_intermediate_uses_slice_floor_and_def_use_upper_bound() {
        let value = ValueId::new(6);
        let mut facts = activation_facts();
        facts.lifetime_estimate = Some(LifetimeClass::Token);
        let env = PredicateEnv::new().with_value(value, facts);
        let bounds = lifetime_bounds(&env, value);

        assert_eq!(bounds.min_required.lifetime, LifetimeClass::Slice);
        assert_eq!(bounds.max_admissible.lifetime, LifetimeClass::Token);
        assert!(bounds.max_admissible.lifetime <= LifetimeClass::Token);
    }

    #[test]
    fn tiny_routed_ffn_fixture_lifetimes_are_within_bounds() {
        let input = crate::storage_plan_test_infra::synth::tiny_routed_ffn_core_input();
        let roles: BTreeMap<_, _> = input
            .values
            .iter()
            .map(|value| (value.value, value.role))
            .collect();

        assert_eq!(roles.get(&ValueId::new(10)), Some(&ValueRole::ExpertWeight));
        assert_eq!(roles.get(&ValueId::new(11)), Some(&ValueRole::ExpertWeight));
        assert_eq!(
            roles.get(&ValueId::new(12)),
            Some(&ValueRole::RouterDecision)
        );
        assert_eq!(roles.get(&ValueId::new(13)), Some(&ValueRole::Scratch));
        assert_eq!(roles.get(&ValueId::new(14)), Some(&ValueRole::Scratch));

        for value in &input.values {
            assert_eq!(
                value.live_range.lifetime_class,
                effective_lifetime_estimate(&input.predicate_env, value.value),
                "fixture live range lifetime should come from effective_lifetime_estimate for {:?}",
                value.value
            );
            validate_lifetime_bounds(
                value.value,
                &value.materialization,
                &lifetime_bounds(&input.predicate_env, value.value),
            )
            .expect("canonical routed-FFN fixture value lifetime is admissible");
        }
    }

    fn node<I, O>(node: u32, inputs: I, outputs: O) -> AbstractDefUseNode
    where
        I: IntoIterator<Item = ValueId>,
        O: IntoIterator<Item = ValueId>,
    {
        AbstractDefUseNode {
            node: NodeId::new(node),
            inputs: inputs.into_iter().collect(),
            outputs: outputs.into_iter().collect(),
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

    fn const_facts(role: ValueRole) -> PredicateValueFacts {
        PredicateValueFacts::new(
            role,
            ValueFormat::ConstTensorRef {
                tensor_id: TensorId::new(1),
            },
        )
    }
}
