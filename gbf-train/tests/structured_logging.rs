use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex};

use gbf_artifact::weight_plan::{
    ScaleFormat, ScaleGranularity, TernaryWeightPlan, ThresholdPlan, WeightEncoding,
};
use gbf_foundation::ByteCost;
use gbf_train::logging::{
    EVENT_NAME_EXPORT_COMPLETE, EVENT_NAME_LOSS_STEP, EVENT_NAME_PHASE_TRANSITION,
    EVENT_NAME_PREFLIGHT, EVENT_NAME_SHADOW_COMPILE, EVENT_NAME_TEACHER_FREEZE, ExportEvent,
    LossBreakdown, PREFLIGHT_CHECK_EXPERT_SLOT_BUDGET, PhaseTransitionEvent, QatHardnessLevels,
    ShadowCompileEvent, TeacherFreezeEvent, TrainingLogEmitter,
};
use gbf_train::preflight::ExpertBudgetPreflightReport;
use gbf_train::teacher::{
    DenseTeacherModel, TeacherFreezeMetadata, TeacherStorageFingerprint, TeacherWeightFingerprint,
    freeze_teacher_with_logging,
};
use tracing_subscriber::prelude::*;

#[test]
fn canonical_event_helpers_are_captured_by_tracing_subscriber() {
    let capture = TraceCapture::default();
    let subscriber = tracing_subscriber::registry().with(capture.clone());

    tracing::subscriber::with_default(subscriber, || {
        let emitter = TrainingLogEmitter::new();
        emitter.loss_step(&sample_loss_breakdown()).unwrap();
        emitter
            .phase_transition(&PhaseTransitionEvent {
                from_phase: "router_warmup".to_owned(),
                to_phase: "expert_ternary_qat".to_owned(),
                step: 20,
                before_hardness: QatHardnessLevels::new(0.0, 0.0, 0.2, 0.4, 0.0).unwrap(),
                after_hardness: QatHardnessLevels::new(1.0, 0.5, 0.6, 0.8, 1.0).unwrap(),
                checkpoint_saved: true,
            })
            .unwrap();
        emitter
            .teacher_freeze(&TeacherFreezeEvent {
                step: 10,
                teacher_checkpoint_id: "teacher-10".to_owned(),
                source_weight_fingerprint: "010203".to_owned(),
                frozen_weight_fingerprint: "010203".to_owned(),
                weights_match: true,
                duration_ms: 7,
            })
            .unwrap();
        emitter
            .export_complete(&ExportEvent {
                step: 30,
                artifact_core_hash: "0123456789abcdef".to_owned(),
                total_bytes: 4096,
                n_experts: 2,
                ternary_weight_plan_summary: "ternary2/per_output_row/q8_8".to_owned(),
                scale_bytes_total: 128,
                duration_ms: 17,
            })
            .unwrap();
        emitter
            .shadow_compile(&ShadowCompileEvent {
                step: 30,
                checkpoint_id: "ckpt-30".to_owned(),
                compile_profile: "tiny-ci".to_owned(),
                fit_status: "fits".to_owned(),
                quality_summary: "frontier stable".to_owned(),
                frontier_size: 3,
                duration_ms: 42,
            })
            .unwrap();
    });

    let records = capture.records();
    assert_event_field(&records, EVENT_NAME_LOSS_STEP, "step", "7");
    assert_event_f64_close(&records, EVENT_NAME_LOSS_STEP, "lm_loss", 1.0);
    assert_event_f64_close(&records, EVENT_NAME_LOSS_STEP, "distill_loss", 0.1);
    assert_event_f64_close(&records, EVENT_NAME_LOSS_STEP, "balance_loss", 0.02);
    assert_event_f64_close(&records, EVENT_NAME_LOSS_STEP, "zrouter_loss", 0.04);
    assert_event_f64_close(&records, EVENT_NAME_LOSS_STEP, "switch_loss", 0.03);
    assert_event_f64_close(&records, EVENT_NAME_LOSS_STEP, "range_loss", 0.05);
    assert_event_f64_close(&records, EVENT_NAME_LOSS_STEP, "zero_loss", 0.06);
    assert_event_f64_close(&records, EVENT_NAME_LOSS_STEP, "shape_loss", 0.01);
    assert_event_f64_close(&records, EVENT_NAME_LOSS_STEP, "overflow_loss", 0.02);
    assert_event_f64_close(&records, EVENT_NAME_LOSS_STEP, "total_loss", 1.33);
    assert_event_field(
        &records,
        EVENT_NAME_PHASE_TRANSITION,
        "from_phase",
        "router_warmup",
    );
    assert_event_field(
        &records,
        EVENT_NAME_PHASE_TRANSITION,
        "to_phase",
        "expert_ternary_qat",
    );
    assert_event_field(&records, EVENT_NAME_PHASE_TRANSITION, "step", "20");
    assert_event_f64_close(
        &records,
        EVENT_NAME_PHASE_TRANSITION,
        "before_ternary_hardness",
        0.0,
    );
    assert_event_f64_close(
        &records,
        EVENT_NAME_PHASE_TRANSITION,
        "before_activation_hardness",
        0.0,
    );
    assert_event_f64_close(
        &records,
        EVENT_NAME_PHASE_TRANSITION,
        "before_norm_hardness",
        0.2,
    );
    assert_event_f64_close(
        &records,
        EVENT_NAME_PHASE_TRANSITION,
        "before_router_hardness",
        0.4,
    );
    assert_event_f64_close(
        &records,
        EVENT_NAME_PHASE_TRANSITION,
        "before_expert_hardness",
        0.0,
    );
    assert_event_f64_close(
        &records,
        EVENT_NAME_PHASE_TRANSITION,
        "after_ternary_hardness",
        1.0,
    );
    assert_event_f64_close(
        &records,
        EVENT_NAME_PHASE_TRANSITION,
        "after_activation_hardness",
        0.5,
    );
    assert_event_f64_close(
        &records,
        EVENT_NAME_PHASE_TRANSITION,
        "after_norm_hardness",
        0.6,
    );
    assert_event_f64_close(
        &records,
        EVENT_NAME_PHASE_TRANSITION,
        "after_router_hardness",
        0.8,
    );
    assert_event_f64_close(
        &records,
        EVENT_NAME_PHASE_TRANSITION,
        "after_expert_hardness",
        1.0,
    );
    assert_event_field(
        &records,
        EVENT_NAME_PHASE_TRANSITION,
        "checkpoint_saved",
        "true",
    );
    assert_event_field(&records, EVENT_NAME_TEACHER_FREEZE, "step", "10");
    assert_event_field(
        &records,
        EVENT_NAME_TEACHER_FREEZE,
        "teacher_checkpoint_id",
        "teacher-10",
    );
    assert_event_field(
        &records,
        EVENT_NAME_TEACHER_FREEZE,
        "source_weight_fingerprint",
        "010203",
    );
    assert_event_field(
        &records,
        EVENT_NAME_TEACHER_FREEZE,
        "frozen_weight_fingerprint",
        "010203",
    );
    assert_event_field(&records, EVENT_NAME_TEACHER_FREEZE, "weights_match", "true");
    assert_event_field(&records, EVENT_NAME_TEACHER_FREEZE, "duration_ms", "7");
    assert_event_field(&records, EVENT_NAME_EXPORT_COMPLETE, "step", "30");
    assert_event_field(
        &records,
        EVENT_NAME_EXPORT_COMPLETE,
        "artifact_core_hash",
        "0123456789abcdef",
    );
    assert_event_field(&records, EVENT_NAME_EXPORT_COMPLETE, "total_bytes", "4096");
    assert_event_field(&records, EVENT_NAME_EXPORT_COMPLETE, "n_experts", "2");
    assert_event_field(
        &records,
        EVENT_NAME_EXPORT_COMPLETE,
        "ternary_weight_plan_summary",
        "ternary2/per_output_row/q8_8",
    );
    assert_event_field(
        &records,
        EVENT_NAME_EXPORT_COMPLETE,
        "scale_bytes_total",
        "128",
    );
    assert_event_field(&records, EVENT_NAME_EXPORT_COMPLETE, "duration_ms", "17");
    assert_event_field(&records, EVENT_NAME_SHADOW_COMPILE, "step", "30");
    assert_event_field(
        &records,
        EVENT_NAME_SHADOW_COMPILE,
        "checkpoint_id",
        "ckpt-30",
    );
    assert_event_field(
        &records,
        EVENT_NAME_SHADOW_COMPILE,
        "compile_profile",
        "tiny-ci",
    );
    assert_event_field(&records, EVENT_NAME_SHADOW_COMPILE, "fit_status", "fits");
    assert_event_field(
        &records,
        EVENT_NAME_SHADOW_COMPILE,
        "quality_summary",
        "frontier stable",
    );
    assert_event_field(&records, EVENT_NAME_SHADOW_COMPILE, "frontier_size", "3");
    assert_event_field(&records, EVENT_NAME_SHADOW_COMPILE, "duration_ms", "42");

    for event_name in [
        EVENT_NAME_LOSS_STEP,
        EVENT_NAME_PHASE_TRANSITION,
        EVENT_NAME_TEACHER_FREEZE,
        EVENT_NAME_EXPORT_COMPLETE,
        EVENT_NAME_SHADOW_COMPILE,
    ] {
        assert_no_message_field(&records, event_name);
    }
}

#[test]
fn teacher_freeze_producer_is_captured_by_tracing_subscriber() {
    let capture = TraceCapture::default();
    let subscriber = tracing_subscriber::registry().with(capture.clone());

    tracing::subscriber::with_default(subscriber, || {
        let emitter = TrainingLogEmitter::new();
        let model = LoggingTeacherModel {
            weights: vec![1.0, 2.0],
            requires_grad: true,
        };
        let metadata = TeacherFreezeMetadata::new(10, "teacher-10").unwrap();

        freeze_teacher_with_logging(&model, metadata, &emitter).unwrap();
    });

    let records = capture.records();
    assert_event_field(&records, EVENT_NAME_TEACHER_FREEZE, "step", "10");
    assert_event_field(
        &records,
        EVENT_NAME_TEACHER_FREEZE,
        "teacher_checkpoint_id",
        "teacher-10",
    );
    assert_event_field(
        &records,
        EVENT_NAME_TEACHER_FREEZE,
        "source_weight_fingerprint",
        "0000803f00000040",
    );
    assert_event_field(
        &records,
        EVENT_NAME_TEACHER_FREEZE,
        "frozen_weight_fingerprint",
        "0000803f00000040",
    );
    assert_event_field(&records, EVENT_NAME_TEACHER_FREEZE, "weights_match", "true");
}

#[test]
fn preflight_producer_is_captured_by_tracing_subscriber() {
    let capture = TraceCapture::default();
    let subscriber = tracing_subscriber::registry().with(capture.clone());

    tracing::subscriber::with_default(subscriber, || {
        let emitter = TrainingLogEmitter::new();
        ExpertBudgetPreflightReport::check_expert_slot_with_logging(
            &default_plan(),
            128,
            224,
            ByteCost::new(16_384),
            &emitter,
        )
        .unwrap();
    });

    let records = capture.records();
    assert_event_field(
        &records,
        EVENT_NAME_PREFLIGHT,
        "check_name",
        PREFLIGHT_CHECK_EXPERT_SLOT_BUDGET,
    );
    assert_event_field(&records, EVENT_NAME_PREFLIGHT, "status", "pass");
    assert_event_field(
        &records,
        EVENT_NAME_PREFLIGHT,
        "detail",
        "expert payload fits slot with 1294 slack bytes",
    );
    assert_event_field(&records, EVENT_NAME_PREFLIGHT, "budget_computed", "true");
    assert_event_field(&records, EVENT_NAME_PREFLIGHT, "expert_bytes", "15090");
    assert_event_field(
        &records,
        EVENT_NAME_PREFLIGHT,
        "expert_slot_usable_bytes",
        "16384",
    );
    assert_event_field(&records, EVENT_NAME_PREFLIGHT, "slack_bytes", "1294");
    assert_event_field(&records, EVENT_NAME_PREFLIGHT, "over_by_bytes", "0");
    assert_no_message_field(&records, EVENT_NAME_PREFLIGHT);
}

#[cfg(feature = "burn-adapter")]
#[test]
fn scalar_metric_sink_uses_canonical_event_name() {
    use gbf_train::adapter::burn::{MetricSink, ScalarMetric, TracingMetricSink};
    use gbf_train::logging::EVENT_NAME_SCALAR_METRIC;

    let capture = TraceCapture::default();
    let subscriber = tracing_subscriber::registry().with(capture.clone());

    tracing::subscriber::with_default(subscriber, || {
        TracingMetricSink.log_scalar(&ScalarMetric::new("loss", 0.25, 7));
    });

    let records = capture.records();
    assert_event_field(&records, EVENT_NAME_SCALAR_METRIC, "metric_name", "loss");
    assert_event_field(&records, EVENT_NAME_SCALAR_METRIC, "metric_value", "0.25");
    assert_event_field(&records, EVENT_NAME_SCALAR_METRIC, "step", "7");
    assert_no_message_field(&records, EVENT_NAME_SCALAR_METRIC);
}

fn event_record<'a>(records: &'a [TraceRecord], event_name: &str) -> &'a TraceRecord {
    records
        .iter()
        .find(|record| {
            record.kind == TraceRecordKind::Event && record.field("event_name") == Some(event_name)
        })
        .unwrap_or_else(|| panic!("missing structured event {event_name}"))
}

fn assert_event_field(records: &[TraceRecord], event_name: &str, field: &str, expected: &str) {
    let record = event_record(records, event_name);
    assert_eq!(record.field(field), Some(expected));
}

fn assert_no_message_field(records: &[TraceRecord], event_name: &str) {
    let record = event_record(records, event_name);
    assert!(
        record.field("message").is_none(),
        "{event_name} must not encode load-bearing data in a message field"
    );
}

fn assert_event_f64_close(records: &[TraceRecord], event_name: &str, field: &str, expected: f64) {
    let record = event_record(records, event_name);
    let actual = record
        .field(field)
        .unwrap_or_else(|| panic!("missing field {field} on event {event_name}"))
        .parse::<f64>()
        .unwrap_or_else(|error| {
            panic!("field {field} on event {event_name} is not numeric: {error}")
        });
    assert!(
        (actual - expected).abs() <= 1.0e-6,
        "field {field} on event {event_name} expected {expected}, got {actual}"
    );
}

fn sample_loss_breakdown() -> LossBreakdown {
    LossBreakdown {
        step: 7,
        lm_loss: 1.0,
        distill_loss: 0.1,
        balance_loss: 0.02,
        zrouter_loss: 0.04,
        switch_loss: 0.03,
        range_loss: 0.05,
        zero_loss: 0.06,
        shape_loss: 0.01,
        overflow_loss: 0.02,
        total_loss: 1.33,
    }
}

#[derive(Clone)]
struct LoggingTeacherModel {
    weights: Vec<f32>,
    requires_grad: bool,
}

impl DenseTeacherModel for LoggingTeacherModel {
    type Input = Vec<f32>;
    type Output = f32;
    type ForwardError = std::convert::Infallible;

    fn detach_for_teacher(&mut self) {
        self.requires_grad = false;
    }

    fn forward_no_grad(&self, input: Self::Input) -> Result<Self::Output, Self::ForwardError> {
        Ok(self
            .weights
            .iter()
            .zip(input.iter())
            .map(|(weight, input)| weight * input)
            .sum())
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
        TeacherStorageFingerprint::new((self.weights.as_ptr() as usize).to_le_bytes()).unwrap()
    }

    fn teacher_requires_grad(&self) -> bool {
        self.requires_grad
    }
}

fn default_plan() -> TernaryWeightPlan {
    TernaryWeightPlan::new(
        WeightEncoding::Ternary2,
        ScaleGranularity::PerOutputRow,
        ScaleFormat::Q8_8,
        ThresholdPlan::FixedQ8_8,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TraceRecordKind {
    Event,
}

#[derive(Debug, Clone)]
struct TraceRecord {
    kind: TraceRecordKind,
    fields: BTreeMap<String, String>,
}

impl TraceRecord {
    fn field(&self, name: &str) -> Option<&str> {
        self.fields.get(name).map(String::as_str)
    }
}

#[derive(Debug, Clone, Default)]
struct TraceCapture {
    records: Arc<Mutex<Vec<TraceRecord>>>,
}

impl TraceCapture {
    fn records(&self) -> Vec<TraceRecord> {
        self.records
            .lock()
            .expect("trace capture mutex is not poisoned")
            .clone()
    }
}

impl<S> tracing_subscriber::layer::Layer<S> for TraceCapture
where
    S: tracing::Subscriber,
{
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut visitor = TraceFieldVisitor::default();
        event.record(&mut visitor);
        self.records
            .lock()
            .expect("trace capture mutex is not poisoned")
            .push(TraceRecord {
                kind: TraceRecordKind::Event,
                fields: visitor.fields,
            });
    }
}

#[derive(Debug, Default)]
struct TraceFieldVisitor {
    fields: BTreeMap<String, String>,
}

impl TraceFieldVisitor {
    fn insert(&mut self, field: &tracing::field::Field, value: String) {
        self.fields.insert(field.name().to_owned(), value);
    }
}

impl tracing::field::Visit for TraceFieldVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn fmt::Debug) {
        self.insert(field, format!("{value:?}"));
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.insert(field, value.to_owned());
    }

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.insert(field, value.to_string());
    }

    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.insert(field, value.to_string());
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.insert(field, value.to_string());
    }

    fn record_f64(&mut self, field: &tracing::field::Field, value: f64) {
        self.insert(field, value.to_string());
    }
}
