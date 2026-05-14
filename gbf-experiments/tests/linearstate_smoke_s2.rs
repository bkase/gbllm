mod common;

use common::tracing_capture::{TraceCapture, captured_events, with_trace_capture};
use gbf_experiments::s1::rng::{Pcg64Mcg, S1Rng, seed128};
use gbf_experiments::s1::schema::S1CanonicalJson;
use gbf_experiments::s2::linearstate_smoke::{
    DECLARED_ACTIVE_PARAMETERS, FIXTURE_BATCH, FIXTURE_DECAY, FIXTURE_HIDDEN_DIM, FIXTURE_SEQ_LEN,
    INPUT_PROJECTION_WEIGHT, LINEARSTATE_INPUT_RNG_DOMAIN, LINEARSTATE_PARAMS_RNG_DOMAIN,
    LINEARSTATE_SMOKE_RNG_SEED, LinearStateSmokeError, LinearStateSmokeMode,
    LinearStateSmokeParameter, STATE_READOUT_OUTPUT_PROJECTION_WEIGHT, declared_active_parameters,
    fixture_block, fixture_input_values, fixture_projection_weights, run_fixture_v1,
    run_fixture_v1_with_mode,
};
use gbf_experiments::s2::schema::write_linearstate_smoke_report;
use serde_json::json;
use std::collections::BTreeSet;

#[test]
fn fixture_v1_forward_gradients_and_determinism_pass() {
    let run = run_fixture_v1().expect("linearstate smoke run");
    let report = &run.report;

    assert_eq!(report.fixture_id, "FIXTURE_V1");
    assert_eq!(report.seq_len, FIXTURE_SEQ_LEN as u64);
    assert_eq!(report.hidden_dim, FIXTURE_HIDDEN_DIM as u64);
    assert_eq!(report.batch, FIXTURE_BATCH as u64);
    assert_eq!(FIXTURE_DECAY, 0.5);
    assert!(report.forward_finite);
    assert!(report.determinism_byte_equal);
    assert!(report.smoke_passed);
    assert!(report.input_grad_norm > 0.0);
    assert_eq!(run.run_1_bytes, run.run_2_bytes);
    assert_declared_active_grad_nonzero(report, INPUT_PROJECTION_WEIGHT);
    assert_declared_active_grad_nonzero(report, STATE_READOUT_OUTPUT_PROJECTION_WEIGHT);
    assert!(report.validate().is_ok());
}

#[test]
fn fixture_v1_uses_rfc_linearstate_smoke_rng_domains() {
    assert_eq!(
        LINEARSTATE_INPUT_RNG_DOMAIN,
        "linearstate_smoke/linearstate_input_v1"
    );
    assert_eq!(
        LINEARSTATE_PARAMS_RNG_DOMAIN,
        "linearstate_smoke/linearstate_params_v1"
    );
    assert_eq!(LINEARSTATE_SMOKE_RNG_SEED, 0);

    let input_values = fixture_input_values();
    assert_eq!(input_values.len(), FIXTURE_SEQ_LEN * FIXTURE_HIDDEN_DIM);
    assert_eq!(input_values, fixture_input_values());
    assert!(input_values.iter().all(|value| value.is_finite()));
    assert_eq!(
        input_values[0].to_bits(),
        expected_first_input_value().to_bits()
    );

    let input_projection = fixture_projection_weights(LinearStateSmokeParameter::InputProjection);
    let state_readout =
        fixture_projection_weights(LinearStateSmokeParameter::StateReadoutOutputProjection);
    assert_eq!(
        input_projection.len(),
        FIXTURE_HIDDEN_DIM * FIXTURE_HIDDEN_DIM
    );
    assert_eq!(state_readout.len(), FIXTURE_HIDDEN_DIM * FIXTURE_HIDDEN_DIM);
    assert_ne!(input_projection, state_readout);
    assert_eq!(
        input_projection[0].to_bits(),
        expected_first_input_projection_value().to_bits()
    );
    assert_eq!(
        state_readout[0].to_bits(),
        expected_first_state_readout_value().to_bits()
    );
}

#[test]
fn declared_active_parameter_set_is_explicit_projection_scope() {
    assert_eq!(
        declared_active_parameters(),
        &[
            INPUT_PROJECTION_WEIGHT,
            STATE_READOUT_OUTPUT_PROJECTION_WEIGHT
        ]
    );
    assert_eq!(DECLARED_ACTIVE_PARAMETERS.len(), 2);

    let run = run_fixture_v1().expect("linearstate smoke run");
    let reported: BTreeSet<_> = run
        .report
        .param_grad_norms
        .keys()
        .map(String::as_str)
        .collect();
    let declared: BTreeSet<_> = declared_active_parameters().iter().copied().collect();
    assert_eq!(reported, declared);
    assert!(run.report.inactive_parameters.is_empty());
}

#[test]
fn fixture_v1_snapshot_pins_passing_report() {
    let run = run_fixture_v1().expect("linearstate smoke run");
    let bytes = S1CanonicalJson::to_vec(&run.report).expect("canonical report");

    insta::assert_snapshot!(
        "linearstate_smoke__fixture_v1_passing",
        String::from_utf8(bytes).unwrap()
    );
}

#[test]
fn fixture_v1_logs_required_events_once() {
    let capture = TraceCapture::default();
    let run = with_trace_capture(&capture, run_fixture_v1).expect("linearstate smoke run");
    assert!(run.report.smoke_passed);

    let events = captured_events(&capture);
    assert_eq!(
        events
            .iter()
            .filter(|event| event.name == "linearstate_smoke_start")
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.name == "linearstate_smoke_byte_compare")
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.name == "linearstate_smoke_finalized")
            .count(),
        1
    );
    assert!(
        events
            .iter()
            .any(|event| event.name == "linearstate_smoke_grad"
                && event.fields.get("parameter")
                    == Some(&json!(STATE_READOUT_OUTPUT_PROJECTION_WEIGHT))
                && event.fields.get("parameter_role")
                    == Some(&json!(
                        "state_readout_output_projection_full_precision_weight"
                    ))
                && event.fields.get("declared_active") == Some(&json!(true)))
    );
}

#[test]
fn structural_dead_recurrence_negative_fixture_fails_ls2_without_posthoc_mutation() {
    let capture = TraceCapture::default();
    let run = with_trace_capture(&capture, || {
        run_fixture_v1_with_mode(LinearStateSmokeMode::StructuralDeadRecurrence)
    })
    .expect("structural dead recurrence report should be emitted");

    assert!(run.report.forward_finite);
    assert!(!run.report.smoke_passed);
    assert!(run.report.input_grad_norm > 0.0);
    assert_declared_active_grad_nonzero(&run.report, INPUT_PROJECTION_WEIGHT);
    assert_eq!(
        run.report
            .param_grad_norms
            .get(STATE_READOUT_OUTPUT_PROJECTION_WEIGHT)
            .copied(),
        Some(0.0)
    );
    assert!(run.report.validate().is_ok());
    assert_eq!(run.run_1_bytes, run.run_2_bytes);

    let events = captured_events(&capture);
    let failed = events
        .iter()
        .find(|event| event.name == "linearstate_smoke_failed")
        .expect("failure event");
    assert_eq!(
        failed.fields.get("reason"),
        Some(&json!("LS-2: param 'recurrence_weight' grad_norm = 0"))
    );
    assert_eq!(
        failed.fields.get("failing_parameter"),
        Some(&json!(STATE_READOUT_OUTPUT_PROJECTION_WEIGHT))
    );
}

#[test]
fn structural_dead_recurrence_snapshot_pins_failure_report() {
    let run = run_fixture_v1_with_mode(LinearStateSmokeMode::StructuralDeadRecurrence)
        .expect("structural dead recurrence report");
    let bytes = S1CanonicalJson::to_vec(&run.report).expect("canonical report");

    insta::assert_snapshot!(
        "linearstate_smoke__fixture_v1_structural_recurrence_dead",
        String::from_utf8(bytes).unwrap()
    );
}

#[test]
fn all_zero_init_is_rejected_as_degenerate() {
    let err = fixture_block(LinearStateSmokeMode::AllZeroInit).expect_err("all-zero init rejects");

    assert!(matches!(
        err,
        LinearStateSmokeError::DegenerateInit {
            parameter: INPUT_PROJECTION_WEIGHT,
            reason: "all-zero weights",
        }
    ));
}

#[test]
fn writer_persists_smoke_report_schema() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("linearstate-smoke.json");
    let run = run_fixture_v1().expect("linearstate smoke run");

    write_linearstate_smoke_report(&path, &run.report).expect("write report");
    let persisted = std::fs::read_to_string(path).expect("persisted report");
    let value: serde_json::Value = serde_json::from_str(&persisted).expect("json report");

    assert_eq!(value["schema"], "s2_linearstate_grad_smoke.v1");
    assert_eq!(value["fixture_id"], "FIXTURE_V1");
    assert_eq!(value["smoke_passed"], true);
}

fn assert_declared_active_grad_nonzero(
    report: &gbf_experiments::s2::schema::LinearStateSmokeReport,
    parameter: &str,
) {
    let grad_norm = report
        .param_grad_norms
        .get(parameter)
        .copied()
        .unwrap_or_else(|| panic!("missing {parameter}"));
    assert!(grad_norm.is_finite(), "{parameter} grad must be finite");
    assert!(grad_norm > 0.0, "{parameter} grad must be nonzero");
}

fn expected_first_input_value() -> f32 {
    let mut rng = Pcg64Mcg::new(seed128(
        LINEARSTATE_INPUT_RNG_DOMAIN,
        LINEARSTATE_SMOKE_RNG_SEED,
    ));
    0.25 + draw_range(&mut rng, 0.0, 0.125)
}

fn expected_first_input_projection_value() -> f32 {
    let mut rng = Pcg64Mcg::new(seed128(
        LINEARSTATE_PARAMS_RNG_DOMAIN,
        LINEARSTATE_SMOKE_RNG_SEED,
    ));
    0.35 + draw_range(&mut rng, -0.045, 0.045)
}

fn expected_first_state_readout_value() -> f32 {
    let mut rng = Pcg64Mcg::new(seed128(
        LINEARSTATE_PARAMS_RNG_DOMAIN,
        LINEARSTATE_SMOKE_RNG_SEED,
    ));
    for _ in 0..(FIXTURE_HIDDEN_DIM * FIXTURE_HIDDEN_DIM) {
        let _ = draw_range(&mut rng, -0.045, 0.045);
    }
    0.35 + 0.045 + draw_range(&mut rng, -0.045, 0.045)
}

fn draw_range(rng: &mut Pcg64Mcg, lo: f32, hi: f32) -> f32 {
    lo + (hi - lo) * draw_unit_f32(rng)
}

fn draw_unit_f32(rng: &mut Pcg64Mcg) -> f32 {
    const MANTISSA_BITS: u32 = 24;
    let mantissa = rng.next_u64() >> (u64::BITS - MANTISSA_BITS);
    mantissa as f32 / (1_u32 << MANTISSA_BITS) as f32
}
