#![cfg(feature = "s4")]

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use gbf_experiments::s4::report::{
    S4GitCommitId, S4PerSeedArtifacts, S4Report, S4ReportError, S4ReportFrontMatter,
    S4ReportValidationError, S4ReportValidator, S4RfcRevisionRef, decision_for_outcome,
    emit_report, emit_report_to_path, predictions_section_hash, report_self_hash, validate_report,
    validate_report_validator, validate_s4_closure_packet,
};
use gbf_experiments::s4::schema::{
    HypothesisStatus, S4Completion, S4Decision, S4Hypothesis, S4Outcome,
};
use gbf_foundation::Hash256;
use serde_json::json;

#[test]
fn s4_report_emits_canonical_markdown_and_passes_validators() {
    let temp = tempfile::tempdir().expect("tempdir");
    let report = pass_clean_report();

    for validator in S4ReportValidator::all() {
        validate_report_validator(&report, *validator).expect("S4-R validator passes");
    }

    let markdown = emit_report(&report).expect("report emits");
    assert!(!markdown.contains(&b'\r'));
    assert!(String::from_utf8_lossy(&markdown).contains(r#""schema":"s4_report.v1""#));
    assert!(
        String::from_utf8_lossy(&markdown).contains(r#""report_self_hash":"sha256:"#),
        "report self-hash is the closure pin"
    );

    let path = temp.path().join("S4-report.md");
    let emitted = emit_report_to_path(&path, &report).expect("report writes");
    assert_eq!(emitted.markdown, markdown);
    assert_eq!(std::fs::read(&path).expect("written report"), markdown);

    let value = serde_json::to_value(&report.front_matter).expect("front matter serializes");
    assert_eq!(value["schema"], json!("s4_report.v1"));
    assert!(value["c_TS_checkpoint_self_hash"].as_str().is_some());
    assert!(value["promotion_gate_self_hash"].as_str().is_some());
    assert!(value["corpus_progression_self_hash"].as_str().is_some());
    assert!(
        value["per_seed_artifacts"][0]["oracle_agreement_self_hash"]
            .as_str()
            .is_some()
    );

    let mut changed_time = report.front_matter.clone();
    changed_time.generated_at = "2026-05-20T01:02:03Z".to_owned();
    assert_eq!(
        report_self_hash(&changed_time, &report.body).expect("hash"),
        report.front_matter.report_self_hash,
        "generated_at is informational and excluded from S4-R-Self-Hash"
    );
}

#[test]
fn s4_r_validators_reject_predictions_all_seeds_and_self_hash_mutations() {
    let mut report = pass_clean_report();
    report.body = report
        .body
        .replace(predictions_text(), "tampered prediction text");
    assert_validation_label(
        validate_report_validator(&report, S4ReportValidator::Predictions),
        "S4-R-Predictions",
    );

    let mut report = pass_clean_report();
    report.front_matter.per_seed_artifacts.pop();
    assert_validation_label(
        validate_report_validator(&report, S4ReportValidator::AllSeeds),
        "S4-R-AllSeeds",
    );

    let mut report = pass_clean_report();
    report.front_matter.report_self_hash = hash(199);
    assert_validation_label(
        validate_report_validator(&report, S4ReportValidator::SelfHash),
        "S4-R-Self-Hash",
    );

    let mut report = pass_clean_report();
    report.body = report.body.replace('\n', "\r\n");
    assert!(matches!(
        validate_report_validator(&report, S4ReportValidator::SelfHash),
        Err(S4ReportError::Validation(
            S4ReportValidationError::InvalidLineEnding
        ))
    ));

    let mut report = pass_clean_report();
    report.front_matter.first_result_commit = report.front_matter.predictions_commit.clone();
    assert!(matches!(
        validate_report_validator(&report, S4ReportValidator::Predictions),
        Err(S4ReportError::Validation(
            S4ReportValidationError::PredictionsCommitEqualsFirstResult { .. }
        ))
    ));
}

#[test]
fn s4_report_rejects_decision_hypothesis_and_closure_artifact_drift() {
    let mut report = pass_clean_report();
    report.front_matter.decision = S4Decision::Halt {
        reason: "wrong".to_owned(),
    };
    assert_validation_label(
        validate_report_validator(&report, S4ReportValidator::Decision),
        "S4-R-Decision",
    );

    let mut report = pass_clean_report();
    report
        .front_matter
        .hypothesis_statuses
        .remove(&S4Hypothesis::H7);
    assert!(matches!(
        validate_report_validator(&report, S4ReportValidator::AllHypotheses),
        Err(S4ReportError::Validation(
            S4ReportValidationError::MissingHypothesis {
                hypothesis: S4Hypothesis::H7
            }
        ))
    ));

    let mut report = pass_clean_report();
    report.front_matter.hypothesis_statuses.insert(
        S4Hypothesis::H4,
        HypothesisStatus::NotEvaluatedDueToPriorGate {
            reason: "prior gate".to_owned(),
        },
    );
    assert_validation_label(
        validate_report_validator(&report, S4ReportValidator::AllHypotheses),
        "S4-R-AllHypotheses",
    );

    let mut report = pass_clean_report();
    report.front_matter.per_seed_artifacts[0].oracle_agreement_self_hash = None;
    assert_validation_label(
        validate_report_validator(&report, S4ReportValidator::ClosureArtifacts),
        "S4-R-ClosureArtifacts",
    );
}

#[test]
fn s4_report_allows_early_failure_without_downstream_closure_artifacts() {
    let body = body_markdown();
    let predictions = predictions_text();
    let (predictions_commit, first_result_commit) = git_commit_pair();
    let mut front_matter = front_matter_for(
        S4Outcome::FailQualityOnGutenberg,
        decision_for_outcome(S4Outcome::FailQualityOnGutenberg),
        predictions_section_hash(predictions).expect("predictions hash"),
        predictions_commit,
        first_result_commit,
    );
    front_matter.corpus_progression_self_hash = None;
    front_matter.per_seed_artifacts[0].oracle_agreement_self_hash = None;

    let mut statuses = all_confirmed_hypotheses();
    statuses.insert(S4Hypothesis::H4, HypothesisStatus::Refuted);
    statuses.insert(
        S4Hypothesis::H5,
        HypothesisStatus::NotEvaluatedDueToPriorGate {
            reason: "H4 stopped oracle agreement".to_owned(),
        },
    );
    front_matter.hypothesis_statuses = statuses;

    let report = S4Report::new(front_matter, body).expect("early failure report emits");
    validate_report_validator(&report, S4ReportValidator::ClosureArtifacts)
        .expect("closure artifacts are required only for closure decisions");
}

#[test]
fn s4_report_rejects_schema_and_missing_required_sections() {
    let mut report = pass_clean_report();
    report.front_matter.schema = "s4_report.v2".to_owned();
    assert!(matches!(
        validate_report(&report),
        Err(S4ReportError::Validation(
            S4ReportValidationError::InvalidSchema {
                expected: "s4_report.v1",
                ..
            }
        ))
    ));

    let mut report = pass_clean_report();
    report.body = report.body.replace(
        "## Surprises\nNo out-of-band surprise notes for this fixture.\n\n",
        "",
    );
    assert!(matches!(
        validate_report(&report),
        Err(S4ReportError::Validation(
            S4ReportValidationError::MissingBodySection {
                heading: "## Surprises"
            }
        ))
    ));
}

#[test]
fn s4_closure_packet_linter_requires_hashes_verdicts_and_transcript() {
    let packet = closure_packet_markdown();
    validate_s4_closure_packet(&packet).expect("complete packet passes");

    let missing_score = packet.replace(&format!("- seed_4_score_self_hash: {}\n", hash(44)), "");
    assert!(matches!(
        validate_s4_closure_packet(&missing_score),
        Err(S4ReportError::Validation(
            S4ReportValidationError::MissingClosurePacketEntry {
                field: "seed_4_score_self_hash"
            }
        ))
    ));

    let empty_verdict = packet.replace("- H6: Confirmed", "- H6: TODO");
    assert!(matches!(
        validate_s4_closure_packet(&empty_verdict),
        Err(S4ReportError::Validation(
            S4ReportValidationError::EmptyClosurePacketEntry { field: "H6" }
        ))
    ));

    let empty_hash = packet.replace(
        &format!("- report_self_hash: {}", hash(153)),
        "- report_self_hash: -",
    );
    assert!(matches!(
        validate_s4_closure_packet(&empty_hash),
        Err(S4ReportError::Validation(
            S4ReportValidationError::EmptyClosurePacketEntry {
                field: "report_self_hash"
            }
        ))
    ));
}

fn pass_clean_report() -> S4Report {
    let body = body_markdown();
    let predictions = predictions_text();
    let (predictions_commit, first_result_commit) = git_commit_pair();
    S4Report::new(
        front_matter_for(
            S4Outcome::PassClean,
            decision_for_outcome(S4Outcome::PassClean),
            predictions_section_hash(predictions).expect("predictions hash"),
            predictions_commit,
            first_result_commit,
        ),
        body,
    )
    .expect("fixture S4 report validates")
}

fn front_matter_for(
    outcome: S4Outcome,
    decision: S4Decision,
    predictions_section_hash: Hash256,
    predictions_commit: S4GitCommitId,
    first_result_commit: S4GitCommitId,
) -> S4ReportFrontMatter {
    S4ReportFrontMatter {
        schema: "s4_report.v1".to_owned(),
        s4_outcome: outcome,
        decision,
        ts_manifest_self_hash: Some(hash(1)),
        gutenberg_manifest_self_hash: Some(hash(2)),
        baseline_gutenberg_self_hash: Some(hash(3)),
        corpus_quality_self_hash: Some(hash(4)),
        contamination_self_hash: Some(hash(5)),
        promotion_gate_self_hash: Some(hash(6)),
        corpus_progression_self_hash: Some(hash(7)),
        c_ts_checkpoint_self_hash: Some(hash(8)),
        per_seed_artifacts: per_seed_artifacts(),
        generated_at: "2026-05-19T00:00:00Z".to_owned(),
        rfc_revision: S4RfcRevisionRef::Hash256(hash(9)),
        predictions_section_hash,
        predictions_commit,
        first_result_commit,
        hypothesis_statuses: all_confirmed_hypotheses(),
        report_self_hash: Hash256::ZERO,
    }
}

fn body_markdown() -> String {
    format!(
        r#"## Pre-registered predictions
{}

## Observed
| seed | bpc_ternary_gutenberg | v0_success_pass | completion |
| --- | --- | --- | --- |
| 0 | 1.00 | true | Completed |
| 1 | 1.01 | true | Completed |
| 2 | 1.02 | true | Completed |
| 3 | 1.03 | true | Completed |
| 4 | 1.04 | true | Completed |

## Hypothesis verdicts
H1 through H7 are recorded in front matter as explicit HypothesisStatus values.

## Falsification analysis
No falsification rule fired for this fixture.

## Surprises
No out-of-band surprise notes for this fixture.

## Decision
ProceedToS5 is justified by all closure validators passing.

## Reproducibility statement
Replay command: `cargo test -p gbf-experiments --features s4 --test s4_report`.
"#,
        predictions_text()
    )
}

fn predictions_text() -> &'static str {
    "Predicted S4 closure requires H1..H6 confirmed, all five Gutenberg seeds scored, seed-0 oracle agreement, and a self-hashed report."
}

fn closure_packet_markdown() -> String {
    format!(
        r#"bd-2hmm closure packet matrix
- predictions_section_hash: {}
- gutenberg_manifest_self_hash: {}
- baseline_gutenberg_self_hash: {}
- promotion_gate_self_hash: {}
- contamination_self_hash: {}
- oracle_agreement_self_hash: {}
- report_self_hash: {}
- seed_0_score_self_hash: {}
- seed_1_score_self_hash: {}
- seed_2_score_self_hash: {}
- seed_3_score_self_hash: {}
- seed_4_score_self_hash: {}
- H1: Confirmed
- H2: Confirmed
- H3: Confirmed
- H4: Confirmed
- H5: Confirmed
- H6: Confirmed
- H7: Confirmed
- F1-broken-S4: Refuted H1
- F2-broken-S4: Refuted H2
- F3-broken-S4: Refuted H3
- F4-broken-S4: Refuted H6
- F5-broken-S4: Refuted H5
- F6-broken-S4: Refuted H1
- determinism_transcript: bd-14ln replay transcript recorded stable pin-ledger and artifact hashes.
"#,
        hash(170),
        hash(2),
        hash(3),
        hash(6),
        hash(5),
        hash(50),
        hash(153),
        hash(40),
        hash(41),
        hash(42),
        hash(43),
        hash(44),
    )
}

fn per_seed_artifacts() -> Vec<S4PerSeedArtifacts> {
    (0..5)
        .map(|seed| S4PerSeedArtifacts {
            seed,
            completion: S4Completion::Completed,
            checkpoint_self_hash: Some(hash(20 + seed as u8)),
            run_log_self_hash: Some(hash(30 + seed as u8)),
            score_self_hash: Some(hash(40 + seed as u8)),
            oracle_agreement_self_hash: (seed == 0).then_some(hash(50)),
        })
        .collect()
}

fn all_confirmed_hypotheses() -> BTreeMap<S4Hypothesis, HypothesisStatus> {
    S4Hypothesis::ALL
        .into_iter()
        .map(|hypothesis| (hypothesis, HypothesisStatus::Confirmed))
        .collect()
}

fn git_commit_pair() -> (S4GitCommitId, S4GitCommitId) {
    let output = Command::new("git")
        .args(["rev-list", "--max-count=2", "HEAD"])
        .output()
        .expect("git rev-list runs");
    assert!(
        output.status.success(),
        "git rev-list failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let commits = String::from_utf8(output.stdout)
        .expect("git output is UTF-8")
        .lines()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    assert!(
        commits.len() >= 2,
        "S4 report tests require HEAD and a parent"
    );
    (
        S4GitCommitId::new(commits[1].clone()).expect("parent is a commit id"),
        S4GitCommitId::new(commits[0].clone()).expect("HEAD is a commit id"),
    )
}

fn hash(byte: u8) -> Hash256 {
    Hash256::from_bytes([byte; 32])
}

fn assert_validation_label(result: Result<(), S4ReportError>, label: &str) {
    let error = result.expect_err("validator must reject fixture mutation");
    assert!(
        error.to_string().contains(label),
        "expected {label} error, got {error}"
    );
}

#[allow(dead_code)]
fn _assert_report_path_is_pathlike(path: &Path) -> &Path {
    path
}
