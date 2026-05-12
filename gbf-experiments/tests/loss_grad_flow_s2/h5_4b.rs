use gbf_experiments::s2::loss_grad_flow::{
    H5_4B_SUBCHECK_NAME, h5_4_fixture_with_zero_raw_honesty, h5_4b_short_circuit_detected_subcheck,
    run_h5_4b_zero_raw_honesty_subcheck,
};
use gbf_experiments::s2::schema::FixtureResult;
use serde_json::json;

use crate::common::tracing_capture::{TraceCapture, captured_events, with_trace_capture};

#[test]
fn h5_4b_honest_zero_loss_subcheck_passes_and_logs() {
    let capture = TraceCapture::default();
    let subcheck =
        with_trace_capture(&capture, run_h5_4b_zero_raw_honesty_subcheck).expect("h5.4b");

    assert_eq!(subcheck.name, H5_4B_SUBCHECK_NAME);
    assert_eq!(subcheck.lambda_value, 0.0);
    assert!(subcheck.raw_loss_computed);
    assert!(subcheck.raw_loss_finite);
    assert_eq!(subcheck.weighted_loss_value, Some(0.0));
    assert!(subcheck.passed);
    subcheck.validate().expect("subcheck validates");
    insta::with_settings!({prepend_module_to_snapshot => false, snapshot_path => "../snapshots"}, {
        insta::assert_snapshot!(
            "h5_4b__honest_raw_computation",
            serde_json::to_string_pretty(&subcheck).expect("snapshot JSON")
        );
    });

    let events = captured_events(&capture);
    let event = events
        .iter()
        .find(|event| event.name == "h5_4b_subcheck_run")
        .expect("h5.4b event");
    assert_eq!(event.fields.get("lambda_value"), Some(&json!(0.0)));
    assert_eq!(event.fields.get("raw_loss_computed"), Some(&json!(true)));
    assert_eq!(event.fields.get("weighted_loss_value"), Some(&json!(0.0)));
    assert_eq!(event.fields.get("passed"), Some(&json!(true)));
}

#[test]
fn h5_4_fixture_attaches_diagnostic_without_becoming_sixth_fixture() {
    let fixture = h5_4_fixture_with_zero_raw_honesty().expect("h5.4 fixture");

    assert_eq!(fixture.sub_hypothesis, "H5.4");
    assert_eq!(fixture.loss_term, "lambda_zero");
    assert_eq!(fixture.diagnostic_subchecks.len(), 1);
    assert_eq!(fixture.diagnostic_subchecks[0].name, H5_4B_SUBCHECK_NAME);
    assert!(fixture.non_default_value_used);
    fixture.validate().expect("h5.4 fixture validates");
}

#[test]
fn h5_4b_short_circuit_subcheck_fails_and_logs_remediation() {
    let capture = TraceCapture::default();
    let subcheck = with_trace_capture(&capture, h5_4b_short_circuit_detected_subcheck);

    assert_eq!(subcheck.name, H5_4B_SUBCHECK_NAME);
    assert!(!subcheck.raw_loss_computed);
    assert!(!subcheck.raw_loss_finite);
    assert!(!subcheck.passed);
    assert!(subcheck.validate().is_ok());

    let events = captured_events(&capture);
    let event = events
        .iter()
        .find(|event| event.name == "h5_4b_short_circuit_detected")
        .expect("short-circuit event");
    assert_eq!(
        event.fields.get("remediation"),
        Some(&json!(
            "zero_loss helper must invoke L1 sum even at lambda=0; see CLAUDE.md 'Training Loss Beads' raw-helper rule"
        ))
    );
}

#[test]
fn h5_4_missing_zero_raw_honesty_subcheck_fails_lgf5() {
    let mut fixture = h5_4_fixture_with_zero_raw_honesty().expect("h5.4 fixture");
    fixture.diagnostic_subchecks.clear();

    assert!(fixture.validate().is_err());
}

#[test]
fn h5_4b_zero_lambda_does_not_fail_lgf2_for_parent_fixture() {
    let fixture = h5_4_fixture_with_zero_raw_honesty().expect("h5.4 fixture");

    assert_eq!(fixture.diagnostic_subchecks[0].lambda_value, 0.0);
    assert!(fixture.non_default_value_used);
    fixture
        .validate()
        .expect("zero diagnostic exempt from LGF-2");
}

#[test]
fn h5_4b_wrong_lambda_fails_lgf5() {
    let mut fixture: FixtureResult = h5_4_fixture_with_zero_raw_honesty().expect("h5.4 fixture");
    fixture.diagnostic_subchecks[0].lambda_value = 0.5;

    assert!(fixture.validate().is_err());
}
