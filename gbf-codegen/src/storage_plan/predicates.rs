//! Closed Stage 6 predicate vocabulary consumed by storage decision rules.

use std::collections::{BTreeMap, BTreeSet};

use gbf_policy::{ReductionSiteId, StorageMaterialization};
use serde::{Deserialize, Serialize};

use crate::s1::quant_graph::TensorId;
use crate::s3::infer_ir::{NodeId, ValueId};
use crate::storage_plan::types::LifetimeClass;

pub const HRAM_ADMITTED_SET_COMPUTED_EVENT: &str = "f_b8.dr_12.admitted_set_computed";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ValueRole {
    Activation,
    Accumulator,
    Scratch,
    RouterDecision,
    RouterScore,
    RouterWeight,
    EmbeddingTable,
    LogitProj,
    NormParam,
    ExpertWeight,
    SequenceStateSlot,
    DecodeConst,
    InputToken,
    OutputToken,
    LutFragment,
    FfnIntermediate,
}

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct QuantFormatId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum FloatPrecision {
    F16,
    F32,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ValueFormat {
    QuantInt {
        quant_format_id: QuantFormatId,
    },
    FloatRef {
        precision: FloatPrecision,
    },
    /// Integer accumulator width, in bits. RFC F-B8 pins this as `u8`
    /// because Stage 6 only distinguishes small scalar accumulator domains.
    IntAccum {
        width_bits: u8,
    },
    TokenIdDomain {
        vocab_size: u32,
    },
    Flag,
    ConstTensorRef {
        tensor_id: TensorId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReductionSiteRef {
    pub node: NodeId,
    pub site: ReductionSiteId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PredicateValueFacts {
    pub role: Option<ValueRole>,
    pub format: Option<ValueFormat>,
    pub logical_size: Option<u32>,
    pub lifetime_estimate: Option<LifetimeClass>,
    pub pure: bool,
    pub hidden_state_output: bool,
}

impl PredicateValueFacts {
    #[must_use]
    pub fn new(role: ValueRole, format: ValueFormat) -> Self {
        Self {
            role: Some(role),
            format: Some(format),
            logical_size: None,
            lifetime_estimate: Some(LifetimeClass::Slice),
            pure: true,
            hidden_state_output: false,
        }
    }

    #[must_use]
    pub fn unknown() -> Self {
        Self {
            role: None,
            format: None,
            logical_size: None,
            lifetime_estimate: None,
            pure: false,
            hidden_state_output: false,
        }
    }
}

/// Predicate facts and precomputed rule surfaces for Stage 6.
///
/// This environment intentionally includes the RFC F-B8 §11 predicate surface
/// needed by the full decision-rule set, including persistence, trace,
/// transcript, HRAM admission, recompute-cost, and lifetime facts. Individual
/// task beads may close only a subset of these helpers, but downstream rules
/// must still use this typed surface rather than rereading upstream products.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PredicateEnv {
    values: BTreeMap<ValueId, PredicateValueFacts>,
    value_reduction_sites: BTreeMap<ValueId, BTreeSet<ReductionSiteRef>>,
    renorm_loop_sites: BTreeSet<ReductionSiteRef>,
    observed_checkpoint_backing_values: BTreeSet<ValueId>,
    forced_recompute_values: BTreeSet<ValueId>,
    recompute_cost_estimates: BTreeMap<ValueId, u32>,
    recompute_promotion: StorageMaterialization,
    recompute_cycle_ceiling: Option<u32>,
    precomputed_hram_admitted_values: BTreeSet<ValueId>,
    precomputed_hram_admission_cumulative_size: Option<u32>,
    allocatable_hram_budget: Option<u32>,
    resume_across_yield_values: BTreeSet<ValueId>,
    power_loss_resume_values: BTreeSet<ValueId>,
    continuation_live_values: BTreeSet<ValueId>,
    harness_io_values: BTreeSet<ValueId>,
    trace_probe_attached_values: BTreeSet<ValueId>,
    trace_capture_admitted_values: BTreeSet<ValueId>,
    transcript_capture_enabled: bool,
    transcript_inline_ceiling: Option<u32>,
    wram_hot_per_value_eligibility_ceiling: Option<u32>,
}

impl PredicateEnv {
    #[must_use]
    pub fn new() -> Self {
        Self {
            values: BTreeMap::new(),
            value_reduction_sites: BTreeMap::new(),
            renorm_loop_sites: BTreeSet::new(),
            observed_checkpoint_backing_values: BTreeSet::new(),
            forced_recompute_values: BTreeSet::new(),
            recompute_cost_estimates: BTreeMap::new(),
            recompute_promotion: StorageMaterialization::PreserveAll,
            recompute_cycle_ceiling: None,
            precomputed_hram_admitted_values: BTreeSet::new(),
            precomputed_hram_admission_cumulative_size: None,
            allocatable_hram_budget: None,
            resume_across_yield_values: BTreeSet::new(),
            power_loss_resume_values: BTreeSet::new(),
            continuation_live_values: BTreeSet::new(),
            harness_io_values: BTreeSet::new(),
            trace_probe_attached_values: BTreeSet::new(),
            trace_capture_admitted_values: BTreeSet::new(),
            transcript_capture_enabled: false,
            transcript_inline_ceiling: None,
            wram_hot_per_value_eligibility_ceiling: None,
        }
    }

    #[must_use]
    pub fn with_value(mut self, value: ValueId, facts: PredicateValueFacts) -> Self {
        self.values.insert(value, facts);
        self
    }

    #[must_use]
    pub fn with_value_reduction_site(mut self, value: ValueId, site: ReductionSiteRef) -> Self {
        self.value_reduction_sites
            .entry(value)
            .or_default()
            .insert(site);
        self
    }

    #[must_use]
    pub fn with_renorm_loop_site(mut self, site: ReductionSiteRef) -> Self {
        self.renorm_loop_sites.insert(site);
        self
    }

    #[must_use]
    pub fn with_observed_checkpoint_backing_value(mut self, value: ValueId) -> Self {
        self.observed_checkpoint_backing_values.insert(value);
        self
    }

    #[must_use]
    pub fn with_forced_recompute_value(mut self, value: ValueId) -> Self {
        self.forced_recompute_values.insert(value);
        self
    }

    #[must_use]
    pub fn with_recompute_cost_estimate(mut self, value: ValueId, cycles: u32) -> Self {
        self.recompute_cost_estimates.insert(value, cycles);
        self
    }

    #[must_use]
    pub fn with_recompute_promotion(mut self, promotion: StorageMaterialization) -> Self {
        self.recompute_promotion = promotion;
        self
    }

    #[must_use]
    pub fn with_recompute_cycle_ceiling(mut self, ceiling: u32) -> Self {
        self.recompute_cycle_ceiling = Some(ceiling);
        self
    }

    #[must_use]
    pub fn with_precomputed_hram_admitted_set(
        mut self,
        admitted: PrecomputedHramAdmittedSet,
    ) -> Self {
        let PrecomputedHramAdmittedSet {
            admitted_values,
            cumulative_logical_size,
            allocatable_budget,
            admission_order: _,
        } = admitted;
        self.precomputed_hram_admitted_values = admitted_values;
        self.precomputed_hram_admission_cumulative_size = Some(cumulative_logical_size);
        self.allocatable_hram_budget = Some(allocatable_budget);
        self
    }

    #[must_use]
    pub fn with_resume_across_yield_value(mut self, value: ValueId) -> Self {
        self.resume_across_yield_values.insert(value);
        self
    }

    #[must_use]
    pub fn with_power_loss_resume_value(mut self, value: ValueId) -> Self {
        self.power_loss_resume_values.insert(value);
        self
    }

    #[must_use]
    pub fn with_continuation_live_value(mut self, value: ValueId) -> Self {
        self.continuation_live_values.insert(value);
        self
    }

    #[must_use]
    pub fn with_harness_io_value(mut self, value: ValueId) -> Self {
        self.harness_io_values.insert(value);
        self
    }

    #[must_use]
    pub fn with_trace_probe_attached_value(mut self, value: ValueId) -> Self {
        self.trace_probe_attached_values.insert(value);
        self
    }

    #[must_use]
    pub fn with_trace_capture_admitted_value(mut self, value: ValueId) -> Self {
        self.trace_capture_admitted_values.insert(value);
        self
    }

    #[must_use]
    pub fn with_transcript_capture_enabled(mut self, enabled: bool) -> Self {
        self.transcript_capture_enabled = enabled;
        self
    }

    #[must_use]
    pub fn with_transcript_inline_ceiling(mut self, ceiling: u32) -> Self {
        self.transcript_inline_ceiling = Some(ceiling);
        self
    }

    #[must_use]
    pub fn with_wram_hot_per_value_eligibility_ceiling(mut self, ceiling: u32) -> Self {
        self.wram_hot_per_value_eligibility_ceiling = Some(ceiling);
        self
    }

    #[must_use]
    pub fn facts(&self, value: ValueId) -> Option<&PredicateValueFacts> {
        self.values.get(&value)
    }
}

impl Default for PredicateEnv {
    fn default() -> Self {
        Self::new()
    }
}

#[must_use]
pub fn role_known(env: &PredicateEnv, value: ValueId) -> bool {
    env.facts(value).and_then(|facts| facts.role).is_some()
}

#[must_use]
pub fn format_known(env: &PredicateEnv, value: ValueId) -> bool {
    env.facts(value)
        .and_then(|facts| facts.format.as_ref())
        .is_some()
}

#[must_use]
pub fn logical_size_known(env: &PredicateEnv, value: ValueId) -> bool {
    env.facts(value)
        .and_then(|facts| facts.logical_size)
        .is_some()
}

#[must_use]
pub fn logical_size_of(env: &PredicateEnv, value: ValueId) -> Option<u32> {
    env.facts(value).and_then(|facts| facts.logical_size)
}

#[must_use]
pub fn value_role_of(env: &PredicateEnv, value: ValueId) -> Option<ValueRole> {
    env.facts(value).and_then(|facts| facts.role)
}

#[must_use]
pub fn value_format_of(env: &PredicateEnv, value: ValueId) -> Option<&ValueFormat> {
    env.facts(value).and_then(|facts| facts.format.as_ref())
}

/// Returns true for scalar values whose role and format may be placed in HRAM.
///
/// This predicate is shape-only. Size and budget admission are enforced by
/// `precompute_hram_admitted_set`, which produces the explicit admitted set
/// consumed by DR-12.
#[must_use]
pub fn is_hot_scalar(env: &PredicateEnv, value: ValueId) -> bool {
    let Some(facts) = env.facts(value) else {
        return false;
    };
    let Some(role) = facts.role else {
        return false;
    };
    let Some(format) = facts.format.as_ref() else {
        return false;
    };

    matches!(
        role,
        ValueRole::Accumulator | ValueRole::RouterScore | ValueRole::RouterDecision
    ) && matches!(
        format,
        ValueFormat::IntAccum { width_bits: 8 | 16 }
            | ValueFormat::Flag
            | ValueFormat::TokenIdDomain { .. }
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrecomputedHramAdmittedSet {
    pub admitted_values: BTreeSet<ValueId>,
    pub admission_order: Vec<ValueId>,
    pub cumulative_logical_size: u32,
    pub allocatable_budget: u32,
}

#[must_use]
pub fn precompute_hram_admitted_set(
    env: &PredicateEnv,
    allocatable_budget: u32,
) -> PrecomputedHramAdmittedSet {
    let mut candidates: Vec<_> = env
        .values
        .iter()
        .filter_map(|(&value, facts)| {
            if !is_hot_scalar(env, value)
                || effective_lifetime_estimate(env, value) != LifetimeClass::Slice
            {
                return None;
            }

            let logical_size = facts.logical_size?;
            let role_priority = hot_scalar_role_priority(facts)?;
            Some((role_priority, logical_size, value))
        })
        .collect();
    candidates.sort_unstable_by_key(|&(role_priority, logical_size, value)| {
        (role_priority, logical_size, value)
    });

    let candidates_count = candidates.len() as u64;
    let mut admitted_values = BTreeSet::new();
    let mut admission_order = Vec::new();
    let mut cumulative = 0_u32;
    for (_, logical_size, value) in candidates {
        let next = u64::from(cumulative) + u64::from(logical_size);
        if next > u64::from(allocatable_budget) {
            break;
        }

        cumulative = next as u32;
        admitted_values.insert(value);
        admission_order.push(value);
    }

    tracing::info!(
        target: "gbf_codegen::storage_plan",
        event = HRAM_ADMITTED_SET_COMPUTED_EVENT,
        candidates_count,
        admitted_count = admission_order.len() as u64,
        total_logical_bytes = cumulative as u64,
        "storage DR-12 HRAM admitted set computed"
    );

    PrecomputedHramAdmittedSet {
        admitted_values,
        admission_order,
        cumulative_logical_size: cumulative,
        allocatable_budget,
    }
}

fn hot_scalar_role_priority(facts: &PredicateValueFacts) -> Option<u8> {
    match facts.role? {
        ValueRole::RouterDecision => Some(0),
        ValueRole::RouterScore => Some(1),
        ValueRole::Accumulator => Some(2),
        _ if matches!(facts.format.as_ref(), Some(ValueFormat::Flag)) => Some(3),
        _ => None,
    }
}

#[must_use]
pub fn is_large_activation(env: &PredicateEnv, value: ValueId) -> bool {
    let Some(facts) = env.facts(value) else {
        return false;
    };
    matches!(
        facts.role,
        Some(ValueRole::Activation | ValueRole::FfnIntermediate)
    ) && match (
        facts.logical_size,
        env.wram_hot_per_value_eligibility_ceiling,
    ) {
        (Some(size), Some(ceiling)) => size > ceiling,
        _ => false,
    }
}

#[must_use]
pub fn is_expert_weight(env: &PredicateEnv, value: ValueId) -> bool {
    value_role_of(env, value) == Some(ValueRole::ExpertWeight)
}

#[must_use]
pub fn is_router_table(env: &PredicateEnv, value: ValueId) -> bool {
    value_role_of(env, value) == Some(ValueRole::RouterWeight)
}

#[must_use]
pub fn is_const_tensor_ref_value(env: &PredicateEnv, value: ValueId) -> bool {
    matches!(
        value_format_of(env, value),
        Some(ValueFormat::ConstTensorRef { .. })
    )
}

#[must_use]
pub fn is_sequence_state_slot(env: &PredicateEnv, value: ValueId) -> bool {
    value_role_of(env, value) == Some(ValueRole::SequenceStateSlot)
}

#[must_use]
pub fn is_renorm_loop_scratch(env: &PredicateEnv, value: ValueId) -> bool {
    if value_role_of(env, value) != Some(ValueRole::Scratch) {
        return false;
    }

    env.value_reduction_sites.get(&value).is_some_and(|sites| {
        sites
            .iter()
            .any(|site| env.renorm_loop_sites.contains(site))
    })
}

#[must_use]
pub fn is_pure_value(env: &PredicateEnv, value: ValueId) -> bool {
    env.facts(value)
        .is_some_and(|facts| facts.pure && !facts.hidden_state_output)
}

#[must_use]
pub fn is_observed_checkpoint_backing_value(env: &PredicateEnv, value: ValueId) -> bool {
    env.observed_checkpoint_backing_values.contains(&value)
}

#[must_use]
pub fn is_forced_recompute_value(env: &PredicateEnv, value: ValueId) -> bool {
    env.forced_recompute_values.contains(&value)
}

#[must_use]
pub fn recompute_promotion_admits_pure_slice_values(env: &PredicateEnv) -> bool {
    env.recompute_promotion >= StorageMaterialization::RecomputePureValues
}

#[must_use]
pub fn recompute_cost_within_cycle_ceiling(env: &PredicateEnv, value: ValueId) -> bool {
    match (
        env.recompute_cost_estimates.get(&value),
        env.recompute_cycle_ceiling,
    ) {
        (Some(cycles), Some(ceiling)) => *cycles <= ceiling,
        _ => false,
    }
}

#[must_use]
pub fn is_precomputed_hram_admitted(env: &PredicateEnv, value: ValueId) -> bool {
    env.precomputed_hram_admitted_values.contains(&value)
}

#[must_use]
pub fn precomputed_hram_admission_exceeds_budget(env: &PredicateEnv) -> bool {
    match (
        env.precomputed_hram_admission_cumulative_size,
        env.allocatable_hram_budget,
    ) {
        (Some(cumulative), Some(budget)) => cumulative > budget,
        _ => false,
    }
}

#[must_use]
pub fn participates_in_resume_across_yield(env: &PredicateEnv, value: ValueId) -> bool {
    env.resume_across_yield_values.contains(&value)
}

#[must_use]
pub fn must_survive_power_loss(env: &PredicateEnv, value: ValueId) -> bool {
    env.power_loss_resume_values.contains(&value)
}

#[must_use]
pub fn is_continuation_live_value(env: &PredicateEnv, value: ValueId) -> bool {
    env.continuation_live_values.contains(&value)
}

#[must_use]
pub fn storage_predicate_transcript_capture_enabled(env: &PredicateEnv) -> bool {
    env.transcript_capture_enabled
}

#[must_use]
pub fn exceeds_transcript_inline_ceiling(env: &PredicateEnv, value: ValueId) -> bool {
    match (
        env.facts(value).and_then(|facts| facts.logical_size),
        env.transcript_inline_ceiling,
    ) {
        (Some(size), Some(ceiling)) => size > ceiling,
        _ => false,
    }
}

#[must_use]
pub fn participates_in_harness_io(env: &PredicateEnv, value: ValueId) -> bool {
    env.harness_io_values.contains(&value)
}

#[must_use]
pub fn is_trace_probe_attached_value(env: &PredicateEnv, value: ValueId) -> bool {
    env.trace_probe_attached_values.contains(&value)
}

#[must_use]
pub fn trace_capture_admits_value(env: &PredicateEnv, value: ValueId) -> bool {
    env.trace_capture_admitted_values.contains(&value)
}

#[must_use]
pub fn predicate_wram_hot_per_value_eligibility_ceiling(env: &PredicateEnv) -> Option<u32> {
    env.wram_hot_per_value_eligibility_ceiling
}

#[must_use]
pub fn recompute_allowed(env: &PredicateEnv, value: ValueId) -> bool {
    let Some(role) = value_role_of(env, value) else {
        return false;
    };

    is_pure_value(env, value)
        && !is_observed_checkpoint_backing_value(env, value)
        && !is_sequence_state_slot(env, value)
        && !matches!(
            role,
            ValueRole::RouterDecision
                | ValueRole::RouterWeight
                | ValueRole::ExpertWeight
                | ValueRole::EmbeddingTable
                | ValueRole::LogitProj
                | ValueRole::NormParam
                | ValueRole::DecodeConst
                | ValueRole::LutFragment
        )
        && effective_lifetime_estimate(env, value) == LifetimeClass::Slice
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum RecomputeAllowedFailurePredicate {
    ValueRoleOf,
    IsPureValue,
    IsObservedCheckpointBackingValue,
    IsSequenceStateSlot,
    EffectiveLifetimeEstimate,
}

#[must_use]
pub fn recompute_allowed_failure_predicates(
    env: &PredicateEnv,
    value: ValueId,
) -> BTreeSet<RecomputeAllowedFailurePredicate> {
    let mut failures = BTreeSet::new();

    match value_role_of(env, value) {
        None => {
            failures.insert(RecomputeAllowedFailurePredicate::ValueRoleOf);
        }
        Some(
            ValueRole::RouterDecision
            | ValueRole::RouterWeight
            | ValueRole::ExpertWeight
            | ValueRole::EmbeddingTable
            | ValueRole::LogitProj
            | ValueRole::NormParam
            | ValueRole::DecodeConst
            | ValueRole::LutFragment,
        ) => {
            failures.insert(RecomputeAllowedFailurePredicate::ValueRoleOf);
        }
        Some(_) => {}
    }

    if !is_pure_value(env, value) {
        failures.insert(RecomputeAllowedFailurePredicate::IsPureValue);
    }
    if is_observed_checkpoint_backing_value(env, value) {
        failures.insert(RecomputeAllowedFailurePredicate::IsObservedCheckpointBackingValue);
    }
    if is_sequence_state_slot(env, value) {
        failures.insert(RecomputeAllowedFailurePredicate::IsSequenceStateSlot);
    }
    if effective_lifetime_estimate(env, value) != LifetimeClass::Slice {
        failures.insert(RecomputeAllowedFailurePredicate::EffectiveLifetimeEstimate);
    }

    failures
}

#[must_use]
pub fn effective_lifetime_estimate(env: &PredicateEnv, value: ValueId) -> LifetimeClass {
    env.facts(value)
        .and_then(|facts| facts.lifetime_estimate.clone())
        .unwrap_or(LifetimeClass::Token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hot_scalar_true_and_false() {
        let hot = ValueId::new(1);
        let cold = ValueId::new(2);
        let env = PredicateEnv::new()
            .with_value(
                hot,
                PredicateValueFacts::new(
                    ValueRole::Accumulator,
                    ValueFormat::IntAccum { width_bits: 16 },
                ),
            )
            .with_value(
                cold,
                PredicateValueFacts::new(
                    ValueRole::Activation,
                    ValueFormat::IntAccum { width_bits: 16 },
                ),
            );

        assert!(is_hot_scalar(&env, hot));
        assert!(!is_hot_scalar(&env, cold));
    }

    #[test]
    fn int_accum_width_bits_is_rfc_u8_surface() {
        let ValueFormat::IntAccum { width_bits } = (ValueFormat::IntAccum { width_bits: 16 })
        else {
            unreachable!("constructed IntAccum")
        };
        let _: u8 = width_bits;
    }

    #[test]
    fn large_activation_uses_policy_ceiling() {
        let large = ValueId::new(1);
        let small = ValueId::new(2);
        let mut large_facts = PredicateValueFacts::new(
            ValueRole::Activation,
            ValueFormat::QuantInt {
                quant_format_id: QuantFormatId(1),
            },
        );
        large_facts.logical_size = Some(65);
        let mut small_facts = large_facts.clone();
        small_facts.logical_size = Some(64);

        let env = PredicateEnv::new()
            .with_wram_hot_per_value_eligibility_ceiling(64)
            .with_value(large, large_facts)
            .with_value(small, small_facts);

        assert!(is_large_activation(&env, large));
        assert!(!is_large_activation(&env, small));
    }

    #[test]
    fn role_predicates_true_and_false() {
        let expert = ValueId::new(1);
        let router = ValueId::new(2);
        let sequence = ValueId::new(3);
        let other = ValueId::new(4);
        let env = PredicateEnv::new()
            .with_value(expert, facts(ValueRole::ExpertWeight))
            .with_value(router, facts(ValueRole::RouterWeight))
            .with_value(sequence, facts(ValueRole::SequenceStateSlot))
            .with_value(other, facts(ValueRole::Activation));

        assert!(is_expert_weight(&env, expert));
        assert!(!is_expert_weight(&env, other));
        assert!(is_router_table(&env, router));
        assert!(!is_router_table(&env, other));
        assert!(is_sequence_state_slot(&env, sequence));
        assert!(!is_sequence_state_slot(&env, other));
    }

    #[test]
    fn renorm_loop_scratch_keys_by_node_and_site() {
        let scratch = ValueId::new(1);
        let same_node_matching_site = site(7, "renorm");
        let same_node_other_site = site(7, "plain");

        let false_env = PredicateEnv::new()
            .with_value(scratch, facts(ValueRole::Scratch))
            .with_value_reduction_site(scratch, same_node_other_site.clone())
            .with_renorm_loop_site(same_node_matching_site.clone());
        let true_env = PredicateEnv::new()
            .with_value(scratch, facts(ValueRole::Scratch))
            .with_value_reduction_site(scratch, same_node_matching_site.clone())
            .with_renorm_loop_site(same_node_matching_site);

        assert!(!is_renorm_loop_scratch(&false_env, scratch));
        assert!(is_renorm_loop_scratch(&true_env, scratch));
    }

    #[test]
    fn pure_value_true_and_false() {
        let pure = ValueId::new(1);
        let effectful = ValueId::new(2);
        let hidden = ValueId::new(3);
        let mut effectful_facts = facts(ValueRole::Activation);
        effectful_facts.pure = false;
        let mut hidden_facts = facts(ValueRole::Activation);
        hidden_facts.hidden_state_output = true;
        let env = PredicateEnv::new()
            .with_value(pure, facts(ValueRole::Activation))
            .with_value(effectful, effectful_facts)
            .with_value(hidden, hidden_facts);

        assert!(is_pure_value(&env, pure));
        assert!(!is_pure_value(&env, effectful));
        assert!(!is_pure_value(&env, hidden));
    }

    #[test]
    fn observed_checkpoint_backing_value_true_and_false() {
        let observed = ValueId::new(1);
        let plain = ValueId::new(2);
        let env = PredicateEnv::new()
            .with_value(observed, facts(ValueRole::Activation))
            .with_value(plain, facts(ValueRole::Activation))
            .with_observed_checkpoint_backing_value(observed);

        assert!(is_observed_checkpoint_backing_value(&env, observed));
        assert!(!is_observed_checkpoint_backing_value(&env, plain));
    }

    #[test]
    fn forced_recompute_value_true_and_false() {
        let forced = ValueId::new(1);
        let plain = ValueId::new(2);
        let env = PredicateEnv::new()
            .with_value(forced, facts(ValueRole::Activation))
            .with_value(plain, facts(ValueRole::Activation))
            .with_forced_recompute_value(forced);

        assert!(is_forced_recompute_value(&env, forced));
        assert!(!is_forced_recompute_value(&env, plain));
    }

    #[test]
    fn recompute_allowed_admits_slice_pure_activation_but_excludes_router_decision() {
        let activation = ValueId::new(1);
        let router_decision = ValueId::new(2);
        let token_lifetime = ValueId::new(3);
        let mut token_lifetime_facts = facts(ValueRole::Activation);
        token_lifetime_facts.lifetime_estimate = Some(LifetimeClass::Token);
        let env = PredicateEnv::new()
            .with_value(activation, facts(ValueRole::Activation))
            .with_value(router_decision, facts(ValueRole::RouterDecision))
            .with_value(token_lifetime, token_lifetime_facts);

        assert!(recompute_allowed(&env, activation));
        assert!(!recompute_allowed(&env, router_decision));
        assert!(!recompute_allowed(&env, token_lifetime));
    }

    #[test]
    fn recompute_allowed_failure_predicates_name_router_role_failure() {
        let router_decision = ValueId::new(1);
        let env = PredicateEnv::new().with_value(router_decision, facts(ValueRole::RouterDecision));

        let failures = recompute_allowed_failure_predicates(&env, router_decision);

        assert!(failures.contains(&RecomputeAllowedFailurePredicate::ValueRoleOf));
    }

    #[test]
    fn known_predicates_true_and_false() {
        let known = ValueId::new(1);
        let unknown = ValueId::new(2);
        let mut known_facts = facts(ValueRole::Activation);
        known_facts.logical_size = Some(8);
        let env = PredicateEnv::new()
            .with_value(known, known_facts)
            .with_value(unknown, PredicateValueFacts::unknown());

        assert!(role_known(&env, known));
        assert!(format_known(&env, known));
        assert!(logical_size_known(&env, known));
        assert!(!role_known(&env, unknown));
        assert!(!format_known(&env, unknown));
        assert!(!logical_size_known(&env, unknown));
    }

    #[test]
    fn effective_lifetime_estimate_collapses_unknown_to_token() {
        let known = ValueId::new(1);
        let unknown = ValueId::new(2);
        let env = PredicateEnv::new()
            .with_value(known, facts(ValueRole::Activation))
            .with_value(unknown, PredicateValueFacts::unknown());

        assert_eq!(
            effective_lifetime_estimate(&env, known),
            LifetimeClass::Slice
        );
        assert_eq!(
            effective_lifetime_estimate(&env, unknown),
            LifetimeClass::Token
        );
    }

    fn facts(role: ValueRole) -> PredicateValueFacts {
        PredicateValueFacts::new(
            role,
            ValueFormat::QuantInt {
                quant_format_id: QuantFormatId(1),
            },
        )
    }

    fn site(node: u32, site: &str) -> ReductionSiteRef {
        ReductionSiteRef {
            node: NodeId::new(node),
            site: ReductionSiteId(site.to_owned()),
        }
    }
}
