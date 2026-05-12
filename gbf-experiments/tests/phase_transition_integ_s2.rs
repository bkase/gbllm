mod common;

use common::tracing_capture::{TraceCapture, captured_events, with_trace_capture};
use gbf_experiments::s1::schema::S1CanonicalJson;
use gbf_experiments::s2::phase_transition_integ::{
    FIXTURE_PHASE_BOUNDARIES, TEACHER_FREEZE_STEP, TRANSITION_STEPS,
    run_phase_transition_integration, write_phase_transition_integration_report,
};
use gbf_experiments::s2::schema::phase_transition_expected_hardness_at_boundary;
use serde_json::json;

#[test]
fn phase_transition_integ_clean_fixture_emits_pt_report() {
    let capture = TraceCapture::default();
    let report = with_trace_capture(&capture, run_phase_transition_integration)
        .expect("phase transition integration report");

    assert!(report.integ_passed);
    assert_eq!(report.fixture_id, "tiny_model_T10.1");
    assert_eq!(
        report.fixture_phase_boundaries,
        FIXTURE_PHASE_BOUNDARIES
            .into_iter()
            .map(|step| step as u32)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        report.transition_event_count,
        u32::try_from(TRANSITION_STEPS.len()).unwrap()
    );
    assert_eq!(report.teacher_freeze_event_count, 1);
    assert_eq!(
        report.hardness_at_boundary,
        phase_transition_expected_hardness_at_boundary()
    );
    assert!(report.skip_phase_test_passed);
    assert!(report.overlap_phase_error_raised);
    assert!(report.empty_phase_error_raised);
    report.validate().unwrap();

    let events = captured_events(&capture);
    assert_eq!(event_count(&events, "phase_transition_integ_start"), 1);
    assert_eq!(event_count(&events, "phase_transition_fired"), 4);
    assert_eq!(event_count(&events, "phase_transition_subcheck"), 3);
    assert_eq!(event_count(&events, "phase_transition_integ_finalized"), 0);
    assert!(events.iter().any(|event| {
        event.name == "phase_transition_subcheck"
            && event.fields.get("name") == Some(&json!("skip_phase"))
            && event.fields.get("passed") == Some(&json!(true))
    }));
    assert!(events.iter().any(|event| {
        event.name == "phase_transition_subcheck"
            && event.fields.get("name") == Some(&json!("overlap"))
            && event.fields.get("passed") == Some(&json!(true))
    }));
    assert!(events.iter().any(|event| {
        event.name == "phase_transition_fired"
            && event.fields.get("fixture_step") == Some(&json!(20))
            && event.fields.get("to") == Some(&json!("phase-c"))
    }));
    assert!(events.iter().any(|event| {
        event.name == "teacher_freeze"
            && event.fields.get("step") == Some(&json!(TEACHER_FREEZE_STEP))
    }));
    let teacher_freeze_index = events
        .iter()
        .position(|event| {
            event.name == "teacher_freeze"
                && event.fields.get("step") == Some(&json!(TEACHER_FREEZE_STEP))
        })
        .expect("teacher freeze event");
    let phase_ab_transition_index = events
        .iter()
        .position(|event| {
            event.name == "phase_transition_fired"
                && event.fields.get("fixture_step") == Some(&json!(TEACHER_FREEZE_STEP))
        })
        .expect("phase A/B transition event");
    assert!(
        teacher_freeze_index < phase_ab_transition_index,
        "teacher freeze must be logged before the phase A/B transition at the shared boundary"
    );
}

#[test]
fn phase_transition_integ_writer_emits_canonical_json_and_one_finalized_event() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("s2_phase_transition_integration.json");
    let capture = TraceCapture::default();

    let report = with_trace_capture(&capture, || {
        write_phase_transition_integration_report(&path).expect("write phase integration")
    });

    assert_eq!(
        std::fs::read(&path).expect("phase integration json"),
        S1CanonicalJson::to_vec(&report).expect("canonical phase integration json")
    );
    assert!(report.integ_passed);

    let events = captured_events(&capture);
    assert_eq!(
        event_count(&events, "s2_phase_transition_integration_writer_open"),
        1
    );
    assert_eq!(event_count(&events, "phase_transition_integ_finalized"), 1);
    let finalized = events
        .iter()
        .find(|event| event.name == "phase_transition_integ_finalized")
        .expect("finalized event");
    assert_eq!(
        finalized.fields.get("transition_event_count"),
        Some(&json!(report.transition_event_count))
    );
    assert_eq!(
        finalized.fields.get("teacher_freeze_event_count"),
        Some(&json!(report.teacher_freeze_event_count))
    );
    assert_eq!(
        finalized.fields.get("integ_self_hash"),
        Some(&json!(report.integ_self_hash.to_string()))
    );
}

#[test]
fn phase_transition_integ_self_hash_is_replay_stable() {
    let first = run_phase_transition_integration().expect("first report");
    let second = run_phase_transition_integration().expect("second report");

    assert_eq!(first, second);
    assert_eq!(first.integ_self_hash, second.integ_self_hash);
    assert_eq!(
        first.canonical_json_bytes().unwrap(),
        second.canonical_json_bytes().unwrap()
    );
}

fn event_count(events: &[common::tracing_capture::TracingEvent], name: &str) -> usize {
    events.iter().filter(|event| event.name == name).count()
}
