use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use gbf_artifact::ids::ArtifactPath;
use gbf_artifact::{
    CanonicalTensor, Dtype, ModelArtifact, PayloadRole, QuantSpec_S3, WeightQuant,
    canonical_payload_sha,
};
use gbf_foundation::{CheckpointId as ShadowCheckpointId, WorkloadId, sha256};
use gbf_policy::{CostEstimate, EstimatedCostDelta, EvidenceClass, UncertaintyEnvelope};
use gbf_train::ema::{EmaDecay, EmaUpdate, EmaWeights, update_ema_slice};
use gbf_train::export_visitor::{ArtifactExportModel, ExportVisitorError};
use gbf_train::logging::{
    TestEvent, TestEventCollector, TestEventKind, TestFieldValue, TrainingLogEmitter,
};
use gbf_train::selection::{
    CheckpointFrontierPoint as SelectionFrontierPoint, CheckpointId as SelectionCheckpointId,
    SelectionStatus, training_selection_json, training_selection_report,
};
use gbf_train::shadow::{
    CompileRequestRef, QualitySummary, ShadowCompilePolicy, ShadowCompileReport,
    ShadowConformanceReport, ShadowPipelineConfig, ShadowPipelineError, ShadowPipelineOwnedInput,
    ShadowPipelineSteps, ShadowStaticBudgetReport, run_shadow_compile_pipeline_owned,
};
use gbf_train::student::{
    HardTernaryStudentModel, StudentStorageFingerprint, StudentWeightFingerprint,
};

#[test]
fn e2e_shadow_compile_runs_cadence_frontier_selection_and_logging() {
    let policy = ShadowCompilePolicy::new(
        20,
        vec![CompileRequestRef::new("compile/bringup.toml").unwrap()],
        vec![WorkloadId::from("tiny-smoke")],
        3,
    )
    .unwrap();
    let output_root = unique_shadow_output_root();
    let collector = TestEventCollector::new();
    let emitter = TrainingLogEmitter::with_test_collector(collector.clone());

    let mut reports = Vec::new();
    for (step, lm_loss, cycles, bank_switches) in [
        (20, 0.90_f32, 80_i64, 1_i64),
        (40, 0.70_f32, 100_i64, 2_i64),
        (60, 0.50_f32, 120_i64, 3_i64),
    ] {
        let input = shadow_input(step, lm_loss);
        let steps = E2eShadowSteps::new(
            512,
            "fits with 512 bytes margin",
            Some(cost_delta(cycles, bank_switches)),
        );
        let config = ShadowPipelineConfig::default()
            .with_full_compile(true)
            .with_output_root(&output_root)
            .with_frontier_size(policy.keep_frontier as u32);

        let report =
            run_shadow_compile_pipeline_owned(&policy, &input, &steps, &config, Some(&emitter))
                .expect("shadow compile pipeline succeeds");
        assert_shadow_report(&report, step, lm_loss);
        reports.push(report);
    }

    let selection_points = reports
        .iter()
        .map(selection_point_from_shadow_report)
        .collect::<Vec<_>>();
    let selection = training_selection_report(
        selection_points.clone(),
        policy.keep_frontier,
        1_700_000_000,
    )
    .expect("training selection report builds");

    assert_eq!(selection.frontier.len(), 3);
    assert!(selection.frontier.len() <= policy.keep_frontier);
    assert!(selection.hard_failure_filter.is_empty());
    assert_eq!(selection.selection.status, SelectionStatus::Selected);
    assert_eq!(
        selection.selection.selected_checkpoint_id,
        Some(SelectionCheckpointId::new("checkpoint.phase-e.060").unwrap())
    );
    assert_eq!(
        selection
            .selected
            .as_ref()
            .map(|point| point.checkpoint_id.as_str()),
        Some("checkpoint.phase-e.060")
    );

    let selection_json =
        training_selection_json(selection_points, policy.keep_frontier, 1_700_000_000)
            .expect("training_selection.json serializes");
    fs::create_dir_all(&output_root).expect("selection output root is creatable");
    let selection_path = output_root.join("training_selection.json");
    fs::write(&selection_path, selection_json.as_bytes())
        .expect("training_selection.json is emitted");
    let decoded: serde_json::Value = serde_json::from_slice(
        &fs::read(&selection_path).expect("training_selection.json is readable"),
    )
    .expect("training_selection.json parses");
    assert_eq!(decoded["schema"], "training_selection.v1");
    assert_eq!(
        decoded["selection"]["selected_checkpoint_id"],
        "checkpoint.phase-e.060"
    );
    assert_eq!(decoded["frontier"].as_array().unwrap().len(), 3);

    let collected_events = collector.events();
    let events = events_of_kind(&collected_events, TestEventKind::ShadowCompile);
    assert_eq!(events.len(), 3);
    assert_eq!(
        events
            .iter()
            .map(|event| field_u64(event, "step"))
            .collect::<Vec<_>>(),
        vec![20, 40, 60]
    );
    for (event, step) in events.iter().zip([20, 40, 60]) {
        assert_eq!(
            event.field("checkpoint_id"),
            Some(&TestFieldValue::String(format!(
                "checkpoint.phase-e.{step:03}"
            )))
        );
        assert_eq!(
            event.field("compile_profile"),
            Some(&TestFieldValue::String("compile/bringup.toml".to_owned()))
        );
        assert_eq!(
            event.field("fit_status"),
            Some(&TestFieldValue::String("fits".to_owned()))
        );
        assert!(
            field_string(event, "quality_summary").contains("lm_loss="),
            "quality summary should carry lm_loss"
        );
        assert_eq!(
            field_u64(event, "frontier_size"),
            policy.keep_frontier as u64
        );
    }

    let _ = fs::remove_dir_all(output_root);
}

fn assert_shadow_report(report: &ShadowCompileReport, step: u64, lm_loss: f32) {
    assert_eq!(report.step, step);
    assert_eq!(
        report.checkpoint,
        ShadowCheckpointId::from(format!("checkpoint.phase-e.{step:03}"))
    );
    assert_eq!(report.ema_update_count, 1);
    assert_eq!(report.request_reports.len(), 1);
    assert_eq!(report.workload_reports.len(), 1);
    assert!(
        report
            .result_dir
            .ends_with(PathBuf::from(format!("step_{step}")))
    );

    let point = &report.frontier_point;
    assert_eq!(point.checkpoint, report.checkpoint);
    assert!((point.quality.lm_loss - lm_loss).abs() <= f32::EPSILON);
    assert!(point.quality.perplexity > 1.0);
    assert!(point.conformance.passes);
    assert!(point.conformance.max_divergence <= 0.015625);
    assert!(point.projected_fit.fits);
    assert_eq!(point.projected_fit.margin_bytes, 512);
    assert!(point.schedule_cost.is_some());
}

fn selection_point_from_shadow_report(report: &ShadowCompileReport) -> SelectionFrontierPoint {
    let schedule_cost = report
        .frontier_point
        .schedule_cost
        .as_ref()
        .expect("full shadow compile report includes schedule cost");
    SelectionFrontierPoint::new(
        report.frontier_point.checkpoint.to_string(),
        report.step,
        1.0 / f64::from(report.frontier_point.quality.lm_loss),
        report.frontier_point.conformance.passes,
        f64::from(report.frontier_point.conformance.max_divergence),
        report.frontier_point.projected_fit.fits,
        report.frontier_point.projected_fit.margin_bytes,
        q16_units(schedule_cost.cycles_per_token.envelope.p50_q16_16),
        q16_units(schedule_cost.bank_switches_per_token.envelope.p50_q16_16),
    )
    .expect("shadow report adapts to selection point")
}

fn q16_units(value: i64) -> u64 {
    u64::try_from(value / UncertaintyEnvelope::Q16_ONE).expect("fixture costs are non-negative")
}

fn shadow_input(step: u64, lm_loss: f32) -> ShadowPipelineOwnedInput<ToyShadowStudent> {
    let mut ema_weights = EmaWeights::new(
        ToyShadowStudent::new(vec![1.0, 3.0], true),
        EmaDecay::new(0.5).unwrap(),
    );
    ema_weights
        .update(&ToyShadowStudent::new(
            vec![step as f32 / 10.0, step as f32 / 5.0],
            true,
        ))
        .unwrap();

    ShadowPipelineOwnedInput {
        step,
        checkpoint: ShadowCheckpointId::from(format!("checkpoint.phase-e.{step:03}")),
        quality: QualitySummary::new(lm_loss, 1.0 + lm_loss).unwrap(),
        ema_weights,
    }
}

#[derive(Debug, Clone)]
struct E2eShadowSteps {
    margin_bytes: i64,
    diagnostic: &'static str,
    schedule_cost: Option<EstimatedCostDelta>,
}

impl E2eShadowSteps {
    fn new(
        margin_bytes: i64,
        diagnostic: &'static str,
        schedule_cost: Option<EstimatedCostDelta>,
    ) -> Self {
        Self {
            margin_bytes,
            diagnostic,
            schedule_cost,
        }
    }
}

impl ShadowPipelineSteps for E2eShadowSteps {
    fn static_budget(
        &self,
        artifact: &ModelArtifact,
        request: &CompileRequestRef,
    ) -> Result<ShadowStaticBudgetReport, ShadowPipelineError> {
        artifact
            .validate()
            .map_err(|error| ShadowPipelineError::StaticBudget(error.to_string()))?;
        ShadowStaticBudgetReport::new(request.clone(), true, self.margin_bytes, self.diagnostic)
    }

    fn conformance(
        &self,
        artifact: &ModelArtifact,
        workload: &WorkloadId,
    ) -> Result<ShadowConformanceReport, ShadowPipelineError> {
        artifact
            .validate()
            .map_err(|error| ShadowPipelineError::Conformance(error.to_string()))?;
        ShadowConformanceReport::new(
            workload.clone(),
            true,
            0.015625,
            "artifact oracle agreement within tolerance",
        )
    }

    fn schedule_cost(
        &self,
        artifact: &ModelArtifact,
        _request: &CompileRequestRef,
    ) -> Result<Option<EstimatedCostDelta>, ShadowPipelineError> {
        artifact
            .validate()
            .map_err(|error| ShadowPipelineError::ScheduleCost(error.to_string()))?;
        Ok(self.schedule_cost.clone())
    }
}

fn cost_delta(cycles_per_token: i64, bank_switches_per_token: i64) -> EstimatedCostDelta {
    EstimatedCostDelta {
        cycles_per_token: estimate(cycles_per_token),
        bank_switches_per_token: estimate(bank_switches_per_token),
        sram_page_switches_per_token: None,
        yields_per_token: estimate(1),
        scheduler_headroom_utilization: estimate(0),
        video_commit_cost_margin: None,
        max_no_progress_estimate: estimate(0),
        time_to_first_token: estimate(100),
        sustained_throughput_tokens_per_megacycle: estimate(5),
        frame_jitter: None,
    }
}

fn estimate(units: i64) -> CostEstimate {
    CostEstimate {
        evidence_class: EvidenceClass::Heuristic,
        envelope: UncertaintyEnvelope::exact_units(units),
        refs: Vec::new(),
        fallback_reason: None,
    }
}

fn events_of_kind(events: &[TestEvent], kind: TestEventKind) -> Vec<&TestEvent> {
    events.iter().filter(|event| event.kind() == kind).collect()
}

fn field_u64(event: &TestEvent, name: &str) -> u64 {
    match event.field(name) {
        Some(TestFieldValue::U64(value)) => *value,
        other => panic!("{name} must be a U64 field, got {other:?}"),
    }
}

fn field_string<'a>(event: &'a TestEvent, name: &str) -> &'a str {
    match event.field(name) {
        Some(TestFieldValue::String(value)) => value,
        other => panic!("{name} must be a string field, got {other:?}"),
    }
}

fn unique_shadow_output_root() -> PathBuf {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock after UNIX_EPOCH")
        .as_nanos();
    std::env::temp_dir().join(format!("shadow_results-e2e-{unique}"))
}

#[derive(Debug, Clone, PartialEq)]
struct ToyShadowStudent {
    weights: Vec<f32>,
    requires_grad: bool,
}

impl ToyShadowStudent {
    fn new(weights: Vec<f32>, requires_grad: bool) -> Self {
        Self {
            weights,
            requires_grad,
        }
    }
}

impl EmaUpdate for ToyShadowStudent {
    fn ema_update_from(
        &mut self,
        current: &Self,
        decay: EmaDecay,
    ) -> Result<(), gbf_train::ema::EmaExportError> {
        update_ema_slice(&mut self.weights, &current.weights, decay)
    }
}

impl HardTernaryStudentModel for ToyShadowStudent {
    fn detach_for_student(&mut self) {
        self.requires_grad = false;
    }

    fn student_weight_fingerprint(&self) -> StudentWeightFingerprint {
        StudentWeightFingerprint::new(weight_bytes(&self.weights)).unwrap()
    }

    fn student_storage_fingerprint(&self) -> StudentStorageFingerprint {
        let mut bytes = Vec::from("toy-shadow-student:f32:");
        bytes.extend_from_slice(&self.weights.len().to_le_bytes());
        bytes.extend_from_slice(&weight_bytes(&self.weights));
        StudentStorageFingerprint::new(bytes).unwrap()
    }

    fn student_storage_identity(&self) -> usize {
        self.weights.as_ptr() as usize
    }

    fn student_requires_grad(&self) -> bool {
        self.requires_grad
    }
}

impl ArtifactExportModel for ToyShadowStudent {
    fn artifact_seed(&self) -> u64 {
        20
    }

    fn artifact_semantic_core_hash(
        &self,
        frozen: &gbf_train::student::FrozenStudent<Self>,
    ) -> gbf_foundation::Hash256 {
        sha256(frozen.weight_fingerprint().bytes())
    }

    fn artifact_quant_spec(&self) -> QuantSpec_S3 {
        QuantSpec_S3::new(BTreeMap::from([(weight_tensor_id(), WeightQuant::Fp32)]))
    }

    fn artifact_tensors(&self) -> Result<Vec<CanonicalTensor>, ExportVisitorError> {
        let payload = weight_bytes(&self.weights);
        Ok(vec![
            CanonicalTensor::new(
                weight_tensor_id(),
                Dtype::Fp32,
                vec![
                    u32::try_from(self.weights.len())
                        .map_err(|_| ExportVisitorError::model("too many weights"))?,
                ],
                canonical_payload_sha(&payload),
                PayloadRole::DeployableWeight,
            )
            .map_err(|error| ExportVisitorError::model(error.to_string()))?,
        ])
    }
}

fn weight_tensor_id() -> ArtifactPath {
    ArtifactPath::new("shadow.ema.weight").unwrap()
}

fn weight_bytes(weights: &[f32]) -> Vec<u8> {
    weights
        .iter()
        .flat_map(|weight| weight.to_le_bytes())
        .collect()
}
