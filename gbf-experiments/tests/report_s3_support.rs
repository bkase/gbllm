#![cfg(feature = "s3")]
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use gbf_experiments::s1::schema::{GitCommitId, RfcRevisionRef};
use gbf_experiments::s3::report::{
    OracleOwnerBeads, PhaseCompletion, S2EnvironmentHashRecord, S3EnvironmentHashRecord,
    S3PerSeedArtifacts, S3Report, S3ReportFrontMatter, decision_for_outcome,
    generated_at_commit_time, predictions_section_hash,
};
use gbf_experiments::s3::schema::{
    HypothesisStatus, OracleFallbackTag, S3Completion, S3Decision, S3Hypothesis, S3Outcome,
};
use gbf_foundation::{Hash256, SemVer, sha256};

pub fn pass_clean_report() -> S3Report {
    report_for_outcome(S3Outcome::PassClean, Vec::new())
}

pub fn pass_with_fallback_report() -> S3Report {
    report_for_outcome(
        S3Outcome::PassWithFallbackOracle,
        vec![OracleFallbackTag::S3DenotationalFallback],
    )
}

pub fn report_for_outcome(outcome: S3Outcome, fallback_used: Vec<OracleFallbackTag>) -> S3Report {
    let body = body_markdown(outcome, &fallback_used);
    let predictions = predictions_text();
    let (predictions_commit, first_result_commit) = git_commit_pair();
    let front_matter = front_matter_for(
        outcome,
        decision_for_outcome(outcome),
        predictions_section_hash(predictions).expect("predictions hash"),
        predictions_commit,
        first_result_commit.clone(),
        git_commit_time(&first_result_commit),
        fallback_used,
    );
    S3Report::new(front_matter, body).expect("fixture S3 report validates")
}

pub fn front_matter_for(
    outcome: S3Outcome,
    decision: S3Decision,
    predictions_section_hash: Hash256,
    predictions_commit: GitCommitId,
    first_result_commit: GitCommitId,
    generated_at_commit_time: String,
    oracle_fallback_used: Vec<OracleFallbackTag>,
) -> S3ReportFrontMatter {
    S3ReportFrontMatter {
        schema: "s3_report.v1".to_owned(),
        s3_outcome: outcome,
        decision,
        charset_self_hash: hash(1),
        baseline_self_hash: hash(2),
        workload_self_hash: hash(3),
        conformance_self_hash: hash(4),
        v0_success_self_hash: hash(5),
        per_seed_artifacts: per_seed_artifacts(),
        oracle_owner_beads: OracleOwnerBeads {
            denotational: "bd-1rcc".to_owned(),
            artifact: "bd-c4wg".to_owned(),
        },
        oracle_fallback_used,
        oracle_re_run_self_hash: Some(hash(6)),
        conformance_owner_bead: "bd-35l3".to_owned(),
        e2e_test_owner_bead: "bd-1wd".to_owned(),
        structured_logging_owner_bead: "bd-2sd7".to_owned(),
        pass_version_s1: SemVer::new(0, 1, 0),
        pass_version_s2: SemVer::new(0, 2, 0),
        pass_version_s3: SemVer::new(0, 3, 0),
        s2_train_config_hash: hash(7),
        s3_train_config_hash: hash(8),
        s2_environment_hash: S2EnvironmentHashRecord {
            build_config_hash: hash(9),
            rust_toolchain_hash: hash(10),
            dependency_lockfile_hash: hash(11),
        },
        s3_environment_hash: S3EnvironmentHashRecord {
            build_config_hash: hash(12),
            rust_toolchain_hash: hash(13),
            dependency_lockfile_hash: hash(14),
            oracle_backend_identity: hash(15),
        },
        s2_pinned_phase_schedule_hash: hash(16),
        generated_at_commit_time,
        rfc_revision: RfcRevisionRef::Hash256(hash(17)),
        predictions_section_hash,
        predictions_commit,
        first_result_commit,
        hypothesis_statuses: all_confirmed_hypotheses(),
        report_self_hash: Hash256::ZERO,
    }
}

pub fn body_markdown(outcome: S3Outcome, fallback_used: &[OracleFallbackTag]) -> String {
    format!(
        r#"## Pre-registered predictions
{}

## Observed
| seed | val_bpc_char_fp | val_bpc_char_ternary | Q1 | Q2 | Q3 | Q4 | Q5 | Q6 | teacher_completion | student_completion |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 0 | 1.10 | 1.20 | true | true | true | true | true | true | Completed | Completed |
| 1 | 1.11 | 1.21 | true | true | true | true | true | true | Completed | Completed |
| 2 | 1.12 | 1.22 | true | true | true | true | true | true | Completed | Completed |
| 3 | 1.13 | 1.23 | true | true | true | true | true | true | Completed | Completed |
| 4 | 1.14 | 1.24 | true | true | true | true | true | true | Completed | Completed |

## Hypothesis verdicts
H1 through H7 are recorded in front matter as explicit HypothesisStatus values.

## Falsification analysis
F1-broken-S3 through F9-broken-S3 each refute their target hypotheses.

## Surprises
No out-of-band surprise notes for this fixture.

## Decision
Outcome `{outcome}` maps to decision `{decision}`.

## Reproducibility statement
Replay command: `cargo test -p gbf-experiments --features s3`. Fallback usage: `{fallback:?}`.
"#,
        predictions_text(),
        outcome = outcome,
        decision = decision_for_outcome(outcome),
        fallback = fallback_used,
    )
}

pub fn predictions_text() -> &'static str {
    "Predicted ranges cover D7 tolerance bands, Q1..Q6 thresholds, and the H6 adversarial-direction expectation."
}

pub fn per_seed_artifacts() -> Vec<S3PerSeedArtifacts> {
    (0..5)
        .map(|seed| S3PerSeedArtifacts {
            seed,
            teacher_completion: S3Completion::Completed,
            student_completion: S3Completion::Completed,
            phase_completion: PhaseCompletion::completed(),
            teacher_checkpoint_self_hash: Some(hash(20 + seed as u8)),
            student_checkpoint_self_hash: Some(hash(30 + seed as u8)),
            bundle_self_hash: Some(hash(40 + seed as u8)),
            artifact_self_hash: Some(hash(50 + seed as u8)),
            agreement_self_hash: Some(hash(60 + seed as u8)),
            generation_log_self_hash: Some(hash(70 + seed as u8)),
        })
        .collect()
}

pub fn all_confirmed_hypotheses() -> BTreeMap<S3Hypothesis, HypothesisStatus> {
    S3Hypothesis::ALL
        .into_iter()
        .map(|hypothesis| (hypothesis, HypothesisStatus::Confirmed))
        .collect()
}

pub fn hash(byte: u8) -> Hash256 {
    Hash256::from_bytes([byte; 32])
}

pub fn git_commit_pair() -> (GitCommitId, GitCommitId) {
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
        "S3 report tests require HEAD and a parent"
    );
    (
        GitCommitId::new(commits[1].clone()).expect("parent is a commit id"),
        GitCommitId::new(commits[0].clone()).expect("HEAD is a commit id"),
    )
}

pub fn git_commit_time(commit: &GitCommitId) -> String {
    generated_at_commit_time(commit).expect("git commit time")
}

pub fn write_report_if_requested(report: &S3Report) {
    let Ok(path) = std::env::var("S3_REPORT_OUT") else {
        return;
    };
    let bytes = report.to_markdown().expect("report renders");
    std::fs::write(Path::new(&path), bytes).expect("writes requested S3 report");
}

pub fn source_hash() -> Hash256 {
    sha256(include_bytes!("report_s3_support.rs"))
}
