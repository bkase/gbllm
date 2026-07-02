use std::collections::BTreeSet;

use gbf_artifact::core::ArtifactCore;
use gbf_artifact::export_facts::ExpertPayloadDigest;
use gbf_artifact::tensor::{CanonicalTensor, CanonicalTensorKind};
use gbf_artifact::weight_plan::{ScaleGranularity, TernaryWeightPlan};
use gbf_foundation::{ExpertId, LayerId};
use gbf_model::qat::{
    ExpertBlockQat, ExpertForwardOptions, ExpertQat, ExportVisitor, ExportedQatArtifact,
    RouterForwardOptions, RouterTrainMode, TernaryLinearQat, VisitedModuleKind,
};
use gbf_test::fixtures::{
    TINY_D_FF, TINY_D_MODEL, TINY_MOE_BLOCK_INDEX, TINY_N_EXPERTS, TINY_N_LAYERS, TINY_VOCAB_SIZE,
    TinyModel, assert_artifact_valid, assert_bytes_equal, make_tiny_model, tiny_prompt_corpus,
};
use gbf_train::logging::{
    ExportEvent, LossBreakdown, TestEvent, TestEventCollector, TestEventKind, TestFieldValue,
    TrainingLogEmitter,
};
use gbf_train::phase::{QuantHardness, TrainPhaseKind, TrainingPhaseSchedule};
use gbf_train::scheduler::{
    PhaseControlledModel, PhaseControls, PhaseStepOutcome, TrainingPhaseScheduler,
};
use gbf_train::teacher::{
    DenseTeacherModel, TeacherFreezeGuard, TeacherFreezeMetadata, TeacherStorageFingerprint,
    TeacherStorageIdentity, TeacherWeightFingerprint,
};

const STEPS_PER_PHASE: u64 = 10;
const TOTAL_STEPS: u64 = STEPS_PER_PHASE * 5;

#[test]
fn e2e_train_export_tiny_pipeline_completes_exports_logs_and_is_deterministic() {
    let first = run_tiny_train_export_pipeline(7);
    let second = run_tiny_train_export_pipeline(7);

    assert_eq!(first.artifact_core_hash, second.artifact_core_hash);
    assert_bytes_equal(&first.artifact_bytes, &second.artifact_bytes);
    assert_eq!(first.packed_ternary_bytes, second.packed_ternary_bytes);

    assert_eq!(first.applied.len(), TOTAL_STEPS as usize);
    assert_eq!(
        first.phase_sequence(),
        TrainPhaseKind::canonical_order().to_vec()
    );
    assert_eq!(
        first.applied[0].phase_kind,
        TrainPhaseKind::DenseTeacherWarmup
    );
    assert_eq!(first.applied[10].phase_kind, TrainPhaseKind::RouterWarmup);
    assert_eq!(
        first.applied[20].phase_kind,
        TrainPhaseKind::ExpertTernaryQat
    );
    assert_eq!(first.applied[30].phase_kind, TrainPhaseKind::FullNumericQat);
    assert_eq!(
        first.applied[40].phase_kind,
        TrainPhaseKind::HardenAndSelect
    );

    assert!(!first.selected_experts.is_empty());
    assert!(
        first
            .selected_experts
            .iter()
            .all(|&expert_id| expert_id < TINY_N_EXPERTS)
    );

    let transitions = events_of_kind(&first.events, TestEventKind::PhaseTransition);
    assert_eq!(
        transition_steps(&transitions),
        vec![10, 20, 30, 40],
        "five phases produce one event per boundary"
    );

    let losses = events_of_kind(&first.events, TestEventKind::LossStep);
    assert_eq!(losses.len(), TOTAL_STEPS as usize);
    for (step, event) in losses.iter().enumerate() {
        assert_eq!(field_u64(event, "step"), step as u64);
        for field in [
            "lm_loss",
            "distill_loss",
            "balance_loss",
            "zrouter_loss",
            "switch_loss",
            "range_loss",
            "zero_loss",
            "shape_loss",
            "overflow_loss",
            "total_loss",
        ] {
            let value = field_f32(event, field);
            assert!(
                value.is_finite() && value >= 0.0,
                "{field} at step {step} must be finite and non-negative, got {value}"
            );
        }
    }
    assert!(
        first.lm_losses.last().unwrap() < first.lm_losses.first().unwrap(),
        "lm_loss should trend down across the tiny diagnostic loop"
    );

    let freezes = events_of_kind(&first.events, TestEventKind::TeacherFreeze);
    assert_eq!(freezes.len(), 1);
    assert_eq!(field_u64(&freezes[0], "step"), STEPS_PER_PHASE);
    assert_eq!(
        freezes[0].field("teacher_checkpoint_id"),
        Some(&TestFieldValue::String("teacher-phase-a-end".to_owned()))
    );
    assert_eq!(
        freezes[0].field("weights_match"),
        Some(&TestFieldValue::Bool(true))
    );

    let exports = events_of_kind(&first.events, TestEventKind::ExportComplete);
    assert_eq!(exports.len(), 1);
    assert_eq!(
        exports[0].field("artifact_core_hash"),
        Some(&TestFieldValue::String(first.artifact_core_hash.clone()))
    );
    assert_eq!(
        exports[0].field("n_experts"),
        Some(&TestFieldValue::U64(TINY_N_EXPERTS as u64))
    );
    assert_eq!(
        exports[0].field("scale_bytes_total"),
        Some(&TestFieldValue::U64(first.scale_bytes_total))
    );
    assert_eq!(
        field_u64(&exports[0], "total_bytes"),
        first.artifact_bytes.len() as u64
    );
}

fn run_tiny_train_export_pipeline(seed: u64) -> PipelineRun {
    let source_model = make_tiny_model();
    assert_tiny_model_contract(&source_model);

    let prompts = tiny_prompt_corpus();
    assert_eq!(prompts.len(), 8);
    assert!(prompts.iter().all(|prompt| !prompt.is_empty()));
    assert!(
        prompts
            .iter()
            .flatten()
            .all(|&token| token < TINY_VOCAB_SIZE as u8)
    );

    let collector = TestEventCollector::new();
    let emitter = TrainingLogEmitter::with_test_collector(collector.clone());
    let mut scheduler = TrainingPhaseScheduler::new(
        TrainingPhaseSchedule::default_five_phase(STEPS_PER_PHASE).unwrap(),
    );
    let mut model = TinyPipelineModel::from_fixture(&source_model);
    let mut teacher = TinyTeacherModel::from_seed(seed);
    let mut teacher_freeze_guard = TeacherFreezeGuard::new();
    let mut lm_losses = Vec::new();
    let mut selected_experts = BTreeSet::new();

    for step in 0..TOTAL_STEPS {
        let outcome = scheduler
            .apply_step_with_checkpoint(step, &mut model, &emitter, |_| true)
            .unwrap();
        if step == 0 {
            assert!(matches!(outcome, PhaseStepOutcome::EnteredInitial { .. }));
        }

        if step == STEPS_PER_PHASE {
            let frozen = teacher_freeze_guard
                .freeze_with_logging(
                    &teacher,
                    TeacherFreezeMetadata::new(step, "teacher-phase-a-end").unwrap(),
                    &emitter,
                )
                .unwrap();
            assert!(!frozen.requires_grad());

            let frozen_output = frozen.forward_no_grad(vec![1.0, 0.5, -0.25]).unwrap();
            teacher.apply_qat_update([0.125, -0.25, 0.0625]);
            let student_output = teacher.forward_with_grad(vec![1.0, 0.5, -0.25]);
            assert!(!frozen_output.requires_grad);
            assert!(student_output.requires_grad);
            assert_ne!(frozen_output.value, student_output.value);
        }

        let input = prompt_input_for_step(&prompts, step, source_model.config().d_model());
        let controls = *model.applied.last().unwrap();
        let router_options = router_options_for_phase(controls.router_mode);
        let routed = model
            .router
            .forward_with_options(&input, &router_options)
            .unwrap();
        selected_experts.insert(routed.expert_index());
        let expert_output = model
            .expert_block
            .forward_with_options(
                &input,
                routed.expert_index(),
                ExpertForwardOptions::for_hardness(controls.expert_qat, controls.activation_qat),
            )
            .unwrap();
        assert_eq!(expert_output.len(), source_model.config().d_model());
        assert!(expert_output.iter().all(|value| value.is_finite()));

        let lm_loss = diagnostic_lm_loss(seed, step, &input, &expert_output);
        let aux = routed.aux_losses();
        let distill_loss = 0.1 * lm_loss;
        let balance_loss = aux.token_balance_proxy_loss().abs();
        let zrouter_loss = aux.z_loss().abs();
        let switch_loss = aux.temporal_smoothness_loss().abs();
        let range_loss = expert_output.iter().map(|value| value.abs()).sum::<f32>() * 0.0001;
        let zero_loss = if controls.expert_qat == QuantHardness::Off {
            0.0
        } else {
            0.001
        };
        let shape_loss = 0.0005;
        let overflow_loss = 0.0;
        let total_loss = lm_loss
            + distill_loss
            + balance_loss
            + zrouter_loss
            + switch_loss
            + range_loss
            + zero_loss
            + shape_loss
            + overflow_loss;

        emitter
            .loss_step(&LossBreakdown {
                step,
                lm_loss,
                distill_loss,
                balance_loss,
                zrouter_loss,
                switch_loss,
                range_loss,
                zero_loss,
                shape_loss,
                overflow_loss,
                total_loss,
            })
            .unwrap();
        lm_losses.push(lm_loss);
    }

    assert!(teacher_freeze_guard.has_fired());

    let artifact = export_tiny_pipeline_model_with_e2e_payload_facts(&source_model, &model);
    assert_artifact_valid(&artifact.core);
    let artifact_core_hash = artifact.artifact_core_hash().to_string();
    let artifact_bytes = serde_json::to_vec(&artifact).unwrap();
    let packed_ternary_bytes = assert_exported_artifact_contract(&artifact);
    let scale_bytes_total = artifact
        .core
        .quant()
        .ternary_weight_plans()
        .iter()
        .map(|entry| tensor_by_id(&artifact.core, entry.scale.as_str()))
        .map(canonical_tensor_payload_bytes)
        .sum::<u64>();

    emitter
        .export_complete(&ExportEvent {
            step: TOTAL_STEPS,
            artifact_core_hash: artifact_core_hash.clone(),
            total_bytes: artifact_bytes.len() as u64,
            n_experts: TINY_N_EXPERTS as u32,
            ternary_weight_plan_summary: "ternary2/per_output_row/q8_8".to_owned(),
            scale_bytes_total,
            duration_ms: 1,
        })
        .unwrap();

    PipelineRun {
        artifact_core_hash,
        artifact_bytes,
        packed_ternary_bytes,
        scale_bytes_total,
        events: collector.events(),
        applied: model.applied,
        lm_losses,
        selected_experts,
    }
}

fn assert_tiny_model_contract(model: &TinyModel) {
    let config = model.config();
    assert_eq!(config.d_model(), TINY_D_MODEL);
    assert_eq!(config.d_ff(), TINY_D_FF);
    assert_eq!(config.n_experts(), TINY_N_EXPERTS);
    assert_eq!(config.n_layers(), TINY_N_LAYERS);
    assert_eq!(config.vocab_size(), TINY_VOCAB_SIZE);
    assert_eq!(config.moe_block_index(), TINY_MOE_BLOCK_INDEX);
    assert_eq!(model.expert_block().experts().len(), TINY_N_EXPERTS);
}

fn export_tiny_pipeline_model_with_e2e_payload_facts(
    model: &TinyModel,
    pipeline: &TinyPipelineModel,
) -> ExportedQatArtifact {
    let config = model.config();
    let embedding = model.embedding();
    let mut visitor = ExportVisitor::new(config.topology().sequence_export_facts());

    visitor
        .visit_embedding(
            "token_embedding",
            config.vocab_size(),
            config.d_model(),
            embedding.embedding_weights(),
        )
        .unwrap();
    visitor
        .visit_classifier(
            "classifier",
            config.vocab_size(),
            config.d_model(),
            embedding.classifier_weights(),
        )
        .unwrap();
    visitor
        .visit_dense_projection("block.0.dense_ffn.up", model.dense_ffn().up_projection())
        .unwrap();
    visitor
        .visit_activation(
            "block.0.dense_ffn.activation",
            model.dense_ffn().activation(),
        )
        .unwrap();
    visitor
        .visit_dense_projection(
            "block.0.dense_ffn.down",
            model.dense_ffn().down_projection(),
        )
        .unwrap();
    visitor
        .visit_router("block.1.router", &pipeline.router)
        .unwrap();
    visitor
        .visit_expert_block("block.1.expert_block", &pipeline.expert_block)
        .unwrap();

    for (expert_index, expert) in pipeline.expert_block.experts().iter().enumerate() {
        visitor
            .record_expert_payload_digest(expert_payload_digest(expert_index, expert))
            .unwrap();
    }

    visitor.finish().unwrap()
}

fn expert_payload_digest(expert_index: usize, expert: &ExpertQat) -> ExpertPayloadDigest {
    let (up_total, up_scale) = projection_payload_parts(expert.up_projection());
    let (down_total, down_scale) = projection_payload_parts(expert.down_projection());
    let total_bytes = up_total + down_total;
    let scale_bytes = up_scale + down_scale;
    let ternary_bytes = total_bytes - scale_bytes;

    ExpertPayloadDigest::new(
        LayerId::new(TINY_MOE_BLOCK_INDEX as u16),
        ExpertId::new(expert_index as u16),
        u32::try_from(total_bytes).unwrap(),
        u32::try_from(ternary_bytes).unwrap(),
        u32::try_from(scale_bytes).unwrap(),
    )
    .unwrap()
}

fn projection_payload_parts(layer: &TernaryLinearQat) -> (u64, u64) {
    let shape = layer.shape();
    let rows = shape.output_rows() as u32;
    let cols = shape.input_cols() as u32;
    let plan = layer.plan();
    let total = plan.compute_byte_cost(rows, cols).as_u64();
    let scale = scale_byte_cost(plan, rows, cols);
    assert!(scale <= total);
    (total, scale)
}

fn scale_byte_cost(plan: TernaryWeightPlan, rows: u32, cols: u32) -> u64 {
    if rows == 0 || cols == 0 {
        return 0;
    }
    let scale_count = match plan.scale_granularity {
        ScaleGranularity::PerTensor => 1,
        ScaleGranularity::PerOutputRow => u64::from(rows),
        ScaleGranularity::PerGroup(group_size) => {
            let elements = u64::from(rows) * u64::from(cols);
            let group_size = u64::from(group_size.get());
            elements.div_ceil(group_size)
        }
    };
    scale_count * u64::from(plan.scale_format.byte_len())
}

fn assert_exported_artifact_contract(artifact: &ExportedQatArtifact) -> u64 {
    assert_eq!(
        artifact.core.sequence_semantics(),
        artifact.facts.sequence.spec()
    );
    assert!(!artifact.facts.activation_ranges.is_empty());
    assert_eq!(artifact.facts.expert_payloads.len(), TINY_N_EXPERTS);

    let visited = artifact
        .visited_modules
        .iter()
        .map(|module| module.kind)
        .collect::<BTreeSet<_>>();
    for kind in [
        VisitedModuleKind::Router,
        VisitedModuleKind::ExpertBlock,
        VisitedModuleKind::Expert,
        VisitedModuleKind::TernaryLinear,
        VisitedModuleKind::Activation,
        VisitedModuleKind::DenseBranchProjection,
    ] {
        assert!(
            visited.contains(&kind),
            "missing visited module kind {kind:?}"
        );
    }

    let entries = artifact.core.quant().ternary_weight_plans();
    assert_eq!(entries.len(), TINY_N_EXPERTS * 2);
    let mut packed_ternary_bytes = 0;
    for entry in entries {
        let projection = entry.projection.as_str();
        assert!(projection.starts_with("block.1.expert_block.expert."));
        assert!(projection.ends_with(".up") || projection.ends_with(".down"));

        let weight = tensor_by_id(&artifact.core, entry.weight.as_str());
        let scale = tensor_by_id(&artifact.core, entry.scale.as_str());
        assert_eq!(weight.kind, CanonicalTensorKind::TernaryWeight);
        assert_eq!(scale.kind, CanonicalTensorKind::TernaryScale);
        assert!(
            weight
                .payload
                .as_i8_slice()
                .unwrap()
                .iter()
                .all(|value| (-1..=1).contains(value))
        );
        assert!(
            scale
                .payload
                .as_u16_slice()
                .unwrap()
                .iter()
                .any(|&value| value > 0)
        );

        let dims = weight.layout.shape.dims();
        assert_eq!(dims.len(), 2);
        let rows = dims[0];
        let cols = dims[1];
        if projection.ends_with(".up") {
            assert_eq!(rows, TINY_D_FF as u32);
            assert_eq!(cols, TINY_D_MODEL as u32);
        } else {
            assert_eq!(rows, TINY_D_MODEL as u32);
            assert_eq!(cols, TINY_D_FF as u32);
        }
        packed_ternary_bytes += entry.plan.compute_byte_cost(rows, cols).as_u64();
    }

    let fact_total = artifact
        .facts
        .expert_payloads
        .iter()
        .map(|payload| u64::from(payload.total_bytes()))
        .sum::<u64>();
    assert_eq!(fact_total, packed_ternary_bytes);
    packed_ternary_bytes
}

fn tensor_by_id<'a>(artifact: &'a ArtifactCore, id: &str) -> &'a CanonicalTensor {
    artifact
        .tensors()
        .iter()
        .find(|tensor| tensor.id.as_str() == id)
        .unwrap_or_else(|| panic!("missing tensor {id}"))
}

fn canonical_tensor_payload_bytes(tensor: &CanonicalTensor) -> u64 {
    match &tensor.payload {
        gbf_artifact::tensor::CanonicalTensorPayload::F32(values) => (values.len() * 4) as u64,
        gbf_artifact::tensor::CanonicalTensorPayload::I8(values) => values.len() as u64,
        gbf_artifact::tensor::CanonicalTensorPayload::U16(values) => (values.len() * 2) as u64,
    }
}

fn prompt_input_for_step(prompts: &[Vec<u8>], step: u64, d_model: usize) -> Vec<f32> {
    let prompt = &prompts[step as usize % prompts.len()];
    (0..d_model)
        .map(|index| {
            let token = prompt[index % prompt.len()] as f32;
            (token + 1.0) / TINY_VOCAB_SIZE as f32 - 0.5
        })
        .collect()
}

fn router_options_for_phase(mode: RouterTrainMode) -> RouterForwardOptions {
    RouterForwardOptions::hard_top1(TINY_N_EXPERTS).with_mode(mode)
}

fn diagnostic_lm_loss(seed: u64, step: u64, input: &[f32], expert_output: &[f32]) -> f32 {
    let residual = input
        .iter()
        .zip(expert_output)
        .map(|(expected, actual)| {
            let diff = expected - actual;
            diff * diff
        })
        .sum::<f32>()
        / input.len() as f32;
    assert!(residual.is_finite() && residual >= 0.0);

    let seed_offset = (seed % 17) as f32 * 0.0001;
    (1.0 + residual.sqrt().min(10.0) * 0.001) / (step + 1) as f32 + seed_offset
}

fn events_of_kind(events: &[TestEvent], kind: TestEventKind) -> Vec<&TestEvent> {
    events.iter().filter(|event| event.kind() == kind).collect()
}

fn transition_steps(events: &[&TestEvent]) -> Vec<u64> {
    events
        .iter()
        .map(|event| field_u64(event, "step"))
        .collect()
}

fn field_u64(event: &TestEvent, name: &str) -> u64 {
    match event.field(name) {
        Some(TestFieldValue::U64(value)) => *value,
        other => panic!("{name} must be a U64 field, got {other:?}"),
    }
}

fn field_f32(event: &TestEvent, name: &str) -> f32 {
    match event.field(name) {
        Some(TestFieldValue::F32(value)) => *value,
        other => panic!("{name} must be an f32 field, got {other:?}"),
    }
}

#[derive(Debug)]
struct PipelineRun {
    artifact_core_hash: String,
    artifact_bytes: Vec<u8>,
    packed_ternary_bytes: u64,
    scale_bytes_total: u64,
    events: Vec<TestEvent>,
    applied: Vec<AppliedControls>,
    lm_losses: Vec<f32>,
    selected_experts: BTreeSet<usize>,
}

impl PipelineRun {
    fn phase_sequence(&self) -> Vec<TrainPhaseKind> {
        let mut sequence = Vec::new();
        for controls in &self.applied {
            if sequence.last() != Some(&controls.phase_kind) {
                sequence.push(controls.phase_kind);
            }
        }
        sequence
    }
}

#[derive(Debug, Clone)]
struct TinyPipelineModel {
    router: gbf_model::qat::Top1RouterQat,
    expert_block: ExpertBlockQat,
    applied: Vec<AppliedControls>,
}

impl TinyPipelineModel {
    fn from_fixture(model: &TinyModel) -> Self {
        Self {
            router: model.router().clone(),
            expert_block: model.expert_block().clone(),
            applied: Vec::new(),
        }
    }
}

impl PhaseControlledModel for TinyPipelineModel {
    fn apply_phase_controls(&mut self, controls: PhaseControls) {
        self.expert_block
            .set_hardness(controls.expert_qat(), controls.activation_qat());
        self.applied.push(AppliedControls {
            step: controls.step(),
            phase_kind: controls.phase().kind(),
            expert_qat: controls.expert_qat(),
            activation_qat: controls.activation_qat(),
            router_mode: controls.router_mode(),
        });
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct AppliedControls {
    step: u64,
    phase_kind: TrainPhaseKind,
    expert_qat: QuantHardness,
    activation_qat: QuantHardness,
    router_mode: RouterTrainMode,
}

#[derive(Debug, Clone)]
struct TinyTeacherModel {
    weights: Vec<f32>,
    requires_grad: bool,
}

impl TinyTeacherModel {
    fn from_seed(seed: u64) -> Self {
        let base = (seed % 13) as f32 * 0.01;
        Self {
            weights: vec![1.0 + base, -0.5 + base, 0.25 - base],
            requires_grad: true,
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
