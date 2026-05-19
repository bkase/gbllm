#![cfg(feature = "s3")]

mod common;

use common::tracing_capture::{TraceCapture, captured_events, with_trace_capture};
use gbf_experiments::s3::oracle_re_run::{
    PASS_VERSION_S3, S1_D7_METRIC_IDS, S3_ORACLE_RE_RUN_LOG_TARGET, s3_oracle_re_run,
};
use serde_json::json;

#[test]
fn oracle_re_run_logging_s3() {
    let capture = TraceCapture::default();
    let report = with_trace_capture(&capture, s3_oracle_re_run).expect("S3 oracle re-run succeeds");
    let events = captured_events(&capture);

    let started = find_event(&events, "s3::oracle_re_run::run_started");
    assert_eq!(
        started.fields.get("binary_pass_version"),
        Some(&json!(PASS_VERSION_S3))
    );
    assert_eq!(
        started.fields.get("s1_oracle_count"),
        Some(&json!(S1_D7_METRIC_IDS.len() as u64))
    );
    assert_eq!(
        started.fields.get("s2_oracle_count"),
        Some(&json!(S1_D7_METRIC_IDS.len() as u64))
    );

    let metric_events = events
        .iter()
        .filter(|event| event.name == "s3::oracle_re_run::metric_evaluated")
        .collect::<Vec<_>>();
    assert_eq!(metric_events.len(), S1_D7_METRIC_IDS.len());
    for event in metric_events {
        assert_eq!(event.fields.get("s1_baseline"), Some(&json!(1.0)));
        assert_eq!(event.fields.get("s2_baseline"), Some(&json!(1.0)));
        assert_eq!(event.fields.get("s3_observed"), Some(&json!(1.0)));
        assert_eq!(event.fields.get("delta_vs_s1"), Some(&json!(0.0)));
        assert_eq!(event.fields.get("delta_vs_s2"), Some(&json!(0.0)));
        assert_eq!(event.fields.get("tolerance"), Some(&json!(0.0)));
        assert_eq!(event.fields.get("passed"), Some(&json!(true)));
    }

    let completed = find_event(&events, "s3::oracle_re_run::run_complete");
    assert_eq!(
        completed.fields.get("s1_oracle_re_run_passed"),
        Some(&json!(true))
    );
    assert_eq!(
        completed.fields.get("s2_oracle_re_run_passed"),
        Some(&json!(true))
    );
    assert_eq!(
        completed.fields.get("oracle_re_run_self_hash"),
        Some(&json!(report.oracle_re_run_self_hash.to_string()))
    );

    assert!(events.iter().any(|event| {
        event.name == "s3::oracle_re_run::run_started"
            && event.fields.get("event_name") == Some(&json!("s3::oracle_re_run::run_started"))
    }));
    let _target_pin = S3_ORACLE_RE_RUN_LOG_TARGET;
}

fn find_event<'a>(
    events: &'a [common::tracing_capture::TracingEvent],
    name: &str,
) -> &'a common::tracing_capture::TracingEvent {
    events
        .iter()
        .find(|event| event.name == name)
        .unwrap_or_else(|| {
            panic!(
                "missing event {name}; saw {:?}",
                events.iter().map(|event| &event.name).collect::<Vec<_>>()
            )
        })
}
