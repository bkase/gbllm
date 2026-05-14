mod common;

use std::path::PathBuf;

use gbf_experiments::s2::report::{
    S2ReportFile, S2ReportInputs, S2ReportValidator, emit_s2_report,
    report_self_hash_from_front_matter_value, validate_report_validator,
};
use gbf_experiments::s2::schema::{
    HypothesisStatus, S2BuildKind, S2CheckpointSelfHashes, S2Completion, S2Decision, S2Hypothesis,
    S2Outcome, S2PerSeedArtifacts, S2VerifierBundle,
};
use gbf_foundation::{Hash256, SemVer};
use serde_json::Value;

use crate::common::helpers::tracing_capture_s2::capture_events;

#[test]
fn clean_inputs_emit_report_and_pass_all_r_s2_validators() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut inputs = report_inputs(temp.path().join("S2-report.md"));

    let (emitted, events) =
        capture_events(|| emit_s2_report(&inputs).expect("clean S2 report emits"));

    assert_eq!(emitted.report.front_matter.s2_outcome, S2Outcome::PassClean);
    assert_eq!(
        emitted.report.front_matter.decision,
        S2Decision::ProceedToS3
    );
    for validator in all_validators() {
        validate_report_validator(&emitted.report, validator).expect("validator passes");
    }
    assert_eq!(
        std::fs::read_to_string(&emitted.path).expect("report file"),
        emitted.markdown
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.name == "report_written")
            .count(),
        1
    );
    insta::assert_snapshot!("report_emitter__pass_clean", emitted.report.body);

    inputs.generated_at = "2026-05-12T01:02:03Z".to_owned();
    let emitted_again = emit_s2_report(&inputs).expect("same report with changed generated_at");
    assert_eq!(
        emitted.report.front_matter.report_self_hash,
        emitted_again.report.front_matter.report_self_hash,
        "generated_at is intentionally omitted from R-S2-Self-Hash"
    );
    assert_eq!(
        markdown_without_generated_at(&emitted.markdown),
        markdown_without_generated_at(&emitted_again.markdown),
        "the emitter is byte-identical modulo the generated_at timestamp"
    );
}

#[test]
fn pass_with_distill_warn_is_rendered_and_validated() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut inputs = report_inputs(temp.path().join("S2-report.md"));
    inputs
        .verifier_bundle
        .hypothesis_statuses
        .insert(S2Hypothesis::H3, HypothesisStatus::Refuted);

    let emitted = emit_s2_report(&inputs).expect("distill warning report emits");

    assert_eq!(
        emitted.report.front_matter.s2_outcome,
        S2Outcome::PassWithDistillWarn
    );
    assert_eq!(
        emitted.report.front_matter.decision,
        S2Decision::ProceedToS3WithDistillReview
    );
    validate_report_validator(&emitted.report, S2ReportValidator::AllHypotheses)
        .expect("H3 refuted is valid only on distill-review decision");
    insta::assert_snapshot!(
        "report_emitter__pass_with_distill_warn",
        emitted.report.body
    );
}

#[test]
fn missing_nodistill_checkpoint_fails_closure_artifact_validator() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut inputs = report_inputs(temp.path().join("S2-report.md"));
    let row = inputs
        .per_seed_artifacts
        .iter_mut()
        .find(|row| row.build_kind == S2BuildKind::s2_ternary_nodistill && row.seed == 0)
        .expect("nodistill seed row");
    row.checkpoint_self_hashes.final_checkpoint = None;

    let error = emit_s2_report(&inputs).expect_err("missing closure artifact is rejected");

    assert!(
        error.to_string().contains("R-S2-ClosureArtifacts failed"),
        "{error}"
    );
    assert!(
        error.to_string().contains("checkpoint_self_hashes"),
        "{error}"
    );
}

#[test]
fn r_s2_validator_negative_controls_cover_public_validator_surface() {
    let mut report = clean_report();
    report.front_matter.decision = S2Decision::Halt {
        reason: "test-decision-mismatch".to_owned(),
    };
    assert_validator_error_contains(&report, S2ReportValidator::Decision, "R-S2-Decision failed");

    let mut report = clean_report();
    report.front_matter.per_seed_artifacts.pop();
    assert_validator_error_contains(
        &report,
        S2ReportValidator::AllSeeds,
        "missing s2-ternary-nodistill seed 4",
    );

    let mut report = clean_report();
    report_row_mut(&mut report, S2BuildKind::s2_ternary_full, 0).build_kind =
        S2BuildKind::s2_ablation;
    assert_validator_error_contains(
        &report,
        S2ReportValidator::AllSeeds,
        "unexpected report build kind",
    );

    let mut report = clean_report();
    let duplicate_row = report_row_mut(&mut report, S2BuildKind::s2_ternary_full, 0).clone();
    report.front_matter.per_seed_artifacts.push(duplicate_row);
    assert_validator_error_contains(
        &report,
        S2ReportValidator::AllSeeds,
        "duplicate s2-ternary-full seed 0",
    );

    let mut report = clean_report();
    report.front_matter.ablation_self_hash = None;
    assert_validator_error_contains(
        &report,
        S2ReportValidator::ClosureArtifacts,
        "missing ablation_self_hash",
    );

    let mut report = clean_report();
    report.front_matter.report_self_hash = hash(99);
    assert_validator_error_contains(
        &report,
        S2ReportValidator::SelfHash,
        "R-S2-Self-Hash failed",
    );

    let mut report = clean_report();
    report.body = report.body.replace("<= 0.5 bpc", "<= 0.4 bpc");
    assert_validator_error_contains(
        &report,
        S2ReportValidator::Predictions,
        "expected predictions_section_hash",
    );

    let mut report = clean_report();
    report.front_matter.first_result_commit = report.front_matter.predictions_commit.clone();
    assert_validator_error_contains(
        &report,
        S2ReportValidator::Predictions,
        "strict ancestry is checked by scripts/s2_preregistration_check.sh",
    );

    let mut report = clean_report();
    report
        .front_matter
        .hypothesis_statuses
        .remove(&S2Hypothesis::H6);
    assert_validator_error_contains(&report, S2ReportValidator::AllHypotheses, "missing H6");

    let mut report = clean_report();
    report.front_matter.hypothesis_statuses.insert(
        S2Hypothesis::H6,
        HypothesisStatus::NotEvaluatedDueToPriorGate {
            reason: "prior gate".to_owned(),
        },
    );
    assert_validator_error_contains(
        &report,
        S2ReportValidator::AllHypotheses,
        "NotEvaluatedDueToPriorGate",
    );
}

#[test]
fn closure_artifacts_negative_controls_cover_gates_and_per_row_hashes() {
    let mut report = clean_report();
    report.front_matter.oracle_re_run_passed = false;
    assert_validator_error_contains(
        &report,
        S2ReportValidator::ClosureArtifacts,
        "oracle_re_run_passed",
    );

    let mut report = clean_report();
    report.front_matter.api_drift_check_passed = false;
    assert_validator_error_contains(
        &report,
        S2ReportValidator::ClosureArtifacts,
        "api_drift_check_passed",
    );

    let mut report = clean_report();
    report.front_matter.phase_transition_integ_passed = false;
    assert_validator_error_contains(
        &report,
        S2ReportValidator::ClosureArtifacts,
        "phase_transition_integ_passed",
    );

    let mut report = clean_report();
    report.front_matter.falsification_s2_passed = false;
    assert_validator_error_contains(
        &report,
        S2ReportValidator::ClosureArtifacts,
        "falsification_s2_passed",
    );

    let mut report = clean_report();
    report_row_mut(&mut report, S2BuildKind::s2_ternary_full, 0).completion =
        S2Completion::NotReached;
    assert_validator_error_contains(
        &report,
        S2ReportValidator::ClosureArtifacts,
        "completion=Completed",
    );

    let mut report = clean_report();
    report_row_mut(&mut report, S2BuildKind::s2_ternary_full, 0).completion =
        S2Completion::DivergedAt { step: 17 };
    assert_validator_error_contains(
        &report,
        S2ReportValidator::ClosureArtifacts,
        "completion=Completed",
    );

    let mut report = clean_report();
    report_row_mut(&mut report, S2BuildKind::s2_ternary_full, 0).phase_log_self_hash = None;
    assert_validator_error_contains(
        &report,
        S2ReportValidator::ClosureArtifacts,
        "phase_log_self_hash",
    );

    let mut report = clean_report();
    report_row_mut(&mut report, S2BuildKind::s2_ternary_full, 0).score_self_hash = None;
    assert_validator_error_contains(
        &report,
        S2ReportValidator::ClosureArtifacts,
        "score_self_hash",
    );

    let mut report = clean_report();
    report_row_mut(&mut report, S2BuildKind::s2_ternary_full, 0).distill_log_self_hash = None;
    assert_validator_error_contains(
        &report,
        S2ReportValidator::ClosureArtifacts,
        "distill_log_self_hash",
    );
}

#[test]
fn all_hypotheses_rejects_refuted_closure_hypotheses_outside_h3_distill_warning() {
    let mut report = clean_report();
    report
        .front_matter
        .hypothesis_statuses
        .insert(S2Hypothesis::H1, HypothesisStatus::Refuted);

    assert_validator_error_contains(
        &report,
        S2ReportValidator::AllHypotheses,
        "closure-candidate H1 is Refuted",
    );

    let mut report = clean_report();
    report
        .front_matter
        .hypothesis_statuses
        .insert(S2Hypothesis::H3, HypothesisStatus::Refuted);

    assert_validator_error_contains(
        &report,
        S2ReportValidator::AllHypotheses,
        "closure-candidate H3 is Refuted",
    );
}

#[test]
fn report_self_hash_ignores_front_matter_key_order_but_not_other_fields() {
    let temp = tempfile::tempdir().expect("tempdir");
    let emitted = emit_s2_report(&report_inputs(temp.path().join("S2-report.md")))
        .expect("clean S2 report emits");
    let value = serde_json::to_value(&emitted.report.front_matter).expect("front matter value");
    let rendered_value = front_matter_value_from_markdown(&emitted.markdown);

    assert_eq!(
        rendered_value, value,
        "front matter is rendered as canonical JSON inside markdown delimiters"
    );
    assert_eq!(
        report_self_hash_from_front_matter_value(&rendered_value, &emitted.report.body)
            .expect("rendered front matter self hash"),
        emitted.report.front_matter.report_self_hash
    );

    let mut reordered = serde_json::Map::new();
    for (key, value) in value.as_object().expect("front matter object").iter().rev() {
        reordered.insert(key.clone(), value.clone());
    }

    assert_eq!(
        report_self_hash_from_front_matter_value(&value, &emitted.report.body).expect("self hash"),
        report_self_hash_from_front_matter_value(&Value::Object(reordered), &emitted.report.body)
            .expect("reordered self hash")
    );

    let mut changed = value.clone();
    changed["pass_version_S2"] = serde_json::json!("9.9.9");
    assert_ne!(
        report_self_hash_from_front_matter_value(&value, &emitted.report.body).expect("self hash"),
        report_self_hash_from_front_matter_value(&changed, &emitted.report.body)
            .expect("changed self hash")
    );
}

fn report_inputs(output_path: PathBuf) -> S2ReportInputs {
    S2ReportInputs {
        output_path,
        baseline_self_hash_carried_from_s1: hash(5),
        oracle_re_run_self_hash: hash(6),
        qat_public_api_snapshot_hash: hash(7),
        linearstate_public_api_snapshot_hash: hash(8),
        per_seed_artifacts: per_seed_artifacts(),
        ablation_self_hash: Some(hash(9)),
        loss_grad_flow_self_hash: hash(10),
        linearstate_smoke_self_hash: hash(11),
        phase_transition_integ_self_hash: hash(12),
        falsification_s2_suite_hash: hash(13),
        generated_at: "2026-05-12T00:00:00Z".to_owned(),
        rfc_revision: gbf_experiments::s1::schema::RfcRevisionRef::GitCommitId(commit('a')),
        predictions_commit: commit('1'),
        first_result_commit: commit('2'),
        pass_version_s2: SemVer::new(0, 1, 0),
        verifier_bundle: S2VerifierBundle::closure_candidate(),
        predictions_markdown: "H2 ternary-full gap remains <= 0.5 bpc.".to_owned(),
        observed_markdown: "All five seeds completed for all three report build kinds.".to_owned(),
        falsification_analysis: "F1 through F6 broken-S2 controls did not pass.".to_owned(),
        surprises: "None.".to_owned(),
        decision_justification: "Closure candidate satisfies all R-S2 validators.".to_owned(),
        replay_command: "gbf s2 replay-full --fixture tiny".to_owned(),
        manifest_references: "tiny fixture manifests and S2 environment hash.".to_owned(),
    }
}

fn per_seed_artifacts() -> Vec<S2PerSeedArtifacts> {
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

fn all_validators() -> [S2ReportValidator; 6] {
    [
        S2ReportValidator::Decision,
        S2ReportValidator::AllSeeds,
        S2ReportValidator::ClosureArtifacts,
        S2ReportValidator::SelfHash,
        S2ReportValidator::Predictions,
        S2ReportValidator::AllHypotheses,
    ]
}

fn clean_report() -> S2ReportFile {
    let temp = tempfile::tempdir().expect("tempdir");
    emit_s2_report(&report_inputs(temp.path().join("S2-report.md")))
        .expect("clean S2 report emits")
        .report
}

fn report_row_mut(
    report: &mut S2ReportFile,
    build_kind: S2BuildKind,
    seed: u64,
) -> &mut S2PerSeedArtifacts {
    report
        .front_matter
        .per_seed_artifacts
        .iter_mut()
        .find(|row| row.build_kind == build_kind && row.seed == seed)
        .expect("report row")
}

fn assert_validator_error_contains(
    report: &S2ReportFile,
    validator: S2ReportValidator,
    needle: &str,
) {
    let error = validate_report_validator(report, validator).expect_err("validator should fail");
    assert!(
        error.to_string().contains(needle),
        "expected {needle:?} in {error}"
    );
}

fn front_matter_value_from_markdown(markdown: &str) -> Value {
    let front_raw = markdown
        .strip_prefix("---\n")
        .expect("opening front matter marker")
        .split_once("\n---\n")
        .expect("closing front matter marker")
        .0;
    serde_json::from_str(front_raw).expect("canonical JSON front matter")
}

fn markdown_without_generated_at(markdown: &str) -> String {
    markdown
        .replace("2026-05-12T00:00:00Z", "<generated_at>")
        .replace("2026-05-12T01:02:03Z", "<generated_at>")
}

fn hash(fill: u8) -> Hash256 {
    Hash256::from_bytes([fill; 32])
}

fn commit(fill: char) -> gbf_experiments::s1::schema::GitCommitId {
    gbf_experiments::s1::schema::GitCommitId::new(fill.to_string().repeat(40))
        .expect("valid test commit")
}
