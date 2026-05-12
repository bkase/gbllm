use gbf_model::qat::RouterTrainMode;
use gbf_train::logging::{
    ExportEvent, LossBreakdown, TestEvent, TestEventCollector, TestEventKind, TestFieldValue,
    TrainingLogEmitter,
};
use gbf_train::phase::{
    PhaseScheduleError, QuantHardness, TRAIN_PHASE_COUNT, TrainPhaseKind, TrainPhaseSpec,
    TrainingPhaseSchedule,
};
use gbf_train::scheduler::{
    PhaseControlledModel, PhaseControls, PhaseSchedulerError, PhaseStepOutcome,
    TrainingPhaseScheduler,
};
use gbf_train::teacher::{
    DenseTeacherModel, TeacherFreezeGuard, TeacherFreezeMetadata, TeacherStorageFingerprint,
    TeacherStorageIdentity, TeacherWeightFingerprint,
};

#[test]
fn phase_tests_default_five_phase_schedule_applies_boundaries_and_logs_transitions() {
    let schedule = TrainingPhaseSchedule::default_five_phase(10).unwrap();
    assert_eq!(schedule.phases(), literal_default_phase_specs().as_slice());

    let mut scheduler = TrainingPhaseScheduler::new(schedule);
    // This is a scheduler/control recorder, not the T10.1 tiny model fixture.
    // It proves phase-control delivery and logging shape only; canonical
    // per-phase loss-term applicability is owned by bd-3d9y.
    let mut model = TinyPhaseModel::default();
    let collector = TestEventCollector::new();
    let emitter = TrainingLogEmitter::with_test_collector(collector.clone());
    let mut checkpoint_steps = Vec::new();

    for step in [0, 9, 10, 19, 20, 24, 29, 30, 39, 40, 49] {
        let outcome = scheduler
            .apply_step_with_checkpoint(step, &mut model, &emitter, |transition| {
                checkpoint_steps.push(transition.step());
                true
            })
            .unwrap();
        assert_eq!(model.active_step(), Some(step));
        if step == 0 {
            assert!(matches!(outcome, PhaseStepOutcome::EnteredInitial { .. }));
            assert!(!model.ternary_control_signal_enabled_proxy());
        }
        if step == 20 {
            assert!(model.ternary_control_signal_enabled_proxy());
        }
    }

    assert_eq!(checkpoint_steps, vec![10, 20, 30, 40]);
    assert_eq!(
        model.applied_kinds(),
        vec![
            TrainPhaseKind::DenseTeacherWarmup,
            TrainPhaseKind::DenseTeacherWarmup,
            TrainPhaseKind::RouterWarmup,
            TrainPhaseKind::RouterWarmup,
            TrainPhaseKind::ExpertTernaryQat,
            TrainPhaseKind::ExpertTernaryQat,
            TrainPhaseKind::ExpertTernaryQat,
            TrainPhaseKind::FullNumericQat,
            TrainPhaseKind::FullNumericQat,
            TrainPhaseKind::HardenAndSelect,
            TrainPhaseKind::HardenAndSelect,
        ]
    );

    assert_controls(
        model.applied()[0],
        ExpectedControls::new(
            TrainPhaseKind::DenseTeacherWarmup,
            QuantHardness::Off,
            QuantHardness::Off,
            QuantHardness::Off,
            RouterTrainMode::SoftTop1,
            0.0,
        ),
    );
    assert_controls(
        model.applied()[4],
        ExpectedControls::new(
            TrainPhaseKind::ExpertTernaryQat,
            QuantHardness::Hard,
            QuantHardness::Soft,
            QuantHardness::Soft,
            RouterTrainMode::SoftTop1,
            0.0,
        ),
    );
    assert_controls(
        model.applied()[6],
        ExpectedControls::new(
            TrainPhaseKind::ExpertTernaryQat,
            QuantHardness::Hard,
            QuantHardness::Soft,
            QuantHardness::Soft,
            RouterTrainMode::SoftTop1,
            1.0,
        ),
    );
    assert_controls(
        model.applied()[7],
        ExpectedControls::new(
            TrainPhaseKind::FullNumericQat,
            QuantHardness::Hard,
            QuantHardness::Hard,
            QuantHardness::Hard,
            RouterTrainMode::HardTop1,
            0.0,
        ),
    );

    let transitions = events_of_kind(&collector, TestEventKind::PhaseTransition);
    assert_eq!(transitions.len(), 4);
    assert_eq!(
        transitions
            .iter()
            .map(|event| match event.field("step") {
                Some(TestFieldValue::U64(step)) => *step,
                other => panic!("phase transition step must be U64, got {other:?}"),
            })
            .collect::<Vec<_>>(),
        checkpoint_steps
    );
    for (event, expected) in transitions.iter().zip(expected_default_transitions()) {
        assert_transition(event, expected);
    }
}

#[test]
fn phase_tests_teacher_freeze_at_phase_boundary_is_immutable_and_logged() {
    let mut scheduler =
        TrainingPhaseScheduler::new(TrainingPhaseSchedule::default_five_phase(10).unwrap());
    let mut phase_model = TinyPhaseModel::default();
    // This teacher is a minimal freeze/logging recorder, not the T10.1 tiny
    // fixture. The assertions below are about immutable teacher snapshots at a
    // scheduler boundary.
    let mut student = TinyTeacherModel::new([1.0, 2.0], true);
    let collector = TestEventCollector::new();
    let emitter = TrainingLogEmitter::with_test_collector(collector.clone());

    scheduler.apply_step(0, &mut phase_model, &emitter).unwrap();
    scheduler.apply_step(9, &mut phase_model, &emitter).unwrap();
    assert_eq!(
        scheduler.current_phase(10).unwrap().kind(),
        TrainPhaseKind::RouterWarmup
    );

    let mut freeze_guard = TeacherFreezeGuard::new();
    let frozen = freeze_guard
        .freeze_with_logging(
            &student,
            TeacherFreezeMetadata::new(10, "teacher-phase-a-end").unwrap(),
            &emitter,
        )
        .unwrap();
    assert!(freeze_guard.has_fired());
    assert!(!frozen.requires_grad());

    let teacher_output = frozen.forward_no_grad(vec![1.0, 1.0]).unwrap();
    student.apply_qat_update([0.25, -0.75]);
    let student_output = student.forward_with_grad(vec![1.0, 1.0]);

    assert_eq!(teacher_output.value, 3.0);
    assert!(!teacher_output.requires_grad);
    assert_eq!(student_output.value, 2.5);
    assert!(student_output.requires_grad);
    assert_ne!(
        frozen.weight_fingerprint().to_hex(),
        student.teacher_weight_fingerprint().to_hex()
    );

    scheduler
        .apply_step(10, &mut phase_model, &emitter)
        .unwrap();
    assert_eq!(
        phase_model.applied().last().unwrap().phase_kind,
        TrainPhaseKind::RouterWarmup
    );

    let freezes = events_of_kind(&collector, TestEventKind::TeacherFreeze);
    assert_eq!(freezes.len(), 1);
    assert_eq!(freezes[0].field("step"), Some(&TestFieldValue::U64(10)));
    assert_eq!(
        freezes[0].field("teacher_checkpoint_id"),
        Some(&TestFieldValue::String("teacher-phase-a-end".to_owned()))
    );
    assert_eq!(
        freezes[0].field("weights_match"),
        Some(&TestFieldValue::Bool(true))
    );
}

#[test]
fn phase_tests_schedule_validation_rejects_bad_edges_skips_and_single_step_phases_transition() {
    assert_eq!(
        TrainingPhaseSchedule::new(Vec::new()).unwrap_err(),
        PhaseScheduleError::WrongPhaseCount {
            expected: TRAIN_PHASE_COUNT,
            actual: 0,
        }
    );

    assert_eq!(
        TrainingPhaseSchedule::new(literal_phase_specs_with_ranges([
            (0, 10),
            (10, 20),
            (19, 30),
            (30, 40),
            (40, 50),
        ]))
        .unwrap_err(),
        PhaseScheduleError::NonContiguous {
            previous_kind: TrainPhaseKind::RouterWarmup,
            next_kind: TrainPhaseKind::ExpertTernaryQat,
            expected_start: 20,
            actual_start: 19,
        }
    );

    let mut skipped = literal_default_phase_specs().to_vec();
    skipped.swap(1, 2);
    // The original skip-phase intent is not accepted by the canonical five-row
    // schedule: omitting or reordering an intermediate phase is rejected rather
    // than treated as a shorter valid run.
    assert_eq!(
        TrainingPhaseSchedule::new(skipped).unwrap_err(),
        PhaseScheduleError::UnexpectedPhaseKind {
            index: 1,
            expected_kind: TrainPhaseKind::RouterWarmup,
            actual_kind: TrainPhaseKind::ExpertTernaryQat,
        }
    );

    assert!(matches!(
        TrainPhaseSpec::new(
            TrainPhaseKind::DenseTeacherWarmup,
            4,
            4,
            QuantHardness::Off,
            QuantHardness::Off,
            QuantHardness::Off,
            RouterTrainMode::SoftTop1,
        ),
        Err(PhaseScheduleError::InvalidPhaseRange { .. })
    ));

    let mut scheduler =
        TrainingPhaseScheduler::new(TrainingPhaseSchedule::default_five_phase(1).unwrap());
    let mut model = TinyPhaseModel::default();
    let collector = TestEventCollector::new();
    let emitter = TrainingLogEmitter::with_test_collector(collector.clone());

    for step in 0..5 {
        scheduler.apply_step(step, &mut model, &emitter).unwrap();
    }

    assert_eq!(
        model.applied_kinds(),
        TrainPhaseKind::canonical_order().to_vec()
    );
    assert_eq!(
        scheduler.apply_step(5, &mut model, &emitter).unwrap_err(),
        PhaseSchedulerError::StepOutOfRange {
            step: 5,
            final_step: 5,
        }
    );
    assert_eq!(
        events_of_kind(&collector, TestEventKind::PhaseTransition).len(),
        4
    );
}

#[test]
fn phase_tests_live_scheduler_rejects_missed_and_skipped_phase_boundaries() {
    let mut scheduler =
        TrainingPhaseScheduler::new(TrainingPhaseSchedule::default_five_phase(10).unwrap());
    let mut model = TinyPhaseModel::default();
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

    assert_eq!(
        model.applied_kinds(),
        vec![TrainPhaseKind::DenseTeacherWarmup]
    );
    assert!(events_of_kind(&collector, TestEventKind::PhaseTransition).is_empty());
}

#[test]
fn phase_tests_logging_capture_keeps_loss_and_export_fields_typed() {
    let collector = TestEventCollector::new();
    let emitter = TrainingLogEmitter::with_test_collector(collector.clone());

    emitter
        .loss_step(&LossBreakdown {
            step: 20,
            lm_loss: 1.0,
            distill_loss: 0.2,
            balance_loss: 0.03,
            zrouter_loss: 0.04,
            switch_loss: 0.05,
            range_loss: 0.06,
            zero_loss: 0.07,
            shape_loss: 0.08,
            overflow_loss: 0.09,
            total_loss: 1.62,
        })
        .unwrap();
    emitter
        .export_complete(&ExportEvent {
            step: 40,
            artifact_core_hash: "phase-test-artifact-core-hash".to_owned(),
            total_bytes: 2048,
            n_experts: 2,
            ternary_weight_plan_summary: "ternary2/per_output_row/q8_8".to_owned(),
            scale_bytes_total: 64,
            duration_ms: 5,
        })
        .unwrap();

    let loss_steps = events_of_kind(&collector, TestEventKind::LossStep);
    assert_eq!(loss_steps.len(), 1);
    for (field, expected) in [
        ("step", TestFieldValue::U64(20)),
        ("lm_loss", TestFieldValue::F32(1.0)),
        ("distill_loss", TestFieldValue::F32(0.2)),
        ("balance_loss", TestFieldValue::F32(0.03)),
        ("zrouter_loss", TestFieldValue::F32(0.04)),
        ("switch_loss", TestFieldValue::F32(0.05)),
        ("range_loss", TestFieldValue::F32(0.06)),
        ("zero_loss", TestFieldValue::F32(0.07)),
        ("shape_loss", TestFieldValue::F32(0.08)),
        ("overflow_loss", TestFieldValue::F32(0.09)),
        ("total_loss", TestFieldValue::F32(1.62)),
    ] {
        assert_eq!(loss_steps[0].field(field), Some(&expected));
    }

    let exports = events_of_kind(&collector, TestEventKind::ExportComplete);
    assert_eq!(exports.len(), 1);
    assert_eq!(
        exports[0].field("artifact_core_hash"),
        Some(&TestFieldValue::String(
            "phase-test-artifact-core-hash".to_owned()
        ))
    );
    assert_eq!(
        exports[0].field("total_bytes"),
        Some(&TestFieldValue::U64(2048))
    );
}

fn literal_default_phase_specs() -> [TrainPhaseSpec; TRAIN_PHASE_COUNT] {
    [
        phase(
            TrainPhaseKind::DenseTeacherWarmup,
            0,
            10,
            QuantHardness::Off,
            QuantHardness::Off,
            QuantHardness::Off,
            RouterTrainMode::SoftTop1,
        ),
        phase(
            TrainPhaseKind::RouterWarmup,
            10,
            20,
            QuantHardness::Off,
            QuantHardness::Off,
            QuantHardness::Off,
            RouterTrainMode::SoftTop1,
        ),
        phase(
            TrainPhaseKind::ExpertTernaryQat,
            20,
            30,
            QuantHardness::Hard,
            QuantHardness::Soft,
            QuantHardness::Soft,
            RouterTrainMode::SoftTop1,
        ),
        phase(
            TrainPhaseKind::FullNumericQat,
            30,
            40,
            QuantHardness::Hard,
            QuantHardness::Hard,
            QuantHardness::Hard,
            RouterTrainMode::HardTop1,
        ),
        phase(
            TrainPhaseKind::HardenAndSelect,
            40,
            50,
            QuantHardness::Hard,
            QuantHardness::Hard,
            QuantHardness::Hard,
            RouterTrainMode::HardTop1,
        ),
    ]
}

fn literal_phase_specs_with_ranges(ranges: [(u64, u64); TRAIN_PHASE_COUNT]) -> Vec<TrainPhaseSpec> {
    literal_default_phase_specs()
        .into_iter()
        .zip(ranges)
        .map(|(default, (start, end))| {
            phase(
                default.kind(),
                start,
                end,
                default.expert_qat(),
                default.activation_qat(),
                default.norm_qat(),
                default.router_mode(),
            )
        })
        .collect()
}

fn phase(
    kind: TrainPhaseKind,
    start_step: u64,
    end_step: u64,
    expert_qat: QuantHardness,
    activation_qat: QuantHardness,
    norm_qat: QuantHardness,
    router_mode: RouterTrainMode,
) -> TrainPhaseSpec {
    TrainPhaseSpec::new(
        kind,
        start_step,
        end_step,
        expert_qat,
        activation_qat,
        norm_qat,
        router_mode,
    )
    .unwrap()
}

fn events_of_kind(collector: &TestEventCollector, kind: TestEventKind) -> Vec<TestEvent> {
    collector
        .events()
        .into_iter()
        .filter(|event| event.kind() == kind)
        .collect()
}

fn expected_default_transitions() -> [ExpectedTransition; 4] {
    [
        ExpectedTransition {
            from: "dense_teacher_warmup",
            to: "router_warmup",
            step: 10,
            before: HardnessFields::new(0.0, 0.0, 0.0, 0.5, 0.0),
            after: HardnessFields::new(0.0, 0.0, 0.0, 0.5, 0.0),
        },
        ExpectedTransition {
            from: "router_warmup",
            to: "expert_ternary_qat",
            step: 20,
            before: HardnessFields::new(0.0, 0.0, 0.0, 0.5, 0.0),
            after: HardnessFields::new(1.0, 0.0, 0.0, 0.5, 1.0),
        },
        ExpectedTransition {
            from: "expert_ternary_qat",
            to: "full_numeric_qat",
            step: 30,
            before: HardnessFields::new(1.0, 1.0, 1.0, 0.5, 1.0),
            after: HardnessFields::new(1.0, 1.0, 1.0, 1.0, 1.0),
        },
        ExpectedTransition {
            from: "full_numeric_qat",
            to: "harden_and_select",
            step: 40,
            before: HardnessFields::new(1.0, 1.0, 1.0, 1.0, 1.0),
            after: HardnessFields::new(1.0, 1.0, 1.0, 1.0, 1.0),
        },
    ]
}

fn assert_transition(event: &TestEvent, expected: ExpectedTransition) {
    assert_eq!(
        event.field("from_phase"),
        Some(&TestFieldValue::String(expected.from.to_owned()))
    );
    assert_eq!(
        event.field("to_phase"),
        Some(&TestFieldValue::String(expected.to.to_owned()))
    );
    assert_eq!(
        event.field("step"),
        Some(&TestFieldValue::U64(expected.step))
    );
    assert_hardness_fields(event, "before", expected.before);
    assert_hardness_fields(event, "after", expected.after);
    assert_eq!(
        event.field("checkpoint_saved"),
        Some(&TestFieldValue::Bool(true))
    );
}

fn assert_hardness_fields(event: &TestEvent, prefix: &str, expected: HardnessFields) {
    assert_eq!(
        event.field(&format!("{prefix}_ternary_hardness")),
        Some(&TestFieldValue::F32(expected.ternary))
    );
    assert_eq!(
        event.field(&format!("{prefix}_activation_hardness")),
        Some(&TestFieldValue::F32(expected.activation))
    );
    assert_eq!(
        event.field(&format!("{prefix}_norm_hardness")),
        Some(&TestFieldValue::F32(expected.norm))
    );
    assert_eq!(
        event.field(&format!("{prefix}_router_hardness")),
        Some(&TestFieldValue::F32(expected.router))
    );
    assert_eq!(
        event.field(&format!("{prefix}_expert_hardness")),
        Some(&TestFieldValue::F32(expected.expert))
    );
}

fn assert_controls(actual: AppliedControls, expected: ExpectedControls) {
    assert_eq!(actual.phase_kind, expected.phase_kind);
    assert_eq!(actual.expert_qat, expected.expert_qat);
    assert_eq!(actual.activation_qat, expected.activation_qat);
    assert_eq!(actual.norm_qat, expected.norm_qat);
    assert_eq!(actual.router_mode, expected.router_mode);
    assert!(
        (actual.soft_progress - expected.soft_progress).abs() <= f32::EPSILON,
        "expected soft progress {}, got {}",
        expected.soft_progress,
        actual.soft_progress
    );
}

#[derive(Debug, Clone, Copy)]
struct ExpectedTransition {
    from: &'static str,
    to: &'static str,
    step: u64,
    before: HardnessFields,
    after: HardnessFields,
}

#[derive(Debug, Clone, Copy)]
struct HardnessFields {
    ternary: f32,
    activation: f32,
    norm: f32,
    router: f32,
    expert: f32,
}

impl HardnessFields {
    const fn new(ternary: f32, activation: f32, norm: f32, router: f32, expert: f32) -> Self {
        Self {
            ternary,
            activation,
            norm,
            router,
            expert,
        }
    }
}

#[derive(Debug, Default)]
struct TinyPhaseModel {
    applied: Vec<AppliedControls>,
}

impl TinyPhaseModel {
    fn applied(&self) -> &[AppliedControls] {
        &self.applied
    }

    fn applied_kinds(&self) -> Vec<TrainPhaseKind> {
        self.applied
            .iter()
            .map(|controls| controls.phase_kind)
            .collect()
    }

    fn active_step(&self) -> Option<u64> {
        self.applied.last().map(|controls| controls.step)
    }

    fn ternary_control_signal_enabled_proxy(&self) -> bool {
        // This is a scheduler control-signal proxy, not proof that a real
        // forward path selected ternary kernels.
        self.applied
            .last()
            .is_some_and(|controls| controls.expert_qat != QuantHardness::Off)
    }
}

impl PhaseControlledModel for TinyPhaseModel {
    fn apply_phase_controls(&mut self, controls: PhaseControls) {
        self.applied.push(AppliedControls {
            step: controls.step(),
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
    step: u64,
    phase_kind: TrainPhaseKind,
    expert_qat: QuantHardness,
    activation_qat: QuantHardness,
    norm_qat: QuantHardness,
    router_mode: RouterTrainMode,
    soft_progress: f32,
}

#[derive(Debug, Clone, Copy)]
struct ExpectedControls {
    phase_kind: TrainPhaseKind,
    expert_qat: QuantHardness,
    activation_qat: QuantHardness,
    norm_qat: QuantHardness,
    router_mode: RouterTrainMode,
    soft_progress: f32,
}

impl ExpectedControls {
    const fn new(
        phase_kind: TrainPhaseKind,
        expert_qat: QuantHardness,
        activation_qat: QuantHardness,
        norm_qat: QuantHardness,
        router_mode: RouterTrainMode,
        soft_progress: f32,
    ) -> Self {
        Self {
            phase_kind,
            expert_qat,
            activation_qat,
            norm_qat,
            router_mode,
            soft_progress,
        }
    }
}

#[derive(Debug, Clone)]
// Minimal teacher-freeze recorder for scheduler/logging tests. It intentionally
// is not the T10.1 tiny model fixture.
struct TinyTeacherModel {
    weights: Vec<f32>,
    requires_grad: bool,
}

impl TinyTeacherModel {
    fn new<const N: usize>(weights: [f32; N], requires_grad: bool) -> Self {
        Self {
            weights: weights.to_vec(),
            requires_grad,
        }
    }

    fn apply_qat_update<const N: usize>(&mut self, delta: [f32; N]) {
        assert_eq!(self.weights.len(), N);
        for (weight, delta) in self.weights.iter_mut().zip(delta) {
            *weight += delta;
        }
    }

    fn forward_with_grad(&self, input: Vec<f32>) -> TinyForwardOutput {
        TinyForwardOutput {
            value: dot(&self.weights, &input),
            requires_grad: self.requires_grad,
        }
    }
}

impl DenseTeacherModel for TinyTeacherModel {
    type Input = Vec<f32>;
    type Output = TinyForwardOutput;
    type ForwardError = std::convert::Infallible;

    fn detach_for_teacher(&mut self) {
        self.requires_grad = false;
    }

    fn forward_no_grad(&self, input: Self::Input) -> Result<Self::Output, Self::ForwardError> {
        Ok(TinyForwardOutput {
            value: dot(&self.weights, &input),
            requires_grad: false,
        })
    }

    fn teacher_weight_fingerprint(&self) -> TeacherWeightFingerprint {
        TeacherWeightFingerprint::new(
            self.weights
                .iter()
                .flat_map(|weight| weight.to_le_bytes())
                .collect::<Vec<_>>(),
        )
        .unwrap()
    }

    fn teacher_storage_fingerprint(&self) -> TeacherStorageFingerprint {
        TeacherStorageFingerprint::new(
            self.weights
                .iter()
                .flat_map(|weight| weight.to_le_bytes())
                .collect::<Vec<_>>(),
        )
        .unwrap()
    }

    fn teacher_storage_identity(&self) -> TeacherStorageIdentity {
        TeacherStorageIdentity::new((self.weights.as_ptr() as usize).to_le_bytes()).unwrap()
    }

    fn teacher_requires_grad(&self) -> bool {
        self.requires_grad
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct TinyForwardOutput {
    value: f32,
    requires_grad: bool,
}

fn dot(weights: &[f32], input: &[f32]) -> f32 {
    weights
        .iter()
        .zip(input.iter())
        .map(|(weight, input)| weight * input)
        .sum()
}
