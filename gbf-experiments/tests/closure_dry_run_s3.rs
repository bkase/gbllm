#![cfg(feature = "s3")]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

#[test]
fn s3_closure_dry_run_emits_truthful_prep_report_and_audit() {
    let temp = tempfile::tempdir().expect("tempdir");
    let audit = temp.path().join("closure-audit.ndjson");
    let output = Command::new(repo_root().join("scripts/s3_closure_dry_run.sh"))
        .current_dir(repo_root())
        .args([
            "--skip-ci",
            "--dispatcher-mode",
            "fixture",
            "--report-dir",
            temp.path().to_str().expect("utf8"),
            "--audit-path",
            audit.to_str().expect("utf8"),
        ])
        .output()
        .expect("closure dry-run launches");

    assert!(
        output.status.success(),
        "closure dry-run failed:\n{}",
        command_output(&output)
    );
    let report_path = temp.path().join("s3-closure-readiness.json");
    let report: Value = serde_json::from_slice(&std::fs::read(&report_path).expect("report reads"))
        .expect("report parses");
    assert_eq!(report["schema"], "s3_closure_readiness.v1");
    assert_eq!(report["current_objective"], "pr-open-prep");
    assert_eq!(report["ready_to_close"], false);
    assert_eq!(report["script_scope"]["does_not_close_beads"], true);
    assert_eq!(report["script_scope"]["does_not_claim_merged_pr"], true);
    assert_eq!(report["dispatcher"]["s3_outcome"], "Pass-clean");
    assert_eq!(report["dispatcher"]["s3_decision"], "ProceedToS4");
    assert!(
        report["current_blockers"]
            .as_array()
            .expect("blockers array")
            .iter()
            .any(|blocker| blocker
                .as_str()
                .is_some_and(|text| text.contains("S3 PR opened"))),
        "prep report must keep PR-open blocker visible: {report:#}"
    );
    assert!(
        report["external_closure_requirements"]
            .as_array()
            .expect("external requirements array")
            .iter()
            .any(
                |requirement| requirement["id"] == "pr_merged" && requirement["satisfied"] == false
            ),
        "prep report must not claim merged-PR evidence: {report:#}"
    );
    assert!(
        report["closure_comment_templates"]["bd-3k8o"]
            .as_str()
            .expect("bd-3k8o template")
            .contains("TEMPLATE ONLY")
    );
    assert!(
        report["closure_comment_templates"]["bd-3w2"]
            .as_str()
            .expect("bd-3w2 template")
            .contains("qat-bead-closure")
    );

    let events = std::fs::read_to_string(&audit)
        .expect("audit reads")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("audit line parses"))
        .collect::<Vec<_>>();
    assert_event(&events, "s3::closure::dry_run_dispatcher_evaluated");
    assert_event(&events, "s3::closure::readiness_summary");
}

#[test]
fn s3_closure_dry_run_post_merge_mode_requires_real_gate_evidence() {
    let temp = tempfile::tempdir().expect("tempdir");
    let audit = temp.path().join("closure-audit.ndjson");
    let output = Command::new(repo_root().join("scripts/s3_closure_dry_run.sh"))
        .current_dir(repo_root())
        .args([
            "--skip-ci",
            "--dispatcher-mode",
            "fixture",
            "--current-objective",
            "post-merge-closure",
            "--pr-opened",
            "--pr-workflow-success",
            "--pr-merged",
            "--r-predictions-ancestry-verified",
            "--bd-3k8o-comment-ready",
            "--bd-3w2-comment-ready",
            "--moved-acceptance-comments-ready",
            "--persona-reviews-approved",
            "--report-dir",
            temp.path().to_str().expect("utf8"),
            "--audit-path",
            audit.to_str().expect("utf8"),
        ])
        .output()
        .expect("closure dry-run launches");

    assert!(
        output.status.success(),
        "closure dry-run failed:\n{}",
        command_output(&output)
    );
    let report_path = temp.path().join("s3-closure-readiness.json");
    let report: Value = serde_json::from_slice(&std::fs::read(&report_path).expect("report reads"))
        .expect("report parses");
    assert_eq!(report["ready_to_close"], false);
    let blockers = report["current_blockers"]
        .as_array()
        .expect("blockers array")
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    assert!(
        blockers
            .iter()
            .any(|blocker| blocker.contains("B24 CI gate scripts were skipped")),
        "post-merge mode must reject skipped CI gates: {report:#}"
    );
    assert!(
        blockers
            .iter()
            .any(|blocker| blocker.contains("B21 dispatcher was not executed via cargo")),
        "post-merge mode must reject fixture dispatcher evidence: {report:#}"
    );
}

fn assert_event(events: &[Value], event_name: &str) {
    assert!(
        events.iter().any(|event| event["event_name"] == event_name),
        "missing {event_name}: {events:#?}"
    );
}

fn command_output(output: &Output) -> String {
    format!(
        "status={}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("gbf-experiments has workspace parent")
        .to_path_buf()
}
