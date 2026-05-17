use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[test]
fn dry_run_preregistration_output_is_byte_identical_across_replays() {
    let first = run_checker();
    let second = run_checker();

    assert!(
        first.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        first.stdout_text(),
        first.stderr_text()
    );
    assert!(
        second.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        second.stdout_text(),
        second.stderr_text()
    );
    assert_eq!(
        first.stdout, second.stdout,
        "dry-run summary output must be byte-identical"
    );
    assert_eq!(
        first.stderr, second.stderr,
        "dry-run NDJSON events must be byte-identical"
    );
    assert_eq!(
        first.stdout_text(),
        "S3 s3-preregistration PASS dry_run=true report=/tmp/s3-preregistration.json\n"
    );

    let events = parse_ndjson(&first.stderr);
    assert_eq!(
        events,
        vec![
            json!({
                "event": "s3_preregistration_check_stage_start",
                "stage": 1,
                "description": "validate preregistration pin and predictions hash",
            }),
            json!({
                "event": "s3_preregistration_check_stage_done",
                "stage": 1,
                "passed": true,
                "detail": {
                    "pin": "experiments/S3/preregistration.toml",
                    "rfc": "history/rfcs/F-S3-v0-success-tinystories.md",
                    "predictions_section_hash": "sha256:77872aef2b0cb83523077015773a999ac713472e89673d22339d6441f520e95a",
                    "pin_predictions_section_hash": "sha256:77872aef2b0cb83523077015773a999ac713472e89673d22339d6441f520e95a",
                },
            }),
            json!({
                "event": "s3_preregistration_check_stage_start",
                "stage": 2,
                "description": "scan for preregistration-breaking result artifacts",
            }),
            json!({
                "event": "s3_preregistration_check_stage_done",
                "stage": 2,
                "passed": true,
                "detail": {
                    "artifact_dirs": ["experiments/S3"],
                    "first_result_artifact": null,
                },
            }),
            json!({
                "event": "s3_preregistration_check_summary",
                "passed": true,
                "script": "s3_preregistration_check",
                "stages": [
                    {
                        "name": "predictions_hash",
                        "passed": true,
                        "detail": {
                            "pin": "experiments/S3/preregistration.toml",
                            "rfc": "history/rfcs/F-S3-v0-success-tinystories.md",
                            "predictions_section_hash": "sha256:77872aef2b0cb83523077015773a999ac713472e89673d22339d6441f520e95a",
                            "pin_predictions_section_hash": "sha256:77872aef2b0cb83523077015773a999ac713472e89673d22339d6441f520e95a",
                        },
                    },
                    {
                        "name": "empty_result_scan",
                        "passed": true,
                        "detail": {
                            "artifact_dirs": ["experiments/S3"],
                            "first_result_artifact": null,
                        },
                    },
                ],
                "exit_code": 0,
                "dry_run": true,
                "evidence_mode": "dry_run",
                "live_evidence": false,
                "summary": "S3 s3-preregistration PASS dry_run=true report=/tmp/s3-preregistration.json",
            }),
        ]
    );
}

#[test]
fn post_result_mode_accepts_registered_result_without_weakening_default() {
    let temp = tempfile::tempdir().expect("tempdir");
    let predictions = "Pinned S3 fixture predictions.";
    let rfc = write_rfc_fixture(temp.path(), predictions);
    let artifact_dir = temp.path().join("artifacts");
    fs::create_dir(&artifact_dir).expect("artifact dir");
    write_result_artifact(&artifact_dir);

    let (predictions_commit, first_result_commit) = current_git_commit_pair();
    let pin = write_pin_fixture(
        temp.path(),
        predictions,
        &predictions_commit,
        &first_result_commit,
    );

    let default = run_checker_with([
        "--dry-run",
        "--pin",
        pin.to_str().expect("pin path UTF-8"),
        "--rfc",
        rfc.to_str().expect("rfc path UTF-8"),
        "--artifact-dir",
        artifact_dir.to_str().expect("artifact path UTF-8"),
    ]);
    assert!(
        !default.status.success(),
        "default pre-result mode must keep rejecting post-result pins"
    );
    assert!(
        default
            .stdout_text()
            .contains("first_result_commit must remain empty before first S3 result"),
        "stdout:\n{}\nstderr:\n{}",
        default.stdout_text(),
        default.stderr_text()
    );

    let post = run_checker_with([
        "--dry-run",
        "--result-state",
        "post",
        "--pin",
        pin.to_str().expect("pin path UTF-8"),
        "--rfc",
        rfc.to_str().expect("rfc path UTF-8"),
        "--artifact-dir",
        artifact_dir.to_str().expect("artifact path UTF-8"),
    ]);
    assert!(
        post.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        post.stdout_text(),
        post.stderr_text()
    );
    let events = parse_ndjson(&post.stderr);
    assert!(
        events.iter().any(|event| {
            event["event"] == "s3_preregistration_check_stage_done"
                && event["stage"] == 2
                && event["passed"] == true
                && event["detail"]["first_result_artifact"]
                    .as_str()
                    .is_some_and(|path| path.ends_with("artifact-metadata.json"))
        }),
        "post-result run should record the registered result artifact: {events:?}"
    );
}

#[test]
fn post_result_mode_rejects_invalid_ancestry_and_missing_artifact() {
    let temp = tempfile::tempdir().expect("tempdir");
    let predictions = "Pinned S3 fixture predictions.";
    let rfc = write_rfc_fixture(temp.path(), predictions);
    let artifact_dir = temp.path().join("artifacts");
    fs::create_dir(&artifact_dir).expect("artifact dir");
    write_result_artifact(&artifact_dir);

    let (_, first_result_commit) = current_git_commit_pair();
    let equal_pin = write_pin_fixture(
        temp.path(),
        predictions,
        &first_result_commit,
        &first_result_commit,
    );
    let equal = run_checker_with([
        "--dry-run",
        "--result-state",
        "post",
        "--pin",
        equal_pin.to_str().expect("pin path UTF-8"),
        "--rfc",
        rfc.to_str().expect("rfc path UTF-8"),
        "--artifact-dir",
        artifact_dir.to_str().expect("artifact path UTF-8"),
    ]);
    assert!(
        !equal.status.success(),
        "equal predictions/result commits should fail:\nstdout:\n{}\nstderr:\n{}",
        equal.stdout_text(),
        equal.stderr_text()
    );
    assert!(
        equal
            .stdout_text()
            .contains("predictions_commit must be a strict ancestor of first_result_commit"),
        "stdout:\n{}\nstderr:\n{}",
        equal.stdout_text(),
        equal.stderr_text()
    );

    let (predictions_commit, first_result_commit) = current_git_commit_pair();
    let missing_artifact_dir = temp.path().join("empty-artifacts");
    fs::create_dir(&missing_artifact_dir).expect("empty artifact dir");
    let valid_pin = write_pin_fixture(
        temp.path(),
        predictions,
        &predictions_commit,
        &first_result_commit,
    );
    let missing_artifact = run_checker_with([
        "--dry-run",
        "--result-state",
        "post",
        "--pin",
        valid_pin.to_str().expect("pin path UTF-8"),
        "--rfc",
        rfc.to_str().expect("rfc path UTF-8"),
        "--artifact-dir",
        missing_artifact_dir.to_str().expect("artifact path UTF-8"),
    ]);
    assert!(
        !missing_artifact.status.success(),
        "post-result mode should require result evidence:\nstdout:\n{}\nstderr:\n{}",
        missing_artifact.stdout_text(),
        missing_artifact.stderr_text()
    );
    assert!(
        missing_artifact
            .stdout_text()
            .contains("missing S3 result artifact evidence in post-result mode"),
        "stdout:\n{}\nstderr:\n{}",
        missing_artifact.stdout_text(),
        missing_artifact.stderr_text()
    );
}

struct ScriptOutput {
    status: std::process::ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

impl ScriptOutput {
    fn stdout_text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }

    fn stderr_text(&self) -> String {
        String::from_utf8_lossy(&self.stderr).into_owned()
    }
}

fn run_checker() -> ScriptOutput {
    run_checker_with(["--dry-run"])
}

fn run_checker_with<const N: usize>(args: [&str; N]) -> ScriptOutput {
    let output = Command::new(workspace_root().join("scripts/s3_preregistration_check.sh"))
        .current_dir(workspace_root())
        .args(args)
        .output()
        .expect("run S3 preregistration checker");
    ScriptOutput {
        status: output.status,
        stdout: output.stdout,
        stderr: output.stderr,
    }
}

fn parse_ndjson(bytes: &[u8]) -> Vec<Value> {
    std::str::from_utf8(bytes)
        .expect("NDJSON is UTF-8")
        .lines()
        .map(|line| serde_json::from_str(line).expect("line is JSON"))
        .collect()
}

fn predictions_hash(section: &str) -> String {
    let canonical = serde_json::to_string(section.trim()).expect("canonical string");
    let digest = Sha256::digest(canonical.as_bytes());
    format!("sha256:{digest:x}")
}

fn write_rfc_fixture(dir: &Path, predictions: &str) -> PathBuf {
    let rfc = dir.join("F-S3-fixture.md");
    fs::write(
        &rfc,
        format!(
            "# Fixture\n\n## Pre-registered predictions\n\n{predictions}\n\n## Observed\n\nResult rows.\n"
        ),
    )
    .expect("write RFC fixture");
    rfc
}

fn write_pin_fixture(
    dir: &Path,
    predictions: &str,
    predictions_commit: &str,
    first_result_commit: &str,
) -> PathBuf {
    let pin = dir.join("preregistration.toml");
    fs::write(
        &pin,
        format!(
            "schema = \"s3_preregistration.v1\"\n\
             predictions_commit = \"{predictions_commit}\"\n\
             predictions_section_hash = \"{}\"\n\
             pass_version_S3 = \"fixture\"\n\
             rfc_revision = \"{predictions_commit}\"\n\
             first_result_commit = \"{first_result_commit}\"\n",
            predictions_hash(predictions),
        ),
    )
    .expect("write pin fixture");
    pin
}

fn write_result_artifact(dir: &Path) {
    fs::write(
        dir.join("artifact-metadata.json"),
        r#"{"v0_success_self_hash":"sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"}"#,
    )
    .expect("write result artifact fixture");
}

fn current_git_commit_pair() -> (String, String) {
    let output = Command::new("git")
        .args(["rev-list", "--max-count=2", "HEAD"])
        .current_dir(workspace_root())
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
        "S3 preregistration tests require HEAD and a parent"
    );
    (commits[1].clone(), commits[0].clone())
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("gbf-experiments parent is workspace root")
        .to_path_buf()
}
