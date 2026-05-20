//! Alias-class construction for Stage 6 storage planning.

use std::collections::{BTreeMap, BTreeSet};

use gbf_foundation::CanonicalJson;
use gbf_policy::StoragePlanDiagnosticCode;
use serde::{Deserialize, Serialize};

use crate::s3::infer_ir::{NodeId, ValueId};
use crate::storage_plan::lifetime::{LiveRangeError, abstract_live_ranges_disjoint};
use crate::storage_plan::predicates::ValueRole;
use crate::storage_plan::types::{
    AbstractLiveRange, AliasClass, AliasClassError, AliasClassFingerprint, AliasClassId,
    AliasIntent, LifetimeClass, Materialization, NonEmptySortedSet, PersistKind, StorageClass,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AliasSeedBinding {
    pub value: ValueId,
    pub materialization: Materialization,
    pub live_range: AbstractLiveRange,
    pub role: ValueRole,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AliasCandidateEdge {
    pub left: ValueId,
    pub right: ValueId,
    pub intent: AliasIntent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AliasEngineDiagnostic {
    pub code: StoragePlanDiagnosticCode,
    pub members: Vec<ValueId>,
    pub intent: Option<AliasIntent>,
    pub intents: Vec<AliasIntent>,
}

pub fn build_alias_classes(
    bindings: &[AliasSeedBinding],
    edges: &[AliasCandidateEdge],
    topological_order: &[NodeId],
) -> Result<Vec<AliasClass>, AliasEngineDiagnostic> {
    build_alias_classes_with_fingerprint_override(bindings, edges, topological_order, None)
}

fn build_alias_classes_with_fingerprint_override(
    bindings: &[AliasSeedBinding],
    edges: &[AliasCandidateEdge],
    topological_order: &[NodeId],
    fingerprint_override: Option<AliasClassFingerprint>,
) -> Result<Vec<AliasClass>, AliasEngineDiagnostic> {
    let value_to_index: BTreeMap<_, _> = bindings
        .iter()
        .enumerate()
        .map(|(index, binding)| (binding.value, index))
        .collect();
    let binding_by_value: BTreeMap<_, _> = bindings
        .iter()
        .map(|binding| (binding.value, binding))
        .collect();
    let mut union_find = UnionFind::new(bindings.len());

    for edge in edges {
        let (Some(&left), Some(&right)) = (
            value_to_index.get(&edge.left),
            value_to_index.get(&edge.right),
        ) else {
            continue;
        };
        union_find.union(left, right);
    }

    let mut components: BTreeMap<usize, ComponentDraft> = BTreeMap::new();
    for binding in bindings {
        let root = union_find.find(value_to_index[&binding.value]);
        components
            .entry(root)
            .or_default()
            .members
            .insert(binding.value);
    }
    for edge in edges {
        let Some(&left) = value_to_index.get(&edge.left) else {
            continue;
        };
        let root = union_find.find(left);
        components
            .entry(root)
            .or_default()
            .intents
            .insert(edge.intent);
    }

    let mut payloads = Vec::new();
    for component in components.into_values() {
        let intent = component_intent(&component)?;
        validate_component(intent, &component, &binding_by_value, topological_order)?;
        let payload = AliasClassPayload {
            members: component.members.iter().copied().collect(),
            intent,
        };
        let members = NonEmptySortedSet::from_btree_set(component.members).map_err(|_| {
            AliasEngineDiagnostic {
                code: StoragePlanDiagnosticCode::StorageAliasIntentCardinalityViolation,
                members: vec![],
                intent: Some(intent),
                intents: vec![intent],
            }
        })?;
        let fingerprint = fingerprint_override.unwrap_or_else(|| {
            AliasClassFingerprint::for_members(&members, intent)
                .expect("alias fingerprint computes")
        });
        let canonical_payload =
            CanonicalJson::to_vec(&payload).expect("alias payload canonicalizes");
        payloads.push(AliasClassDraft {
            fingerprint,
            canonical_payload,
            members,
            intent,
        });
    }

    reject_fingerprint_collisions(&payloads)?;
    payloads.sort_by(|left, right| {
        (left.fingerprint, &left.canonical_payload)
            .cmp(&(right.fingerprint, &right.canonical_payload))
    });

    payloads
        .into_iter()
        .enumerate()
        .map(|(index, payload)| {
            AliasClass::new(AliasClassId(index as u32), payload.members, payload.intent)
                .map_err(alias_class_error_to_diagnostic)
        })
        .collect()
}

pub fn pp_scratch_reuse(
    left: &AliasSeedBinding,
    right: &AliasSeedBinding,
    topological_order: &[NodeId],
) -> Result<bool, LiveRangeError> {
    Ok(is_wram_hot_slice(left)
        && is_wram_hot_slice(right)
        && matches!(
            (left.role, right.role),
            (
                ValueRole::Scratch | ValueRole::Accumulator,
                ValueRole::Scratch | ValueRole::Accumulator
            )
        )
        && abstract_live_ranges_disjoint(topological_order, &left.live_range, &right.live_range)?)
}

pub fn pp_ping_pong(left: &AliasSeedBinding, right: &AliasSeedBinding, typed_pair: bool) -> bool {
    is_wram_hot(left)
        && is_wram_hot(right)
        && left.role == right.role
        && matches!(
            left.role,
            ValueRole::Activation | ValueRole::FfnIntermediate
        )
        && typed_pair
}

pub fn pp_resume_overlap(
    survivor: &AliasSeedBinding,
    slice: &AliasSeedBinding,
    resume_boundary: bool,
) -> bool {
    is_hot_resume_window(survivor) && is_hot_slice(slice) && resume_boundary
}

pub fn pp_persist_rotation(
    left: &AliasSeedBinding,
    right: &AliasSeedBinding,
    left_kind: PersistKind,
    right_kind: PersistKind,
    rotation_pair_declared: bool,
) -> bool {
    let (
        Materialization::Persist {
            page: left_page,
            commit_group: left_group,
        },
        Materialization::Persist {
            page: right_page,
            commit_group: right_group,
        },
    ) = (&left.materialization, &right.materialization)
    else {
        return false;
    };

    left_group == right_group
        && left_page != right_page
        && left_kind == right_kind
        && rotation_pair_declared
}

fn is_wram_hot_slice(binding: &AliasSeedBinding) -> bool {
    matches!(
        binding.materialization,
        Materialization::Materialize {
            class: StorageClass::WramHot,
            lifetime: LifetimeClass::Slice
        }
    )
}

fn is_hot_slice(binding: &AliasSeedBinding) -> bool {
    matches!(
        binding.materialization,
        Materialization::Materialize {
            class: StorageClass::WramHot | StorageClass::HramHot,
            lifetime: LifetimeClass::Slice
        }
    )
}

fn is_hot_resume_window(binding: &AliasSeedBinding) -> bool {
    matches!(
        binding.materialization,
        Materialization::Materialize {
            class: StorageClass::WramHot | StorageClass::HramHot,
            lifetime: LifetimeClass::ResumeWindow
        }
    )
}

fn is_wram_hot(binding: &AliasSeedBinding) -> bool {
    matches!(
        binding.materialization,
        Materialization::Materialize {
            class: StorageClass::WramHot,
            ..
        }
    )
}

fn component_intent(component: &ComponentDraft) -> Result<AliasIntent, AliasEngineDiagnostic> {
    if component.members.len() == 1 {
        return Ok(AliasIntent::NoAlias);
    }
    if component.intents.len() > 1 {
        return Err(AliasEngineDiagnostic {
            code: StoragePlanDiagnosticCode::StorageAliasMixedIntentComponent,
            members: component.members.iter().copied().collect(),
            intent: None,
            intents: component.intents.iter().copied().collect(),
        });
    }
    component
        .intents
        .iter()
        .copied()
        .next()
        .ok_or_else(|| AliasEngineDiagnostic {
            code: StoragePlanDiagnosticCode::StorageAliasIntentCardinalityViolation,
            members: component.members.iter().copied().collect(),
            intent: None,
            intents: vec![],
        })
}

fn validate_component(
    intent: AliasIntent,
    component: &ComponentDraft,
    binding_by_value: &BTreeMap<ValueId, &AliasSeedBinding>,
    topological_order: &[NodeId],
) -> Result<(), AliasEngineDiagnostic> {
    match intent {
        AliasIntent::NoAlias => Ok(()),
        AliasIntent::ScratchReuse => {
            validate_scratch_reuse(component, binding_by_value, topological_order)
        }
        AliasIntent::PingPong | AliasIntent::PersistRotation if component.members.len() != 2 => {
            Err(AliasEngineDiagnostic {
                code: StoragePlanDiagnosticCode::StorageAliasIntentCardinalityViolation,
                members: component.members.iter().copied().collect(),
                intent: Some(intent),
                intents: vec![intent],
            })
        }
        AliasIntent::PingPong | AliasIntent::PersistRotation | AliasIntent::ResumeOverlap => Ok(()),
    }
}

fn validate_scratch_reuse(
    component: &ComponentDraft,
    binding_by_value: &BTreeMap<ValueId, &AliasSeedBinding>,
    topological_order: &[NodeId],
) -> Result<(), AliasEngineDiagnostic> {
    let members: Vec<_> = component.members.iter().copied().collect();
    for (left_index, left) in members.iter().enumerate() {
        for right in members.iter().skip(left_index + 1) {
            let left_binding = binding_by_value[left];
            let right_binding = binding_by_value[right];
            let disjoint = abstract_live_ranges_disjoint(
                topological_order,
                &left_binding.live_range,
                &right_binding.live_range,
            )
            .map_err(|_| AliasEngineDiagnostic {
                code: StoragePlanDiagnosticCode::StorageAliasClassOverlapWithoutIntent,
                members: vec![*left, *right],
                intent: Some(AliasIntent::ScratchReuse),
                intents: vec![AliasIntent::ScratchReuse],
            })?;
            if !disjoint {
                return Err(AliasEngineDiagnostic {
                    code: StoragePlanDiagnosticCode::StorageAliasClassOverlapWithoutIntent,
                    members: vec![*left, *right],
                    intent: Some(AliasIntent::ScratchReuse),
                    intents: vec![AliasIntent::ScratchReuse],
                });
            }
        }
    }
    Ok(())
}

fn reject_fingerprint_collisions(
    payloads: &[AliasClassDraft],
) -> Result<(), AliasEngineDiagnostic> {
    let mut seen: BTreeMap<AliasClassFingerprint, Vec<u8>> = BTreeMap::new();
    for payload in payloads {
        if let Some(existing) = seen.get(&payload.fingerprint) {
            if existing != &payload.canonical_payload {
                return Err(AliasEngineDiagnostic {
                    code: StoragePlanDiagnosticCode::StorageAliasClassFingerprintCollision,
                    members: payload.members.iter().copied().collect(),
                    intent: Some(payload.intent),
                    intents: vec![payload.intent],
                });
            }
        } else {
            seen.insert(payload.fingerprint, payload.canonical_payload.clone());
        }
    }
    Ok(())
}

fn alias_class_error_to_diagnostic(error: AliasClassError) -> AliasEngineDiagnostic {
    AliasEngineDiagnostic {
        code: StoragePlanDiagnosticCode::StorageAliasIntentCardinalityViolation,
        members: vec![],
        intent: match error {
            AliasClassError::SingletonMustBeNoAlias { intent } => Some(intent),
            AliasClassError::NoAliasRequiresSingleton { .. }
            | AliasClassError::EmptyMembers
            | AliasClassError::Fingerprint(_) => None,
        },
        intents: vec![],
    }
}

#[derive(Default)]
struct ComponentDraft {
    members: BTreeSet<ValueId>,
    intents: BTreeSet<AliasIntent>,
}

struct AliasClassDraft {
    fingerprint: AliasClassFingerprint,
    canonical_payload: Vec<u8>,
    members: NonEmptySortedSet<ValueId>,
    intent: AliasIntent,
}

#[derive(Serialize)]
struct AliasClassPayload {
    members: Vec<ValueId>,
    intent: AliasIntent,
}

struct UnionFind {
    parents: Vec<usize>,
}

impl UnionFind {
    fn new(len: usize) -> Self {
        Self {
            parents: (0..len).collect(),
        }
    }

    fn find(&mut self, index: usize) -> usize {
        let parent = self.parents[index];
        if parent == index {
            index
        } else {
            let root = self.find(parent);
            self.parents[index] = root;
            root
        }
    }

    fn union(&mut self, left: usize, right: usize) {
        let left_root = self.find(left);
        let right_root = self.find(right);
        if left_root != right_root {
            self.parents[right_root] = left_root;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage_plan::types::{CommitGroupId, PersistPageId};
    #[test]
    fn mixed_intent_component_rejects_store_031() {
        let bindings = bindings([1, 2, 3]);
        let error = build_alias_classes(
            &bindings,
            &[
                edge(1, 2, AliasIntent::ScratchReuse),
                edge(2, 3, AliasIntent::PingPong),
            ],
            &topological_order(),
        )
        .expect_err("mixed intent component rejects");

        assert_eq!(
            error.code,
            StoragePlanDiagnosticCode::StorageAliasMixedIntentComponent
        );
        assert_eq!(
            error.intents,
            vec![AliasIntent::ScratchReuse, AliasIntent::PingPong]
        );
    }

    #[test]
    fn ping_pong_three_member_component_rejects_store_032() {
        let bindings = activation_bindings([1, 2, 3]);
        let error = build_alias_classes(
            &bindings,
            &[
                edge(1, 2, AliasIntent::PingPong),
                edge(2, 3, AliasIntent::PingPong),
            ],
            &topological_order(),
        )
        .expect_err("three-member ping-pong rejects");

        assert_eq!(
            error.code,
            StoragePlanDiagnosticCode::StorageAliasIntentCardinalityViolation
        );
    }

    #[test]
    fn scratch_reuse_checks_whole_component_for_live_range_overlap() {
        let mut bindings = bindings([1, 2, 3]);
        bindings[0].live_range = live_range(1, Some(3));
        bindings[1].live_range = live_range(4, Some(5));
        bindings[2].live_range = live_range(2, Some(6));

        let error = build_alias_classes(
            &bindings,
            &[
                edge(1, 2, AliasIntent::ScratchReuse),
                edge(2, 3, AliasIntent::ScratchReuse),
            ],
            &topological_order(),
        )
        .expect_err("component overlap rejects");

        assert_eq!(
            error.code,
            StoragePlanDiagnosticCode::StorageAliasClassOverlapWithoutIntent
        );
        assert_eq!(error.members, vec![ValueId::new(1), ValueId::new(3)]);
    }

    #[test]
    fn singleton_class_gets_noalias_intent() {
        let classes =
            build_alias_classes(&[binding(1, ValueRole::Scratch)], &[], &topological_order())
                .expect("singleton builds");

        assert_eq!(classes.len(), 1);
        assert_eq!(classes[0].intent(), AliasIntent::NoAlias);
        assert_eq!(*classes[0].id(), AliasClassId(0));
    }

    #[test]
    fn fingerprint_is_stable_even_when_dense_id_changes() {
        let first = build_alias_classes(
            &bindings([1, 2]),
            &[edge(1, 2, AliasIntent::ScratchReuse)],
            &topological_order(),
        )
        .expect("first build");
        let second = build_alias_classes(
            &bindings([1, 2, 3]),
            &[edge(1, 2, AliasIntent::ScratchReuse)],
            &topological_order(),
        )
        .expect("second build");
        let first_pair = first
            .iter()
            .find(|class| class.intent() == AliasIntent::ScratchReuse)
            .expect("pair exists");
        let second_pair = second
            .iter()
            .find(|class| class.intent() == AliasIntent::ScratchReuse)
            .expect("pair exists");

        assert_eq!(first_pair.fingerprint(), second_pair.fingerprint());
    }

    #[test]
    fn fingerprint_collision_rejects_store_035() {
        let forced = AliasClassFingerprint(gbf_foundation::Hash256::from_bytes([7; 32]));
        let error = build_alias_classes_with_fingerprint_override(
            &activation_bindings([1, 2, 3]),
            &[edge(1, 2, AliasIntent::PingPong)],
            &topological_order(),
            Some(forced),
        )
        .expect_err("forced collision rejects");

        assert_eq!(
            error.code,
            StoragePlanDiagnosticCode::StorageAliasClassFingerprintCollision
        );
    }

    #[test]
    fn scratch_reuse_pair_predicate_uses_abstract_live_range_disjointness() {
        let order = topological_order();
        let left = binding(1, ValueRole::Scratch);
        let mut right = binding(2, ValueRole::Accumulator);
        right.live_range = live_range(4, Some(5));

        assert!(pp_scratch_reuse(&left, &right, &order).expect("predicate computes"));
    }

    #[test]
    fn resume_overlap_predicate_requires_resume_window_slice_hot_pair_and_boundary() {
        let mut survivor = binding(1, ValueRole::Activation);
        survivor.materialization = Materialization::Materialize {
            class: StorageClass::HramHot,
            lifetime: LifetimeClass::ResumeWindow,
        };
        let slice = binding(2, ValueRole::Activation);

        assert!(pp_resume_overlap(&survivor, &slice, true));
        assert!(!pp_resume_overlap(&survivor, &slice, false));
        assert!(!pp_resume_overlap(&slice, &survivor, true));
    }

    #[test]
    fn persist_rotation_predicate_requires_same_group_distinct_pages_kind_and_declaration() {
        let left = persist_binding(1, 1, 7);
        let right = persist_binding(2, 2, 7);
        let same_page = persist_binding(3, 1, 7);
        let other_group = persist_binding(4, 3, 8);

        assert!(pp_persist_rotation(
            &left,
            &right,
            PersistKind::Trace,
            PersistKind::Trace,
            true,
        ));
        assert!(!pp_persist_rotation(
            &left,
            &right,
            PersistKind::Trace,
            PersistKind::Harness,
            true,
        ));
        assert!(!pp_persist_rotation(
            &left,
            &right,
            PersistKind::Trace,
            PersistKind::Trace,
            false,
        ));
        assert!(!pp_persist_rotation(
            &left,
            &same_page,
            PersistKind::Trace,
            PersistKind::Trace,
            true,
        ));
        assert!(!pp_persist_rotation(
            &left,
            &other_group,
            PersistKind::Trace,
            PersistKind::Trace,
            true,
        ));
    }

    fn bindings<const N: usize>(values: [u32; N]) -> Vec<AliasSeedBinding> {
        values
            .into_iter()
            .map(|value| binding(value, ValueRole::Scratch))
            .collect()
    }

    fn activation_bindings<const N: usize>(values: [u32; N]) -> Vec<AliasSeedBinding> {
        values
            .into_iter()
            .map(|value| binding(value, ValueRole::Activation))
            .collect()
    }

    fn binding(value: u32, role: ValueRole) -> AliasSeedBinding {
        let value_id = ValueId::new(value);
        AliasSeedBinding {
            value: value_id,
            materialization: Materialization::Materialize {
                class: StorageClass::WramHot,
                lifetime: LifetimeClass::Slice,
            },
            live_range: live_range(value * 2, Some(value * 2 + 1)),
            role,
        }
    }

    fn persist_binding(value: u32, page: u32, group: u32) -> AliasSeedBinding {
        AliasSeedBinding {
            value: ValueId::new(value),
            materialization: Materialization::Persist {
                page: PersistPageId(page),
                commit_group: CommitGroupId(group),
            },
            live_range: live_range(value * 2, Some(value * 2 + 1)),
            role: ValueRole::OutputToken,
        }
    }

    fn live_range(def: u32, last_use: Option<u32>) -> AbstractLiveRange {
        AbstractLiveRange {
            def_node: NodeId::new(def),
            first_use_node: last_use.map(NodeId::new),
            last_use_node: last_use.map(NodeId::new),
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

    fn topological_order() -> Vec<NodeId> {
        (1..=8).map(NodeId::new).collect()
    }
}
