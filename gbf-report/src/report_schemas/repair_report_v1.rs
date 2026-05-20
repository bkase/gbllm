//! `repair_report.v1` F-B16 feasibility-refinement report schema.
//!
//! The proposal and terminal-state records are generic so the compiler driver
//! can serialize its existing F-B16 record types without a parallel schema copy.

use serde::{Deserialize, Serialize};

use gbf_foundation::Hash256;
use gbf_policy::{
    CompileKnobBounds, CompileKnobOverrides, CompileKnobValues, CompileKnobs, DiagnosticSeverity,
    ValidationCode, ValidationDetail, ValidationOrigin,
};

use crate::{CanonicalJsonError, ReportBody, ReportOutcome, ValidationDiagnostic, domain_hash};

pub const SCHEMA_ID: &str = "repair_report.v1";
pub const SCHEMA_VERSION: &str = "1.0.0";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepairReportBody<P, S, T> {
    pub report_inputs: RepairReportInputsSection,
    pub initial_knobs: CompileKnobsSnapshot,
    pub final_knobs: CompileKnobsSnapshot,
    pub proposals: Vec<P>,
    pub stage_iteration_counts: Vec<StageIterationCount<S>>,
    pub termination: T,
    pub global_iters_used: u8,
    pub authorized_relaxation_applied: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepairReportInputsSection {
    pub policy_resolution_self_hash: Hash256,
    pub artifact_validation_self_hash: Hash256,
    pub static_budget_self_hash: Option<Hash256>,
    pub schedule_cost_self_hash: Option<Hash256>,
}

impl RepairReportInputsSection {
    /// Narrow-v1 placeholder used by the loop driver until Stage 0/0.5 hands
    /// self-hashes into F-B16. Required upstream hashes use the zero hash as an
    /// explicit "not yet plumbed" sentinel rather than omitting the fields.
    #[must_use]
    pub const fn narrow_v1_unknown() -> Self {
        Self {
            policy_resolution_self_hash: Hash256::ZERO,
            artifact_validation_self_hash: Hash256::ZERO,
            static_budget_self_hash: None,
            schedule_cost_self_hash: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompileKnobsSnapshot {
    pub values: CompileKnobValues,
    pub bounds: CompileKnobBounds,
    pub overrides: CompileKnobOverrides,
    pub locks: gbf_policy::KnobLockSet,
    pub snapshot_hash: Hash256,
}

impl CompileKnobsSnapshot {
    pub fn from_compile_knobs(knobs: &CompileKnobs) -> Result<Self, CanonicalJsonError> {
        let material = CompileKnobsSnapshotMaterial {
            values: knobs.global.clone(),
            bounds: knobs.bounds.clone(),
            overrides: knobs.overrides.clone(),
            locks: knobs.locks.clone(),
        };
        let snapshot_hash = domain_hash("CompileKnobsSnapshot", SCHEMA_ID, &material)?;
        Ok(Self {
            values: material.values,
            bounds: material.bounds,
            overrides: material.overrides,
            locks: material.locks,
            snapshot_hash,
        })
    }
}

#[derive(Serialize)]
struct CompileKnobsSnapshotMaterial {
    values: CompileKnobValues,
    bounds: CompileKnobBounds,
    overrides: CompileKnobOverrides,
    locks: gbf_policy::KnobLockSet,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StageIterationCount<S> {
    pub stage: S,
    pub iterations: u8,
}

pub trait RepairReportProposalRecord {
    fn proposal_id(&self) -> &str;
    fn iter_emitted(&self) -> u8;
    fn accepted_authorized_relaxation(&self) -> bool;
}

pub trait RepairReportTermination {
    fn is_converged(&self) -> bool;
}

impl<P, S, T> ReportBody for RepairReportBody<P, S, T>
where
    P: RepairReportProposalRecord,
    S: Ord,
    T: RepairReportTermination,
{
    const REPORT_TYPE: &'static str = "RepairReport";
    const SCHEMA_ID: &'static str = SCHEMA_ID;
    const SCHEMA_VERSION: &'static str = SCHEMA_VERSION;

    fn validate_semantics(&self, outcome: ReportOutcome) -> Result<(), Vec<ValidationDiagnostic>> {
        let mut errors = Vec::new();

        if (outcome == ReportOutcome::Passed) != self.termination.is_converged() {
            errors.push(semantic_error("termination"));
        }

        if self
            .stage_iteration_counts
            .windows(2)
            .any(|pair| pair[0].stage >= pair[1].stage)
        {
            errors.push(semantic_error("stage_iteration_counts"));
        }

        let mut proposal_ids = std::collections::BTreeSet::new();
        for proposal in &self.proposals {
            if !proposal_ids.insert(proposal.proposal_id()) {
                errors.push(semantic_error("proposals.id"));
                break;
            }
        }

        if self.proposals.windows(2).any(|pair| {
            (pair[0].iter_emitted(), pair[0].proposal_id())
                > (pair[1].iter_emitted(), pair[1].proposal_id())
        }) {
            errors.push(semantic_error("proposals"));
        }

        let has_authorized_relaxation = self
            .proposals
            .iter()
            .any(RepairReportProposalRecord::accepted_authorized_relaxation);
        if self.authorized_relaxation_applied != has_authorized_relaxation {
            errors.push(semantic_error("authorized_relaxation_applied"));
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

fn semantic_error(field: &'static str) -> ValidationDiagnostic {
    let field = gbf_foundation::FieldPath::from(field);
    ValidationDiagnostic {
        severity: DiagnosticSeverity::Hard,
        origin: ValidationOrigin::Schema,
        code: ValidationCode::ReportSemanticInvariantViolated {
            field: field.clone(),
        },
        detail: ValidationDetail::Field { field },
        provenance: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use gbf_foundation::FieldPath;
    use gbf_policy::{
        CompileKnobId, CompileKnobOverrides, CompileKnobPath, CompileKnobProvenanceEntry,
        CompileKnobValues, ConstraintDelta, ConstraintOperation, ConstraintProvenance,
        DeltaRejection, KnobDelta, KnobLockSet, ObservabilityMode, ObservationKnob, OverlayKnob,
        OverlayPromotion, PlacementKnob, PlacementProfile, PolicySource, ProbeCollectionLevel,
        RangeKnob, ReductionPlanCeiling, RepairPolicy, RepairPolicyProfile, RepairProposalId,
        RepairReason, RomKernelDuplicationBias, RomKernelResidencyBias, RomWindowKnob,
        ScheduleKnob, ScheduleResourcePressure, ScheduleSliceCoarsening, ScheduleTileSearch,
        SramKnob, SramPageAggression, StorageKnob, StorageMaterialization,
        canonical_default_bounds_fixture,
    };

    use super::*;
    use crate::{ReportEnvelope, canonicalize, round_trip_self_hash};

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct ProposalRecordFixture {
        id: RepairProposalId,
        source_stage: StageFixture,
        reason: RepairReason,
        delta: ConstraintDelta,
        knob_delta: Option<KnobDelta>,
        resource_pressure: Option<gbf_policy::ResourcePressureUpdate>,
        estimated_cost_delta: Option<EstimatedCostFixture>,
        iter_emitted: u8,
        outcome: ProposalOutcomeFixture,
    }

    impl RepairReportProposalRecord for ProposalRecordFixture {
        fn proposal_id(&self) -> &str {
            self.id.0.as_str()
        }

        fn iter_emitted(&self) -> u8 {
            self.iter_emitted
        }

        fn accepted_authorized_relaxation(&self) -> bool {
            let ProposalOutcomeFixture::Accepted { knobs_delta, .. } = &self.outcome else {
                return false;
            };
            knobs_delta.per_knob.iter().any(|change| {
                matches!(
                    change.operation,
                    ConstraintOperation::AuthorizedRelaxation { .. }
                )
            })
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct EstimatedCostFixture {
        cycles: Option<u64>,
        bytes: Option<u64>,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(tag = "kind", deny_unknown_fields)]
    enum ProposalOutcomeFixture {
        Accepted {
            applied_at_iter: u8,
            knobs_delta: KnobDeltaSummaryFixture,
        },
        Rejected {
            reason: DeltaRejection,
        },
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct KnobDeltaSummaryFixture {
        changed_knobs: BTreeSet<CompileKnobId>,
        changes: Vec<KnobDelta>,
        per_knob: Vec<PerKnobDeltaSummaryFixture>,
        before: CompileKnobs,
        after: CompileKnobs,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct PerKnobDeltaSummaryFixture {
        knob: CompileKnobId,
        before: String,
        after: String,
        operation: ConstraintOperation,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
    #[serde(rename_all = "PascalCase")]
    enum StageFixture {
        RangePlan,
        StoragePlan,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(tag = "kind", deny_unknown_fields)]
    enum TerminalFixture {
        Converged,
        StagedFailureUnrepairable {
            stage: StageFixture,
            last_error: String,
        },
    }

    impl RepairReportTermination for TerminalFixture {
        fn is_converged(&self) -> bool {
            matches!(self, Self::Converged)
        }
    }

    #[test]
    fn schemas_repair_report_versioned() {
        let report = report_fixture();
        let value = serde_json::to_value(&report).expect("report serializes");

        assert_eq!(value["schema"], serde_json::json!("repair_report.v1"));
        assert_eq!(value["schema_version"], serde_json::json!("1.0.0"));
        assert!(value["report_inputs"].is_object());
        assert!(value["initial_knobs"]["snapshot_hash"].is_string());
        assert!(value["final_knobs"]["snapshot_hash"].is_string());
        assert!(value["stage_iteration_counts"].is_array());
        assert!(value["stage_iteration_counts"][0]["stage"].is_string());
        assert!(value["authorized_relaxation_applied"].is_boolean());
        assert_eq!(
            value["proposals"][0]["estimated_cost_delta"]["cycles"],
            serde_json::json!(10)
        );
        assert_eq!(value["proposals"][0]["iter_emitted"], serde_json::json!(1));
        assert_eq!(
            value["proposals"][0]["knob_delta"]["kind"],
            serde_json::json!("AdvancePlacementProfile")
        );
        assert_eq!(value["termination"]["kind"], serde_json::json!("Converged"));

        canonicalize(&report).expect("repair report canonicalizes");
        round_trip_self_hash(&report).expect("repair report self-hash round-trips");
    }

    #[test]
    fn repair_report_rejects_duplicate_or_unsorted_stage_counts() {
        let mut body = report_fixture().body;
        body.stage_iteration_counts = vec![
            StageIterationCount {
                stage: StageFixture::StoragePlan,
                iterations: 1,
            },
            StageIterationCount {
                stage: StageFixture::RangePlan,
                iterations: 1,
            },
        ];

        assert!(body.validate_semantics(ReportOutcome::Passed).is_err());
    }

    #[test]
    fn repair_report_rejects_outcome_termination_mismatch() {
        let body = report_fixture().body;

        assert!(body.validate_semantics(ReportOutcome::Failed).is_err());
    }

    #[test]
    fn repair_report_rejects_duplicate_proposal_ids() {
        let mut body = report_fixture().body;
        body.proposals[1].id = body.proposals[0].id.clone();

        assert!(body.validate_semantics(ReportOutcome::Passed).is_err());
    }

    #[test]
    fn repair_report_rejects_unsorted_proposals() {
        let mut body = report_fixture().body;
        body.proposals.swap(0, 1);

        assert!(body.validate_semantics(ReportOutcome::Passed).is_err());
    }

    fn report_fixture()
    -> ReportEnvelope<RepairReportBody<ProposalRecordFixture, StageFixture, TerminalFixture>> {
        let initial = compile_knobs_fixture();
        let mut final_knobs = initial.clone();
        final_knobs.global.placement.profile = PlacementProfile::Budgeted;

        ReportEnvelope::new(
            ReportOutcome::Passed,
            RepairReportBody {
                report_inputs: RepairReportInputsSection {
                    policy_resolution_self_hash: hash(1),
                    artifact_validation_self_hash: hash(2),
                    static_budget_self_hash: Some(hash(3)),
                    schedule_cost_self_hash: None,
                },
                initial_knobs: CompileKnobsSnapshot::from_compile_knobs(&initial)
                    .expect("initial snapshot hashes"),
                final_knobs: CompileKnobsSnapshot::from_compile_knobs(&final_knobs)
                    .expect("final snapshot hashes"),
                proposals: vec![
                    ProposalRecordFixture {
                        id: RepairProposalId("rp-accepted".to_owned()),
                        source_stage: StageFixture::RangePlan,
                        reason: RepairReason::PlacementProfileFallback,
                        delta: ConstraintDelta {
                            changes: vec![KnobDelta::AdvancePlacementProfile {
                                to: PlacementProfile::Budgeted,
                            }],
                        },
                        knob_delta: Some(KnobDelta::AdvancePlacementProfile {
                            to: PlacementProfile::Budgeted,
                        }),
                        resource_pressure: None,
                        estimated_cost_delta: Some(EstimatedCostFixture {
                            cycles: Some(10),
                            bytes: None,
                        }),
                        iter_emitted: 1,
                        outcome: ProposalOutcomeFixture::Accepted {
                            applied_at_iter: 1,
                            knobs_delta: KnobDeltaSummaryFixture {
                                changed_knobs: BTreeSet::from([CompileKnobId::PlacementProfile]),
                                changes: vec![KnobDelta::AdvancePlacementProfile {
                                    to: PlacementProfile::Budgeted,
                                }],
                                per_knob: vec![PerKnobDeltaSummaryFixture {
                                    knob: CompileKnobId::PlacementProfile,
                                    before: "\"StrictOnePerBank\"".to_owned(),
                                    after: "\"Budgeted\"".to_owned(),
                                    operation: ConstraintOperation::AppliedRepairProposal {
                                        id: RepairProposalId("rp-accepted".to_owned()),
                                    },
                                }],
                                before: initial.clone(),
                                after: final_knobs.clone(),
                            },
                        },
                    },
                    ProposalRecordFixture {
                        id: RepairProposalId("rp-rejected".to_owned()),
                        source_stage: StageFixture::StoragePlan,
                        reason: RepairReason::AccumulatorOverflow,
                        delta: ConstraintDelta {
                            changes: vec![KnobDelta::PromoteRecomputeLevel {
                                to: StorageMaterialization::SpillColdValues,
                            }],
                        },
                        knob_delta: Some(KnobDelta::PromoteRecomputeLevel {
                            to: StorageMaterialization::SpillColdValues,
                        }),
                        resource_pressure: None,
                        estimated_cost_delta: Some(EstimatedCostFixture {
                            cycles: None,
                            bytes: Some(4),
                        }),
                        iter_emitted: 2,
                        outcome: ProposalOutcomeFixture::Rejected {
                            reason: DeltaRejection::KnobLocked {
                                knob: CompileKnobId::Storage,
                            },
                        },
                    },
                ],
                stage_iteration_counts: vec![
                    StageIterationCount {
                        stage: StageFixture::RangePlan,
                        iterations: 2,
                    },
                    StageIterationCount {
                        stage: StageFixture::StoragePlan,
                        iterations: 1,
                    },
                ],
                termination: TerminalFixture::Converged,
                global_iters_used: 3,
                authorized_relaxation_applied: false,
            },
        )
        .expect("report envelope")
        .with_computed_self_hash()
        .expect("self hash computes")
    }

    fn compile_knobs_fixture() -> CompileKnobs {
        CompileKnobs {
            global: CompileKnobValues {
                placement: PlacementKnob {
                    profile: PlacementProfile::StrictOnePerBank,
                },
                observation: ObservationKnob {
                    observability: ObservabilityMode::Flexible,
                    probe_level: ProbeCollectionLevel::RequiredOnly,
                },
                range: RangeKnob {
                    reduction_ceiling: ReductionPlanCeiling::ExactOnly,
                },
                storage: StorageKnob {
                    materialization: StorageMaterialization::PreserveAll,
                },
                sram: SramKnob {
                    page_aggression: SramPageAggression::Preserve,
                },
                rom_window: RomWindowKnob {
                    kernel_residency_bias: RomKernelResidencyBias::PreferCommonBank,
                    kernel_duplication_bias: RomKernelDuplicationBias::Share,
                },
                overlay: OverlayKnob {
                    promotion: OverlayPromotion::Disabled,
                },
                schedule: ScheduleKnob {
                    tile_search: ScheduleTileSearch::Fixed,
                    slice_coarsening: ScheduleSliceCoarsening::Fine,
                    resource_pressure: ScheduleResourcePressure::Conservative,
                },
            },
            bounds: canonical_default_bounds_fixture(),
            locks: KnobLockSet::default(),
            overrides: CompileKnobOverrides::default(),
            provenance: vec![CompileKnobProvenanceEntry {
                path: CompileKnobPath {
                    knob: CompileKnobId::Placement,
                    selector: None,
                    field: Some(FieldPath::from("repair_report.fixture")),
                },
                chain: vec![ConstraintProvenance {
                    source: PolicySource::ProfileDefault,
                    operation: ConstraintOperation::SeedDefault,
                    evidence: Vec::new(),
                }],
            }],
        }
    }

    fn hash(byte: u8) -> Hash256 {
        Hash256::from_bytes([byte; 32])
    }

    #[allow(dead_code)]
    fn _repair_policy_type_stays_public() -> RepairPolicy {
        RepairPolicy::for_profile(RepairPolicyProfile::Default)
    }
}
