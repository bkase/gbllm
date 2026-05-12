mod common;

use common::tracing_capture::{TraceCapture, captured_events, with_trace_capture};
use gbf_experiments::s1::oracle::MetricOracleResults;
use gbf_experiments::s1::schema::S1CanonicalJson;
use gbf_experiments::s2::oracle_re_run::{
    ORACLE_CASE_IDS, S1_ORACLE_SUITE_VERSION, emit_s2_oracle_re_run_from_results,
    run_s1_oracle_re_run_under_s2_binary,
};
use gbf_experiments::s2::schema::S2OracleReRunReport;
use serde_json::json;

#[test]
fn oracle_re_run_real_s1_suite_passes_under_s2_binary() {
    let capture = TraceCapture::default();
    let report =
        with_trace_capture(&capture, run_s1_oracle_re_run_under_s2_binary).expect("oracle re-run");

    assert_eq!(report.schema, "s2_oracle_re_run.v1");
    assert_eq!(report.s1_oracle_suite_version, S1_ORACLE_SUITE_VERSION);
    assert!(report.metric_oracle_passed);
    assert_eq!(
        report.oracle_cases,
        ORACLE_CASE_IDS
            .iter()
            .map(|case| (*case).to_owned())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        report.oracle_re_run_self_hash,
        report
            .computed_self_hash()
            .expect("oracle re-run self hash")
    );
    let events = captured_events(&capture);
    assert!(
        events
            .iter()
            .any(|event| event.name == "oracle_re_run_start")
    );
    assert!(
        events
            .iter()
            .any(|event| event.name == "oracle_re_run_finalized"
                && event.fields.get("metric_oracle_passed") == Some(&json!(true)))
    );
    insta::assert_snapshot!(
        "oracle_re_run__all_pass",
        String::from_utf8(S1CanonicalJson::to_vec(&report).expect("canonical oracle JSON"))
            .expect("utf8 JSON")
    );
}

#[test]
fn oracle_re_run_mocked_failure_records_failed_case() {
    let capture = TraceCapture::default();
    let report = with_trace_capture(&capture, || {
        emit_s2_oracle_re_run_from_results(MetricOracleResults {
            o_metric_0: true,
            o_metric_1: true,
            o_metric_2: false,
            o_metric_3: true,
            o_metric_4: true,
        })
    })
    .expect("oracle re-run");

    assert!(!report.metric_oracle_passed);
    let events = captured_events(&capture);
    assert!(events.iter().any(|event| {
        event.name == "oracle_case_failed"
            && event.fields.get("case") == Some(&json!("O-metric-2"))
            && event
                .fields
                .get("expected")
                .and_then(|value| value.as_str())
                .is_some_and(|value| value.contains(r#""case":"O-metric-2""#))
            && event
                .fields
                .get("observed")
                .and_then(|value| value.as_str())
                .is_some_and(|value| value.contains(r#""case":"O-metric-2""#))
    }));
}

#[test]
fn oracle_re_run_self_hash_is_deterministic_and_round_trips() {
    let first_capture = TraceCapture::default();
    let first = with_trace_capture(&first_capture, run_s1_oracle_re_run_under_s2_binary)
        .expect("first oracle re-run");
    let second_capture = TraceCapture::default();
    let second = with_trace_capture(&second_capture, run_s1_oracle_re_run_under_s2_binary)
        .expect("second oracle re-run");
    let bytes = S1CanonicalJson::to_vec(&first).expect("canonical oracle JSON");
    let decoded: S2OracleReRunReport = serde_json::from_slice(&bytes).expect("round trip");

    assert_eq!(
        first.oracle_re_run_self_hash,
        second.oracle_re_run_self_hash
    );
    assert_eq!(decoded, first);
    assert_eq!(
        S1CanonicalJson::to_vec(&decoded).expect("decoded canonical JSON"),
        bytes
    );
    assert_eq!(
        oracle_case_event_verdicts(&captured_events(&first_capture)),
        oracle_case_event_verdicts(&captured_events(&second_capture))
    );
}

#[test]
fn oracle_re_run_schema_rejects_forged_suite_or_case_ids() {
    let mut wrong_suite = run_s1_oracle_re_run_under_s2_binary().expect("oracle re-run");
    wrong_suite.s1_oracle_suite_version = "fake-suite".to_owned();
    assert!(wrong_suite.validate().is_err());

    let mut wrong_cases = run_s1_oracle_re_run_under_s2_binary().expect("oracle re-run");
    wrong_cases.oracle_cases = vec!["fake-case".to_owned()];
    assert!(wrong_cases.validate().is_err());
}

fn oracle_case_event_verdicts(
    events: &[common::tracing_capture::TracingEvent],
) -> Vec<(String, bool)> {
    events
        .iter()
        .filter(|event| event.name == "oracle_case_invoked")
        .map(|event| {
            (
                event
                    .fields
                    .get("case")
                    .and_then(|value| value.as_str())
                    .expect("case field")
                    .to_owned(),
                event
                    .fields
                    .get("passed")
                    .and_then(|value| value.as_bool())
                    .expect("passed field"),
            )
        })
        .collect()
}
