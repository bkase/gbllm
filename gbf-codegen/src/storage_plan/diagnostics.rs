//! STORE-* diagnostic builders and ordering helpers for Stage 6.

use std::{error::Error, fmt};

#[cfg(test)]
use gbf_foundation::Hash256;
use gbf_foundation::{CanonicalJson, CanonicalJsonError, EvidenceRef, FieldPath};
use gbf_policy::{
    StoragePlanDiagnosticCode, StoragePlanDiagnosticProvenance, ValidationCode, ValidationDetail,
    ValidationDiagnostic, ValidationOrigin,
};

pub use gbf_policy::{
    StoragePlanDiagnosticCode as StoreDiagnosticCode,
    StoragePlanDiagnosticProvenance as StoreDiagnosticProvenance,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoragePlanDiagnosticBuildError {
    pub code: StoragePlanDiagnosticCode,
    pub expected_schema: &'static str,
}

impl fmt::Display for StoragePlanDiagnosticBuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {} requires provenance schema {}",
            self.code.as_str(),
            self.code.name(),
            self.expected_schema
        )
    }
}

impl Error for StoragePlanDiagnosticBuildError {}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct StoragePlanDiagnosticSortKey {
    code: &'static str,
    provenance_canonical_form: Vec<u8>,
}

pub fn storage_plan_diagnostic(
    code: StoragePlanDiagnosticCode,
    provenance: StoragePlanDiagnosticProvenance,
    evidence: Vec<EvidenceRef>,
) -> Result<ValidationDiagnostic, StoragePlanDiagnosticBuildError> {
    if !storage_plan_code_accepts_provenance(code, &provenance) {
        return Err(StoragePlanDiagnosticBuildError {
            code,
            expected_schema: storage_plan_provenance_schema(code),
        });
    }

    Ok(ValidationDiagnostic::hard(
        ValidationOrigin::StoragePlanConstruction,
        ValidationCode::StoragePlan { code, provenance },
        storage_plan_detail(code),
        evidence,
    ))
}

pub fn storage_plan_diagnostic_sort_key(
    diagnostic: &ValidationDiagnostic,
) -> Result<StoragePlanDiagnosticSortKey, CanonicalJsonError> {
    match &diagnostic.code {
        ValidationCode::StoragePlan { code, provenance } => Ok(StoragePlanDiagnosticSortKey {
            code: code.as_str(),
            provenance_canonical_form: CanonicalJson::to_vec(provenance)?,
        }),
        other => Ok(StoragePlanDiagnosticSortKey {
            code: "NON-STORE",
            provenance_canonical_form: CanonicalJson::to_vec(other)?,
        }),
    }
}

pub fn sort_storage_plan_diagnostics(
    diagnostics: &mut [ValidationDiagnostic],
) -> Result<(), CanonicalJsonError> {
    let mut keyed = diagnostics
        .iter()
        .cloned()
        .map(|diagnostic| Ok((storage_plan_diagnostic_sort_key(&diagnostic)?, diagnostic)))
        .collect::<Result<Vec<_>, CanonicalJsonError>>()?;
    keyed.sort_by(|left, right| left.0.cmp(&right.0));

    for (slot, (_, diagnostic)) in diagnostics.iter_mut().zip(keyed) {
        *slot = diagnostic;
    }

    Ok(())
}

#[must_use]
pub const fn storage_plan_provenance_schema(code: StoragePlanDiagnosticCode) -> &'static str {
    match code {
        StoragePlanDiagnosticCode::StorageNoAdmittingDecisionRule => "ValueClassification",
        StoragePlanDiagnosticCode::StorageBindingCoverageGap => "ValueProducer",
        StoragePlanDiagnosticCode::StorageBindingDoubleBind => "BindingSet",
        StoragePlanDiagnosticCode::StorageRomConstWriteViolation => "ProducerOp",
        StoragePlanDiagnosticCode::StorageHramAdmissionInvariantViolation => "BudgetSet",
        StoragePlanDiagnosticCode::StorageRecomputeForbiddenForObservedValue => {
            "ObservationCheckpoint"
        }
        StoragePlanDiagnosticCode::StoragePersistSequenceStateUnsupportedV1 => "SequenceState",
        StoragePlanDiagnosticCode::StoragePersistBindingKindMismatch => "PersistBinding",
        StoragePlanDiagnosticCode::StoragePersistPageNotReferenced => "PersistPage",
        StoragePlanDiagnosticCode::StorageCommitGroupEmpty => "CommitGroup",
        StoragePlanDiagnosticCode::StorageCommitGroupKindMix => "CommitGroupKind",
        StoragePlanDiagnosticCode::StorageCommitGroupDurabilityMix => "CommitGroupDurability",
        StoragePlanDiagnosticCode::StorageAliasIntentMaterializationMismatch => {
            "AliasMaterialization"
        }
        StoragePlanDiagnosticCode::StorageAliasClassOverlapWithoutIntent => "AliasOverlap",
        StoragePlanDiagnosticCode::StorageAliasClassMembershipFunctionalViolation => {
            "AliasMembership"
        }
        StoragePlanDiagnosticCode::StorageRecomputeAliasNotIsolated => "RecomputeAlias",
        StoragePlanDiagnosticCode::StorageLifetimeAdmissibilityViolation => "LifetimeAdmissibility",
        StoragePlanDiagnosticCode::StorageForbiddenSpatialEnumLeak
        | StoragePlanDiagnosticCode::StorageReservedShapeEmitted => "JsonPath",
        StoragePlanDiagnosticCode::StorageDeterminismRequiresStableRules => "RuleInstability",
        StoragePlanDiagnosticCode::StorageRangePlanHashMismatch
        | StoragePlanDiagnosticCode::StorageInferIrHashMismatch
        | StoragePlanDiagnosticCode::StorageObservationPlanHashMismatch
        | StoragePlanDiagnosticCode::StorageQuantGraphHashMismatch
        | StoragePlanDiagnosticCode::StoragePolicyHashMismatch => "HashMismatch",
        StoragePlanDiagnosticCode::StorageIterationInputInvalid => "Iteration",
        StoragePlanDiagnosticCode::StorageOverlayLensViolation => "OverlayLens",
        StoragePlanDiagnosticCode::StorageRepairProposalIllegal => "RepairProposal",
        StoragePlanDiagnosticCode::StorageInferIrEffectClassUnknown => "EffectClass",
        StoragePlanDiagnosticCode::StorageQuantGraphRoutingMismatch => "RoutingMismatch",
        StoragePlanDiagnosticCode::StorageAliasMixedIntentComponent => "AliasMixedIntent",
        StoragePlanDiagnosticCode::StorageAliasIntentCardinalityViolation => "AliasCardinality",
        StoragePlanDiagnosticCode::StorageForcedRecomputeNotAllowed => "ForcedRecompute",
        StoragePlanDiagnosticCode::StoragePolicyBudgetUnderflow => "PolicyBudget",
        StoragePlanDiagnosticCode::StorageAliasClassFingerprintCollision => "FingerprintCollision",
    }
}

#[must_use]
pub fn storage_plan_code_accepts_provenance(
    code: StoragePlanDiagnosticCode,
    provenance: &StoragePlanDiagnosticProvenance,
) -> bool {
    matches!(
        (code, provenance),
        (
            StoragePlanDiagnosticCode::StorageNoAdmittingDecisionRule,
            StoragePlanDiagnosticProvenance::ValueClassification { .. }
        ) | (
            StoragePlanDiagnosticCode::StorageBindingCoverageGap,
            StoragePlanDiagnosticProvenance::ValueProducer { .. }
        ) | (
            StoragePlanDiagnosticCode::StorageBindingDoubleBind,
            StoragePlanDiagnosticProvenance::BindingSet { .. }
        ) | (
            StoragePlanDiagnosticCode::StorageRomConstWriteViolation,
            StoragePlanDiagnosticProvenance::ProducerOp { .. }
        ) | (
            StoragePlanDiagnosticCode::StorageHramAdmissionInvariantViolation,
            StoragePlanDiagnosticProvenance::BudgetSet { .. }
        ) | (
            StoragePlanDiagnosticCode::StorageRecomputeForbiddenForObservedValue,
            StoragePlanDiagnosticProvenance::ObservationCheckpoint { .. }
        ) | (
            StoragePlanDiagnosticCode::StoragePersistSequenceStateUnsupportedV1,
            StoragePlanDiagnosticProvenance::SequenceState { .. }
        ) | (
            StoragePlanDiagnosticCode::StoragePersistBindingKindMismatch,
            StoragePlanDiagnosticProvenance::PersistBinding { .. }
        ) | (
            StoragePlanDiagnosticCode::StoragePersistPageNotReferenced,
            StoragePlanDiagnosticProvenance::PersistPage { .. }
        ) | (
            StoragePlanDiagnosticCode::StorageCommitGroupEmpty,
            StoragePlanDiagnosticProvenance::CommitGroup { .. }
        ) | (
            StoragePlanDiagnosticCode::StorageCommitGroupKindMix,
            StoragePlanDiagnosticProvenance::CommitGroupKind { .. }
        ) | (
            StoragePlanDiagnosticCode::StorageCommitGroupDurabilityMix,
            StoragePlanDiagnosticProvenance::CommitGroupDurability { .. }
        ) | (
            StoragePlanDiagnosticCode::StorageAliasIntentMaterializationMismatch,
            StoragePlanDiagnosticProvenance::AliasMaterialization { .. }
        ) | (
            StoragePlanDiagnosticCode::StorageAliasClassOverlapWithoutIntent,
            StoragePlanDiagnosticProvenance::AliasOverlap { .. }
        ) | (
            StoragePlanDiagnosticCode::StorageAliasClassMembershipFunctionalViolation,
            StoragePlanDiagnosticProvenance::AliasMembership { .. }
        ) | (
            StoragePlanDiagnosticCode::StorageRecomputeAliasNotIsolated,
            StoragePlanDiagnosticProvenance::RecomputeAlias { .. }
        ) | (
            StoragePlanDiagnosticCode::StorageLifetimeAdmissibilityViolation,
            StoragePlanDiagnosticProvenance::LifetimeAdmissibility { .. }
        ) | (
            StoragePlanDiagnosticCode::StorageForbiddenSpatialEnumLeak
                | StoragePlanDiagnosticCode::StorageReservedShapeEmitted,
            StoragePlanDiagnosticProvenance::JsonPath { .. }
        ) | (
            StoragePlanDiagnosticCode::StorageDeterminismRequiresStableRules,
            StoragePlanDiagnosticProvenance::RuleInstability { .. }
        ) | (
            StoragePlanDiagnosticCode::StorageRangePlanHashMismatch
                | StoragePlanDiagnosticCode::StorageInferIrHashMismatch
                | StoragePlanDiagnosticCode::StorageObservationPlanHashMismatch
                | StoragePlanDiagnosticCode::StorageQuantGraphHashMismatch
                | StoragePlanDiagnosticCode::StoragePolicyHashMismatch,
            StoragePlanDiagnosticProvenance::HashMismatch { .. }
        ) | (
            StoragePlanDiagnosticCode::StorageIterationInputInvalid,
            StoragePlanDiagnosticProvenance::Iteration { .. }
        ) | (
            StoragePlanDiagnosticCode::StorageOverlayLensViolation,
            StoragePlanDiagnosticProvenance::OverlayLens { .. }
        ) | (
            StoragePlanDiagnosticCode::StorageRepairProposalIllegal,
            StoragePlanDiagnosticProvenance::RepairProposal { .. }
        ) | (
            StoragePlanDiagnosticCode::StorageInferIrEffectClassUnknown,
            StoragePlanDiagnosticProvenance::EffectClass { .. }
        ) | (
            StoragePlanDiagnosticCode::StorageQuantGraphRoutingMismatch,
            StoragePlanDiagnosticProvenance::RoutingMismatch { .. }
        ) | (
            StoragePlanDiagnosticCode::StorageAliasMixedIntentComponent,
            StoragePlanDiagnosticProvenance::AliasMixedIntent { .. }
        ) | (
            StoragePlanDiagnosticCode::StorageAliasIntentCardinalityViolation,
            StoragePlanDiagnosticProvenance::AliasCardinality { .. }
        ) | (
            StoragePlanDiagnosticCode::StorageForcedRecomputeNotAllowed,
            StoragePlanDiagnosticProvenance::ForcedRecompute { .. }
        ) | (
            StoragePlanDiagnosticCode::StoragePolicyBudgetUnderflow,
            StoragePlanDiagnosticProvenance::PolicyBudget { .. }
        ) | (
            StoragePlanDiagnosticCode::StorageAliasClassFingerprintCollision,
            StoragePlanDiagnosticProvenance::FingerprintCollision { .. }
        )
    )
}

#[must_use]
pub fn storage_plan_detail(code: StoragePlanDiagnosticCode) -> ValidationDetail {
    ValidationDetail::Field {
        // ValidationDetail has no free-text variant; this stable field names the
        // per-code renderer template that carries the human-readable detail.
        field: FieldPath::from(format!(
            "storage_plan.diagnostics.{}.{}.detail_template.{}",
            code.as_str(),
            code.name(),
            storage_plan_detail_template_id(code)
        )),
    }
}

#[must_use]
pub const fn storage_plan_detail_template_id(code: StoragePlanDiagnosticCode) -> &'static str {
    match code {
        StoragePlanDiagnosticCode::StorageNoAdmittingDecisionRule => "no_admitting_decision_rule",
        StoragePlanDiagnosticCode::StorageBindingCoverageGap => "binding_coverage_gap",
        StoragePlanDiagnosticCode::StorageBindingDoubleBind => "binding_double_bind",
        StoragePlanDiagnosticCode::StorageRomConstWriteViolation => "rom_const_write_violation",
        StoragePlanDiagnosticCode::StorageHramAdmissionInvariantViolation => {
            "hram_admission_invariant_violation"
        }
        StoragePlanDiagnosticCode::StorageRecomputeForbiddenForObservedValue => {
            "recompute_forbidden_for_observed_value"
        }
        StoragePlanDiagnosticCode::StoragePersistSequenceStateUnsupportedV1 => {
            "persist_sequence_state_unsupported_v1"
        }
        StoragePlanDiagnosticCode::StoragePersistBindingKindMismatch => {
            "persist_binding_kind_mismatch"
        }
        StoragePlanDiagnosticCode::StoragePersistPageNotReferenced => "persist_page_not_referenced",
        StoragePlanDiagnosticCode::StorageCommitGroupEmpty => "commit_group_empty",
        StoragePlanDiagnosticCode::StorageCommitGroupKindMix => "commit_group_kind_mix",
        StoragePlanDiagnosticCode::StorageCommitGroupDurabilityMix => "commit_group_durability_mix",
        StoragePlanDiagnosticCode::StorageAliasIntentMaterializationMismatch => {
            "alias_intent_materialization_mismatch"
        }
        StoragePlanDiagnosticCode::StorageAliasClassOverlapWithoutIntent => {
            "alias_class_overlap_without_intent"
        }
        StoragePlanDiagnosticCode::StorageAliasClassMembershipFunctionalViolation => {
            "alias_class_membership_functional_violation"
        }
        StoragePlanDiagnosticCode::StorageRecomputeAliasNotIsolated => {
            "recompute_alias_not_isolated"
        }
        StoragePlanDiagnosticCode::StorageLifetimeAdmissibilityViolation => {
            "lifetime_admissibility_violation"
        }
        StoragePlanDiagnosticCode::StorageForbiddenSpatialEnumLeak => "forbidden_spatial_enum_leak",
        StoragePlanDiagnosticCode::StorageDeterminismRequiresStableRules => {
            "determinism_requires_stable_rules"
        }
        StoragePlanDiagnosticCode::StorageRangePlanHashMismatch => "range_plan_hash_mismatch",
        StoragePlanDiagnosticCode::StorageInferIrHashMismatch => "infer_ir_hash_mismatch",
        StoragePlanDiagnosticCode::StorageObservationPlanHashMismatch => {
            "observation_plan_hash_mismatch"
        }
        StoragePlanDiagnosticCode::StorageQuantGraphHashMismatch => "quant_graph_hash_mismatch",
        StoragePlanDiagnosticCode::StoragePolicyHashMismatch => "policy_hash_mismatch",
        StoragePlanDiagnosticCode::StorageIterationInputInvalid => "iteration_input_invalid",
        StoragePlanDiagnosticCode::StorageOverlayLensViolation => "overlay_lens_violation",
        StoragePlanDiagnosticCode::StorageRepairProposalIllegal => "repair_proposal_illegal",
        StoragePlanDiagnosticCode::StorageInferIrEffectClassUnknown => {
            "infer_ir_effect_class_unknown"
        }
        StoragePlanDiagnosticCode::StorageQuantGraphRoutingMismatch => {
            "quant_graph_routing_mismatch"
        }
        StoragePlanDiagnosticCode::StorageReservedShapeEmitted => "reserved_shape_emitted",
        StoragePlanDiagnosticCode::StorageAliasMixedIntentComponent => {
            "alias_mixed_intent_component"
        }
        StoragePlanDiagnosticCode::StorageAliasIntentCardinalityViolation => {
            "alias_intent_cardinality_violation"
        }
        StoragePlanDiagnosticCode::StorageForcedRecomputeNotAllowed => {
            "forced_recompute_not_allowed"
        }
        StoragePlanDiagnosticCode::StoragePolicyBudgetUnderflow => "policy_budget_underflow",
        StoragePlanDiagnosticCode::StorageAliasClassFingerprintCollision => {
            "alias_class_fingerprint_collision"
        }
    }
}

#[must_use]
pub const fn storage_plan_detail_template(code: StoragePlanDiagnosticCode) -> &'static str {
    match code {
        StoragePlanDiagnosticCode::StorageNoAdmittingDecisionRule => {
            "Value classification has no admitting decision rule."
        }
        StoragePlanDiagnosticCode::StorageBindingCoverageGap => {
            "Produced value is missing a storage binding."
        }
        StoragePlanDiagnosticCode::StorageBindingDoubleBind => {
            "Value has more than one storage binding."
        }
        StoragePlanDiagnosticCode::StorageRomConstWriteViolation => {
            "RomConst binding is assigned to a non-const producer."
        }
        StoragePlanDiagnosticCode::StorageHramAdmissionInvariantViolation => {
            "HRAM admission exceeds the deterministic budget."
        }
        StoragePlanDiagnosticCode::StorageRecomputeForbiddenForObservedValue => {
            "Observed checkpoint value was marked for recompute."
        }
        StoragePlanDiagnosticCode::StoragePersistSequenceStateUnsupportedV1 => {
            "Persistent sequence state is unsupported in v1."
        }
        StoragePlanDiagnosticCode::StoragePersistBindingKindMismatch => {
            "Persist binding kind disagrees with its page or commit group."
        }
        StoragePlanDiagnosticCode::StoragePersistPageNotReferenced => {
            "Persist page is not referenced by any binding."
        }
        StoragePlanDiagnosticCode::StorageCommitGroupEmpty => "Commit group is empty.",
        StoragePlanDiagnosticCode::StorageCommitGroupKindMix => {
            "Commit group mixes incompatible persist kinds."
        }
        StoragePlanDiagnosticCode::StorageCommitGroupDurabilityMix => {
            "Commit group mixes incompatible durability classes."
        }
        StoragePlanDiagnosticCode::StorageAliasIntentMaterializationMismatch => {
            "Alias intent is incompatible with member materializations."
        }
        StoragePlanDiagnosticCode::StorageAliasClassOverlapWithoutIntent => {
            "Alias class overlaps without an overlap-capable intent."
        }
        StoragePlanDiagnosticCode::StorageAliasClassMembershipFunctionalViolation => {
            "Binding maps to more than one alias class."
        }
        StoragePlanDiagnosticCode::StorageRecomputeAliasNotIsolated => {
            "Recompute binding shares an alias class."
        }
        StoragePlanDiagnosticCode::StorageLifetimeAdmissibilityViolation => {
            "Computed lifetime falls outside the admissible interval."
        }
        StoragePlanDiagnosticCode::StorageForbiddenSpatialEnumLeak => {
            "Storage output exposes a forbidden spatial schema field."
        }
        StoragePlanDiagnosticCode::StorageDeterminismRequiresStableRules => {
            "Determinism requires stable rule inputs."
        }
        StoragePlanDiagnosticCode::StorageRangePlanHashMismatch => {
            "RangePlan input hash does not match the recorded Stage 6 input."
        }
        StoragePlanDiagnosticCode::StorageInferIrHashMismatch => {
            "InferIR input hash does not match the recorded Stage 6 input."
        }
        StoragePlanDiagnosticCode::StorageObservationPlanHashMismatch => {
            "ObservationPlan input hash does not match the recorded Stage 6 input."
        }
        StoragePlanDiagnosticCode::StorageQuantGraphHashMismatch => {
            "QuantGraph input hash does not match the recorded Stage 6 input."
        }
        StoragePlanDiagnosticCode::StoragePolicyHashMismatch => {
            "Policy input hash does not match the recorded Stage 6 input."
        }
        StoragePlanDiagnosticCode::StorageIterationInputInvalid => {
            "Stage 6 iteration input exceeds the refinement ceiling."
        }
        StoragePlanDiagnosticCode::StorageOverlayLensViolation => {
            "Overlay eligibility lens rejected the emitted materialization."
        }
        StoragePlanDiagnosticCode::StorageRepairProposalIllegal => {
            "Repair proposal violates locks or bounds."
        }
        StoragePlanDiagnosticCode::StorageInferIrEffectClassUnknown => {
            "InferIR effect class is unknown to StoragePlan."
        }
        StoragePlanDiagnosticCode::StorageQuantGraphRoutingMismatch => {
            "QuantGraph routed-FFN topology disagrees with InferIR routing."
        }
        StoragePlanDiagnosticCode::StorageReservedShapeEmitted => {
            "StoragePlan emitted a schema-reserved v1 shape."
        }
        StoragePlanDiagnosticCode::StorageAliasMixedIntentComponent => {
            "Alias component contains multiple intents."
        }
        StoragePlanDiagnosticCode::StorageAliasIntentCardinalityViolation => {
            "Alias intent member count violates its cardinality."
        }
        StoragePlanDiagnosticCode::StorageForcedRecomputeNotAllowed => {
            "Forced recompute override is not allowed for this value."
        }
        StoragePlanDiagnosticCode::StoragePolicyBudgetUnderflow => {
            "Storage budget reservation exceeds the policy soft budget."
        }
        StoragePlanDiagnosticCode::StorageAliasClassFingerprintCollision => {
            "Alias class fingerprint collision detected."
        }
    }
}

#[must_use]
pub fn storage_plan_fix_hint(code: StoragePlanDiagnosticCode) -> &'static str {
    match code {
        StoragePlanDiagnosticCode::StorageNoAdmittingDecisionRule => {
            "amend an existing rule or fix the upstream value role/format"
        }
        StoragePlanDiagnosticCode::StorageBindingCoverageGap => {
            "fix construction order so every ValueId is visited"
        }
        StoragePlanDiagnosticCode::StorageBindingDoubleBind => {
            "ensure bindings are functional by ValueId"
        }
        StoragePlanDiagnosticCode::StorageRomConstWriteViolation => {
            "fix the RomConst rule or upstream const classification"
        }
        StoragePlanDiagnosticCode::StorageHramAdmissionInvariantViolation => {
            "fix deterministic HRAM candidate admission"
        }
        StoragePlanDiagnosticCode::StorageRecomputeForbiddenForObservedValue => {
            "remove the override or exclude observation checkpoint values"
        }
        StoragePlanDiagnosticCode::StoragePersistSequenceStateUnsupportedV1 => {
            "reject non-identity sequence blocks upstream"
        }
        StoragePlanDiagnosticCode::StoragePersistBindingKindMismatch => {
            "adjust the binding to match the per-kind persist rule"
        }
        StoragePlanDiagnosticCode::StoragePersistPageNotReferenced => {
            "remove orphan persist page declarations"
        }
        StoragePlanDiagnosticCode::StorageCommitGroupEmpty => "remove empty commit groups",
        StoragePlanDiagnosticCode::StorageCommitGroupKindMix => {
            "split the group or amend the cross-kind table"
        }
        StoragePlanDiagnosticCode::StorageCommitGroupDurabilityMix => {
            "split the group by durability class"
        }
        StoragePlanDiagnosticCode::StorageAliasIntentMaterializationMismatch => {
            "split the alias class or choose a compatible intent"
        }
        StoragePlanDiagnosticCode::StorageAliasClassOverlapWithoutIntent => {
            "prove disjointness or use an overlap-capable intent"
        }
        StoragePlanDiagnosticCode::StorageAliasClassMembershipFunctionalViolation => {
            "repair binding-to-alias membership consistency"
        }
        StoragePlanDiagnosticCode::StorageRecomputeAliasNotIsolated => {
            "isolate recompute bindings in singleton NoAlias classes"
        }
        StoragePlanDiagnosticCode::StorageLifetimeAdmissibilityViolation => {
            "choose a lifetime inside the admissible interval"
        }
        StoragePlanDiagnosticCode::StorageForbiddenSpatialEnumLeak => {
            "remove forbidden spatial schema surface from storage output"
        }
        StoragePlanDiagnosticCode::StorageDeterminismRequiresStableRules => {
            "pin unstable rule inputs for deterministic artifacts"
        }
        StoragePlanDiagnosticCode::StorageRangePlanHashMismatch
        | StoragePlanDiagnosticCode::StorageInferIrHashMismatch
        | StoragePlanDiagnosticCode::StorageObservationPlanHashMismatch
        | StoragePlanDiagnosticCode::StorageQuantGraphHashMismatch
        | StoragePlanDiagnosticCode::StoragePolicyHashMismatch => {
            "re-run or repair the mismatched upstream input"
        }
        StoragePlanDiagnosticCode::StorageIterationInputInvalid => {
            "stop invoking Stage 6 past the refinement-loop ceiling"
        }
        StoragePlanDiagnosticCode::StorageOverlayLensViolation => {
            "fix the overlay eligibility lens"
        }
        StoragePlanDiagnosticCode::StorageRepairProposalIllegal => {
            "validate locks and bounds before emitting proposals"
        }
        StoragePlanDiagnosticCode::StorageInferIrEffectClassUnknown => {
            "reject unknown effect classes upstream"
        }
        StoragePlanDiagnosticCode::StorageQuantGraphRoutingMismatch => {
            "repair routed FFN topology mismatch"
        }
        StoragePlanDiagnosticCode::StorageReservedShapeEmitted => {
            "remove schema-reserved shapes from v1 output"
        }
        StoragePlanDiagnosticCode::StorageAliasMixedIntentComponent => {
            "split mixed-intent alias components"
        }
        StoragePlanDiagnosticCode::StorageAliasIntentCardinalityViolation => {
            "split the class or amend the intent definition"
        }
        StoragePlanDiagnosticCode::StorageForcedRecomputeNotAllowed => {
            "remove the override or make the value recomputable"
        }
        StoragePlanDiagnosticCode::StoragePolicyBudgetUnderflow => {
            "raise the soft budget or reduce runtime-chrome reservation"
        }
        StoragePlanDiagnosticCode::StorageAliasClassFingerprintCollision => {
            "change the fingerprint domain or schema version"
        }
    }
}

#[must_use]
#[cfg(test)]
fn synthetic_storage_plan_provenance(
    code: StoragePlanDiagnosticCode,
) -> StoragePlanDiagnosticProvenance {
    match code {
        StoragePlanDiagnosticCode::StorageNoAdmittingDecisionRule => {
            StoragePlanDiagnosticProvenance::ValueClassification {
                value_id: 1,
                producer_node: Some(2),
                value_role: Some("Activation".to_owned()),
                value_format: Some("QuantInt".to_owned()),
            }
        }
        StoragePlanDiagnosticCode::StorageBindingCoverageGap => {
            StoragePlanDiagnosticProvenance::ValueProducer {
                value_id: 2,
                producer_node: 3,
            }
        }
        StoragePlanDiagnosticCode::StorageBindingDoubleBind => {
            StoragePlanDiagnosticProvenance::BindingSet {
                value_id: 3,
                binding_count: 2,
            }
        }
        StoragePlanDiagnosticCode::StorageRomConstWriteViolation => {
            StoragePlanDiagnosticProvenance::ProducerOp {
                value_id: 4,
                producer_node: 5,
                op_tag: "MatVec".to_owned(),
            }
        }
        StoragePlanDiagnosticCode::StorageHramAdmissionInvariantViolation => {
            StoragePlanDiagnosticProvenance::BudgetSet {
                values: vec![1, 2],
                observed_bytes: 144,
                budget_bytes: 127,
            }
        }
        StoragePlanDiagnosticCode::StorageRecomputeForbiddenForObservedValue => {
            StoragePlanDiagnosticProvenance::ObservationCheckpoint {
                value_id: 6,
                semantic_anchor: "anchor.semantic".to_owned(),
                checkpoint_id: 7,
            }
        }
        StoragePlanDiagnosticCode::StoragePersistSequenceStateUnsupportedV1 => {
            StoragePlanDiagnosticProvenance::SequenceState {
                value_id: 7,
                state_slot_id: 8,
                layer_id: 1,
            }
        }
        StoragePlanDiagnosticCode::StoragePersistBindingKindMismatch => {
            StoragePlanDiagnosticProvenance::PersistBinding {
                value_id: 8,
                persist_page_id: 9,
                commit_group_id: 10,
                persist_kind: "Transcript".to_owned(),
                expected: "Critical".to_owned(),
            }
        }
        StoragePlanDiagnosticCode::StoragePersistPageNotReferenced => {
            StoragePlanDiagnosticProvenance::PersistPage {
                invariant: "SC7".to_owned(),
                persist_page_id: 9,
            }
        }
        StoragePlanDiagnosticCode::StorageCommitGroupEmpty => {
            StoragePlanDiagnosticProvenance::CommitGroup {
                invariant: "SC8".to_owned(),
                commit_group_id: 10,
            }
        }
        StoragePlanDiagnosticCode::StorageCommitGroupKindMix => {
            StoragePlanDiagnosticProvenance::CommitGroupKind {
                commit_group_id: 11,
                kinds: vec!["Trace".to_owned(), "Critical".to_owned()],
                allowed_table: "CG-Wf-3".to_owned(),
            }
        }
        StoragePlanDiagnosticCode::StorageCommitGroupDurabilityMix => {
            StoragePlanDiagnosticProvenance::CommitGroupDurability {
                commit_group_id: 12,
                durabilities: vec!["BestEffort".to_owned(), "Critical".to_owned()],
            }
        }
        StoragePlanDiagnosticCode::StorageAliasIntentMaterializationMismatch => {
            StoragePlanDiagnosticProvenance::AliasMaterialization {
                alias_class_id: 13,
                members: vec![1, 2],
                intent: "ScratchReuse".to_owned(),
                materializations: vec!["Recompute".to_owned(), "WramHot".to_owned()],
            }
        }
        StoragePlanDiagnosticCode::StorageAliasClassOverlapWithoutIntent => {
            StoragePlanDiagnosticProvenance::AliasOverlap {
                alias_class_id: 14,
                members: vec![1, 2],
            }
        }
        StoragePlanDiagnosticCode::StorageAliasClassMembershipFunctionalViolation => {
            StoragePlanDiagnosticProvenance::AliasMembership {
                invariant: "SC4".to_owned(),
                value_id: 15,
                alias_class_id: 16,
            }
        }
        StoragePlanDiagnosticCode::StorageRecomputeAliasNotIsolated => {
            StoragePlanDiagnosticProvenance::RecomputeAlias {
                value_id: 16,
                alias_class_id: 17,
            }
        }
        StoragePlanDiagnosticCode::StorageLifetimeAdmissibilityViolation => {
            StoragePlanDiagnosticProvenance::LifetimeAdmissibility {
                value_id: 17,
                computed_lifetime: "Slice".to_owned(),
                min_lifetime: "Token".to_owned(),
                max_lifetime: "Persistent".to_owned(),
                source: "SC10".to_owned(),
            }
        }
        StoragePlanDiagnosticCode::StorageForbiddenSpatialEnumLeak => {
            StoragePlanDiagnosticProvenance::JsonPath {
                json_path: "$.body.result.bindings[0]".to_owned(),
                field_or_tag: "forbidden_spatial_field".to_owned(),
            }
        }
        StoragePlanDiagnosticCode::StorageDeterminismRequiresStableRules => {
            StoragePlanDiagnosticProvenance::RuleInstability {
                rule_id: 11,
                evidence: "cost_estimate_unstable".to_owned(),
            }
        }
        StoragePlanDiagnosticCode::StorageRangePlanHashMismatch => hash_mismatch("RangePlan", 20),
        StoragePlanDiagnosticCode::StorageInferIrHashMismatch => hash_mismatch("InferIr", 21),
        StoragePlanDiagnosticCode::StorageObservationPlanHashMismatch => {
            hash_mismatch("ObservationPlan", 22)
        }
        StoragePlanDiagnosticCode::StorageQuantGraphHashMismatch => hash_mismatch("QuantGraph", 23),
        StoragePlanDiagnosticCode::StoragePolicyHashMismatch => hash_mismatch("Policy", 24),
        StoragePlanDiagnosticCode::StorageIterationInputInvalid => {
            StoragePlanDiagnosticProvenance::Iteration {
                iteration: 3,
                ceiling: 2,
            }
        }
        StoragePlanDiagnosticCode::StorageOverlayLensViolation => {
            StoragePlanDiagnosticProvenance::OverlayLens {
                value_id: 26,
                materialization: "WramHot".to_owned(),
                forced_override: false,
            }
        }
        StoragePlanDiagnosticCode::StorageRepairProposalIllegal => {
            StoragePlanDiagnosticProvenance::RepairProposal {
                proposal_id: "proposal-27".to_owned(),
                delta: "PromoteRecomputeLevel".to_owned(),
                locks_bounds: "max_recompute_promotion".to_owned(),
            }
        }
        StoragePlanDiagnosticCode::StorageInferIrEffectClassUnknown => {
            StoragePlanDiagnosticProvenance::EffectClass {
                effect_id: 28,
                effect_class: "ExternalIo".to_owned(),
            }
        }
        StoragePlanDiagnosticCode::StorageQuantGraphRoutingMismatch => {
            StoragePlanDiagnosticProvenance::RoutingMismatch {
                layer_id: 2,
                expected_entry: "routing_table.layers[2]".to_owned(),
            }
        }
        StoragePlanDiagnosticCode::StorageReservedShapeEmitted => {
            StoragePlanDiagnosticProvenance::JsonPath {
                json_path: "$.body.result.persist_pages[0]".to_owned(),
                field_or_tag: "OrderedRecoverable".to_owned(),
            }
        }
        StoragePlanDiagnosticCode::StorageAliasMixedIntentComponent => {
            StoragePlanDiagnosticProvenance::AliasMixedIntent {
                members: vec![1, 2, 3],
                edge_count: 2,
                intents: vec!["ScratchReuse".to_owned(), "PingPong".to_owned()],
            }
        }
        StoragePlanDiagnosticCode::StorageAliasIntentCardinalityViolation => {
            StoragePlanDiagnosticProvenance::AliasCardinality {
                alias_class_id: 32,
                intent: "PingPong".to_owned(),
                members: vec![1, 2, 3],
            }
        }
        StoragePlanDiagnosticCode::StorageForcedRecomputeNotAllowed => {
            StoragePlanDiagnosticProvenance::ForcedRecompute {
                value_id: 33,
                failed_predicates: vec!["RouterDecision".to_owned()],
            }
        }
        StoragePlanDiagnosticCode::StoragePolicyBudgetUnderflow => {
            StoragePlanDiagnosticProvenance::PolicyBudget {
                storage_class: "WramHot".to_owned(),
                soft_bytes: 64,
                reserved_bytes: 128,
            }
        }
        StoragePlanDiagnosticCode::StorageAliasClassFingerprintCollision => {
            StoragePlanDiagnosticProvenance::FingerprintCollision {
                first_payload_hash: hash(35),
                second_payload_hash: hash(36),
            }
        }
    }
}

#[cfg(test)]
fn hash_mismatch(product: &str, byte: u8) -> StoragePlanDiagnosticProvenance {
    StoragePlanDiagnosticProvenance::HashMismatch {
        product: product.to_owned(),
        recorded: hash(byte),
        computed: hash(byte.wrapping_add(1)),
    }
}

#[cfg(test)]
fn hash(byte: u8) -> Hash256 {
    Hash256::from_bytes([byte; 32])
}

#[cfg(test)]
mod tests {
    use gbf_policy::DiagnosticSeverity;

    use super::*;

    fn evidence() -> Vec<EvidenceRef> {
        vec![EvidenceRef {
            kind: "StoragePlanDiagnosticFixture".to_owned(),
            reference: "synthetic".to_owned(),
            hash: Some(hash(0xaa)),
        }]
    }

    #[test]
    fn every_store_code_builds_stable_canonical_json_diagnostic() {
        let mut snapshots = Vec::new();

        for code in StoragePlanDiagnosticCode::ALL {
            let diagnostic =
                storage_plan_diagnostic(code, synthetic_storage_plan_provenance(code), evidence())
                    .expect("synthetic provenance matches code");
            let first = CanonicalJson::to_vec(&diagnostic).expect("diagnostic canonicalizes");
            let second = CanonicalJson::to_vec(&diagnostic).expect("diagnostic canonicalizes");

            assert_eq!(first, second);
            assert_eq!(diagnostic.severity, DiagnosticSeverity::Hard);
            assert_eq!(diagnostic.origin, ValidationOrigin::StoragePlanConstruction);
            assert!(
                String::from_utf8(first.clone())
                    .expect("canonical diagnostic is utf8")
                    .contains(code.name())
            );
            snapshots.push(first);
        }

        assert_eq!(snapshots.len(), 35);
    }

    #[test]
    fn every_store_code_has_typed_provenance_schema() {
        for code in StoragePlanDiagnosticCode::ALL {
            let provenance = synthetic_storage_plan_provenance(code);
            assert!(storage_plan_code_accepts_provenance(code, &provenance));
            assert_ne!(storage_plan_provenance_schema(code), "");
            assert_ne!(storage_plan_detail_template_id(code), "");
            assert_ne!(storage_plan_detail_template(code), "");
            assert_ne!(storage_plan_fix_hint(code), "");
        }

        let mismatch = storage_plan_diagnostic(
            StoragePlanDiagnosticCode::StoragePolicyBudgetUnderflow,
            StoragePlanDiagnosticProvenance::CommitGroup {
                invariant: "SC8".to_owned(),
                commit_group_id: 1,
            },
            evidence(),
        )
        .expect_err("mismatched provenance schema rejects");
        assert_eq!(
            mismatch.expected_schema,
            storage_plan_provenance_schema(StoragePlanDiagnosticCode::StoragePolicyBudgetUnderflow)
        );
    }

    #[test]
    fn diagnostic_sort_is_total_and_stable() {
        let mut diagnostics = vec![
            storage_plan_diagnostic(
                StoragePlanDiagnosticCode::StoragePolicyHashMismatch,
                synthetic_storage_plan_provenance(
                    StoragePlanDiagnosticCode::StoragePolicyHashMismatch,
                ),
                evidence(),
            )
            .expect("diagnostic builds"),
            storage_plan_diagnostic(
                StoragePlanDiagnosticCode::StorageBindingCoverageGap,
                StoragePlanDiagnosticProvenance::ValueProducer {
                    value_id: 9,
                    producer_node: 2,
                },
                evidence(),
            )
            .expect("diagnostic builds"),
            storage_plan_diagnostic(
                StoragePlanDiagnosticCode::StorageBindingCoverageGap,
                StoragePlanDiagnosticProvenance::ValueProducer {
                    value_id: 1,
                    producer_node: 2,
                },
                evidence(),
            )
            .expect("diagnostic builds"),
        ];
        let mut second = diagnostics.clone();

        sort_storage_plan_diagnostics(&mut diagnostics).expect("sort succeeds");
        sort_storage_plan_diagnostics(&mut second).expect("sort succeeds");

        assert_eq!(diagnostics, second);
        assert!(matches!(
            diagnostics[0].code,
            ValidationCode::StoragePlan {
                code: StoragePlanDiagnosticCode::StorageBindingCoverageGap,
                ..
            }
        ));
        assert!(matches!(
            diagnostics[2].code,
            ValidationCode::StoragePlan {
                code: StoragePlanDiagnosticCode::StoragePolicyHashMismatch,
                ..
            }
        ));
    }

    #[test]
    fn store_code_table_is_closed_and_ordered() {
        let codes: Vec<_> = StoragePlanDiagnosticCode::ALL
            .iter()
            .map(|code| code.as_str())
            .collect();

        assert_eq!(codes.first(), Some(&"STORE-001"));
        assert_eq!(codes.last(), Some(&"STORE-035"));
        assert_eq!(codes.len(), 35);
        assert!(
            StoragePlanDiagnosticCode::ALL
                .windows(2)
                .all(|pair| pair[0].number() + 1 == pair[1].number())
        );
    }

    #[test]
    fn validation_detail_names_store_code_and_rfc_name() {
        let detail =
            storage_plan_detail(StoragePlanDiagnosticCode::StorageLifetimeAdmissibilityViolation);

        assert_eq!(
            detail,
            ValidationDetail::Field {
                field: FieldPath::from(
                    "storage_plan.diagnostics.STORE-017.StorageLifetimeAdmissibilityViolation.detail_template.lifetime_admissibility_violation"
                )
            }
        );
    }

    #[test]
    fn validation_detail_templates_are_distinct_per_code() {
        let coverage = StoragePlanDiagnosticCode::StorageBindingCoverageGap;
        let lifetime = StoragePlanDiagnosticCode::StorageLifetimeAdmissibilityViolation;

        assert_ne!(
            storage_plan_detail_template_id(coverage),
            storage_plan_detail_template_id(lifetime)
        );
        assert_ne!(
            storage_plan_detail_template(coverage),
            storage_plan_detail_template(lifetime)
        );
        assert_ne!(storage_plan_detail(coverage), storage_plan_detail(lifetime));
    }
}
