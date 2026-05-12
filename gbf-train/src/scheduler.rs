//! Phase scheduler boundary for applying explicit training phase rows.
//!
//! Pure schedule lookup stays separate from live-run effects. The training
//! loop can reconstruct the active phase from `(schedule, step)` after a
//! restart, while this stateful wrapper prevents duplicate transition effects
//! during a live run and rejects missed phase boundaries.

use std::error::Error;
use std::fmt;

use gbf_model::qat::RouterTrainMode;

use crate::logging::{
    LoggingEventError, PhaseTransitionEvent, QatHardnessLevels, TrainingLogEmitter,
};
use crate::phase::{QuantHardness, TrainPhaseKind, TrainPhaseSpec, TrainingPhaseSchedule};

#[derive(Debug, Clone)]
pub struct TrainingPhaseScheduler {
    schedule: TrainingPhaseSchedule,
    applied_position: Option<PhasePosition>,
}

impl TrainingPhaseScheduler {
    #[must_use]
    pub fn new(schedule: TrainingPhaseSchedule) -> Self {
        Self {
            schedule,
            applied_position: None,
        }
    }

    #[must_use]
    pub fn schedule(&self) -> &TrainingPhaseSchedule {
        &self.schedule
    }

    #[must_use]
    pub fn applied_phase(&self) -> Option<TrainPhaseSpec> {
        self.applied_position.map(PhasePosition::phase)
    }

    #[must_use]
    pub fn applied_position(&self) -> Option<PhasePosition> {
        self.applied_position
    }

    pub fn current_phase(&self, step: u64) -> Result<TrainPhaseSpec, PhaseSchedulerError> {
        self.phase_position(step).map(PhasePosition::phase)
    }

    pub fn phase_position(&self, step: u64) -> Result<PhasePosition, PhaseSchedulerError> {
        let (phase_index, phase) = self
            .schedule
            .phases()
            .iter()
            .copied()
            .enumerate()
            .find(|(_, phase)| step >= phase.start_step() && step < phase.end_step())
            .ok_or_else(|| PhaseSchedulerError::StepOutOfRange {
                step,
                final_step: self.final_step(),
            })?;

        Ok(PhasePosition {
            phase,
            phase_index,
            step,
            offset_steps: step - phase.start_step(),
            total_steps: phase.len_steps(),
        })
    }

    pub fn apply_step<M: PhaseControlledModel>(
        &mut self,
        step: u64,
        model: &mut M,
        log_emitter: &TrainingLogEmitter,
    ) -> Result<PhaseStepOutcome, PhaseSchedulerError> {
        self.apply_step_with_checkpoint(step, model, log_emitter, |_| false)
    }

    pub fn apply_step_with_checkpoint<M, F>(
        &mut self,
        step: u64,
        model: &mut M,
        log_emitter: &TrainingLogEmitter,
        save_checkpoint: F,
    ) -> Result<PhaseStepOutcome, PhaseSchedulerError>
    where
        M: PhaseControlledModel,
        F: FnOnce(&PhaseTransitionPlan) -> bool,
    {
        let position = self.phase_position(step)?;
        let controls = PhaseControls::from_position(position);

        let Some(previous_position) = self.applied_position else {
            model.apply_phase_controls(controls);
            self.applied_position = Some(position);
            return Ok(PhaseStepOutcome::EnteredInitial { controls });
        };

        if step < previous_position.step() {
            return Err(PhaseSchedulerError::NonMonotonicStep {
                previous_step: previous_position.step(),
                step,
            });
        }

        let previous_controls = PhaseControls::from_position(previous_position);
        if step == previous_position.step() {
            return Ok(PhaseStepOutcome::Unchanged {
                controls: previous_controls,
            });
        }

        if previous_position.phase().kind() == position.phase().kind() {
            model.apply_phase_controls(controls);
            self.applied_position = Some(position);
            return Ok(PhaseStepOutcome::Advanced {
                previous_step: previous_position.step(),
                controls,
            });
        }

        self.validate_live_transition(previous_position, position)?;

        let transition = PhaseTransitionPlan {
            step,
            from: previous_controls,
            to: controls,
        };
        let checkpoint_saved = save_checkpoint(&transition);

        log_emitter.phase_transition(&PhaseTransitionEvent {
            from_phase: transition.from().phase().kind().to_string(),
            to_phase: transition.to().phase().kind().to_string(),
            step,
            before_hardness: hardness_levels_for_controls(transition.from())?,
            after_hardness: hardness_levels_for_controls(transition.to())?,
            checkpoint_saved,
        })?;

        model.apply_phase_controls(controls);
        self.applied_position = Some(position);
        Ok(PhaseStepOutcome::Transitioned {
            transition,
            checkpoint_saved,
        })
    }

    fn validate_live_transition(
        &self,
        from: PhasePosition,
        to: PhasePosition,
    ) -> Result<(), PhaseSchedulerError> {
        if to.step() != to.phase().start_step() {
            return Err(PhaseSchedulerError::MissedPhaseBoundary {
                from_phase: from.phase().kind(),
                to_phase: to.phase().kind(),
                boundary_step: to.phase().start_step(),
                step: to.step(),
            });
        }

        if to.phase_index() != from.phase_index() + 1 {
            return Err(PhaseSchedulerError::SkippedPhaseTransition {
                from_phase: from.phase().kind(),
                to_phase: to.phase().kind(),
                previous_step: from.step(),
                step: to.step(),
            });
        }

        Ok(())
    }

    fn final_step(&self) -> u64 {
        self.schedule
            .phases()
            .last()
            .expect("validated training phase schedule contains phases")
            .end_step()
    }
}

pub trait PhaseControlledModel {
    fn apply_phase_controls(&mut self, controls: PhaseControls);
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PhaseStepOutcome {
    EnteredInitial {
        controls: PhaseControls,
    },
    Advanced {
        previous_step: u64,
        controls: PhaseControls,
    },
    Unchanged {
        controls: PhaseControls,
    },
    Transitioned {
        transition: PhaseTransitionPlan,
        checkpoint_saved: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PhaseTransitionPlan {
    step: u64,
    from: PhaseControls,
    to: PhaseControls,
}

impl PhaseTransitionPlan {
    #[must_use]
    pub const fn step(self) -> u64 {
        self.step
    }

    #[must_use]
    pub const fn from(self) -> PhaseControls {
        self.from
    }

    #[must_use]
    pub const fn to(self) -> PhaseControls {
        self.to
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PhaseControls {
    position: PhasePosition,
    soft_progress: f32,
    threshold_schedule_progress: UnitIntervalProgress,
}

impl PhaseControls {
    fn from_position(position: PhasePosition) -> Self {
        Self {
            position,
            soft_progress: position.progress_fraction(),
            threshold_schedule_progress: UnitIntervalProgress::from_phase_position(position),
        }
    }

    #[must_use]
    pub const fn position(self) -> PhasePosition {
        self.position
    }

    #[must_use]
    pub const fn phase(self) -> TrainPhaseSpec {
        self.position.phase()
    }

    #[must_use]
    pub const fn step(self) -> u64 {
        self.position.step()
    }

    #[must_use]
    pub const fn expert_qat(self) -> QuantHardness {
        self.phase().expert_qat()
    }

    #[must_use]
    pub const fn activation_qat(self) -> QuantHardness {
        self.phase().activation_qat()
    }

    #[must_use]
    pub const fn norm_qat(self) -> QuantHardness {
        self.phase().norm_qat()
    }

    #[must_use]
    pub const fn router_mode(self) -> RouterTrainMode {
        self.phase().router_mode()
    }

    #[must_use]
    pub const fn soft_progress(self) -> f32 {
        self.soft_progress
    }

    #[must_use]
    pub const fn threshold_schedule_progress(self) -> UnitIntervalProgress {
        self.threshold_schedule_progress
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UnitIntervalProgress {
    value: f32,
}

impl UnitIntervalProgress {
    pub fn new(value: f32) -> Result<Self, UnitIntervalProgressError> {
        if value.is_finite() && (0.0..=1.0).contains(&value) {
            Ok(Self { value })
        } else {
            Err(UnitIntervalProgressError { value })
        }
    }

    #[must_use]
    pub const fn start() -> Self {
        Self { value: 0.0 }
    }

    #[must_use]
    pub const fn complete() -> Self {
        Self { value: 1.0 }
    }

    #[must_use]
    pub const fn value(self) -> f32 {
        self.value
    }

    fn from_phase_position(position: PhasePosition) -> Self {
        match position.phase().kind() {
            TrainPhaseKind::DenseTeacherWarmup | TrainPhaseKind::RouterWarmup => Self::start(),
            TrainPhaseKind::ExpertTernaryQat => Self::from_bounded_phase_fraction(position),
            TrainPhaseKind::FullNumericQat | TrainPhaseKind::HardenAndSelect => Self::complete(),
        }
    }

    fn from_bounded_phase_fraction(position: PhasePosition) -> Self {
        if position.total_steps() <= 1 {
            return Self::complete();
        }
        let denominator = position.total_steps() - 1;
        debug_assert!(position.offset_steps() <= denominator);
        let value = (position.offset_steps() as f64 / denominator as f64) as f32;
        debug_assert!(value.is_finite() && (0.0..=1.0).contains(&value));
        Self { value }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UnitIntervalProgressError {
    value: f32,
}

impl UnitIntervalProgressError {
    #[must_use]
    pub const fn value(self) -> f32 {
        self.value
    }
}

impl fmt::Display for UnitIntervalProgressError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "unit interval progress must be finite and in [0, 1], got {}",
            self.value
        )
    }
}

impl Error for UnitIntervalProgressError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhasePosition {
    phase: TrainPhaseSpec,
    phase_index: usize,
    step: u64,
    offset_steps: u64,
    total_steps: u64,
}

impl PhasePosition {
    #[must_use]
    pub const fn phase(self) -> TrainPhaseSpec {
        self.phase
    }

    #[must_use]
    pub const fn phase_index(self) -> usize {
        self.phase_index
    }

    #[must_use]
    pub const fn step(self) -> u64 {
        self.step
    }

    #[must_use]
    pub const fn offset_steps(self) -> u64 {
        self.offset_steps
    }

    #[must_use]
    pub const fn total_steps(self) -> u64 {
        self.total_steps
    }

    #[must_use]
    pub fn progress_fraction(self) -> f32 {
        if self.total_steps <= 1 {
            1.0
        } else {
            self.offset_steps as f32 / (self.total_steps - 1) as f32
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum PhaseSchedulerError {
    StepOutOfRange {
        step: u64,
        final_step: u64,
    },
    NonMonotonicStep {
        previous_step: u64,
        step: u64,
    },
    MissedPhaseBoundary {
        from_phase: TrainPhaseKind,
        to_phase: TrainPhaseKind,
        boundary_step: u64,
        step: u64,
    },
    SkippedPhaseTransition {
        from_phase: TrainPhaseKind,
        to_phase: TrainPhaseKind,
        previous_step: u64,
        step: u64,
    },
    Logging(LoggingEventError),
}

impl fmt::Display for PhaseSchedulerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StepOutOfRange { step, final_step } => write!(
                f,
                "training step {step} is outside the phase schedule ending at step {final_step}"
            ),
            Self::NonMonotonicStep {
                previous_step,
                step,
            } => write!(
                f,
                "training step {step} moved backward after step {previous_step}"
            ),
            Self::MissedPhaseBoundary {
                from_phase,
                to_phase,
                boundary_step,
                step,
            } => write!(
                f,
                "training transition from {from_phase} to {to_phase} must be applied at boundary step {boundary_step}, got step {step}"
            ),
            Self::SkippedPhaseTransition {
                from_phase,
                to_phase,
                previous_step,
                step,
            } => write!(
                f,
                "training transition from {from_phase} to {to_phase} skipped at least one phase between steps {previous_step} and {step}"
            ),
            Self::Logging(error) => write!(f, "failed to log phase transition: {error}"),
        }
    }
}

impl Error for PhaseSchedulerError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::StepOutOfRange { .. }
            | Self::NonMonotonicStep { .. }
            | Self::MissedPhaseBoundary { .. }
            | Self::SkippedPhaseTransition { .. } => None,
            Self::Logging(error) => Some(error),
        }
    }
}

impl From<LoggingEventError> for PhaseSchedulerError {
    fn from(error: LoggingEventError) -> Self {
        Self::Logging(error)
    }
}

pub fn hardness_levels_for_controls(
    controls: PhaseControls,
) -> Result<QatHardnessLevels, LoggingEventError> {
    let expert = quant_hardness_value(controls.expert_qat(), controls.soft_progress());
    QatHardnessLevels::new(
        expert,
        quant_hardness_value(controls.activation_qat(), controls.soft_progress()),
        quant_hardness_value(controls.norm_qat(), controls.soft_progress()),
        router_hardness_value(controls.router_mode()),
        expert,
    )
}

fn quant_hardness_value(hardness: QuantHardness, soft_progress: f32) -> f32 {
    match hardness {
        QuantHardness::Off => 0.0,
        QuantHardness::Soft => soft_progress,
        QuantHardness::Hard => 1.0,
    }
}

fn router_hardness_value(router_mode: RouterTrainMode) -> f32 {
    match router_mode {
        RouterTrainMode::SoftTop1 => 0.5,
        RouterTrainMode::HardTop1 => 1.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logging::{TestEventCollector, TestEventKind, TestFieldValue};

    #[test]
    fn scheduler_current_phase_uses_half_open_boundaries() {
        let scheduler = scheduler();

        assert_eq!(
            scheduler.current_phase(0).unwrap().kind(),
            TrainPhaseKind::DenseTeacherWarmup
        );
        assert_eq!(
            scheduler.current_phase(9).unwrap().kind(),
            TrainPhaseKind::DenseTeacherWarmup
        );
        assert_eq!(
            scheduler.current_phase(10).unwrap().kind(),
            TrainPhaseKind::RouterWarmup
        );
        assert_eq!(
            scheduler.current_phase(49).unwrap().kind(),
            TrainPhaseKind::HardenAndSelect
        );
        assert_eq!(
            scheduler.current_phase(50).unwrap_err(),
            PhaseSchedulerError::StepOutOfRange {
                step: 50,
                final_step: 50,
            }
        );
    }

    #[test]
    fn scheduler_phase_position_exposes_progress_for_soft_annealing() {
        let scheduler = scheduler();

        let start = scheduler.phase_position(20).unwrap();
        assert_eq!(start.phase().kind(), TrainPhaseKind::ExpertTernaryQat);
        assert_eq!(start.offset_steps(), 0);
        assert_eq!(start.total_steps(), 10);
        assert_eq!(start.progress_fraction(), 0.0);

        let middle = scheduler.phase_position(24).unwrap();
        assert_eq!(middle.offset_steps(), 4);
        assert!(middle.progress_fraction() > 0.44);
        assert!(middle.progress_fraction() < 0.45);

        let end = scheduler.phase_position(29).unwrap();
        assert_eq!(end.progress_fraction(), 1.0);
    }

    #[test]
    fn scheduler_threshold_schedule_progress_stays_complete_after_expert_qat() {
        let scheduler = scheduler();

        let before = PhaseControls::from_position(scheduler.phase_position(10).unwrap());
        assert_eq!(before.phase().kind(), TrainPhaseKind::RouterWarmup);
        assert_eq!(before.threshold_schedule_progress().value(), 0.0);

        let start = PhaseControls::from_position(scheduler.phase_position(20).unwrap());
        assert_eq!(start.phase().kind(), TrainPhaseKind::ExpertTernaryQat);
        assert_eq!(start.threshold_schedule_progress().value(), 0.0);

        let middle = PhaseControls::from_position(scheduler.phase_position(24).unwrap());
        assert!(middle.threshold_schedule_progress().value() > 0.44);
        assert!(middle.threshold_schedule_progress().value() < 0.45);

        let end = PhaseControls::from_position(scheduler.phase_position(29).unwrap());
        assert_eq!(end.threshold_schedule_progress().value(), 1.0);

        let full_numeric = PhaseControls::from_position(scheduler.phase_position(30).unwrap());
        assert_eq!(full_numeric.phase().kind(), TrainPhaseKind::FullNumericQat);
        assert_eq!(full_numeric.threshold_schedule_progress().value(), 1.0);

        let harden = PhaseControls::from_position(scheduler.phase_position(40).unwrap());
        assert_eq!(harden.phase().kind(), TrainPhaseKind::HardenAndSelect);
        assert_eq!(harden.threshold_schedule_progress().value(), 1.0);
    }

    #[test]
    fn scheduler_phase_controls_threshold_progress_is_typed_unit_interval() {
        let scheduler = scheduler();

        for step in 0..50 {
            let controls = PhaseControls::from_position(scheduler.phase_position(step).unwrap());
            let progress = controls.threshold_schedule_progress();
            assert!(
                progress.value().is_finite(),
                "threshold progress at step {step} must be finite"
            );
            assert!(
                (0.0..=1.0).contains(&progress.value()),
                "threshold progress at step {step} must stay in [0, 1]"
            );
        }

        assert!(UnitIntervalProgress::new(f32::NAN).is_err());
        assert!(UnitIntervalProgress::new(-0.1).is_err());
        assert!(UnitIntervalProgress::new(1.1).is_err());
    }

    #[test]
    fn scheduler_apply_step_is_idempotent_with_repeated_step() {
        let mut scheduler = scheduler();
        let mut model = RecordingModel::default();
        let collector = TestEventCollector::new();
        let emitter = TrainingLogEmitter::with_test_collector(collector.clone());

        let first = scheduler.apply_step(0, &mut model, &emitter).unwrap();
        assert!(matches!(first, PhaseStepOutcome::EnteredInitial { .. }));
        assert_eq!(model.calls.len(), 1);
        assert!(collector.events().is_empty());

        let repeated = scheduler.apply_step(0, &mut model, &emitter).unwrap();
        assert!(matches!(repeated, PhaseStepOutcome::Unchanged { .. }));
        assert_eq!(model.calls.len(), 1);
        assert!(collector.events().is_empty());

        scheduler.apply_step(10, &mut model, &emitter).unwrap();
        scheduler.apply_step(10, &mut model, &emitter).unwrap();

        assert_eq!(model.calls.len(), 2);
        assert_eq!(collector.events().len(), 1);
    }

    #[test]
    fn scheduler_applies_soft_progress_on_in_phase_steps() {
        let mut scheduler = scheduler();
        let mut model = RecordingModel::default();
        let emitter = TrainingLogEmitter::new();

        scheduler.apply_step(20, &mut model, &emitter).unwrap();
        scheduler.apply_step(24, &mut model, &emitter).unwrap();
        scheduler.apply_step(24, &mut model, &emitter).unwrap();

        assert_eq!(model.calls.len(), 2);
        assert_eq!(model.calls[0].phase_kind, TrainPhaseKind::ExpertTernaryQat);
        assert_eq!(model.calls[0].soft_progress, 0.0);
        assert_eq!(model.calls[1].activation_qat, QuantHardness::Soft);
        assert_eq!(model.calls[1].norm_qat, QuantHardness::Soft);
        assert!(model.calls[1].soft_progress > 0.44);
        assert!(model.calls[1].soft_progress < 0.45);
    }

    #[test]
    fn scheduler_transition_applies_model_logs_event_and_uses_checkpoint_callback() {
        let mut scheduler = scheduler();
        let mut model = RecordingModel::default();
        let collector = TestEventCollector::new();
        let emitter = TrainingLogEmitter::with_test_collector(collector.clone());
        let mut checkpoint_calls = 0;

        scheduler.apply_step(0, &mut model, &emitter).unwrap();
        scheduler.apply_step(10, &mut model, &emitter).unwrap();
        collector.clear();

        let outcome = scheduler
            .apply_step_with_checkpoint(20, &mut model, &emitter, |transition| {
                checkpoint_calls += 1;
                assert_eq!(
                    transition.from().phase().kind(),
                    TrainPhaseKind::RouterWarmup
                );
                assert_eq!(
                    transition.to().phase().kind(),
                    TrainPhaseKind::ExpertTernaryQat
                );
                true
            })
            .unwrap();

        assert!(matches!(
            outcome,
            PhaseStepOutcome::Transitioned {
                transition,
                checkpoint_saved: true,
            } if transition.from().phase().kind() == TrainPhaseKind::RouterWarmup
                && transition.to().phase().kind() == TrainPhaseKind::ExpertTernaryQat
        ));
        assert_eq!(checkpoint_calls, 1);
        assert_eq!(
            model.calls.last().copied(),
            Some(AppliedControls {
                phase_kind: TrainPhaseKind::ExpertTernaryQat,
                expert_qat: QuantHardness::Hard,
                activation_qat: QuantHardness::Soft,
                norm_qat: QuantHardness::Soft,
                router_mode: RouterTrainMode::SoftTop1,
                soft_progress: 0.0,
            })
        );

        let events = collector.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind(), TestEventKind::PhaseTransition);
        assert_eq!(
            events[0].field("from_phase"),
            Some(&TestFieldValue::String("router_warmup".to_owned()))
        );
        assert_eq!(
            events[0].field("to_phase"),
            Some(&TestFieldValue::String("expert_ternary_qat".to_owned()))
        );
        assert_eq!(events[0].field("step"), Some(&TestFieldValue::U64(20)));
        assert_eq!(
            events[0].field("before_ternary_hardness"),
            Some(&TestFieldValue::F32(0.0))
        );
        assert_eq!(
            events[0].field("after_activation_hardness"),
            Some(&TestFieldValue::F32(0.0))
        );
        assert_eq!(
            events[0].field("after_norm_hardness"),
            Some(&TestFieldValue::F32(0.0))
        );
        assert_eq!(
            events[0].field("after_router_hardness"),
            Some(&TestFieldValue::F32(0.5))
        );
        assert_eq!(
            events[0].field("checkpoint_saved"),
            Some(&TestFieldValue::Bool(true))
        );
    }

    #[test]
    fn scheduler_transition_applies_hard_router_at_exact_boundary() {
        let mut scheduler = scheduler();
        let mut model = RecordingModel::default();
        let collector = TestEventCollector::new();
        let emitter = TrainingLogEmitter::with_test_collector(collector.clone());

        scheduler.apply_step(20, &mut model, &emitter).unwrap();
        scheduler.apply_step(29, &mut model, &emitter).unwrap();
        collector.clear();

        scheduler.apply_step(30, &mut model, &emitter).unwrap();

        assert_eq!(
            model.calls.last().copied(),
            Some(AppliedControls {
                phase_kind: TrainPhaseKind::FullNumericQat,
                expert_qat: QuantHardness::Hard,
                activation_qat: QuantHardness::Hard,
                norm_qat: QuantHardness::Hard,
                router_mode: RouterTrainMode::HardTop1,
                soft_progress: 0.0,
            })
        );

        let events = collector.events();
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].field("from_phase"),
            Some(&TestFieldValue::String("expert_ternary_qat".to_owned()))
        );
        assert_eq!(
            events[0].field("to_phase"),
            Some(&TestFieldValue::String("full_numeric_qat".to_owned()))
        );
        assert_eq!(
            events[0].field("before_norm_hardness"),
            Some(&TestFieldValue::F32(1.0))
        );
        assert_eq!(
            events[0].field("after_router_hardness"),
            Some(&TestFieldValue::F32(1.0))
        );
    }

    #[test]
    fn scheduler_rejects_out_of_schedule_step_without_side_effects() {
        let mut scheduler = scheduler();
        let mut model = RecordingModel::default();
        let collector = TestEventCollector::new();
        let emitter = TrainingLogEmitter::with_test_collector(collector.clone());

        assert_eq!(
            scheduler.apply_step(50, &mut model, &emitter).unwrap_err(),
            PhaseSchedulerError::StepOutOfRange {
                step: 50,
                final_step: 50,
            }
        );
        assert!(model.calls.is_empty());
        assert!(collector.events().is_empty());
        assert_eq!(scheduler.applied_phase(), None);
    }

    #[test]
    fn scheduler_rejects_backward_steps_without_side_effects() {
        let mut scheduler = scheduler();
        let mut model = RecordingModel::default();
        let collector = TestEventCollector::new();
        let emitter = TrainingLogEmitter::with_test_collector(collector.clone());

        scheduler.apply_step(10, &mut model, &emitter).unwrap();
        assert_eq!(
            scheduler.apply_step(9, &mut model, &emitter).unwrap_err(),
            PhaseSchedulerError::NonMonotonicStep {
                previous_step: 10,
                step: 9,
            }
        );

        assert_eq!(model.calls.len(), 1);
        assert!(collector.events().is_empty());
    }

    #[test]
    fn scheduler_rejects_missed_or_skipped_live_phase_boundaries() {
        let mut scheduler = scheduler();
        let mut model = RecordingModel::default();
        let collector = TestEventCollector::new();
        let emitter = TrainingLogEmitter::with_test_collector(collector.clone());

        scheduler.apply_step(0, &mut model, &emitter).unwrap();
        assert_eq!(
            scheduler.apply_step(15, &mut model, &emitter).unwrap_err(),
            PhaseSchedulerError::MissedPhaseBoundary {
                from_phase: TrainPhaseKind::DenseTeacherWarmup,
                to_phase: TrainPhaseKind::RouterWarmup,
                boundary_step: 10,
                step: 15,
            }
        );
        assert_eq!(
            scheduler.apply_step(20, &mut model, &emitter).unwrap_err(),
            PhaseSchedulerError::SkippedPhaseTransition {
                from_phase: TrainPhaseKind::DenseTeacherWarmup,
                to_phase: TrainPhaseKind::ExpertTernaryQat,
                previous_step: 0,
                step: 20,
            }
        );

        assert_eq!(model.calls.len(), 1);
        assert!(collector.events().is_empty());
    }

    #[test]
    fn scheduler_can_reconstruct_current_controls_after_restart() {
        let mut scheduler = scheduler();
        let mut model = RecordingModel::default();
        let emitter = TrainingLogEmitter::new();

        let outcome = scheduler.apply_step(24, &mut model, &emitter).unwrap();

        assert!(matches!(
            outcome,
            PhaseStepOutcome::EnteredInitial {
                controls,
            } if controls.phase().kind() == TrainPhaseKind::ExpertTernaryQat
                && controls.soft_progress() > 0.44
                && controls.soft_progress() < 0.45
        ));
        assert_eq!(model.calls.len(), 1);
        assert_eq!(model.calls[0].phase_kind, TrainPhaseKind::ExpertTernaryQat);
    }

    fn scheduler() -> TrainingPhaseScheduler {
        TrainingPhaseScheduler::new(TrainingPhaseSchedule::default_five_phase(10).unwrap())
    }

    #[derive(Debug, Default)]
    struct RecordingModel {
        calls: Vec<AppliedControls>,
    }

    impl PhaseControlledModel for RecordingModel {
        fn apply_phase_controls(&mut self, controls: PhaseControls) {
            self.calls.push(AppliedControls {
                phase_kind: controls.phase().kind(),
                expert_qat: controls.expert_qat(),
                activation_qat: controls.activation_qat(),
                norm_qat: controls.norm_qat(),
                router_mode: controls.router_mode(),
                soft_progress: controls.soft_progress(),
            });
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq)]
    struct AppliedControls {
        phase_kind: TrainPhaseKind,
        expert_qat: QuantHardness,
        activation_qat: QuantHardness,
        norm_qat: QuantHardness,
        router_mode: RouterTrainMode,
        soft_progress: f32,
    }
}
