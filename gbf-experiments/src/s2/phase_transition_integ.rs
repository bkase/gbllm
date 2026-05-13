//! S2 D8 phase-transition integration wrapper.
//!
//! This wrapper executes the miniature five-phase fixture against the public
//! `gbf-train` phase scheduler, then emits the established
//! `s2_phase_transition_integration.v1` schema.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::path::Path;

use gbf_train::logging::{TestEventCollector, TestEventKind, TrainingLogEmitter};
use gbf_train::phase::{
    PhaseScheduleError, TRAIN_PHASE_COUNT, TrainPhaseKind, TrainPhaseSpec, TrainingPhaseSchedule,
};
use gbf_train::scheduler::{
    PhaseControlledModel, PhaseControls, PhaseSchedulerError, PhaseStepOutcome,
    TrainingPhaseScheduler,
};

use super::schema::{
    HardnessTriple, PhaseBoundaryHardnessProjection, PhaseTransitionIntegReport,
    S2ReportWriteError, phase_transition_expected_boundary_projections,
    write_phase_transition_integ_report,
};
use crate::s1::schema::S1SchemaError;

/// Fixture identifier written into the phase-transition integration report.
pub const FIXTURE_ID: &str = "tiny_model_T10.1";
/// Inclusive/exclusive five-phase fixture boundaries.
pub const FIXTURE_PHASE_BOUNDARIES: [u64; 6] = [0, 10, 20, 30, 40, 50];
/// Steps at which the fixture should emit phase transitions.
pub const TRANSITION_STEPS: [u64; 4] = [10, 20, 30, 40];
/// Step at which the fixture emits the teacher-freeze event.
pub const TEACHER_FREEZE_STEP: u64 = 10;

/// Execute the S2 D8 fixture and return the canonical report.
pub fn run_phase_transition_integration()
-> Result<PhaseTransitionIntegReport, PhaseTransitionIntegError> {
    tracing::info!(
        event_name = "phase_transition_integ_start",
        fixture_id = FIXTURE_ID,
        phase_intervals = "[[0,10),[10,20),[20,30),[30,40),[40,50)]",
        "s2 phase-transition integration start"
    );

    let observed = run_clean_fixture()?;
    let skip_phase_test_passed = run_skip_phase_fixture()?;
    log_subcheck("skip_phase", skip_phase_test_passed);

    let overlap_phase_error_raised = overlap_fixture_raises_expected_error();
    log_subcheck("overlap", overlap_phase_error_raised);

    let empty_phase_error_raised = empty_fixture_raises_error();
    log_subcheck("empty", empty_phase_error_raised);

    if observed.transition_event_count != TRANSITION_STEPS.len() as u32 {
        return Err(PhaseTransitionIntegError::Invariant {
            name: "PT-1",
            detail: format!(
                "expected {} transition events, got {}",
                TRANSITION_STEPS.len(),
                observed.transition_event_count
            ),
        });
    }
    if observed.teacher_freeze_event_count != 1 {
        return Err(PhaseTransitionIntegError::Invariant {
            name: "PT-2",
            detail: format!(
                "expected one teacher_freeze event, got {}",
                observed.teacher_freeze_event_count
            ),
        });
    }
    if !skip_phase_test_passed || !overlap_phase_error_raised || !empty_phase_error_raised {
        return Err(PhaseTransitionIntegError::Invariant {
            name: "PT-subcheck",
            detail: "one or more phase-transition subchecks failed".to_owned(),
        });
    }

    Ok(PhaseTransitionIntegReport::new(
        observed.transition_event_count,
        observed.teacher_freeze_event_count,
        observed.hardness_at_boundary,
        skip_phase_test_passed,
        overlap_phase_error_raised,
        empty_phase_error_raised,
    )?)
}

/// Execute the S2 D8 fixture and write the canonical report to disk.
pub fn write_phase_transition_integration_report(
    path: impl AsRef<Path>,
) -> Result<PhaseTransitionIntegReport, PhaseTransitionIntegWriteError> {
    let report = run_phase_transition_integration()?;
    write_phase_transition_integ_report(path, &report)?;
    Ok(report)
}

fn run_clean_fixture() -> Result<ObservedFixture, PhaseTransitionIntegError> {
    assert_expected_boundary_projection_inputs_match_live_schedule()?;

    let mut scheduler = TrainingPhaseScheduler::new(TrainingPhaseSchedule::default_five_phase(10)?);
    let mut model = RecordingPhaseModel::default();
    let collector = TestEventCollector::new();
    let emitter = TrainingLogEmitter::with_test_collector(collector.clone());
    let mut hardness_at_boundary = BTreeMap::new();

    for step in 0..FIXTURE_PHASE_BOUNDARIES[5] {
        if step == TEACHER_FREEZE_STEP {
            // D8 owns scheduler/logging integration, not the production train
            // loop. The fixture hand-fires the freeze event immediately before
            // applying the phase-A/B scheduler step so logs pin freeze-before-
            // transition ordering at the boundary.
            emitter.teacher_freeze(&gbf_train::logging::TeacherFreezeEvent {
                step,
                teacher_checkpoint_id: "teacher-phase-a-end".to_owned(),
                source_weight_fingerprint: "01020304".to_owned(),
                frozen_weight_fingerprint: "01020304".to_owned(),
                weights_match: true,
                duration_ms: 0,
            })?;
        }

        let outcome = scheduler.apply_step(step, &mut model, &emitter)?;
        if let PhaseStepOutcome::Transitioned { transition, .. } = outcome {
            let to = transition.to();
            let hardness =
                observed_hardness_for_transition(step, transition.from(), transition.to());
            hardness_at_boundary.insert(step.to_string(), hardness);
            tracing::debug!(
                event_name = "phase_transition_fired",
                fixture_step = step,
                from = %fixture_phase_name(transition.from().phase().kind()),
                to = %fixture_phase_name(to.phase().kind()),
                hardness = ?hardness,
                "s2 phase-transition fixture transition fired"
            );
        }
    }

    let events = collector.events();
    Ok(ObservedFixture {
        transition_event_count: events
            .iter()
            .filter(|event| event.kind() == TestEventKind::PhaseTransition)
            .count() as u32,
        teacher_freeze_event_count: events
            .iter()
            .filter(|event| event.kind() == TestEventKind::TeacherFreeze)
            .count() as u32,
        hardness_at_boundary,
    })
}

fn run_skip_phase_fixture() -> Result<bool, PhaseTransitionIntegError> {
    let mut scheduler = TrainingPhaseScheduler::new(TrainingPhaseSchedule::default_five_phase(10)?);
    let mut model = RecordingPhaseModel::default();
    let emitter = TrainingLogEmitter::new();

    scheduler.apply_step(0, &mut model, &emitter)?;
    let missed_boundary = matches!(
        scheduler.apply_step(15, &mut model, &emitter),
        Err(PhaseSchedulerError::MissedPhaseBoundary {
            from_phase: TrainPhaseKind::DenseTeacherWarmup,
            to_phase: TrainPhaseKind::RouterWarmup,
            boundary_step: 10,
            step: 15,
        })
    );
    let skipped_phase = matches!(
        scheduler.apply_step(20, &mut model, &emitter),
        Err(PhaseSchedulerError::SkippedPhaseTransition {
            from_phase: TrainPhaseKind::DenseTeacherWarmup,
            to_phase: TrainPhaseKind::ExpertTernaryQat,
            previous_step: 0,
            step: 20,
        })
    );
    let no_extra_controls = model.applied.len() == 1
        && model.applied[0].phase_kind == TrainPhaseKind::DenseTeacherWarmup;

    Ok(missed_boundary && skipped_phase && no_extra_controls)
}

fn overlap_fixture_raises_expected_error() -> bool {
    matches!(
        TrainingPhaseSchedule::new(literal_phase_specs_with_ranges([
            (0, 10),
            (10, 20),
            (19, 30),
            (30, 40),
            (40, 50),
        ])),
        Err(PhaseScheduleError::NonContiguous {
            previous_kind: TrainPhaseKind::RouterWarmup,
            next_kind: TrainPhaseKind::ExpertTernaryQat,
            expected_start: 20,
            actual_start: 19,
        })
    )
}

fn empty_fixture_raises_error() -> bool {
    matches!(
        TrainingPhaseSchedule::new(Vec::new()),
        Err(PhaseScheduleError::WrongPhaseCount {
            expected: TRAIN_PHASE_COUNT,
            actual: 0,
        })
    )
}

fn observed_hardness_for_transition(
    step: u64,
    from: PhaseControls,
    to: PhaseControls,
) -> HardnessTriple {
    PhaseBoundaryHardnessProjection::new(
        step,
        hardness_triple_from_controls(from),
        hardness_triple_from_controls(to),
    )
    .projected()
}

fn hardness_triple_from_controls(controls: PhaseControls) -> HardnessTriple {
    HardnessTriple::new(
        controls.expert_qat(),
        controls.activation_qat(),
        controls.norm_qat(),
    )
}

fn assert_expected_boundary_projection_inputs_match_live_schedule()
-> Result<(), PhaseTransitionIntegError> {
    let expected = phase_transition_expected_boundary_projections();
    let observed = live_schedule_boundary_projections()?;
    if observed.len() != expected.len() {
        return Err(PhaseTransitionIntegError::Invariant {
            name: "PT-3",
            detail: format!(
                "expected {} boundary projection input rows, got {}",
                expected.len(),
                observed.len()
            ),
        });
    }
    for (observed, expected) in observed.into_iter().zip(expected) {
        if observed != expected {
            return Err(PhaseTransitionIntegError::Invariant {
                name: "PT-3",
                detail: format!(
                    "projection input mismatch at boundary {}: expected {:?}, observed {:?}",
                    expected.step, expected, observed
                ),
            });
        }
    }
    Ok(())
}

fn live_schedule_boundary_projections()
-> Result<Vec<PhaseBoundaryHardnessProjection>, PhaseTransitionIntegError> {
    let phases = TrainingPhaseSchedule::default_five_phase(10)?.into_phases();
    Ok(phases
        .windows(2)
        .map(|window| {
            let from = window[0];
            let to = window[1];
            PhaseBoundaryHardnessProjection::new(
                from.end_step(),
                hardness_triple_from_phase(from),
                hardness_triple_from_phase(to),
            )
        })
        .collect())
}

fn hardness_triple_from_phase(phase: TrainPhaseSpec) -> HardnessTriple {
    HardnessTriple::new(phase.expert_qat(), phase.activation_qat(), phase.norm_qat())
}

fn log_subcheck(name: &'static str, passed: bool) {
    tracing::debug!(
        event_name = "phase_transition_subcheck",
        name = name,
        passed = passed,
        "s2 phase-transition integration subcheck"
    );
}

fn fixture_phase_name(kind: TrainPhaseKind) -> &'static str {
    match kind {
        TrainPhaseKind::DenseTeacherWarmup => "phase-a",
        TrainPhaseKind::RouterWarmup => "phase-b",
        TrainPhaseKind::ExpertTernaryQat => "phase-c",
        TrainPhaseKind::FullNumericQat => "phase-d",
        TrainPhaseKind::HardenAndSelect => "phase-e",
    }
}

fn literal_phase_specs_with_ranges(ranges: [(u64, u64); TRAIN_PHASE_COUNT]) -> Vec<TrainPhaseSpec> {
    TrainingPhaseSchedule::default_five_phase(10)
        .expect("canonical fixture schedule")
        .into_phases()
        .into_iter()
        .zip(ranges)
        .map(|(phase, (start, end))| {
            TrainPhaseSpec::new(
                phase.kind(),
                start,
                end,
                phase.expert_qat(),
                phase.activation_qat(),
                phase.norm_qat(),
                phase.router_mode(),
            )
            .expect("test fixture phase range is non-empty")
        })
        .collect()
}

#[derive(Debug)]
struct ObservedFixture {
    transition_event_count: u32,
    teacher_freeze_event_count: u32,
    hardness_at_boundary: BTreeMap<String, HardnessTriple>,
}

#[derive(Debug, Default)]
struct RecordingPhaseModel {
    applied: Vec<AppliedControls>,
}

impl PhaseControlledModel for RecordingPhaseModel {
    fn apply_phase_controls(&mut self, controls: PhaseControls) {
        self.applied.push(AppliedControls {
            phase_kind: controls.phase().kind(),
        });
    }
}

#[derive(Debug)]
struct AppliedControls {
    phase_kind: TrainPhaseKind,
}

/// Errors raised by the S2 phase-transition integration wrapper.
#[derive(Debug)]
pub enum PhaseTransitionIntegError {
    /// Phase schedule construction failed.
    Schedule(PhaseScheduleError),
    /// Runtime scheduler stepping failed.
    Scheduler(gbf_train::scheduler::PhaseSchedulerError),
    /// Structured logging capture failed.
    Logging(gbf_train::logging::LoggingEventError),
    /// Canonical S2 schema validation failed.
    Schema(S1SchemaError),
    /// One of the PT invariants failed.
    Invariant {
        /// Invariant name.
        name: &'static str,
        /// Human-readable invariant diagnostic.
        detail: String,
    },
}

impl fmt::Display for PhaseTransitionIntegError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Schedule(error) => write!(f, "{error}"),
            Self::Scheduler(error) => write!(f, "{error}"),
            Self::Logging(error) => write!(f, "{error}"),
            Self::Schema(error) => write!(f, "{error}"),
            Self::Invariant { name, detail } => {
                write!(f, "phase-transition invariant {name} failed: {detail}")
            }
        }
    }
}

impl Error for PhaseTransitionIntegError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Schedule(error) => Some(error),
            Self::Scheduler(error) => Some(error),
            Self::Logging(error) => Some(error),
            Self::Schema(error) => Some(error),
            Self::Invariant { .. } => None,
        }
    }
}

impl From<PhaseScheduleError> for PhaseTransitionIntegError {
    fn from(error: PhaseScheduleError) -> Self {
        Self::Schedule(error)
    }
}

impl From<gbf_train::scheduler::PhaseSchedulerError> for PhaseTransitionIntegError {
    fn from(error: gbf_train::scheduler::PhaseSchedulerError) -> Self {
        Self::Scheduler(error)
    }
}

impl From<gbf_train::logging::LoggingEventError> for PhaseTransitionIntegError {
    fn from(error: gbf_train::logging::LoggingEventError) -> Self {
        Self::Logging(error)
    }
}

impl From<S1SchemaError> for PhaseTransitionIntegError {
    fn from(error: S1SchemaError) -> Self {
        Self::Schema(error)
    }
}

/// Errors raised while writing the S2 phase-transition integration report.
#[derive(Debug)]
pub enum PhaseTransitionIntegWriteError {
    /// Integration wrapper failed before writing.
    Integ(PhaseTransitionIntegError),
    /// Canonical report write failed.
    Write(S2ReportWriteError),
}

impl fmt::Display for PhaseTransitionIntegWriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Integ(error) => write!(f, "{error}"),
            Self::Write(error) => write!(f, "{error}"),
        }
    }
}

impl Error for PhaseTransitionIntegWriteError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Integ(error) => Some(error),
            Self::Write(error) => Some(error),
        }
    }
}

impl From<PhaseTransitionIntegError> for PhaseTransitionIntegWriteError {
    fn from(error: PhaseTransitionIntegError) -> Self {
        Self::Integ(error)
    }
}

impl From<S2ReportWriteError> for PhaseTransitionIntegWriteError {
    fn from(error: S2ReportWriteError) -> Self {
        Self::Write(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skip_phase_fixture_uses_live_scheduler_rejections() {
        assert!(run_skip_phase_fixture().expect("skip phase fixture"));
    }

    #[test]
    fn overlap_fixture_requires_noncontiguous_schedule_error() {
        assert!(overlap_fixture_raises_expected_error());
    }

    #[test]
    fn expected_boundary_projection_inputs_match_live_five_phase_schedule() {
        assert!(assert_expected_boundary_projection_inputs_match_live_schedule().is_ok());
    }
}
