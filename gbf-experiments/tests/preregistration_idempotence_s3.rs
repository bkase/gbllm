use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};

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
    let output = Command::new(workspace_root().join("scripts/s3_preregistration_check.sh"))
        .current_dir(workspace_root())
        .arg("--dry-run")
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

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("gbf-experiments parent is workspace root")
        .to_path_buf()
}
