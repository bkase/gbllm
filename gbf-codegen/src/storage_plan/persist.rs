//! Persist page and commit-group resolver for Stage 6.

use std::collections::{BTreeMap, BTreeSet};

use gbf_policy::StoragePlanDiagnosticCode;

use crate::s3::infer_ir::ValueId;
use crate::storage_plan::types::{
    CommitAtomicityClass, CommitGroupDecl, CommitGroupId, CommitGroupReason, DurabilityClass,
    NonEmptySortedSet, PersistKind, PersistPageDecl, PersistPageId, PersistSchemaPin,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PersistBindingInput {
    pub value: ValueId,
    pub page: PersistPageId,
    pub commit_group: CommitGroupId,
    pub kind: PersistKind,
    pub reason: CommitGroupReason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistResolution {
    pub persist_pages: BTreeMap<PersistPageId, PersistPageDecl>,
    pub commit_groups: BTreeMap<CommitGroupId, CommitGroupDecl>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistResolverDiagnostic {
    pub code: StoragePlanDiagnosticCode,
    pub value: Option<ValueId>,
    pub page: Option<PersistPageId>,
    pub commit_group: Option<CommitGroupId>,
    pub kind: Option<PersistKind>,
    pub reason: Option<CommitGroupReason>,
    pub kind_set: BTreeSet<PersistKind>,
}

pub fn resolve_persist_bindings(
    bindings: &[PersistBindingInput],
) -> Result<PersistResolution, PersistResolverDiagnostic> {
    let mut pages = BTreeMap::<PersistPageId, PersistPageDecl>::new();
    let mut groups = BTreeMap::<CommitGroupId, GroupDraft>::new();

    for binding in bindings {
        validate_kind_reason(binding)?;
        let page = persist_page_decl(binding.page, binding.kind, binding.reason);
        let durability = page.durability;
        if let Some(existing) = pages.get(&binding.page) {
            if existing.kind != page.kind {
                return Err(PersistResolverDiagnostic {
                    code: StoragePlanDiagnosticCode::StoragePersistBindingKindMismatch,
                    value: Some(binding.value),
                    page: Some(binding.page),
                    commit_group: Some(binding.commit_group),
                    kind: Some(binding.kind),
                    reason: Some(binding.reason),
                    kind_set: BTreeSet::new(),
                });
            }
        } else {
            pages.insert(binding.page, page);
        }

        let group = groups
            .entry(binding.commit_group)
            .or_insert_with(|| GroupDraft::new(binding.reason));
        if group.reason != binding.reason {
            return Err(PersistResolverDiagnostic {
                code: StoragePlanDiagnosticCode::StorageCommitGroupKindMix,
                value: Some(binding.value),
                page: Some(binding.page),
                commit_group: Some(binding.commit_group),
                kind: Some(binding.kind),
                reason: Some(binding.reason),
                kind_set: group.kind_set.clone(),
            });
        }
        group.members.insert(binding.page);
        group.kind_set.insert(binding.kind);
        group.durabilities.insert(durability);
    }

    let mut commit_groups = BTreeMap::new();
    for (id, group) in groups {
        validate_allowed_kind_set(id, &group)?;
        reject_reserved_shape(id, &group)?;
        validate_durability_mix(id, &group)?;

        let members = NonEmptySortedSet::from_btree_set(group.members).map_err(|_| {
            PersistResolverDiagnostic {
                code: StoragePlanDiagnosticCode::StorageCommitGroupEmpty,
                value: None,
                page: None,
                commit_group: Some(id),
                kind: None,
                reason: Some(group.reason),
                kind_set: group.kind_set.clone(),
            }
        })?;
        commit_groups.insert(
            id,
            CommitGroupDecl {
                id,
                members,
                kind_set: group.kind_set,
                atomicity: CommitAtomicityClass::AllOrNothing,
            },
        );
    }

    Ok(PersistResolution {
        persist_pages: pages,
        commit_groups,
    })
}

pub fn persist_page_decl(
    id: PersistPageId,
    kind: PersistKind,
    reason: CommitGroupReason,
) -> PersistPageDecl {
    PersistPageDecl {
        id,
        kind,
        durability: durability_for(kind, reason),
        schema_pin: schema_pin_for(kind),
    }
}

pub const fn schema_pin_for(kind: PersistKind) -> PersistSchemaPin {
    match kind {
        PersistKind::SequenceState => PersistSchemaPin {
            state_schema: 1,
            requires_semantic_state_hash: true,
            requires_resume_abi_hash: false,
            requires_build_identity_hash: false,
        },
        PersistKind::Continuation => PersistSchemaPin {
            state_schema: 2,
            requires_semantic_state_hash: false,
            requires_resume_abi_hash: true,
            requires_build_identity_hash: false,
        },
        PersistKind::Transcript | PersistKind::Harness | PersistKind::Trace => PersistSchemaPin {
            state_schema: 3,
            requires_semantic_state_hash: false,
            requires_resume_abi_hash: false,
            requires_build_identity_hash: true,
        },
    }
}

pub const fn durability_for(kind: PersistKind, reason: CommitGroupReason) -> DurabilityClass {
    match (kind, reason) {
        (PersistKind::SequenceState, _) => DurabilityClass::Critical,
        (PersistKind::Continuation, _) => DurabilityClass::Recoverable,
        (PersistKind::Transcript, CommitGroupReason::SequenceStateWithTranscript) => {
            DurabilityClass::Critical
        }
        (PersistKind::Transcript | PersistKind::Harness | PersistKind::Trace, _) => {
            DurabilityClass::BestEffort
        }
    }
}

fn validate_kind_reason(binding: &PersistBindingInput) -> Result<(), PersistResolverDiagnostic> {
    if binding.reason == CommitGroupReason::OrderedRecoverable {
        return Err(PersistResolverDiagnostic {
            code: StoragePlanDiagnosticCode::StorageReservedShapeEmitted,
            value: Some(binding.value),
            page: Some(binding.page),
            commit_group: Some(binding.commit_group),
            kind: Some(binding.kind),
            reason: Some(binding.reason),
            kind_set: BTreeSet::from([binding.kind]),
        });
    }

    let ok = match binding.kind {
        PersistKind::SequenceState => matches!(
            binding.reason,
            CommitGroupReason::PerSequenceStateSlot
                | CommitGroupReason::SequenceStateWithTranscript
                | CommitGroupReason::ContinuationWithSequenceState
        ),
        PersistKind::Continuation => matches!(
            binding.reason,
            CommitGroupReason::ContinuationWithSequenceState | CommitGroupReason::ContinuationOnly
        ),
        PersistKind::Transcript => matches!(
            binding.reason,
            CommitGroupReason::Independent | CommitGroupReason::SequenceStateWithTranscript
        ),
        PersistKind::Harness | PersistKind::Trace => {
            binding.reason == CommitGroupReason::Independent
        }
    };

    if ok {
        Ok(())
    } else {
        Err(PersistResolverDiagnostic {
            code: StoragePlanDiagnosticCode::StoragePersistBindingKindMismatch,
            value: Some(binding.value),
            page: Some(binding.page),
            commit_group: Some(binding.commit_group),
            kind: Some(binding.kind),
            reason: Some(binding.reason),
            kind_set: BTreeSet::new(),
        })
    }
}

fn validate_allowed_kind_set(
    id: CommitGroupId,
    group: &GroupDraft,
) -> Result<(), PersistResolverDiagnostic> {
    let kinds: Vec<_> = group.kind_set.iter().copied().collect();
    let allowed = matches!(
        (group.reason, kinds.as_slice()),
        (
            CommitGroupReason::PerSequenceStateSlot,
            [PersistKind::SequenceState]
        ) | (
            CommitGroupReason::SequenceStateWithTranscript,
            [PersistKind::SequenceState, PersistKind::Transcript]
        ) | (
            CommitGroupReason::ContinuationWithSequenceState,
            [PersistKind::SequenceState, PersistKind::Continuation]
        ) | (
            CommitGroupReason::ContinuationOnly,
            [PersistKind::Continuation]
        ) | (CommitGroupReason::Independent, [PersistKind::Transcript])
            | (CommitGroupReason::Independent, [PersistKind::Harness])
            | (CommitGroupReason::Independent, [PersistKind::Trace])
    );

    if allowed {
        Ok(())
    } else {
        Err(PersistResolverDiagnostic {
            code: StoragePlanDiagnosticCode::StorageCommitGroupKindMix,
            value: None,
            page: None,
            commit_group: Some(id),
            kind: None,
            reason: Some(group.reason),
            kind_set: group.kind_set.clone(),
        })
    }
}

fn validate_durability_mix(
    id: CommitGroupId,
    group: &GroupDraft,
) -> Result<(), PersistResolverDiagnostic> {
    let has_critical = group.durabilities.contains(&DurabilityClass::Critical);
    let has_recoverable = group.durabilities.contains(&DurabilityClass::Recoverable);
    let has_best_effort = group.durabilities.contains(&DurabilityClass::BestEffort);
    let ok = if has_critical {
        !has_recoverable && !has_best_effort
    } else if has_best_effort {
        !has_recoverable
    } else {
        true
    };

    if ok {
        Ok(())
    } else {
        Err(PersistResolverDiagnostic {
            code: StoragePlanDiagnosticCode::StorageCommitGroupDurabilityMix,
            value: None,
            page: None,
            commit_group: Some(id),
            kind: None,
            reason: Some(group.reason),
            kind_set: group.kind_set.clone(),
        })
    }
}

fn reject_reserved_shape(
    id: CommitGroupId,
    group: &GroupDraft,
) -> Result<(), PersistResolverDiagnostic> {
    if matches!(
        group.reason,
        CommitGroupReason::ContinuationWithSequenceState
            | CommitGroupReason::ContinuationOnly
            | CommitGroupReason::OrderedRecoverable
    ) {
        return Err(PersistResolverDiagnostic {
            code: StoragePlanDiagnosticCode::StorageReservedShapeEmitted,
            value: None,
            page: None,
            commit_group: Some(id),
            kind: None,
            reason: Some(group.reason),
            kind_set: group.kind_set.clone(),
        });
    }

    Ok(())
}

struct GroupDraft {
    reason: CommitGroupReason,
    members: BTreeSet<PersistPageId>,
    kind_set: BTreeSet<PersistKind>,
    durabilities: BTreeSet<DurabilityClass>,
}

impl GroupDraft {
    fn new(reason: CommitGroupReason) -> Self {
        Self {
            reason,
            members: BTreeSet::new(),
            kind_set: BTreeSet::new(),
            durabilities: BTreeSet::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcript_grouped_with_sequence_state_is_promoted_to_critical() {
        let resolution = resolve_persist_bindings(&[
            binding(
                1,
                1,
                1,
                PersistKind::SequenceState,
                CommitGroupReason::SequenceStateWithTranscript,
            ),
            binding(
                2,
                2,
                1,
                PersistKind::Transcript,
                CommitGroupReason::SequenceStateWithTranscript,
            ),
        ])
        .expect("sequence plus transcript resolves");

        assert_eq!(
            resolution.persist_pages[&PersistPageId(2)].durability,
            DurabilityClass::Critical
        );
    }

    #[test]
    fn continuation_with_sequence_state_is_reserved_v1_store_030() {
        let error = resolve_persist_bindings(&[
            binding(
                1,
                1,
                1,
                PersistKind::SequenceState,
                CommitGroupReason::ContinuationWithSequenceState,
            ),
            binding(
                2,
                2,
                1,
                PersistKind::Continuation,
                CommitGroupReason::ContinuationWithSequenceState,
            ),
        ])
        .expect_err("reserved continuation group rejects");

        assert_eq!(
            error.code,
            StoragePlanDiagnosticCode::StorageReservedShapeEmitted
        );
    }

    #[test]
    fn ordered_recoverable_reason_is_reserved_shape_store_030() {
        let error = resolve_persist_bindings(&[binding(
            1,
            1,
            1,
            PersistKind::Trace,
            CommitGroupReason::OrderedRecoverable,
        )])
        .expect_err("ordered recoverable is a reserved v1 shape");

        assert_eq!(
            error.code,
            StoragePlanDiagnosticCode::StorageReservedShapeEmitted
        );
        assert_eq!(error.reason, Some(CommitGroupReason::OrderedRecoverable));
    }

    #[test]
    fn ungrouped_sequence_state_page_is_critical_and_semantic_state_pinned() {
        let resolution = resolve_persist_bindings(&[binding(
            1,
            1,
            1,
            PersistKind::SequenceState,
            CommitGroupReason::PerSequenceStateSlot,
        )])
        .expect("single sequence-state page resolves");
        let page = &resolution.persist_pages[&PersistPageId(1)];

        assert_eq!(page.durability, DurabilityClass::Critical);
        assert_eq!(
            page.schema_pin,
            PersistSchemaPin {
                state_schema: 1,
                requires_semantic_state_hash: true,
                requires_resume_abi_hash: false,
                requires_build_identity_hash: false,
            }
        );
    }

    #[test]
    fn trace_harness_kind_mix_rejects_store_011() {
        let error = resolve_persist_bindings(&[
            binding(1, 1, 1, PersistKind::Trace, CommitGroupReason::Independent),
            binding(
                2,
                2,
                1,
                PersistKind::Harness,
                CommitGroupReason::Independent,
            ),
        ])
        .expect_err("trace harness mix rejects");

        assert_eq!(
            error.code,
            StoragePlanDiagnosticCode::StorageCommitGroupKindMix
        );
    }

    #[test]
    fn promoted_sequence_transcript_group_passes_durability_wellformedness() {
        let resolution = resolve_persist_bindings(&[
            binding(
                1,
                1,
                1,
                PersistKind::SequenceState,
                CommitGroupReason::SequenceStateWithTranscript,
            ),
            binding(
                2,
                2,
                1,
                PersistKind::Transcript,
                CommitGroupReason::SequenceStateWithTranscript,
            ),
        ])
        .expect("post-promotion durability is consistent");

        assert_eq!(
            resolution.commit_groups[&CommitGroupId(1)].kind_set,
            BTreeSet::from([PersistKind::SequenceState, PersistKind::Transcript])
        );
    }

    #[test]
    fn invalid_kind_reason_triple_rejects_store_008() {
        let error = resolve_persist_bindings(&[binding(
            1,
            1,
            1,
            PersistKind::Trace,
            CommitGroupReason::SequenceStateWithTranscript,
        )])
        .expect_err("trace cannot use sequence-state transcript reason");

        assert_eq!(
            error.code,
            StoragePlanDiagnosticCode::StoragePersistBindingKindMismatch
        );
    }

    fn binding(
        value: u32,
        page: u32,
        group: u32,
        kind: PersistKind,
        reason: CommitGroupReason,
    ) -> PersistBindingInput {
        PersistBindingInput {
            value: ValueId::new(value),
            page: PersistPageId(page),
            commit_group: CommitGroupId(group),
            kind,
            reason,
        }
    }
}
