//! Bounded monotone feasibility-refinement loop driver.
//!
//! The concrete planning stages still own their local IR construction. This
//! module owns the RFC F-B16 controller contract: only the loop applies policy
//! deltas, every proposal is recorded, and accepted deltas restart the wrapped
//! stage sequence from Stage 5.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

use gbf_foundation::FieldPath;
use gbf_policy::{
    CompileKnobId, CompileKnobPath, CompileKnobProvenanceEntry, CompileKnobs, ConstraintDelta,
    ConstraintOperation, ConstraintProvenance, DeltaRejection, KnobDelta, ObservabilityMode,
    PolicySource, RecomputePurityFacts, RepairPolicy, RepairPolicyProfile, RepairProposalId,
    RepairReason, ResourcePressureUpdate, StageIterationLimits,
    check_delta_admissible_with_recompute_purity,
};
use gbf_report::report_schemas::repair_report_v1::{
    CompileKnobsSnapshot, RepairReportBody, RepairReportInputsSection, RepairReportProposalRecord,
    RepairReportTermination, StageIterationCount,
};
use gbf_report::{ReportEnvelope, ReportOutcome, canonicalize as canonicalize_report};
use serde::{Deserialize, Serialize};

pub const WRAPPED_PLANNING_STAGES: [PlanningStage; 8] = [
    PlanningStage::RangePlan,
    PlanningStage::StoragePlan,
    PlanningStage::SramPagePlan,
    PlanningStage::RomWindowPlan,
    PlanningStage::OverlayPlan,
    PlanningStage::ArenaPlan,
    PlanningStage::GbSchedIr,
    PlanningStage::ResourceStateValidation,
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoopState {
    pub knobs: CompileKnobs,
    pub repair_policy: RepairPolicy,
    pub observability: ObservabilityMode,
    pub recompute_purity: RecomputePurityFacts,
    pub accepted_iters_remaining: u8,
    pub global_iters_remaining: u8,
    pub stage_iters_remaining: StageIterationCeilings,
    pub history: RepairHistory,
}

impl LoopState {
    #[must_use]
    pub fn from_profile(
        knobs: CompileKnobs,
        profile: RepairPolicyProfile,
        observability: ObservabilityMode,
        recompute_purity: RecomputePurityFacts,
    ) -> Self {
        let repair_policy = RepairPolicy::for_profile(profile);
        let stage_iters_remaining =
            StageIterationCeilings::from_limits(knobs.global.schedule.stage_iteration_ceilings);
        let global_iters_remaining = stage_iters_remaining.total_saturating();
        Self {
            knobs,
            repair_policy,
            observability,
            recompute_purity,
            accepted_iters_remaining: repair_policy.max_refinement_iters,
            global_iters_remaining,
            stage_iters_remaining,
            history: RepairHistory::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StageIterationCeilings {
    /// Remaining stage execution attempts. This is deliberately separate from
    /// RepairPolicy::max_refinement_iters, which bounds accepted repair deltas.
    pub range_plan: u8,
    pub storage_plan: u8,
    pub sram_page_plan: u8,
    pub rom_window_plan: u8,
    pub overlay_plan: u8,
    pub arena_plan: u8,
    pub gb_sched_ir: u8,
    pub resource_state_validation: u8,
}

impl StageIterationCeilings {
    #[must_use]
    pub const fn uniform(limit: u8) -> Self {
        Self {
            range_plan: limit,
            storage_plan: limit,
            sram_page_plan: limit,
            rom_window_plan: limit,
            overlay_plan: limit,
            arena_plan: limit,
            gb_sched_ir: limit,
            resource_state_validation: limit,
        }
    }

    #[must_use]
    pub const fn from_limits(limits: StageIterationLimits) -> Self {
        Self {
            range_plan: limits.range_plan,
            storage_plan: limits.storage_plan,
            sram_page_plan: limits.sram_page_plan,
            rom_window_plan: limits.rom_window_plan,
            overlay_plan: limits.overlay_plan,
            arena_plan: limits.arena_plan,
            gb_sched_ir: limits.gb_sched_ir,
            resource_state_validation: limits.resource_state_validation,
        }
    }

    #[must_use]
    pub const fn total_saturating(self) -> u8 {
        let mut total = self.range_plan;
        total = total.saturating_add(self.storage_plan);
        total = total.saturating_add(self.sram_page_plan);
        total = total.saturating_add(self.rom_window_plan);
        total = total.saturating_add(self.overlay_plan);
        total = total.saturating_add(self.arena_plan);
        total = total.saturating_add(self.gb_sched_ir);
        total.saturating_add(self.resource_state_validation)
    }

    #[must_use]
    pub const fn remaining(self, stage: PlanningStage) -> u8 {
        match stage {
            PlanningStage::RangePlan => self.range_plan,
            PlanningStage::StoragePlan => self.storage_plan,
            PlanningStage::SramPagePlan => self.sram_page_plan,
            PlanningStage::RomWindowPlan => self.rom_window_plan,
            PlanningStage::OverlayPlan => self.overlay_plan,
            PlanningStage::ArenaPlan => self.arena_plan,
            PlanningStage::GbSchedIr => self.gb_sched_ir,
            PlanningStage::ResourceStateValidation => self.resource_state_validation,
        }
    }

    pub fn decrement(&mut self, stage: PlanningStage) {
        match stage {
            PlanningStage::RangePlan => self.range_plan = self.range_plan.saturating_sub(1),
            PlanningStage::StoragePlan => self.storage_plan = self.storage_plan.saturating_sub(1),
            PlanningStage::SramPagePlan => {
                self.sram_page_plan = self.sram_page_plan.saturating_sub(1);
            }
            PlanningStage::RomWindowPlan => {
                self.rom_window_plan = self.rom_window_plan.saturating_sub(1);
            }
            PlanningStage::OverlayPlan => self.overlay_plan = self.overlay_plan.saturating_sub(1),
            PlanningStage::ArenaPlan => self.arena_plan = self.arena_plan.saturating_sub(1),
            PlanningStage::GbSchedIr => self.gb_sched_ir = self.gb_sched_ir.saturating_sub(1),
            PlanningStage::ResourceStateValidation => {
                self.resource_state_validation = self.resource_state_validation.saturating_sub(1);
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepairHistory {
    pub proposals: Vec<RepairProposalRecord>,
    pub stage_iteration_counts: BTreeMap<PlanningStage, u8>,
}

impl RepairHistory {
    fn record_stage_run(&mut self, stage: PlanningStage) {
        let count = self.stage_iteration_counts.entry(stage).or_insert(0);
        *count = count.saturating_add(1);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum PlanningStage {
    RangePlan,
    StoragePlan,
    SramPagePlan,
    RomWindowPlan,
    OverlayPlan,
    ArenaPlan,
    GbSchedIr,
    ResourceStateValidation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepairProposal {
    pub id: RepairProposalId,
    pub source_stage: PlanningStage,
    pub reason: RepairReason,
    pub delta: ConstraintDelta,
    pub estimated_cost: EstimatedCostDelta,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EstimatedCostDelta {
    pub cycles: Option<u64>,
    pub bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepairProposalRecord {
    pub id: RepairProposalId,
    pub source_stage: PlanningStage,
    pub reason: RepairReason,
    pub delta: ConstraintDelta,
    pub knob_delta: Option<KnobDelta>,
    pub resource_pressure: Option<ResourcePressureUpdate>,
    pub estimated_cost_delta: Option<EstimatedCostDelta>,
    pub iter_emitted: u8,
    pub outcome: ProposalOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ProposalOutcome {
    Accepted {
        applied_at_iter: u8,
        knobs_delta: Box<KnobDeltaSummary>,
    },
    Rejected {
        reason: DeltaRejection,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KnobDeltaSummary {
    pub changed_knobs: BTreeSet<CompileKnobId>,
    pub changes: Vec<KnobDelta>,
    pub per_knob: Vec<PerKnobDeltaSummary>,
    pub before: CompileKnobs,
    pub after: CompileKnobs,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PerKnobDeltaSummary {
    pub knob: CompileKnobId,
    pub before: String,
    pub after: String,
    pub operation: ConstraintOperation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum StageRunOutcome {
    Succeeded,
    ProposedRepair { proposal: RepairProposal },
    Unrepairable { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum TerminalState {
    Converged,
    AcceptedRefinementBudgetExhausted {
        stage: PlanningStage,
    },
    GlobalBudgetExhausted,
    StageBudgetExhausted {
        stage: PlanningStage,
    },
    StagedFailureUnrepairable {
        stage: PlanningStage,
        last_error: String,
    },
}

impl RepairReportTermination for TerminalState {
    fn is_converged(&self) -> bool {
        matches!(self, Self::Converged)
    }
}

impl RepairReportProposalRecord for RepairProposalRecord {
    fn proposal_id(&self) -> &str {
        self.id.0.as_str()
    }

    fn iter_emitted(&self) -> u8 {
        self.iter_emitted
    }

    fn accepted_authorized_relaxation(&self) -> bool {
        let ProposalOutcome::Accepted { knobs_delta, .. } = &self.outcome else {
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RefinementLoopResult {
    pub state: LoopState,
    pub terminal: TerminalState,
}

pub const REPAIR_REPORT_FILE_NAME: &str = "repair_report.json";

pub type RefinementRepairReportBody =
    RepairReportBody<RepairProposalRecord, PlanningStage, TerminalState>;
pub type RefinementRepairReport = ReportEnvelope<RefinementRepairReportBody>;

pub trait CompilerPipeline {
    fn run_stage(
        &mut self,
        stage: PlanningStage,
        state: &LoopState,
    ) -> Result<StageRunOutcome, RefinementLoopError>;

    fn handle_rejected_repair(
        &mut self,
        _stage: PlanningStage,
        _state: &LoopState,
        _rejected: &RepairProposalRecord,
    ) -> Result<Option<RepairProposal>, RefinementLoopError> {
        Ok(None)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefinementLoopError {
    ProposalStageMismatch {
        expected: PlanningStage,
        actual: PlanningStage,
    },
}

impl fmt::Display for RefinementLoopError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ProposalStageMismatch { expected, actual } => write!(
                f,
                "repair proposal source stage {actual:?} did not match running stage {expected:?}"
            ),
        }
    }
}

impl Error for RefinementLoopError {}

pub fn run_refinement_loop<P>(
    initial_state: LoopState,
    pipeline: &mut P,
) -> Result<RefinementLoopResult, RefinementLoopError>
where
    P: CompilerPipeline,
{
    let mut state = initial_state;
    let mut accepted_iters = 0u8;

    'restart: loop {
        for stage in WRAPPED_PLANNING_STAGES {
            if state.global_iters_remaining == 0 {
                return Ok(terminal(state, TerminalState::GlobalBudgetExhausted));
            }
            if state.stage_iters_remaining.remaining(stage) == 0 {
                return Ok(terminal(
                    state,
                    TerminalState::StageBudgetExhausted { stage },
                ));
            }

            state.global_iters_remaining = state.global_iters_remaining.saturating_sub(1);
            state.stage_iters_remaining.decrement(stage);
            state.history.record_stage_run(stage);

            match pipeline.run_stage(stage, &state)? {
                StageRunOutcome::Succeeded => {}
                StageRunOutcome::Unrepairable { message } => {
                    return Ok(terminal(
                        state,
                        TerminalState::StagedFailureUnrepairable {
                            stage,
                            last_error: message,
                        },
                    ));
                }
                StageRunOutcome::ProposedRepair { proposal } => {
                    let mut proposal = proposal;
                    loop {
                        let iter_emitted = accepted_iters.saturating_add(1);
                        if proposal.source_stage != stage {
                            return Err(RefinementLoopError::ProposalStageMismatch {
                                expected: stage,
                                actual: proposal.source_stage,
                            });
                        }
                        match first_rejection(&proposal.delta, &state) {
                            Some(rejection) => {
                                let record = record_rejected(
                                    &mut state.history,
                                    proposal,
                                    iter_emitted,
                                    rejection.clone(),
                                );
                                match pipeline.handle_rejected_repair(stage, &state, &record)? {
                                    Some(alternative) => {
                                        proposal = alternative;
                                        continue;
                                    }
                                    None => {
                                        return Ok(terminal(
                                            state,
                                            TerminalState::StagedFailureUnrepairable {
                                                stage,
                                                last_error: format!(
                                                    "repair proposal rejected: {rejection:?}"
                                                ),
                                            },
                                        ));
                                    }
                                }
                            }
                            None => {
                                if state.accepted_iters_remaining == 0 {
                                    let rejection =
                                        DeltaRejection::AcceptedRefinementBudgetExhausted {
                                            max_refinement_iters: state
                                                .repair_policy
                                                .max_refinement_iters,
                                        };
                                    record_rejected(
                                        &mut state.history,
                                        proposal,
                                        iter_emitted,
                                        rejection,
                                    );
                                    return Ok(terminal(
                                        state,
                                        TerminalState::AcceptedRefinementBudgetExhausted { stage },
                                    ));
                                }
                                let before = state.knobs.clone();
                                apply_constraint_delta(&mut state.knobs, &proposal);
                                let after = state.knobs.clone();
                                let summary = KnobDeltaSummary::from_delta(
                                    &proposal.delta,
                                    before,
                                    after,
                                    &proposal.id,
                                );
                                accepted_iters = accepted_iters.saturating_add(1);
                                state.accepted_iters_remaining =
                                    state.accepted_iters_remaining.saturating_sub(1);
                                record_accepted(
                                    &mut state.history,
                                    proposal,
                                    iter_emitted,
                                    accepted_iters,
                                    summary,
                                );
                                continue 'restart;
                            }
                        }
                    }
                }
            }
        }

        return Ok(terminal(state, TerminalState::Converged));
    }
}

fn terminal(state: LoopState, terminal: TerminalState) -> RefinementLoopResult {
    RefinementLoopResult { state, terminal }
}

fn first_rejection(proposal: &ConstraintDelta, state: &LoopState) -> Option<DeltaRejection> {
    // Atomic multi-change proposals fail on the first inadmissible change;
    // the report records that first blocker rather than every failing change.
    proposal.changes.iter().find_map(|change| {
        check_delta_admissible_with_recompute_purity(
            change,
            &state.knobs,
            &state.repair_policy,
            state.observability,
            &state.recompute_purity,
        )
        .err()
    })
}

fn record_accepted(
    history: &mut RepairHistory,
    proposal: RepairProposal,
    iter_emitted: u8,
    applied_at_iter: u8,
    knobs_delta: KnobDeltaSummary,
) {
    let knob_delta = proposal.delta.changes.first().cloned();
    let resource_pressure = proposal
        .delta
        .changes
        .iter()
        .find_map(resource_pressure_update);
    history.proposals.push(RepairProposalRecord {
        id: proposal.id,
        source_stage: proposal.source_stage,
        reason: proposal.reason,
        delta: proposal.delta,
        knob_delta,
        resource_pressure,
        estimated_cost_delta: Some(proposal.estimated_cost),
        iter_emitted,
        outcome: ProposalOutcome::Accepted {
            applied_at_iter,
            knobs_delta: Box::new(knobs_delta),
        },
    });
}

fn record_rejected(
    history: &mut RepairHistory,
    proposal: RepairProposal,
    iter_emitted: u8,
    reason: DeltaRejection,
) -> RepairProposalRecord {
    let knob_delta = proposal.delta.changes.first().cloned();
    let resource_pressure = proposal
        .delta
        .changes
        .iter()
        .find_map(resource_pressure_update);
    let record = RepairProposalRecord {
        id: proposal.id,
        source_stage: proposal.source_stage,
        reason: proposal.reason,
        delta: proposal.delta,
        knob_delta,
        resource_pressure,
        estimated_cost_delta: Some(proposal.estimated_cost),
        iter_emitted,
        outcome: ProposalOutcome::Rejected { reason },
    };
    history.proposals.push(record.clone());
    record
}

impl KnobDeltaSummary {
    #[must_use]
    pub fn from_delta(
        delta: &ConstraintDelta,
        before: CompileKnobs,
        after: CompileKnobs,
        proposal_id: &RepairProposalId,
    ) -> Self {
        Self {
            changed_knobs: delta.changes.iter().map(KnobDelta::knob_id).collect(),
            changes: delta.changes.clone(),
            per_knob: delta
                .changes
                .iter()
                .map(|change| PerKnobDeltaSummary {
                    knob: change.knob_id(),
                    before: knob_value_string(&before, change.knob_id()),
                    after: knob_value_string(&after, change.knob_id()),
                    operation: ConstraintOperation::AppliedRepairProposal {
                        id: proposal_id.clone(),
                    },
                })
                .collect(),
            before,
            after,
        }
    }
}

fn resource_pressure_update(change: &KnobDelta) -> Option<ResourcePressureUpdate> {
    match change {
        KnobDelta::UpdatePressureThreshold { update } => Some(*update),
        _ => None,
    }
}

fn knob_value_string(knobs: &CompileKnobs, knob: CompileKnobId) -> String {
    let value = match knob {
        CompileKnobId::Placement | CompileKnobId::PlacementProfile => {
            serde_json::to_value(knobs.global.placement.profile)
        }
        CompileKnobId::Observation
        | CompileKnobId::ObservationTraceDemotion
        | CompileKnobId::ObservationProbeSelection => {
            serde_json::to_value(knobs.global.observation.trace_demotion)
        }
        CompileKnobId::Range | CompileKnobId::RangeReductionCeiling => {
            serde_json::to_value(knobs.global.range.reduction_ceiling)
        }
        CompileKnobId::Storage
        | CompileKnobId::StorageRecomputePromotion
        | CompileKnobId::StorageMaterializationOverrides => {
            serde_json::to_value(knobs.global.storage.materialization)
        }
        CompileKnobId::Sram | CompileKnobId::SramPageAggression => {
            serde_json::to_value(knobs.global.sram.page_aggression)
        }
        CompileKnobId::SramSpillPolicy => serde_json::to_value(knobs.global.sram.spill_policy),
        CompileKnobId::RomWindow | CompileKnobId::RomKernelResidencyBias => {
            serde_json::to_value(knobs.global.rom_window.kernel_residency_bias)
        }
        CompileKnobId::RomKernelDuplicationBias | CompileKnobId::RomKernelResidencyOverrides => {
            serde_json::to_value(knobs.global.rom_window.kernel_duplication_bias)
        }
        CompileKnobId::Overlay | CompileKnobId::OverlayPromotion => {
            serde_json::to_value(knobs.global.overlay.promotion)
        }
        CompileKnobId::Schedule | CompileKnobId::ScheduleTileSearch => {
            serde_json::to_value(knobs.global.schedule.tile_search)
        }
        CompileKnobId::ScheduleSliceCoarsening => {
            serde_json::to_value(knobs.global.schedule.slice_coarsening)
        }
        CompileKnobId::ScheduleResourcePressure => Ok(serde_json::json!({
            "resource_pressure": knobs.global.schedule.resource_pressure,
            "pressure_thresholds": knobs.global.schedule.pressure_thresholds,
        })),
        CompileKnobId::StageIterationCeilings => {
            serde_json::to_value(knobs.global.schedule.stage_iteration_ceilings)
        }
    }
    .expect("knob value serializes");
    serde_json::to_string(&value).expect("knob value string serializes")
}

#[must_use]
pub fn repair_report_body(
    initial_state: &LoopState,
    result: &RefinementLoopResult,
    report_inputs: RepairReportInputsSection,
) -> RefinementRepairReportBody {
    RepairReportBody {
        report_inputs,
        initial_knobs: CompileKnobsSnapshot::from_compile_knobs(&initial_state.knobs)
            .expect("initial compile knobs snapshot hashes"),
        final_knobs: CompileKnobsSnapshot::from_compile_knobs(&result.state.knobs)
            .expect("final compile knobs snapshot hashes"),
        proposals: result.state.history.proposals.clone(),
        stage_iteration_counts: result
            .state
            .history
            .stage_iteration_counts
            .iter()
            .map(|(stage, iterations)| StageIterationCount {
                stage: *stage,
                iterations: *iterations,
            })
            .collect(),
        termination: result.terminal.clone(),
        global_iters_used: initial_state
            .global_iters_remaining
            .saturating_sub(result.state.global_iters_remaining),
        authorized_relaxation_applied: has_authorized_relaxation(&result.state.knobs),
    }
}

pub fn repair_report(
    initial_state: &LoopState,
    result: &RefinementLoopResult,
    report_inputs: RepairReportInputsSection,
) -> RefinementRepairReport {
    ReportEnvelope::new(
        repair_report_outcome(&result.terminal),
        repair_report_body(initial_state, result, report_inputs),
    )
    .expect("repair_report.v1 schema constants are valid")
    .with_computed_self_hash()
    .expect("repair_report.v1 self-hash computes")
}

pub fn repair_report_json(
    initial_state: &LoopState,
    result: &RefinementLoopResult,
    report_inputs: RepairReportInputsSection,
) -> Vec<u8> {
    canonicalize_report(&repair_report(initial_state, result, report_inputs))
        .expect("repair_report.v1 canonicalizes")
}

fn repair_report_outcome(terminal: &TerminalState) -> ReportOutcome {
    match terminal {
        TerminalState::Converged => ReportOutcome::Passed,
        TerminalState::AcceptedRefinementBudgetExhausted { .. }
        | TerminalState::GlobalBudgetExhausted
        | TerminalState::StageBudgetExhausted { .. }
        | TerminalState::StagedFailureUnrepairable { .. } => ReportOutcome::Failed,
    }
}

fn has_authorized_relaxation(knobs: &CompileKnobs) -> bool {
    knobs.provenance.iter().any(|entry| {
        entry.chain.iter().any(|frame| {
            matches!(
                frame.operation,
                ConstraintOperation::AuthorizedRelaxation { .. }
            )
        })
    })
}

fn apply_constraint_delta(knobs: &mut CompileKnobs, proposal: &RepairProposal) {
    for change in &proposal.delta.changes {
        apply_knob_delta(knobs, change);
        knobs.provenance.push(repair_provenance_entry(
            change.knob_id(),
            proposal.id.clone(),
        ));
    }
}

fn apply_knob_delta(knobs: &mut CompileKnobs, delta: &KnobDelta) {
    match delta {
        KnobDelta::AdvancePlacementProfile { to } => knobs.global.placement.profile = *to,
        KnobDelta::SetTraceDemotion { to } => knobs.global.observation.trace_demotion = *to,
        KnobDelta::DisableOptionalProbes { probes } => {
            knobs
                .overrides
                .disabled_optional_probes
                .extend(probes.iter().cloned());
        }
        KnobDelta::RaiseReductionCeiling { selector, to } => match selector {
            Some(selector) => {
                knobs
                    .overrides
                    .reduction_ceiling_overrides
                    .insert(selector.clone(), *to);
            }
            None => knobs.global.range.reduction_ceiling = *to,
        },
        KnobDelta::PromoteRecomputeLevel { to } => knobs.global.storage.materialization = *to,
        KnobDelta::ForceRecompute { values } => {
            knobs
                .overrides
                .forced_recompute
                .extend(values.iter().cloned());
        }
        KnobDelta::AdvanceSramPageAggression { to } => knobs.global.sram.page_aggression = *to,
        KnobDelta::AdvanceSramSpillPolicy { to } => knobs.global.sram.spill_policy = *to,
        KnobDelta::AdvanceKernelResidencyBias { to } => {
            knobs.global.rom_window.kernel_residency_bias = *to;
        }
        KnobDelta::AdvanceKernelDuplicationBias { to } => {
            knobs.global.rom_window.kernel_duplication_bias = *to;
        }
        KnobDelta::ForceKernelResidency { selector, to } => {
            knobs
                .overrides
                .forced_kernel_residency
                .insert(selector.clone(), *to);
        }
        KnobDelta::PromoteOverlay { to } => knobs.global.overlay.promotion = *to,
        KnobDelta::NarrowTileClasses {
            selector,
            remaining,
        } => {
            knobs
                .overrides
                .tile_class_overrides
                .insert(selector.clone(), remaining.clone());
        }
        KnobDelta::SetSliceCoarsening { to } => knobs.global.schedule.slice_coarsening = *to,
        KnobDelta::UpdatePressureThreshold { update } => apply_pressure_update(knobs, update),
    }
}

fn apply_pressure_update(knobs: &mut CompileKnobs, update: &ResourcePressureUpdate) {
    match update {
        ResourcePressureUpdate::WramHot { limit } => {
            knobs.global.schedule.pressure_thresholds.wram_hot = *limit;
        }
        ResourcePressureUpdate::HramHot { limit } => {
            knobs.global.schedule.pressure_thresholds.hram_hot = *limit;
        }
        ResourcePressureUpdate::Bank0Rom { limit } => {
            knobs.global.schedule.pressure_thresholds.bank0_rom = *limit;
        }
        ResourcePressureUpdate::SwitchableRomWindow { limit } => {
            knobs
                .global
                .schedule
                .pressure_thresholds
                .switchable_rom_window = *limit;
        }
        ResourcePressureUpdate::SramWindow { limit } => {
            knobs.global.schedule.pressure_thresholds.sram_window = *limit;
        }
        ResourcePressureUpdate::SliceCycles { limit } => {
            knobs.global.schedule.pressure_thresholds.slice_cycles = *limit;
        }
        ResourcePressureUpdate::InterruptLatency { limit } => {
            knobs.global.schedule.pressure_thresholds.interrupt_latency = *limit;
        }
        ResourcePressureUpdate::TraceBytesPerFrame { limit } => {
            knobs
                .global
                .schedule
                .pressure_thresholds
                .trace_bytes_per_frame = *limit;
        }
        ResourcePressureUpdate::PersistBytesPerFrame { limit } => {
            knobs
                .global
                .schedule
                .pressure_thresholds
                .persist_bytes_per_frame = *limit;
        }
        ResourcePressureUpdate::OverlayInstallsPerFrame { limit } => {
            knobs
                .global
                .schedule
                .pressure_thresholds
                .overlay_installs_per_frame = *limit;
        }
        ResourcePressureUpdate::BankSwitchesPerToken { limit } => {
            knobs
                .global
                .schedule
                .pressure_thresholds
                .bank_switches_per_token = *limit;
        }
        ResourcePressureUpdate::SramPageSwitchesPerToken { limit } => {
            knobs
                .global
                .schedule
                .pressure_thresholds
                .sram_page_switches_per_token = *limit;
        }
    }
}

fn repair_provenance_entry(
    knob: CompileKnobId,
    id: RepairProposalId,
) -> CompileKnobProvenanceEntry {
    CompileKnobProvenanceEntry {
        path: CompileKnobPath {
            knob,
            selector: None,
            field: Some(FieldPath::from("repair.delta")),
        },
        chain: vec![ConstraintProvenance {
            source: PolicySource::RepairProposal { id: id.clone() },
            operation: ConstraintOperation::AppliedRepairProposal { id },
            evidence: Vec::new(),
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gbf_foundation::Hash256;
    use gbf_policy::{
        CompileKnobBounds, CompileKnobOverrides, CompileKnobValues, KernelResidency,
        KernelSelector, KernelSpecId, KnobLockSet, ObservationKnob, ObservationKnobBounds,
        OverlayKnob, OverlayKnobBounds, OverlayPromotion, PlacementKnob, PlacementKnobBounds,
        PlacementProfile, PressureLimit, ProbeCollectionLevel, RangeKnob, RangeKnobBounds,
        ReductionPlanCeiling, RepairPolicyProfile, ResourcePressureThresholds,
        ResourcePressureUpdate, RomKernelDuplicationBias, RomKernelResidencyBias, RomWindowKnob,
        RomWindowKnobBounds, ScheduleKnob, ScheduleKnobBounds, ScheduleResourcePressure,
        ScheduleSliceCoarsening, ScheduleTileSearch, SramKnob, SramKnobBounds, SramPageAggression,
        SramSpillPolicy, StageIterationLimits, StorageKnob, StorageKnobBounds,
        StorageMaterialization, TraceDemotionLevel, ValueId, ValueSelector,
    };
    use gbf_report::round_trip_self_hash;

    #[derive(Default)]
    struct ScriptedPipeline {
        outcomes: BTreeMap<PlanningStage, Vec<StageRunOutcome>>,
        rejection_alternatives: BTreeMap<PlanningStage, Vec<RepairProposal>>,
    }

    impl ScriptedPipeline {
        fn with(stage: PlanningStage, outcomes: Vec<StageRunOutcome>) -> Self {
            Self {
                outcomes: BTreeMap::from([(stage, outcomes)]),
                rejection_alternatives: BTreeMap::new(),
            }
        }

        fn with_rejection_alternatives(
            stage: PlanningStage,
            outcomes: Vec<StageRunOutcome>,
            alternatives: Vec<RepairProposal>,
        ) -> Self {
            Self {
                outcomes: BTreeMap::from([(stage, outcomes)]),
                rejection_alternatives: BTreeMap::from([(stage, alternatives)]),
            }
        }
    }

    impl CompilerPipeline for ScriptedPipeline {
        fn run_stage(
            &mut self,
            stage: PlanningStage,
            _state: &LoopState,
        ) -> Result<StageRunOutcome, RefinementLoopError> {
            Ok(self
                .outcomes
                .get_mut(&stage)
                .and_then(|outcomes| {
                    if outcomes.is_empty() {
                        None
                    } else {
                        Some(outcomes.remove(0))
                    }
                })
                .unwrap_or(StageRunOutcome::Succeeded))
        }

        fn handle_rejected_repair(
            &mut self,
            stage: PlanningStage,
            _state: &LoopState,
            _rejected: &RepairProposalRecord,
        ) -> Result<Option<RepairProposal>, RefinementLoopError> {
            Ok(self
                .rejection_alternatives
                .get_mut(&stage)
                .and_then(|alternatives| {
                    if alternatives.is_empty() {
                        None
                    } else {
                        Some(alternatives.remove(0))
                    }
                }))
        }
    }

    #[test]
    fn convergence_no_proposals() {
        let mut pipeline = ScriptedPipeline::default();
        let result = run_refinement_loop(
            loop_state(8, StageIterationCeilings::uniform(2)),
            &mut pipeline,
        )
        .expect("loop runs");

        assert_eq!(result.terminal, TerminalState::Converged);
        assert_eq!(result.state.history.proposals, Vec::new());
        assert_eq!(
            result
                .state
                .history
                .stage_iteration_counts
                .get(&PlanningStage::RangePlan),
            Some(&1)
        );
    }

    #[test]
    fn loop_state_from_profile_separates_repair_and_stage_budgets() {
        let mut knobs = compile_knobs_fixture();
        knobs.global.schedule.stage_iteration_ceilings = StageIterationLimits {
            range_plan: 1,
            storage_plan: 2,
            sram_page_plan: 3,
            rom_window_plan: 4,
            overlay_plan: 5,
            arena_plan: 6,
            gb_sched_ir: 7,
            resource_state_validation: 8,
        };

        let state = LoopState::from_profile(
            knobs,
            RepairPolicyProfile::Recovery,
            ObservabilityMode::Flexible,
            RecomputePurityFacts::default(),
        );

        assert_eq!(state.accepted_iters_remaining, 6);
        assert_eq!(state.global_iters_remaining, 36);
        assert_eq!(state.stage_iters_remaining.range_plan, 1);
        assert_eq!(state.stage_iters_remaining.resource_state_validation, 8);
        assert_eq!(
            state.repair_policy,
            RepairPolicy::for_profile(RepairPolicyProfile::Recovery)
        );
    }

    #[test]
    fn placement_fallback_advance() {
        let proposal = proposal(
            "rp-placement",
            PlanningStage::StoragePlan,
            RepairReason::PlacementProfileFallback,
            KnobDelta::AdvancePlacementProfile {
                to: PlacementProfile::Budgeted,
            },
        );
        let mut pipeline = ScriptedPipeline::with(
            PlanningStage::StoragePlan,
            vec![StageRunOutcome::ProposedRepair { proposal }],
        );
        let result = run_refinement_loop(
            loop_state(16, StageIterationCeilings::uniform(4)),
            &mut pipeline,
        )
        .expect("loop runs");

        assert_eq!(result.terminal, TerminalState::Converged);
        assert_eq!(
            result.state.knobs.global.placement.profile,
            PlacementProfile::Budgeted
        );
        assert!(matches!(
            result.state.history.proposals[0].outcome,
            ProposalOutcome::Accepted {
                applied_at_iter: 1,
                ..
            }
        ));
    }

    #[test]
    fn overlay_promotion_advance() {
        let proposal = proposal(
            "rp-overlay",
            PlanningStage::OverlayPlan,
            RepairReason::OverlayBudgetExceeded,
            KnobDelta::PromoteOverlay {
                to: OverlayPromotion::TinyLuts,
            },
        );
        let mut pipeline = ScriptedPipeline::with(
            PlanningStage::OverlayPlan,
            vec![StageRunOutcome::ProposedRepair { proposal }],
        );
        let result = run_refinement_loop(
            loop_state(16, StageIterationCeilings::uniform(4)),
            &mut pipeline,
        )
        .expect("loop runs");

        assert_eq!(result.terminal, TerminalState::Converged);
        assert_eq!(
            result.state.knobs.global.overlay.promotion,
            OverlayPromotion::TinyLuts
        );
    }

    #[test]
    fn effectful_recompute_rejected() {
        let value = ValueSelector::Value { id: ValueId(7) };
        let proposal = proposal(
            "rp-recompute",
            PlanningStage::StoragePlan,
            RepairReason::AccumulatorOverflow,
            KnobDelta::ForceRecompute {
                values: BTreeSet::from([value.clone()]),
            },
        );
        let mut pipeline = ScriptedPipeline::with(
            PlanningStage::StoragePlan,
            vec![StageRunOutcome::ProposedRepair { proposal }],
        );
        let mut state = loop_state(8, StageIterationCeilings::uniform(2));
        state
            .recompute_purity
            .effectful_values
            .insert(value.clone());

        let result = run_refinement_loop(state, &mut pipeline).expect("loop runs");

        assert!(matches!(
            result.terminal,
            TerminalState::StagedFailureUnrepairable {
                stage: PlanningStage::StoragePlan,
                ..
            }
        ));
        assert!(matches!(
            result.state.history.proposals[0].outcome,
            ProposalOutcome::Rejected {
                reason: DeltaRejection::EffectfulRecompute { .. }
            }
        ));
    }

    #[test]
    fn rejected_proposal_can_return_to_stage_for_alternative() {
        let rejected = proposal(
            "rp-1-trace",
            PlanningStage::RangePlan,
            RepairReason::ScheduleCostMissedTarget,
            KnobDelta::SetTraceDemotion {
                to: TraceDemotionLevel::DropBestEffort,
            },
        );
        let alternative = proposal(
            "rp-2-placement",
            PlanningStage::RangePlan,
            RepairReason::PlacementProfileFallback,
            KnobDelta::AdvancePlacementProfile {
                to: PlacementProfile::Budgeted,
            },
        );
        let mut pipeline = ScriptedPipeline::with_rejection_alternatives(
            PlanningStage::RangePlan,
            vec![StageRunOutcome::ProposedRepair { proposal: rejected }],
            vec![alternative],
        );
        let mut state = loop_state(16, StageIterationCeilings::uniform(4));
        state.observability = ObservabilityMode::Invariant;

        let result = run_refinement_loop(state, &mut pipeline).expect("loop runs");

        assert_eq!(result.terminal, TerminalState::Converged);
        assert_eq!(
            result.state.knobs.global.placement.profile,
            PlacementProfile::Budgeted
        );
        assert!(matches!(
            result.state.history.proposals[0].outcome,
            ProposalOutcome::Rejected {
                reason: DeltaRejection::InvariantObservabilityViolation { .. }
            }
        ));
        assert!(matches!(
            result.state.history.proposals[1].outcome,
            ProposalOutcome::Accepted { .. }
        ));
    }

    #[test]
    fn invariant_blocks_trace_demotion() {
        let proposal = proposal(
            "rp-trace",
            PlanningStage::RangePlan,
            RepairReason::ScheduleCostMissedTarget,
            KnobDelta::SetTraceDemotion {
                to: TraceDemotionLevel::DropBestEffort,
            },
        );
        let mut pipeline = ScriptedPipeline::with(
            PlanningStage::RangePlan,
            vec![StageRunOutcome::ProposedRepair { proposal }],
        );
        let mut state = loop_state(8, StageIterationCeilings::uniform(2));
        state.observability = ObservabilityMode::Invariant;

        let result = run_refinement_loop(state, &mut pipeline).expect("loop runs");

        assert!(matches!(
            result.state.history.proposals[0].outcome,
            ProposalOutcome::Rejected {
                reason: DeltaRejection::InvariantObservabilityViolation { .. }
            }
        ));
    }

    #[test]
    fn sram_spill_policy_advance_applies_to_compile_knobs() {
        let proposal = proposal(
            "rp-spill",
            PlanningStage::SramPagePlan,
            RepairReason::ResourceStateValidationFailed,
            KnobDelta::AdvanceSramSpillPolicy {
                to: SramSpillPolicy::SpillOnPressure,
            },
        );
        let mut pipeline = ScriptedPipeline::with(
            PlanningStage::SramPagePlan,
            vec![StageRunOutcome::ProposedRepair { proposal }],
        );

        let result = run_refinement_loop(
            loop_state(8, StageIterationCeilings::uniform(2)),
            &mut pipeline,
        )
        .expect("loop runs");

        assert_eq!(
            result.state.knobs.global.sram.spill_policy,
            SramSpillPolicy::SpillOnPressure
        );
        assert!(matches!(
            result.state.history.proposals[0].outcome,
            ProposalOutcome::Accepted { .. }
        ));
    }

    #[test]
    fn pressure_threshold_update_applies_to_compile_knobs() {
        let proposal = proposal(
            "rp-pressure",
            PlanningStage::ResourceStateValidation,
            RepairReason::ResourceStateValidationFailed,
            KnobDelta::UpdatePressureThreshold {
                update: ResourcePressureUpdate::WramHot {
                    limit: PressureLimit {
                        soft: 7000,
                        hard: 8192,
                    },
                },
            },
        );
        let mut pipeline = ScriptedPipeline::with(
            PlanningStage::ResourceStateValidation,
            vec![StageRunOutcome::ProposedRepair { proposal }],
        );

        let result = run_refinement_loop(
            loop_state(8, StageIterationCeilings::uniform(2)),
            &mut pipeline,
        )
        .expect("loop runs");

        assert!(matches!(
            result.state.history.proposals[0].outcome,
            ProposalOutcome::Accepted { .. }
        ));
        assert_eq!(
            result
                .state
                .knobs
                .global
                .schedule
                .pressure_thresholds
                .wram_hot,
            PressureLimit {
                soft: 7000,
                hard: 8192
            }
        );
        match &result.state.history.proposals[0].outcome {
            ProposalOutcome::Accepted { knobs_delta, .. } => {
                assert_eq!(
                    knobs_delta.per_knob[0].knob,
                    CompileKnobId::ScheduleResourcePressure
                );
                assert!(
                    knobs_delta.per_knob[0]
                        .before
                        .contains("\"resource_pressure\"")
                );
                assert!(knobs_delta.per_knob[0].before.contains("Conservative"));
                assert!(
                    knobs_delta.per_knob[0]
                        .after
                        .contains("\"pressure_thresholds\"")
                );
                assert!(knobs_delta.per_knob[0].before.contains("\"soft\":6144"));
                assert!(knobs_delta.per_knob[0].after.contains("\"soft\":7000"));
            }
            ProposalOutcome::Rejected { .. } => panic!("pressure update should be accepted"),
        }
    }

    #[test]
    fn proposal_stage_mismatch_errors() {
        let proposal = proposal(
            "rp-mismatch",
            PlanningStage::StoragePlan,
            RepairReason::ArenaOverflow,
            KnobDelta::AdvancePlacementProfile {
                to: PlacementProfile::Budgeted,
            },
        );
        let mut pipeline = ScriptedPipeline::with(
            PlanningStage::RangePlan,
            vec![StageRunOutcome::ProposedRepair { proposal }],
        );

        let err = run_refinement_loop(
            loop_state(8, StageIterationCeilings::uniform(2)),
            &mut pipeline,
        )
        .expect_err("mismatched proposal source stage rejects");

        assert_eq!(
            err,
            RefinementLoopError::ProposalStageMismatch {
                expected: PlanningStage::RangePlan,
                actual: PlanningStage::StoragePlan,
            }
        );
    }

    #[test]
    fn stage_budget_exhausted() {
        let mut ceilings = StageIterationCeilings::uniform(2);
        ceilings.storage_plan = 0;
        let mut pipeline = ScriptedPipeline::default();
        let result =
            run_refinement_loop(loop_state(8, ceilings), &mut pipeline).expect("loop runs");

        assert_eq!(
            result.terminal,
            TerminalState::StageBudgetExhausted {
                stage: PlanningStage::StoragePlan
            }
        );
    }

    #[test]
    fn global_budget_exhausted() {
        let mut pipeline = ScriptedPipeline::default();
        let result = run_refinement_loop(
            loop_state(0, StageIterationCeilings::uniform(2)),
            &mut pipeline,
        )
        .expect("loop runs");

        assert_eq!(result.terminal, TerminalState::GlobalBudgetExhausted);
    }

    #[test]
    fn accepted_refinement_budget_exhausted_records_admissible_proposal() {
        let proposal = proposal(
            "rp-budget",
            PlanningStage::StoragePlan,
            RepairReason::PlacementProfileFallback,
            KnobDelta::AdvancePlacementProfile {
                to: PlacementProfile::Budgeted,
            },
        );
        let mut pipeline = ScriptedPipeline::with(
            PlanningStage::StoragePlan,
            vec![StageRunOutcome::ProposedRepair { proposal }],
        );
        let mut initial = loop_state(8, StageIterationCeilings::uniform(2));
        initial.accepted_iters_remaining = 0;

        let result = run_refinement_loop(initial.clone(), &mut pipeline).expect("loop runs");

        assert_eq!(
            result.terminal,
            TerminalState::AcceptedRefinementBudgetExhausted {
                stage: PlanningStage::StoragePlan
            }
        );
        assert_eq!(
            result.state.knobs.global.placement.profile,
            PlacementProfile::StrictOnePerBank
        );
        assert_eq!(result.state.history.proposals.len(), 1);
        assert!(matches!(
            &result.state.history.proposals[0].outcome,
            ProposalOutcome::Rejected {
                reason: DeltaRejection::AcceptedRefinementBudgetExhausted {
                    max_refinement_iters
                }
            } if *max_refinement_iters == initial.repair_policy.max_refinement_iters
        ));
        assert_eq!(
            repair_report(&initial, &result, report_inputs()).outcome,
            ReportOutcome::Failed
        );
    }

    #[test]
    fn accepted_force_kernel_residency_records_repair_provenance() {
        let proposal = proposal(
            "rp-kernel",
            PlanningStage::RomWindowPlan,
            RepairReason::KernelResidencyImpossible,
            KnobDelta::ForceKernelResidency {
                selector: KernelSelector::KernelSpec {
                    id: KernelSpecId(4),
                },
                to: KernelResidency::WramOverlay,
            },
        );
        let mut pipeline = ScriptedPipeline::with(
            PlanningStage::RomWindowPlan,
            vec![StageRunOutcome::ProposedRepair { proposal }],
        );
        let result = run_refinement_loop(
            loop_state(16, StageIterationCeilings::uniform(4)),
            &mut pipeline,
        )
        .expect("loop runs");

        assert!(result.state.knobs.provenance.iter().any(|entry| matches!(
            entry.chain.first().map(|frame| &frame.source),
            Some(PolicySource::RepairProposal { .. })
        )));
    }

    #[test]
    fn repair_report_round_trip_records_initial_final_knobs_and_iterations() {
        let proposal = proposal(
            "rp-placement",
            PlanningStage::StoragePlan,
            RepairReason::PlacementProfileFallback,
            KnobDelta::AdvancePlacementProfile {
                to: PlacementProfile::Budgeted,
            },
        );
        let mut pipeline = ScriptedPipeline::with(
            PlanningStage::StoragePlan,
            vec![StageRunOutcome::ProposedRepair { proposal }],
        );
        let initial = loop_state(16, StageIterationCeilings::uniform(4));
        let result = run_refinement_loop(initial.clone(), &mut pipeline).expect("loop runs");
        let report = repair_report(&initial, &result, report_inputs());

        assert_eq!(REPAIR_REPORT_FILE_NAME, "repair_report.json");
        assert_eq!(report.schema.as_str(), "repair_report.v1");
        assert_eq!(report.outcome, ReportOutcome::Passed);
        assert_eq!(
            report.body.report_inputs.policy_resolution_self_hash,
            Hash256::from_bytes([1; 32])
        );
        assert_eq!(
            report.body.report_inputs.artifact_validation_self_hash,
            Hash256::from_bytes([2; 32])
        );
        assert_eq!(
            report.body.initial_knobs.values.placement.profile,
            PlacementProfile::StrictOnePerBank
        );
        assert_eq!(
            report.body.final_knobs.values.placement.profile,
            PlacementProfile::Budgeted
        );
        assert_ne!(
            report.body.initial_knobs.snapshot_hash,
            report.body.final_knobs.snapshot_hash
        );
        assert_eq!(report.body.proposals[0].iter_emitted, 1);
        assert!(report.body.proposals[0].knob_delta.is_some());
        assert_eq!(
            report.body.stage_iteration_counts[0].stage,
            PlanningStage::RangePlan
        );
        assert!(report.body.global_iters_used > 0);

        match &report.body.proposals[0].outcome {
            ProposalOutcome::Accepted { knobs_delta, .. } => {
                assert_eq!(
                    knobs_delta.before.global.placement.profile,
                    PlacementProfile::StrictOnePerBank
                );
                assert_eq!(
                    knobs_delta.after.global.placement.profile,
                    PlacementProfile::Budgeted
                );
                assert_eq!(
                    knobs_delta.per_knob[0].knob,
                    CompileKnobId::PlacementProfile
                );
                assert_eq!(
                    knobs_delta.per_knob[0].operation,
                    ConstraintOperation::AppliedRepairProposal {
                        id: RepairProposalId("rp-placement".to_owned())
                    }
                );
            }
            ProposalOutcome::Rejected { .. } => panic!("proposal should be accepted"),
        }

        round_trip_self_hash(&report).expect("repair report self-hash round-trips");
    }

    #[test]
    fn repair_report_includes_rejected_proposals() {
        let value = ValueSelector::Value { id: ValueId(7) };
        let proposal = proposal(
            "rp-recompute",
            PlanningStage::StoragePlan,
            RepairReason::AccumulatorOverflow,
            KnobDelta::ForceRecompute {
                values: BTreeSet::from([value.clone()]),
            },
        );
        let mut pipeline = ScriptedPipeline::with(
            PlanningStage::StoragePlan,
            vec![StageRunOutcome::ProposedRepair { proposal }],
        );
        let mut initial = loop_state(8, StageIterationCeilings::uniform(2));
        initial.recompute_purity.effectful_values.insert(value);

        let result = run_refinement_loop(initial.clone(), &mut pipeline).expect("loop runs");
        let report = repair_report(&initial, &result, report_inputs());
        let bytes = repair_report_json(&initial, &result, report_inputs());
        let value: serde_json::Value =
            serde_json::from_slice(&bytes).expect("canonical report parses");

        assert_eq!(report.outcome, ReportOutcome::Failed);
        assert!(matches!(
            report.body.proposals[0].outcome,
            ProposalOutcome::Rejected {
                reason: DeltaRejection::EffectfulRecompute { .. }
            }
        ));
        assert_eq!(
            value["proposals"][0]["outcome"]["kind"],
            serde_json::json!("Rejected")
        );
        assert_eq!(value["proposals"][0]["iter_emitted"], serde_json::json!(1));
        assert_eq!(
            value["proposals"][0]["estimated_cost_delta"]["cycles"],
            serde_json::Value::Null
        );
        assert_eq!(
            value["termination"]["kind"],
            serde_json::json!("StagedFailureUnrepairable")
        );
        round_trip_self_hash(&report).expect("rejected repair report self-hash round-trips");
    }

    fn proposal(
        id: &str,
        source_stage: PlanningStage,
        reason: RepairReason,
        change: KnobDelta,
    ) -> RepairProposal {
        RepairProposal {
            id: RepairProposalId(id.to_owned()),
            source_stage,
            reason,
            delta: ConstraintDelta {
                changes: vec![change],
            },
            estimated_cost: EstimatedCostDelta::default(),
        }
    }

    fn report_inputs() -> RepairReportInputsSection {
        RepairReportInputsSection {
            policy_resolution_self_hash: Hash256::from_bytes([1; 32]),
            artifact_validation_self_hash: Hash256::from_bytes([2; 32]),
            static_budget_self_hash: Some(Hash256::from_bytes([3; 32])),
            schedule_cost_self_hash: Some(Hash256::from_bytes([4; 32])),
        }
    }

    fn loop_state(
        global_iters_remaining: u8,
        stage_iters_remaining: StageIterationCeilings,
    ) -> LoopState {
        let mut recompute_purity = RecomputePurityFacts::default();
        recompute_purity
            .pure_values
            .insert(ValueSelector::Value { id: ValueId(3) });
        LoopState {
            knobs: compile_knobs_fixture(),
            repair_policy: RepairPolicy::for_profile(RepairPolicyProfile::Default),
            observability: ObservabilityMode::Flexible,
            recompute_purity,
            accepted_iters_remaining: RepairPolicy::for_profile(RepairPolicyProfile::Default)
                .max_refinement_iters,
            global_iters_remaining,
            stage_iters_remaining,
            history: RepairHistory::default(),
        }
    }

    fn compile_knobs_fixture() -> CompileKnobs {
        CompileKnobs {
            global: CompileKnobValues {
                placement: PlacementKnob {
                    profile: PlacementProfile::StrictOnePerBank,
                },
                observation: ObservationKnob {
                    observability: ObservabilityMode::Flexible,
                    trace_demotion: TraceDemotionLevel::None,
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
                    spill_policy: SramSpillPolicy::NoSpill,
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
                    pressure_thresholds: ResourcePressureThresholds::default(),
                    stage_iteration_ceilings: StageIterationLimits::uniform(4),
                },
            },
            bounds: CompileKnobBounds {
                placement: PlacementKnobBounds {
                    max_profile: PlacementProfile::PackedExperts,
                },
                observation: ObservationKnobBounds {
                    max_trace_demotion: TraceDemotionLevel::RequiredOnly,
                    max_probe_level: ProbeCollectionLevel::Verbose,
                },
                range: RangeKnobBounds {
                    max_reduction_ceiling: ReductionPlanCeiling::Adaptive,
                },
                storage: StorageKnobBounds {
                    max_materialization: StorageMaterialization::SpillColdValues,
                },
                sram: SramKnobBounds {
                    max_page_aggression: SramPageAggression::MinimizeResident,
                    max_spill_policy: SramSpillPolicy::SpillEager,
                },
                rom_window: RomWindowKnobBounds {
                    max_kernel_residency_bias: RomKernelResidencyBias::PreferWramOverlay,
                    max_kernel_duplication_bias: RomKernelDuplicationBias::DuplicateAllFit,
                },
                overlay: OverlayKnobBounds {
                    max_promotion: OverlayPromotion::EligibleKernels,
                },
                schedule: ScheduleKnobBounds {
                    max_tile_search: ScheduleTileSearch::ProfileGuided,
                    max_slice_coarsening: ScheduleSliceCoarsening::Coarse,
                    max_resource_pressure: ScheduleResourcePressure::FitFirst,
                    max_pressure_thresholds: gbf_policy::canonical_default_bounds_fixture()
                        .schedule
                        .max_pressure_thresholds,
                    max_stage_iteration_ceilings: StageIterationLimits::uniform(u8::MAX),
                },
            },
            locks: KnobLockSet::default(),
            overrides: CompileKnobOverrides::default(),
            provenance: Vec::new(),
        }
    }
}
