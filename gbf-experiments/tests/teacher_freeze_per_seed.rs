#![cfg(feature = "s7")]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use gbf_artifact::S7Topology;
use gbf_experiments::S7_LOG_TARGET;
use gbf_experiments::s7::run::{S7TrainAttempt, s7_train_run};
use gbf_experiments::s7::state::{
    S7_PHASE_A_END_STEP, S7_TEACHER_FREEZE_BOUNDARY_EVENT, S7_TEACHER_FREEZE_BOUNDARY_SCHEMA,
    S7TrainRunState,
};
use gbf_foundation::Hash256;
use serde_json::{Value, json};
use tracing_subscriber::layer::Context;
use tracing_subscriber::prelude::*;

const RFC: &str = include_str!("../../history/rfcs/F-S7-moe-beats-dense.md");

#[test]
fn replay_moe_seed_zero_freezes_byte_identical_teacher() {
    let first = run_with_trace_capture(S7Topology::MoeTiny, 0).0;
    let replay = run_with_trace_capture(S7Topology::MoeTiny, 0).0;

    assert_eq!(
        first.frozen_teacher_checkpoint_sha,
        replay.frozen_teacher_checkpoint_sha
    );
    assert_ne!(first.frozen_teacher_checkpoint_sha, Hash256::ZERO);
    assert_eq!(first.phase_a_teacher.phase_a_end_step, S7_PHASE_A_END_STEP);
}

#[test]
fn teacher_checkpoint_is_scoped_by_seed_and_topology() {
    let moe_seed_0 = run_with_trace_capture(S7Topology::MoeTiny, 0).0;
    let moe_seed_1 = run_with_trace_capture(S7Topology::MoeTiny, 1).0;
    let dense_seed_0 = run_with_trace_capture(S7Topology::MoeTinyDenseMatched, 0).0;

    assert_ne!(
        moe_seed_0.frozen_teacher_checkpoint_sha,
        moe_seed_1.frozen_teacher_checkpoint_sha
    );
    assert_ne!(
        moe_seed_0.frozen_teacher_checkpoint_sha,
        dense_seed_0.frozen_teacher_checkpoint_sha
    );
    assert_eq!(
        dense_seed_0.run_id.topology,
        S7Topology::MoeTinyDenseMatched
    );
    assert_ne!(dense_seed_0.frozen_teacher_checkpoint_sha, Hash256::ZERO);
}

#[test]
fn phase_a_teacher_freeze_transitions_directly_to_train_attempted() {
    let mut state = S7TrainRunState::baseline_matched(S7Topology::MoeTiny, 0);

    let boundary = state
        .freeze_teacher_at_phase_a_boundary()
        .expect("teacher freezes");

    assert_eq!(
        state.frozen_teacher_checkpoint_sha(),
        Some(boundary.teacher_checkpoint_sha)
    );
    assert!(matches!(state, S7TrainRunState::TrainAttempted { .. }));
    assert!(state.freeze_teacher_at_phase_a_boundary().is_err());
}

#[test]
fn phase_a_teacher_freeze_emits_structured_boundary_event() {
    let (attempt, events) = run_with_trace_capture(S7Topology::MoeTiny, 0);

    let event = events
        .iter()
        .find(|event| {
            event.fields.get("event_name") == Some(&json!(S7_TEACHER_FREEZE_BOUNDARY_EVENT))
        })
        .unwrap_or_else(|| panic!("teacher-freeze boundary event, captured={events:?}"));
    let expected = attempt.frozen_teacher_checkpoint_sha.to_string();

    assert_eq!(event.target, S7_LOG_TARGET);
    assert_eq!(
        event.fields.get("schema"),
        Some(&json!(S7_TEACHER_FREEZE_BOUNDARY_SCHEMA))
    );
    assert_eq!(event.fields.get("topology"), Some(&json!("MoeTiny")));
    assert_eq!(event.fields.get("seed"), Some(&json!(0)));
    assert_eq!(event.fields.get("phase"), Some(&json!("PhaseA")));
    assert_eq!(event.fields.get("boundary"), Some(&json!("PhaseAEnd")));
    assert_eq!(
        event.fields.get("phase_a_end_step"),
        Some(&json!(S7_PHASE_A_END_STEP))
    );
    assert_eq!(
        event.fields.get("teacher_checkpoint_sha"),
        Some(&json!(expected))
    );
    assert_eq!(
        event.fields.get("frozen_teacher_checkpoint_sha"),
        Some(&json!(expected))
    );
}

#[test]
fn rfc_pins_internal_teacher_freeze_and_provenance_contract() {
    let state_machine = rfc_section(
        "# 5. Experiment state machine",
        "# 6. MoeTiny + dense matched-bytes contract",
    );
    let state_enum = rfc_section_within(state_machine, "State :=", "Transitions:");

    assert!(!state_enum.contains("TeacherFrozen("));
    assert!(state_enum.contains("TrainAttempted(state, topology, seed, phase_products)"));
    assert!(state_machine.contains("T2 train-with-internal-teacher-freeze"));
    assert!(state_machine.contains("BaselineMatched(c, _, _) → TrainAttempted"));
    assert!(!state_machine.contains("BaselineMatched(c, _, _) → TeacherFrozen"));
    assert!(!state_machine.contains("dense_teacher_checkpoint_per_topology"));
    assert!(!state_machine.contains("phase_a_teacher(c, \"MoeTiny\")"));
    assert!(state_machine.contains("Within each s7_train_run(topology, seed):"));
    assert!(state_machine.contains("same\n      (topology, seed) is frozen"));
    assert!(state_machine.contains("same-topology, same-seed teacher"));
    assert!(state_machine.contains("run-internal boundary"));
    assert!(state_machine.contains("T3 moe-train"));
    assert!(state_machine.contains("T4 dense-train"));
    assert!(state_machine.contains("BaselineMatched(c, _, _) → MoeTrainAttempted"));
    assert!(state_machine.contains("BaselineMatched(c, _, _) → DenseTrainAttempted"));

    let run_log = rfc_section("## 13.1 s7_run_log.v1", "## 13.2 s7_score.v1");
    assert!(run_log.contains("frozen_teacher_checkpoint_sha"));
    assert!(run_log.contains("same-topology"));
    assert!(run_log.contains("same-seed Phase A teacher"));

    let oracle = rfc_section(
        "## 13.8 s7_oracle_routed.v1",
        "## 13.9 s7_emulator_one_token.v1",
    );
    assert!(oracle.contains("frozen_teacher_checkpoint_sha"));
    assert!(oracle.contains("RunLog.frozen_teacher_checkpoint_sha"));
}

fn run_with_trace_capture(topology: S7Topology, seed: u64) -> (S7TrainAttempt, Vec<CapturedEvent>) {
    let mut attempt = None;
    let events = capture_events(|| {
        attempt = Some(s7_train_run(topology, seed).expect("s7 train run"));
    });
    (attempt.expect("attempt captured"), events)
}

#[derive(Clone, Debug, Default)]
struct TraceCapture {
    events: Arc<Mutex<Vec<CapturedEvent>>>,
}

#[derive(Clone, Debug)]
struct CapturedEvent {
    target: String,
    fields: BTreeMap<String, Value>,
}

impl<S> tracing_subscriber::layer::Layer<S> for TraceCapture
where
    S: tracing::Subscriber,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = FieldVisitor::default();
        event.record(&mut visitor);
        self.events
            .lock()
            .expect("trace capture mutex")
            .push(CapturedEvent {
                target: event.metadata().target().to_owned(),
                fields: visitor.fields,
            });
    }
}

fn capture_events(f: impl FnOnce()) -> Vec<CapturedEvent> {
    let capture = TraceCapture::default();
    let subscriber = tracing_subscriber::registry().with(capture.clone());
    tracing::subscriber::with_default(subscriber, f);
    capture.events.lock().expect("trace capture mutex").clone()
}

#[derive(Debug, Default)]
struct FieldVisitor {
    fields: BTreeMap<String, Value>,
}

impl FieldVisitor {
    fn insert(&mut self, field: &tracing::field::Field, value: Value) {
        self.fields.insert(field.name().to_owned(), value);
    }
}

impl tracing::field::Visit for FieldVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        let text = format!("{value:?}");
        if let Ok(value) = serde_json::from_str::<Value>(&text) {
            self.insert(field, value);
        } else if let Ok(value) = text.parse::<u64>() {
            self.insert(field, json!(value));
        } else if let Ok(value) = text.parse::<f64>() {
            self.insert(field, json!(value));
        } else {
            self.insert(field, json!(trim_debug_string(&text)));
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.insert(field, json!(value));
    }

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.insert(field, json!(value));
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.insert(field, json!(value));
    }

    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.insert(field, json!(value));
    }

    fn record_f64(&mut self, field: &tracing::field::Field, value: f64) {
        self.insert(field, json!(value));
    }
}

fn trim_debug_string(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|stripped| stripped.strip_suffix('"'))
        .unwrap_or(value)
}

fn rfc_section(start: &str, end: &str) -> &'static str {
    let start_index = RFC.find(start).expect("section start");
    let rest = &RFC[start_index..];
    let end_index = rest.find(end).expect("section end");
    &rest[..end_index]
}

fn rfc_section_within<'a>(haystack: &'a str, start: &str, end: &str) -> &'a str {
    let start_index = haystack.find(start).expect("nested section start");
    let rest = &haystack[start_index..];
    let end_index = rest.find(end).expect("nested section end");
    &rest[..end_index]
}
