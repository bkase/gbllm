#![cfg(feature = "falsify")]

mod common;
mod common_s3;

use std::collections::BTreeSet;
use std::fs;

use common_s3::helpers::tracing_capture_s3::{capture_events, events_to_ndjson};
use gbf_experiments::s3::falsify::{
    BrokenKind, EVENT_NAME_SUBSTITUTE_COMPLETE, EVENT_NAME_SUBSTITUTE_RUN,
    EVENT_NAME_SUITE_COMPLETE, EVENT_NAME_SUITE_STARTED, FALSIFICATION_S3_SUITE_HASH,
    SUBSTITUTE_COUNT, run_suite_with_logging,
};
use gbf_experiments::s3::schema::EVENT_NAME_S3_PHASE_LOG;
use serde_json::json;

#[test]
fn falsification_s3_logging_emits_suite_and_substitute_events() {
    let (results, events) =
        capture_events(|| run_suite_with_logging(FALSIFICATION_S3_SUITE_HASH.parse().unwrap()));
    write_capture_if_requested(&events);

    assert_eq!(results.len(), SUBSTITUTE_COUNT);
    assert!(results.iter().all(|result| result.matches_expected));

    let suite_started = event_by_name(&events, EVENT_NAME_SUITE_STARTED);
    assert_eq!(
        suite_started.fields.get("substitute_count"),
        Some(&json!(SUBSTITUTE_COUNT as u64))
    );
    assert_eq!(
        suite_started.fields.get("suite_hash"),
        Some(&json!(FALSIFICATION_S3_SUITE_HASH))
    );

    let run_events = events_by_name(&events, EVENT_NAME_SUBSTITUTE_RUN);
    assert_eq!(run_events.len(), SUBSTITUTE_COUNT);
    assert!(
        run_events
            .iter()
            .all(
                |event| event.fields.get("expected_verdict") == Some(&json!("H1 Refuted"))
                    || event.fields.get("expected_verdict") == Some(&json!("H2 Refuted"))
                    || event.fields.get("expected_verdict") == Some(&json!("H3 Refuted"))
                    || event.fields.get("expected_verdict") == Some(&json!("H4+H6 Refuted"))
                    || event.fields.get("expected_verdict") == Some(&json!("H5 Refuted"))
                    || event.fields.get("expected_verdict") == Some(&json!("H4 Refuted"))
                    || event.fields.get("expected_verdict") == Some(&json!("H7 Refuted"))
            )
    );

    let complete_events = events_by_name(&events, EVENT_NAME_SUBSTITUTE_COMPLETE);
    assert_eq!(complete_events.len(), SUBSTITUTE_COUNT);
    let observed_names = complete_events
        .iter()
        .map(|event| {
            event
                .fields
                .get("substitute_name")
                .and_then(serde_json::Value::as_str)
                .expect("substitute_name field")
        })
        .collect::<BTreeSet<_>>();
    let expected_names = BrokenKind::ALL
        .into_iter()
        .map(BrokenKind::substitute_name)
        .collect::<BTreeSet<_>>();
    assert_eq!(observed_names, expected_names);
    assert!(
        complete_events
            .iter()
            .all(|event| event.fields.get("matches_expected") == Some(&json!(true)))
    );

    let phase_log = event_by_name(&events, EVENT_NAME_S3_PHASE_LOG);
    assert_eq!(
        phase_log.fields.get("event_kind"),
        Some(&json!("student_freeze"))
    );

    let suite_complete = event_by_name(&events, EVENT_NAME_SUITE_COMPLETE);
    assert_eq!(
        suite_complete.fields.get("all_substitutes_refuted_target"),
        Some(&json!(true))
    );
    assert_eq!(
        suite_complete.fields.get("suite_passed"),
        Some(&json!(true))
    );
}

fn events_by_name<'a>(
    events: &'a [common::tracing_capture::TracingEvent],
    name: &str,
) -> Vec<&'a common::tracing_capture::TracingEvent> {
    events.iter().filter(|event| event.name == name).collect()
}

fn event_by_name<'a>(
    events: &'a [common::tracing_capture::TracingEvent],
    name: &str,
) -> &'a common::tracing_capture::TracingEvent {
    events
        .iter()
        .find(|event| event.name == name)
        .unwrap_or_else(|| panic!("missing event {name:?}; events: {events:#?}"))
}

fn write_capture_if_requested(events: &[common::tracing_capture::TracingEvent]) {
    let Ok(path) = std::env::var("S3_FALSIFICATION_CAPTURE_EVENTS") else {
        return;
    };
    fs::write(path, events_to_ndjson(events)).expect("writes S3 falsification event capture");
}
