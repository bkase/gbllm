mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::tracing_capture::{TraceCapture, captured_events, with_trace_capture};
use gbf_experiments::s1::schema::{GitCommitId, RfcRevisionRef, S1CanonicalJson, S1SchemaError};
use gbf_experiments::s2;
use gbf_experiments::s2::ablation::write_ablation_report;
use gbf_experiments::s2::oracle_re_run::{
    ORACLE_CASE_IDS, S1_ORACLE_SUITE_VERSION, write_oracle_re_run_report,
};
use gbf_experiments::s2::report::{
    S2ReportFile, S2ReportValidator, decision_for_outcome, predictions_section_hash,
    report_self_hash, report_self_hash_from_front_matter_value, validate_report_validator,
    write_report,
};
use gbf_experiments::s2::schema::{
    DiagnosticSubcheckResult, FailureKindS2, FixtureResult, HardnessTriple, HypothesisStatus,
    LinearStateSmokeReport, LossGradFlowReport, PhaseEffectiveLambda, PhaseKindFixture,
    PhaseKindS2, PhaseTransitionIntegReport, QuantHardness, QuantHardnessOverride, RouterTrainMode,
    S2AblationReport, S2BuildKind, S2CheckpointSelfHashes, S2Completion, S2Decision, S2Hypothesis,
    S2OracleReRunReport, S2Outcome, S2PerSeedArtifacts, S2ReportFrontMatter, S2TensorMismatch,
    TrainingLossUnit, phase_transition_expected_hardness_at_boundary,
    write_linearstate_smoke_report, write_loss_grad_flow_report,
    write_phase_transition_integ_report,
};
use gbf_foundation::{Hash256, SemVer};
use proptest::prelude::*;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

#[test]
fn s2_enum_variants_round_trip_through_s1_canonical_json() {
    assert_canonical_round_trip(&PhaseKindS2::PhaseA);
    assert_canonical_round_trip(&PhaseKindS2::PhaseB);
    assert_canonical_round_trip(&PhaseKindS2::PhaseC);
    assert_canonical_round_trip(&PhaseKindS2::PhaseD);

    assert_canonical_round_trip(&PhaseKindFixture::PhaseE);
    assert_canonical_round_trip(&RouterTrainMode::NoRouter);
    assert_canonical_round_trip(&RouterTrainMode::SoftTop1);
    assert_canonical_round_trip(&RouterTrainMode::HardTop1);
    assert_canonical_round_trip(&S2BuildKind::s2_ternary_full);
    assert_canonical_round_trip(&S2BuildKind::s2_fp_full);
    assert_canonical_round_trip(&S2BuildKind::s2_ternary_nodistill);
    assert_canonical_round_trip(&S2BuildKind::s2_ablation);
    assert_canonical_round_trip(&QuantHardnessOverride::None);
    assert_canonical_round_trip(&QuantHardnessOverride::AllOff);
    assert_canonical_round_trip(&TrainingLossUnit::Nats);
    assert_canonical_round_trip(&FailureKindS2::LossGradFlow);
    assert_canonical_round_trip(&S2Outcome::PassClean);
    assert_canonical_round_trip(&S2Decision::ProceedToS3WithDistillReview);
    assert_canonical_round_trip(&S2Hypothesis::H6);
    assert_canonical_round_trip(&S2Completion::Completed);
}

#[test]
fn hardness_triple_field_order_is_stable() {
    let triple = HardnessTriple::new(QuantHardness::Off, QuantHardness::Soft, QuantHardness::Hard);
    let bytes = S1CanonicalJson::to_vec(&triple).expect("canonical S2 JSON");

    assert_eq!(
        bytes,
        br#"{"activation_qat":"soft","expert_qat":"off","norm_qat":"hard"}"#
    );
    insta::assert_snapshot!(
        "types_s2__hardness_triple_off",
        String::from_utf8(bytes).expect("utf8 JSON")
    );
}

#[test]
fn phase_effective_lambda_has_all_eight_fields_and_stable_bytes() {
    let lambdas = PhaseEffectiveLambda::phase_cd_defaults();
    let bytes = S1CanonicalJson::to_vec(&lambdas).expect("canonical S2 JSON");
    let value: Value = serde_json::from_slice(&bytes).expect("JSON");

    assert_eq!(
        value,
        json!({
            "lambda_balance": 0.0,
            "lambda_distill": 1.0,
            "lambda_overflow": 0.0,
            "lambda_range": 0.009999999776482582_f64,
            "lambda_shape": 0.0,
            "lambda_switch": 0.0,
            "lambda_zero": 0.00009999999747378752_f64,
            "lambda_zrouter": 0.0,
        })
    );
    assert_eq!(
        bytes,
        br#"{"lambda_balance":0.0,"lambda_distill":1.0,"lambda_overflow":0.0,"lambda_range":0.009999999776482582,"lambda_shape":0.0,"lambda_switch":0.0,"lambda_zero":0.00009999999747378752,"lambda_zrouter":0.0}"#
    );
    insta::assert_snapshot!(
        "types_s2__phase_effective_lambda_default",
        String::from_utf8(bytes).expect("utf8 JSON")
    );
}

#[test]
fn s2_build_kind_serializes_as_kebab_case_strings() {
    let value = json!([
        S2BuildKind::s2_ternary_full,
        S2BuildKind::s2_fp_full,
        S2BuildKind::s2_ternary_nodistill,
        S2BuildKind::s2_ablation,
    ]);
    let bytes = S1CanonicalJson::value_to_vec(&value).expect("canonical S2 JSON");

    assert_eq!(
        bytes,
        br#"["s2-ternary-full","s2-fp-full","s2-ternary-nodistill","s2-ablation"]"#
    );
    insta::assert_snapshot!(
        "types_s2__build_kind_serialization",
        String::from_utf8(bytes).expect("utf8 JSON")
    );
}

#[test]
fn hypothesis_status_prior_gate_preserves_reason_byte_equal() {
    let status = HypothesisStatus::NotEvaluatedDueToPriorGate {
        reason: "H1 refuted before H2".to_owned(),
    };
    let bytes = S1CanonicalJson::to_vec(&status).expect("canonical S2 JSON");
    let decoded: HypothesisStatus = serde_json::from_slice(&bytes).expect("round trip");
    let decoded_bytes = S1CanonicalJson::to_vec(&decoded).expect("canonical S2 JSON");

    assert_eq!(decoded, status);
    assert_eq!(decoded_bytes, bytes);
    assert_eq!(
        bytes,
        br#"{"kind":"not-evaluated-due-to-prior-gate","reason":"H1 refuted before H2"}"#
    );
}

#[test]
fn phase_effective_lambda_rejects_non_finite_and_negative_values() {
    let mut value =
        serde_json::to_value(PhaseEffectiveLambda::phase_cd_defaults()).expect("lambdas serialize");
    value["lambda_zero"] = json!(-0.0001);
    assert!(serde_json::from_value::<PhaseEffectiveLambda>(value).is_err());

    let mut invalid = PhaseEffectiveLambda::phase_cd_defaults();
    invalid.lambda_range = f32::INFINITY;
    assert!(matches!(
        S1CanonicalJson::to_vec(&invalid),
        Err(S1SchemaError::NonFiniteFloat)
    ));
    assert!(invalid.validate().is_err());
}

#[test]
fn s2_module_loaded_event_is_emitted_once() {
    let capture = TraceCapture::default();

    with_trace_capture(&capture, || {
        s2::ensure_module_loaded();
        s2::ensure_module_loaded();
    });

    let events = captured_events(&capture);
    let module_events = events
        .iter()
        .filter(|event| event.name == "s2::module_loaded")
        .collect::<Vec<_>>();
    assert_eq!(module_events.len(), 1);
    let event = module_events[0];
    assert_eq!(event.fields.get("schema_count"), Some(&json!(1)));
    assert_eq!(event.fields.get("type_count"), Some(&json!(23)));
    assert_eq!(
        event.fields.get("s2_full_enabled"),
        Some(&json!(cfg!(feature = "s2-full")))
    );
    assert_eq!(
        event.fields.get("s2_ablation_enabled"),
        Some(&json!(cfg!(feature = "s2-ablation")))
    );
}

#[test]
fn pretrain_verifier_reports_round_trip_through_canonical_json() {
    let loss = loss_grad_flow_report();
    let linear = linearstate_smoke_report();
    let phase = phase_transition_report();

    assert_report_round_trip(&loss);
    assert_report_round_trip(&linear);
    assert_report_round_trip(&phase);

    insta::assert_snapshot!(
        "loss_grad_flow_s2__all_five_pass",
        String::from_utf8(loss.canonical_json_bytes().expect("loss canonical JSON")).unwrap()
    );
    insta::assert_snapshot!(
        "loss_grad_flow_s2__h5_4b_subcheck",
        String::from_utf8(
            S1CanonicalJson::to_vec(&fixture_result("H5.4", "lambda_zero"))
                .expect("h5.4 fixture canonical JSON")
        )
        .unwrap()
    );
    insta::assert_snapshot!(
        "linearstate_smoke_s2__fixture_v1",
        String::from_utf8(
            linear
                .canonical_json_bytes()
                .expect("linear canonical JSON")
        )
        .unwrap()
    );
    insta::assert_snapshot!(
        "phase_transition_integ_s2__five_phase_fixture",
        String::from_utf8(phase.canonical_json_bytes().expect("phase canonical JSON")).unwrap()
    );
}

#[test]
fn loss_grad_flow_invariants_reject_non_default_false_and_missing_h5_4b() {
    let mut fixture = fixture_result("H5.3", "lambda_range");
    fixture.non_default_value_used = false;
    fixture.sub_passed = false;
    assert!(fixture.validate().is_err());

    let mut h5_4 = fixture_result("H5.4", "lambda_zero");
    h5_4.diagnostic_subchecks.clear();
    h5_4.sub_passed = true;
    assert!(h5_4.validate().is_err());
}

#[test]
fn loss_grad_flow_lgf1_requires_five_ordered_fixtures() {
    let mut report = loss_grad_flow_report();
    report.fixtures.pop();
    report.overall_passed = true;
    assert!(report.validate().is_err());

    let mut report = loss_grad_flow_report();
    report.fixtures.swap(0, 1);
    assert!(report.validate().is_err());
}

#[test]
fn loss_grad_flow_lgf3_requires_overall_and_of_fixture_verdicts() {
    let mut report = loss_grad_flow_report();
    report.overall_passed = false;
    assert!(report.validate().is_err());

    let report = loss_grad_flow_report();
    assert!(report.validate().is_ok());
    assert_eq!(
        report.overall_passed,
        report.fixtures.iter().all(|fixture| fixture.sub_passed)
    );
}

#[test]
fn loss_grad_flow_lgf4_requires_h5_5_exact_teacher_logits_stop_gradient() {
    let mut h5_5 = fixture_result("H5.5", "lambda_distill");
    assert!(h5_5.validate().is_ok());

    h5_5.stop_gradient_grad_norms.remove("teacher_logits");
    assert!(h5_5.validate().is_err());

    let mut h5_5 = fixture_result("H5.5", "lambda_distill");
    h5_5.stop_gradient_grad_norms.remove("teacher_logits");
    h5_5.detached_grad_absence
        .insert("teacher_logits".to_owned(), true);
    assert!(h5_5.validate().is_ok());

    let mut h5_5 = fixture_result("H5.5", "lambda_distill");
    h5_5.stop_gradient_grad_norms
        .insert("teacher_logits".to_owned(), 1.0e-7);
    assert!(h5_5.validate().is_err());
}

#[test]
fn loss_grad_flow_detached_absence_true_requires_absent_or_exact_zero_stop_gradient() {
    let mut h5_5 = fixture_result("H5.5", "lambda_distill");
    h5_5.detached_grad_absence
        .insert("teacher_logits".to_owned(), true);
    h5_5.stop_gradient_grad_norms
        .insert("teacher_logits".to_owned(), 0.0);
    assert!(h5_5.validate().is_ok());

    h5_5.stop_gradient_grad_norms
        .insert("teacher_logits".to_owned(), 1.0e-7);
    h5_5.sub_passed = false;
    assert!(h5_5.validate().is_err());
}

#[test]
fn loss_grad_flow_lgf5_requires_h5_4_zero_weight_raw_honesty_subcheck() {
    let h5_4 = fixture_result("H5.4", "lambda_zero");
    assert!(h5_4.validate().is_ok());

    let mut wrong_lambda = h5_4.clone();
    let subcheck = wrong_lambda
        .diagnostic_subchecks
        .iter_mut()
        .find(|subcheck| subcheck.name == "lambda_zero_raw_honesty_at_zero_weight")
        .expect("h5.4 subcheck");
    subcheck.lambda_value = 0.5;
    assert!(wrong_lambda.validate().is_err());
}

#[test]
fn linearstate_smoke_positive_fixture_satisfies_ls1_through_ls5() {
    let report = linearstate_smoke_report();
    assert!(report.forward_finite);
    assert!(report.param_grad_norms.values().all(|value| *value > 0.0));
    assert!(report.input_grad_norm > 0.0);
    assert!(report.determinism_byte_equal);
    assert_eq!(
        report.smoke_passed,
        report.forward_finite
            && report.param_grad_norms.values().all(|value| *value > 0.0)
            && report.input_grad_norm > 0.0
            && report.determinism_byte_equal
    );
    assert!(report.validate().is_ok());
}

#[test]
fn linearstate_smoke_ls1_through_ls5_reject_mismatched_invariants() {
    let mut report = linearstate_smoke_report();
    report.forward_finite = false;
    report.smoke_passed = true;
    assert!(report.validate().is_err());

    let mut report = linearstate_smoke_report();
    report
        .param_grad_norms
        .insert("linear_state.recurrent".to_owned(), 0.0);
    report.smoke_passed = true;
    assert!(report.validate().is_err());

    let mut report = linearstate_smoke_report();
    report.input_grad_norm = 0.0;
    report.smoke_passed = true;
    assert!(report.validate().is_err());

    let mut report = linearstate_smoke_report();
    report.determinism_byte_equal = false;
    report.smoke_passed = true;
    assert!(report.validate().is_err());

    let mut report = linearstate_smoke_report();
    report.smoke_passed = false;
    assert!(report.validate().is_err());
}

#[test]
fn linearstate_smoke_requires_fixture_v1_dimensions() {
    let mut report = linearstate_smoke_report();
    report.seq_len = 7;
    assert!(report.validate().is_err());
}

#[test]
fn linearstate_smoke_ls2_skips_explicitly_inactive_parameters() {
    let mut report = linearstate_smoke_report();
    report
        .param_grad_norms
        .insert("linear_state.recurrent".to_owned(), 0.0);
    report
        .inactive_parameters
        .insert("linear_state.recurrent".to_owned());
    assert!(report.validate().is_ok());

    report.inactive_parameters = BTreeSet::new();
    assert!(report.validate().is_err());
}

#[test]
fn phase_transition_integ_positive_fixture_satisfies_pt1_through_pt7() {
    let report = phase_transition_report();
    assert_eq!(report.transition_event_count, 4);
    assert_eq!(report.teacher_freeze_event_count, 1);
    assert_eq!(
        report.hardness_at_boundary,
        phase_transition_expected_hardness_at_boundary()
    );
    assert!(report.skip_phase_test_passed);
    assert!(report.overlap_phase_error_raised);
    assert!(report.empty_phase_error_raised);
    assert_eq!(
        report.integ_passed,
        report.skip_phase_test_passed
            && report.overlap_phase_error_raised
            && report.empty_phase_error_raised
    );
    assert!(report.validate().is_ok());
}

#[test]
fn phase_transition_integ_pt1_through_pt7_reject_mismatched_invariants() {
    let mut report = phase_transition_report();
    report.fixture_phase_boundaries = vec![0, 10, 20, 30, 40];
    report.integ_passed = true;
    assert!(report.validate().is_err());

    let mut report = phase_transition_report();
    report.transition_event_count = 5;
    assert!(report.validate().is_err());

    let mut report = phase_transition_report();
    report.teacher_freeze_event_count = 0;
    assert!(report.validate().is_err());

    let mut report = phase_transition_report();
    report.hardness_at_boundary.insert(
        "20".to_owned(),
        HardnessTriple::new(
            QuantHardness::Hard,
            QuantHardness::Soft,
            QuantHardness::Soft,
        ),
    );
    assert!(report.validate().is_err());

    let mut report = phase_transition_report();
    report.hardness_at_boundary.remove("20");
    assert!(report.validate().is_err());

    let mut report = phase_transition_report();
    report.skip_phase_test_passed = false;
    report.integ_passed = true;
    assert!(report.validate().is_err());

    let mut report = phase_transition_report();
    report.overlap_phase_error_raised = false;
    report.integ_passed = true;
    assert!(report.validate().is_err());

    let mut report = phase_transition_report();
    report.empty_phase_error_raised = false;
    report.integ_passed = true;
    assert!(report.validate().is_err());

    let mut report = phase_transition_report();
    report.integ_passed = false;
    assert!(report.validate().is_err());
}

#[test]
fn phase_transition_integ_pt7_allows_truthful_negative_fixture_result() {
    let mut report = phase_transition_report();
    report.transition_event_count = 3;
    report.integ_passed = false;
    assert!(report.validate().is_ok());
}

#[test]
fn pretrain_verifier_emitters_write_canonical_json_and_logs() {
    let temp = tempfile::tempdir().expect("tempdir");
    let loss_path = temp.path().join("loss-grad-flow.json");
    let linear_path = temp.path().join("linearstate-smoke.json");
    let phase_path = temp.path().join("phase-transition.json");
    let loss = loss_grad_flow_report();
    let linear = linearstate_smoke_report();
    let phase = phase_transition_report();
    let capture = TraceCapture::default();

    with_trace_capture(&capture, || {
        write_loss_grad_flow_report(&loss_path, &loss).expect("loss writer");
        write_linearstate_smoke_report(&linear_path, &linear).expect("linear writer");
        write_phase_transition_integ_report(&phase_path, &phase).expect("phase writer");
    });

    assert_eq!(
        std::fs::read(&loss_path).expect("loss file"),
        S1CanonicalJson::to_vec(&loss).expect("loss canonical JSON")
    );
    assert_eq!(
        std::fs::read(&linear_path).expect("linear file"),
        S1CanonicalJson::to_vec(&linear).expect("linear canonical JSON")
    );
    assert_eq!(
        std::fs::read(&phase_path).expect("phase file"),
        S1CanonicalJson::to_vec(&phase).expect("phase canonical JSON")
    );

    let events = captured_events(&capture);
    assert_eq!(event_count(&events, "s2_loss_grad_flow_writer_open"), 1);
    assert_eq!(
        event_count(&events, "s2_linearstate_grad_smoke_writer_open"),
        1
    );
    assert_eq!(
        event_count(&events, "s2_phase_transition_integration_writer_open"),
        1
    );
    assert_eq!(event_count(&events, "grad_flow_fixture_emit"), 5);
    assert_eq!(event_count(&events, "loss_grad_flow_finalized"), 1);
    assert_eq!(event_count(&events, "linearstate_smoke_finalized"), 1);
    assert_eq!(event_count(&events, "phase_transition_integ_finalized"), 1);
    let phase_finalized = events
        .iter()
        .find(|event| event.name == "phase_transition_integ_finalized")
        .expect("phase finalized event");
    assert_eq!(
        phase_finalized.fields.get("transition_event_count"),
        Some(&json!(phase.transition_event_count))
    );
    assert_eq!(
        phase_finalized.fields.get("teacher_freeze_event_count"),
        Some(&json!(phase.teacher_freeze_event_count))
    );
    assert_eq!(
        phase_finalized.fields.get("integ_self_hash"),
        Some(&json!(phase.integ_self_hash.to_string()))
    );
}

#[test]
fn linearstate_smoke_writer_logs_error_event_on_invariant_failure() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("linearstate-smoke.json");
    let mut report = linearstate_smoke_report();
    report
        .param_grad_norms
        .insert("linear_state.recurrent".to_owned(), 0.0);
    report.smoke_passed = true;
    let capture = TraceCapture::default();

    with_trace_capture(&capture, || {
        assert!(write_linearstate_smoke_report(&path, &report).is_err());
    });

    let events = captured_events(&capture);
    let event = events
        .iter()
        .find(|event| event.name == "linearstate_smoke_invariant_failed")
        .expect("linearstate invariant failure event");
    assert_eq!(event.level, "ERROR");
    assert_eq!(event.fields.get("ls_id"), Some(&json!("LS-2")));
    assert_eq!(
        event.fields.get("parameter"),
        Some(&json!("linear_state.recurrent"))
    );
    let observed = event
        .fields
        .get("observed")
        .and_then(Value::as_str)
        .and_then(|observed| serde_json::from_str::<Value>(observed).ok())
        .expect("observed JSON");
    assert_eq!(observed["observed_grad_norm"], json!(0.0));
}

#[test]
fn phase_transition_writer_logs_error_event_on_invariant_failure() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("phase-transition.json");
    let mut report = phase_transition_report();
    report.hardness_at_boundary.insert(
        "20".to_owned(),
        HardnessTriple::new(
            QuantHardness::Hard,
            QuantHardness::Soft,
            QuantHardness::Soft,
        ),
    );
    report.integ_passed = true;
    let capture = TraceCapture::default();

    with_trace_capture(&capture, || {
        assert!(write_phase_transition_integ_report(&path, &report).is_err());
    });

    let events = captured_events(&capture);
    let event = events
        .iter()
        .find(|event| event.name == "phase_transition_integ_invariant_failed")
        .expect("phase-transition invariant failure event");
    assert_eq!(event.level, "ERROR");
    assert_eq!(event.fields.get("pt_id"), Some(&json!("PT-3")));
    assert_eq!(event.fields.get("boundary"), Some(&json!(20)));
    let observed = event
        .fields
        .get("observed")
        .and_then(Value::as_str)
        .and_then(|observed| serde_json::from_str::<Value>(observed).ok())
        .expect("observed JSON");
    assert_eq!(
        observed["hardness_at_boundary"]["20"]["expert_qat"],
        json!("hard")
    );
}

#[test]
fn closure_artifacts_round_trip_through_canonical_json() {
    let ablation = s2_ablation_report(true).expect("ablation report");
    let oracle = s2_oracle_re_run_report(true).expect("oracle report");
    let report = s2_report().expect("s2 report");

    assert_report_round_trip(&ablation);
    assert_report_round_trip(&oracle);
    assert_canonical_round_trip(&report.front_matter);
    assert_eq!(
        report.front_matter.report_self_hash,
        report_self_hash(&report.front_matter, &report.body).expect("report self hash")
    );

    insta::assert_snapshot!(
        "ablation_s2__seed0_payload_match",
        String::from_utf8(S1CanonicalJson::to_vec(&ablation).expect("ablation canonical JSON"))
            .expect("utf8 JSON")
    );
    insta::assert_snapshot!(
        "oracle_re_run_s2__all_cases_pass",
        String::from_utf8(S1CanonicalJson::to_vec(&oracle).expect("oracle canonical JSON"))
            .expect("utf8 JSON")
    );
    insta::assert_snapshot!(
        "report_s2__pass_clean_canonical",
        report.to_markdown().expect("report markdown")
    );

    let mut warn_front_matter = report.front_matter.clone();
    warn_front_matter.s2_outcome = S2Outcome::PassWithDistillWarn;
    warn_front_matter.decision = S2Decision::ProceedToS3WithDistillReview;
    warn_front_matter
        .hypothesis_statuses
        .insert(S2Hypothesis::H3, HypothesisStatus::Refuted);
    warn_front_matter.report_self_hash =
        report_self_hash(&warn_front_matter, &report.body).expect("warn report self hash");
    let warn = S2ReportFile {
        front_matter: warn_front_matter,
        body: report.body.clone(),
    };
    insta::assert_snapshot!(
        "report_s2__pass_with_distill_warn",
        warn.to_markdown().expect("warn report markdown")
    );
}

#[test]
fn ablation_ab1_ab2_and_oracle_or1_or2_invariants_are_checked() {
    let mut ablation = s2_ablation_report(true).expect("ablation report");
    ablation.phase_a_eq_ablation = false;
    assert!(ablation.validate().is_err());

    let ablation_mismatch = s2_ablation_report(false).expect("mismatch ablation report");
    assert!(ablation_mismatch.validate().is_ok());
    assert!(ablation_mismatch.validate_closure().is_err());

    let mut missing_mismatch = ablation_mismatch.clone();
    missing_mismatch.first_mismatch = None;
    assert!(missing_mismatch.validate().is_err());

    let mut wrong_seed = s2_ablation_report(true).expect("ablation report");
    wrong_seed.seed = 1;
    assert!(wrong_seed.validate().is_err());

    let oracle = s2_oracle_re_run_report(true).expect("oracle report");
    assert!(oracle.validate_closure().is_ok());
    let oracle_hash = oracle.oracle_re_run_self_hash;
    let report = s2_report().expect("s2 report");
    assert_eq!(report.front_matter.oracle_re_run_self_hash, oracle_hash);

    let oracle_failed = s2_oracle_re_run_report(false).expect("failed oracle report");
    assert!(oracle_failed.validate().is_ok());
    assert!(oracle_failed.validate_closure().is_err());

    let mut forged_suite = oracle.clone();
    forged_suite.s1_oracle_suite_version = "fake".to_owned();
    assert!(forged_suite.validate().is_err());

    let mut forged_cases = oracle;
    forged_cases.oracle_cases = vec!["fake".to_owned()];
    assert!(forged_cases.validate().is_err());
}

#[test]
fn report_validators_accept_complete_report_and_reject_each_failure_mode() {
    let report = s2_report().expect("s2 report");
    for validator in [
        S2ReportValidator::Decision,
        S2ReportValidator::AllSeeds,
        S2ReportValidator::ClosureArtifacts,
        S2ReportValidator::SelfHash,
        S2ReportValidator::Predictions,
        S2ReportValidator::AllHypotheses,
    ] {
        validate_report_validator(&report, validator).expect("positive R-S2 validator");
    }

    let mut invalid = report.clone();
    invalid.front_matter.decision = S2Decision::Halt {
        reason: "wrong-tag".to_owned(),
    };
    assert!(validate_report_validator(&invalid, S2ReportValidator::Decision).is_err());

    let mut invalid = report.clone();
    invalid.front_matter.per_seed_artifacts.pop();
    invalid.front_matter.report_self_hash =
        report_self_hash(&invalid.front_matter, &invalid.body).expect("rehash");
    assert!(validate_report_validator(&invalid, S2ReportValidator::AllSeeds).is_err());

    let mut invalid = report.clone();
    invalid.front_matter.ablation_self_hash = None;
    invalid.front_matter.report_self_hash =
        report_self_hash(&invalid.front_matter, &invalid.body).expect("rehash");
    assert!(validate_report_validator(&invalid, S2ReportValidator::ClosureArtifacts).is_err());

    let mut invalid = report.clone();
    invalid.front_matter.phase_transition_integ_passed = false;
    invalid.front_matter.report_self_hash =
        report_self_hash(&invalid.front_matter, &invalid.body).expect("rehash");
    assert!(validate_report_validator(&invalid, S2ReportValidator::ClosureArtifacts).is_err());

    let mut invalid = report.clone();
    invalid.body.push_str("\nextra byte");
    assert!(validate_report_validator(&invalid, S2ReportValidator::SelfHash).is_err());

    let mut invalid = report.clone();
    invalid.front_matter.predictions_commit = invalid.front_matter.first_result_commit.clone();
    invalid.front_matter.report_self_hash =
        report_self_hash(&invalid.front_matter, &invalid.body).expect("rehash");
    assert!(validate_report_validator(&invalid, S2ReportValidator::Predictions).is_err());

    let mut invalid = report.clone();
    invalid
        .front_matter
        .hypothesis_statuses
        .remove(&S2Hypothesis::H6);
    invalid.front_matter.report_self_hash =
        report_self_hash(&invalid.front_matter, &invalid.body).expect("rehash");
    assert!(validate_report_validator(&invalid, S2ReportValidator::AllHypotheses).is_err());

    let mut invalid = report.clone();
    invalid
        .front_matter
        .hypothesis_statuses
        .insert(S2Hypothesis::H1, HypothesisStatus::Refuted);
    invalid.front_matter.report_self_hash =
        report_self_hash(&invalid.front_matter, &invalid.body).expect("rehash");
    assert!(validate_report_validator(&invalid, S2ReportValidator::AllHypotheses).is_err());

    let mut warn = report.clone();
    warn.front_matter.s2_outcome = S2Outcome::PassWithDistillWarn;
    warn.front_matter.decision = S2Decision::ProceedToS3WithDistillReview;
    warn.front_matter
        .hypothesis_statuses
        .insert(S2Hypothesis::H3, HypothesisStatus::Refuted);
    warn.front_matter.report_self_hash =
        report_self_hash(&warn.front_matter, &warn.body).expect("rehash");
    validate_report_validator(&warn, S2ReportValidator::AllHypotheses)
        .expect("H3 refuted is explicit distill-review path");
}

#[test]
fn report_self_hash_ignores_front_matter_key_order_and_generated_at() {
    let report = s2_report().expect("s2 report");
    let value = serde_json::to_value(&report.front_matter).expect("front matter JSON");
    let mut reordered = serde_json::Map::new();
    for (key, value) in value.as_object().expect("object").iter().rev() {
        reordered.insert(key.clone(), value.clone());
    }
    let reordered = Value::Object(reordered);

    assert_eq!(
        report_self_hash_from_front_matter_value(&value, &report.body).expect("value report hash"),
        report_self_hash_from_front_matter_value(&reordered, &report.body)
            .expect("reordered report hash")
    );

    let mut changed_time = value.clone();
    changed_time["generated_at"] = json!("2030-01-01T00:00:00Z");
    assert_eq!(
        report_self_hash_from_front_matter_value(&value, &report.body).expect("value report hash"),
        report_self_hash_from_front_matter_value(&changed_time, &report.body)
            .expect("time-independent report hash")
    );
}

#[test]
fn closure_emitters_write_canonical_artifacts_and_logs() {
    let temp = tempfile::tempdir().expect("tempdir");
    let ablation_path = temp.path().join("ablation-report.json");
    let oracle_path = temp.path().join("oracle-result.json");
    let report_path = temp.path().join("S2-report.md");
    let ablation = s2_ablation_report(true).expect("ablation report");
    let oracle = s2_oracle_re_run_report(true).expect("oracle report");
    let report = s2_report().expect("s2 report");
    let capture = TraceCapture::default();

    with_trace_capture(&capture, || {
        write_ablation_report(&ablation_path, &ablation).expect("ablation writer");
        write_oracle_re_run_report(&oracle_path, &oracle).expect("oracle writer");
        write_report(&report_path, &report).expect("report writer");
    });

    assert_eq!(
        std::fs::read(&ablation_path).expect("ablation file"),
        S1CanonicalJson::to_vec(&ablation).expect("ablation canonical JSON")
    );
    assert_eq!(
        std::fs::read(&oracle_path).expect("oracle file"),
        S1CanonicalJson::to_vec(&oracle).expect("oracle canonical JSON")
    );
    assert_eq!(
        std::fs::read_to_string(&report_path).expect("report file"),
        report.to_markdown().expect("report markdown")
    );

    let events = captured_events(&capture);
    assert!(
        events
            .iter()
            .any(|event| event.name == "ablation_comparator_run")
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.name == "oracle_re_run_persisted")
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.name == "r_s2_validator_run")
            .count(),
        6
    );
    assert!(events.iter().all(|event| {
        event.name != "r_s2_validator_run"
            || event
                .fields
                .get("diagnostic")
                .and_then(|value| value.as_str())
                .is_some_and(|value| {
                    value == "null" || serde_json::from_str::<Value>(value).is_ok()
                })
    }));
    assert_eq!(
        events
            .iter()
            .filter(|event| event.name == "s2_report_finalized")
            .count(),
        1
    );
}

proptest! {
    #[test]
    fn loss_grad_flow_report_round_trips_and_lgf3_tracks_fixture_verdicts(
        passed in prop::array::uniform5(any::<bool>())
    ) {
        let report = LossGradFlowReport::new(vec![
            fixture_result_with_verdict("H5.1", "lambda_zrouter", passed[0]),
            fixture_result_with_verdict("H5.2", "lambda_balance", passed[1]),
            fixture_result_with_verdict("H5.3", "lambda_range", passed[2]),
            fixture_result_with_verdict("H5.4", "lambda_zero", passed[3]),
            fixture_result_with_verdict("H5.5", "lambda_distill", passed[4]),
        ]).expect("loss-grad-flow report");

        prop_assert_eq!(
            report.overall_passed,
            report.fixtures.iter().all(|fixture| fixture.sub_passed)
        );
        let bytes = S1CanonicalJson::to_vec(&report).expect("canonical JSON");
        let decoded: LossGradFlowReport = serde_json::from_slice(&bytes).expect("round trip");
        prop_assert_eq!(S1CanonicalJson::to_vec(&decoded).expect("canonical JSON"), bytes);
    }

    #[test]
    fn fixture_result_round_trips_actual_schema_shape(
        fixture in arb_schema_fixture_result()
    ) {
        fixture.validate().expect("generated fixture is schema-valid");
        let bytes = S1CanonicalJson::to_vec(&fixture).expect("canonical JSON");
        let decoded: FixtureResult = serde_json::from_slice(&bytes).expect("round trip");
        prop_assert_eq!(S1CanonicalJson::to_vec(&decoded).expect("canonical JSON"), bytes);
    }

    #[test]
    fn diagnostic_subcheck_round_trips_actual_schema_shape(
        subcheck in arb_schema_diagnostic_subcheck()
    ) {
        subcheck.validate().expect("generated diagnostic subcheck is schema-valid");
        let bytes = S1CanonicalJson::to_vec(&subcheck).expect("canonical JSON");
        let decoded: DiagnosticSubcheckResult = serde_json::from_slice(&bytes).expect("round trip");
        prop_assert_eq!(S1CanonicalJson::to_vec(&decoded).expect("canonical JSON"), bytes);
    }

    #[test]
    fn linearstate_smoke_report_round_trips_and_ls5_tracks_invariant_conjunction(
        forward_finite in any::<bool>(),
        param_alive in any::<bool>(),
        input_alive in any::<bool>(),
        determinism_byte_equal in any::<bool>(),
    ) {
        let mut report = linearstate_smoke_report();
        report.forward_finite = forward_finite;
        if !param_alive {
            report
                .param_grad_norms
                .insert("linear_state.recurrent".to_owned(), 0.0);
        }
        if !input_alive {
            report.input_grad_norm = 0.0;
        }
        report.determinism_byte_equal = determinism_byte_equal;
        report.smoke_passed = report.forward_finite
            && report.param_grad_norms.values().all(|value| value.is_finite() && *value > 0.0)
            && report.input_grad_norm.is_finite()
            && report.input_grad_norm > 0.0
            && report.determinism_byte_equal;
        report = report.with_computed_self_hash().expect("linearstate smoke report");

        prop_assert_eq!(
            report.smoke_passed,
            report.forward_finite
                && report.param_grad_norms.values().all(|value| value.is_finite() && *value > 0.0)
                && report.input_grad_norm.is_finite()
                && report.input_grad_norm > 0.0
                && report.determinism_byte_equal
        );
        let bytes = S1CanonicalJson::to_vec(&report).expect("canonical JSON");
        let decoded: LinearStateSmokeReport = serde_json::from_slice(&bytes).expect("round trip");
        prop_assert_eq!(S1CanonicalJson::to_vec(&decoded).expect("canonical JSON"), bytes);
    }

    #[test]
    fn s2_ablation_report_round_trips_and_ab1_tracks_payload_equality(equal in any::<bool>()) {
        let report = s2_ablation_report(equal).expect("ablation report");

        prop_assert_eq!(
            report.phase_a_eq_ablation,
            report.s2_ternary_tensor_payload_sha == report.s2_ablation_tensor_payload_sha
        );
        prop_assert_eq!(report.validate_closure().is_ok(), equal);
        let bytes = S1CanonicalJson::to_vec(&report).expect("canonical JSON");
        let decoded: S2AblationReport = serde_json::from_slice(&bytes).expect("round trip");
        prop_assert_eq!(S1CanonicalJson::to_vec(&decoded).expect("canonical JSON"), bytes);
    }

    #[test]
    fn s2_oracle_re_run_report_round_trips_and_or1_tracks_verdict(passed in any::<bool>()) {
        let report = s2_oracle_re_run_report(passed).expect("oracle report");

        prop_assert_eq!(report.validate_closure().is_ok(), passed);
        prop_assert_eq!(report.s1_oracle_suite_version.as_str(), S1_ORACLE_SUITE_VERSION);
        let expected_cases = ORACLE_CASE_IDS
            .iter()
            .map(|case| (*case).to_owned())
            .collect::<Vec<_>>();
        prop_assert_eq!(report.oracle_cases.as_slice(), expected_cases.as_slice());
        let bytes = S1CanonicalJson::to_vec(&report).expect("canonical JSON");
        let decoded: S2OracleReRunReport = serde_json::from_slice(&bytes).expect("round trip");
        prop_assert_eq!(S1CanonicalJson::to_vec(&decoded).expect("canonical JSON"), bytes);
    }

    #[test]
    fn s2_report_self_hash_is_idempotent_for_prediction_payload(seed in 0u8..=32) {
        let predictions = format!("H2 ternary-full gap remains <= {} bpc.", seed);
        let report = s2_report_with_predictions(&predictions).expect("s2 report");
        let recomputed = report_self_hash(&report.front_matter, &report.body)
            .expect("report self hash");

        prop_assert_eq!(report.front_matter.report_self_hash, recomputed);
        let markdown = report.to_markdown().expect("report markdown");
        let reparsed_front_matter = serde_json::to_value(&report.front_matter)
            .expect("front matter JSON");
        prop_assert!(markdown.contains(&predictions));
        prop_assert_eq!(
            report_self_hash_from_front_matter_value(&reparsed_front_matter, &report.body)
                .expect("value report hash"),
            recomputed
        );
    }
}

fn assert_canonical_round_trip<T>(value: &T)
where
    T: Clone + Serialize + DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let bytes = S1CanonicalJson::to_vec(value).expect("canonical S2 JSON");
    let decoded: T = serde_json::from_slice(&bytes).expect("round trip");
    let decoded_bytes = S1CanonicalJson::to_vec(&decoded).expect("canonical S2 JSON");
    assert_eq!(&decoded, value);
    assert_eq!(decoded_bytes, bytes);
}

fn assert_report_round_trip<T>(report: &T)
where
    T: Clone + Serialize + DeserializeOwned + PartialEq + std::fmt::Debug,
{
    assert_canonical_round_trip(report);
}

fn event_count(events: &[common::tracing_capture::TracingEvent], name: &str) -> usize {
    events.iter().filter(|event| event.name == name).count()
}

fn arb_schema_diagnostic_subcheck() -> impl Strategy<Value = DiagnosticSubcheckResult> {
    (
        "[a-z][a-z0-9_]{0,31}",
        0.0_f32..=4.0,
        prop::option::of(0.0_f32..=16.0),
        any::<bool>(),
        any::<bool>(),
    )
        .prop_map(
            |(name, lambda_value, weighted_loss_value, raw_loss_computed, raw_loss_finite)| {
                let passed = raw_loss_computed && raw_loss_finite;
                DiagnosticSubcheckResult {
                    name,
                    lambda_value,
                    raw_loss_computed,
                    raw_loss_finite,
                    weighted_loss_value,
                    passed,
                }
            },
        )
}

fn arb_schema_fixture_result() -> impl Strategy<Value = FixtureResult> {
    prop_oneof![
        Just(("H5.1", "lambda_zrouter")),
        Just(("H5.2", "lambda_balance")),
        Just(("H5.3", "lambda_range")),
        Just(("H5.4", "lambda_zero")),
        Just(("H5.5", "lambda_distill")),
    ]
    .prop_flat_map(|(sub_hypothesis, loss_term)| {
        (
            Just(sub_hypothesis),
            Just(loss_term),
            0.000_001_f32..=8.0,
            0.0_f32..=1.0e-7,
            any::<bool>(),
            any::<bool>(),
            arb_schema_diagnostic_subcheck(),
        )
    })
    .prop_map(
        |(
            sub_hypothesis,
            loss_term,
            in_scope_norm,
            stop_norm,
            numerical_stability_passed,
            h5_5_teacher_absent,
            diagnostic_subcheck,
        )| {
            let mut in_scope_grad_norms = BTreeMap::new();
            in_scope_grad_norms.insert(format!("{loss_term}_target"), in_scope_norm);
            let mut stop_gradient_grad_norms = BTreeMap::new();
            stop_gradient_grad_norms.insert(format!("{loss_term}_detached"), stop_norm);
            let mut detached_grad_absence = BTreeMap::new();
            if sub_hypothesis == "H5.5" {
                if h5_5_teacher_absent {
                    detached_grad_absence.insert("teacher_logits".to_owned(), true);
                } else {
                    stop_gradient_grad_norms.insert("teacher_logits".to_owned(), 0.0);
                    detached_grad_absence.insert("teacher_logits".to_owned(), false);
                }
            }
            let mut diagnostic_subchecks = vec![diagnostic_subcheck];
            if sub_hypothesis == "H5.4" {
                diagnostic_subchecks.push(DiagnosticSubcheckResult {
                    name: "lambda_zero_raw_honesty_at_zero_weight".to_owned(),
                    lambda_value: 0.0,
                    raw_loss_computed: true,
                    raw_loss_finite: true,
                    weighted_loss_value: Some(0.0),
                    passed: true,
                });
            }
            let sub_passed = numerical_stability_passed
                && diagnostic_subchecks.iter().all(|subcheck| subcheck.passed)
                && in_scope_grad_norms.values().all(|value| *value > 0.0)
                && stop_gradient_grad_norms
                    .values()
                    .all(|value| *value <= 1.0e-6);
            FixtureResult {
                sub_hypothesis: sub_hypothesis.to_owned(),
                loss_term: loss_term.to_owned(),
                in_scope_grad_norms,
                stop_gradient_grad_norms,
                non_default_value_used: true,
                numerical_stability_passed,
                diagnostic_subchecks,
                detached_grad_absence,
                sub_passed,
            }
        },
    )
}

fn loss_grad_flow_report() -> LossGradFlowReport {
    LossGradFlowReport::new(vec![
        fixture_result("H5.1", "lambda_zrouter"),
        fixture_result("H5.2", "lambda_balance"),
        fixture_result("H5.3", "lambda_range"),
        fixture_result("H5.4", "lambda_zero"),
        fixture_result("H5.5", "lambda_distill"),
    ])
    .expect("loss-grad-flow report")
}

fn fixture_result(sub_hypothesis: &str, loss_term: &str) -> FixtureResult {
    let mut in_scope_grad_norms = BTreeMap::new();
    in_scope_grad_norms.insert(format!("{loss_term}_target"), 0.25);
    let mut stop_gradient_grad_norms = BTreeMap::new();
    stop_gradient_grad_norms.insert(format!("{loss_term}_detached"), 0.0);
    if sub_hypothesis == "H5.5" {
        stop_gradient_grad_norms.insert("teacher_logits".to_owned(), 0.0);
    }
    let mut detached_grad_absence = BTreeMap::new();
    detached_grad_absence.insert(format!("{loss_term}_teacher"), sub_hypothesis == "H5.5");
    let mut diagnostic_subchecks = vec![DiagnosticSubcheckResult {
        name: format!("{loss_term}_finite_raw"),
        lambda_value: 0.5,
        raw_loss_computed: true,
        raw_loss_finite: true,
        weighted_loss_value: Some(0.125),
        passed: true,
    }];
    if sub_hypothesis == "H5.4" {
        diagnostic_subchecks.push(DiagnosticSubcheckResult {
            name: "lambda_zero_raw_honesty_at_zero_weight".to_owned(),
            lambda_value: 0.0,
            raw_loss_computed: true,
            raw_loss_finite: true,
            weighted_loss_value: Some(0.0),
            passed: true,
        });
    }
    FixtureResult {
        sub_hypothesis: sub_hypothesis.to_owned(),
        loss_term: loss_term.to_owned(),
        in_scope_grad_norms,
        stop_gradient_grad_norms,
        non_default_value_used: true,
        numerical_stability_passed: true,
        diagnostic_subchecks,
        detached_grad_absence,
        sub_passed: true,
    }
}

fn fixture_result_with_verdict(
    sub_hypothesis: &str,
    loss_term: &str,
    passed: bool,
) -> FixtureResult {
    let mut fixture = fixture_result(sub_hypothesis, loss_term);
    if !passed {
        fixture.numerical_stability_passed = false;
        fixture.sub_passed = false;
    }
    fixture
}

fn linearstate_smoke_report() -> LinearStateSmokeReport {
    let mut param_grad_norms = BTreeMap::new();
    param_grad_norms.insert("linear_state.input".to_owned(), 0.5);
    param_grad_norms.insert("linear_state.recurrent".to_owned(), 0.25);
    LinearStateSmokeReport::new(param_grad_norms, 0.75).expect("linearstate smoke")
}

fn phase_transition_report() -> PhaseTransitionIntegReport {
    PhaseTransitionIntegReport::new(
        4,
        1,
        phase_transition_expected_hardness_at_boundary(),
        true,
        true,
        true,
    )
    .expect("phase transition report")
}

fn s2_ablation_report(equal: bool) -> Result<S2AblationReport, S1SchemaError> {
    let ternary_payload = hash(3);
    let ablation_payload = if equal { ternary_payload } else { hash(4) };
    let mismatch = (!equal).then(|| S2TensorMismatch {
        tensor: "toy0.linear.weight".to_owned(),
        byte_offset: 12,
    });
    S2AblationReport::new(
        0,
        hash(1),
        hash(2),
        ternary_payload,
        ablation_payload,
        mismatch,
    )
}

fn s2_oracle_re_run_report(passed: bool) -> Result<S2OracleReRunReport, S1SchemaError> {
    S2OracleReRunReport::new(
        S1_ORACLE_SUITE_VERSION,
        passed,
        ORACLE_CASE_IDS
            .iter()
            .map(|case| (*case).to_owned())
            .collect(),
    )
}

fn s2_report() -> Result<S2ReportFile, Box<dyn std::error::Error>> {
    s2_report_with_predictions("H2 ternary-full gap remains <= 0.5 bpc.")
}

fn s2_report_with_predictions(
    predictions: &str,
) -> Result<S2ReportFile, Box<dyn std::error::Error>> {
    let body = s2_report_body(predictions);
    let mut hypothesis_statuses = BTreeMap::new();
    for hypothesis in [
        S2Hypothesis::H1,
        S2Hypothesis::H2,
        S2Hypothesis::H3,
        S2Hypothesis::H4,
        S2Hypothesis::H5,
        S2Hypothesis::H6,
    ] {
        hypothesis_statuses.insert(hypothesis, HypothesisStatus::Confirmed);
    }
    let ablation = s2_ablation_report(true)?;
    let oracle = s2_oracle_re_run_report(true)?;
    let front_matter = S2ReportFrontMatter {
        schema: "s2_report.v1".to_owned(),
        s2_outcome: S2Outcome::PassClean,
        decision: decision_for_outcome(S2Outcome::PassClean),
        baseline_self_hash_carried_from_s1: hash(5),
        oracle_re_run_passed: true,
        oracle_re_run_self_hash: oracle.oracle_re_run_self_hash,
        api_drift_check_passed: true,
        qat_public_api_snapshot_hash: hash(6),
        linearstate_public_api_snapshot_hash: hash(7),
        per_seed_artifacts: s2_per_seed_artifacts(),
        ablation_self_hash: Some(ablation.ablation_self_hash),
        loss_grad_flow_self_hash: hash(8),
        linearstate_smoke_self_hash: hash(9),
        phase_transition_integ_self_hash: hash(10),
        phase_transition_integ_passed: true,
        falsification_s2_passed: true,
        falsification_s2_suite_hash: hash(11),
        generated_at: "2026-05-10T00:00:00Z".to_owned(),
        rfc_revision: RfcRevisionRef::GitCommitId(commit('a')),
        predictions_section_hash: predictions_section_hash(predictions)?,
        predictions_commit: commit('1'),
        first_result_commit: commit('2'),
        hypothesis_statuses,
        pass_version_s2: SemVer::new(0, 1, 0),
        report_self_hash: Hash256::ZERO,
    };
    Ok(S2ReportFile::new(front_matter, body)?)
}

fn s2_per_seed_artifacts() -> Vec<S2PerSeedArtifacts> {
    let mut rows = Vec::new();
    for (build_index, build_kind) in [
        S2BuildKind::s2_ternary_full,
        S2BuildKind::s2_fp_full,
        S2BuildKind::s2_ternary_nodistill,
    ]
    .into_iter()
    .enumerate()
    {
        for seed in 0..5 {
            let fill = 20 + (build_index as u8 * 20) + seed as u8;
            rows.push(S2PerSeedArtifacts {
                seed,
                build_kind,
                completion: S2Completion::Completed,
                checkpoint_self_hashes: S2CheckpointSelfHashes {
                    phase_a: Some(hash(fill)),
                    phase_b: Some(hash(fill + 1)),
                    phase_c: Some(hash(fill + 2)),
                    final_checkpoint: Some(hash(fill + 3)),
                },
                phase_log_self_hash: Some(hash(fill + 4)),
                score_self_hash: Some(hash(fill + 5)),
                distill_log_self_hash: (build_kind != S2BuildKind::s2_ternary_nodistill)
                    .then(|| hash(fill + 6)),
            });
        }
    }
    rows
}

fn s2_report_body(predictions: &str) -> String {
    format!(
        "# S2 Report\n\n\
         ## Pre-registered predictions\n\n\
         {predictions}\n\n\
         ## Observed\n\n\
         All five seeds completed for all three report build kinds.\n\n\
         ## Hypothesis verdicts\n\n\
         H1 through H6 are explicit in front matter.\n\n\
         ## Falsification analysis\n\n\
         No falsification rule fired.\n\n\
         ## Surprises\n\n\
         None.\n\n\
         ## Decision\n\n\
         `ProceedToS3`. Closure candidate.\n\n\
         ## Reproducibility statement\n\n\
         `cargo run -p gbf-experiments -- s2 replay` with pass_version_S2 0.1.0.\n"
    )
}

fn hash(fill: u8) -> Hash256 {
    Hash256::from_bytes([fill; 32])
}

fn commit(fill: char) -> GitCommitId {
    GitCommitId::new(fill.to_string().repeat(40)).expect("valid test commit")
}
