use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};

#[test]
fn dry_run_preregistration_output_is_byte_identical_across_replays() {
    let first = run_checker();
    let second = run_checker();

    assert!(first.status.success(), "stdout:\n{}", first.stdout_text());
    assert!(second.status.success(), "stdout:\n{}", second.stdout_text());
    assert_eq!(
        first.stdout, second.stdout,
        "dry-run NDJSON output must be byte-identical"
    );

    let events = parse_ndjson(&first.stdout);
    assert_eq!(
        events,
        vec![
            json!({
                "event": "s3_preregistration_check_stage_start",
                "stage": 1,
                "description": "verify predictions_section_hash",
            }),
            json!({
                "event": "s3_preregistration_check_stage_done",
                "stage": 1,
                "passed": true,
                "detail": "hash matches",
            }),
            json!({
                "event": "s3_preregistration_check_stage_start",
                "stage": 2,
                "description": "verify ancestry",
            }),
            json!({
                "event": "s3_preregistration_check_stage_done",
                "stage": 2,
                "passed": true,
                "detail": "first_result_commit is sentinel",
            }),
            json!({
                "event": "s3_preregistration_check_stage_start",
                "stage": 3,
                "description": "verify earliest result-artifact commit",
            }),
            json!({
                "event": "s3_preregistration_check_stage_done",
                "stage": 3,
                "passed": true,
                "detail": "no result artifacts yet",
            }),
            json!({
                "event": "s3_preregistration_check_done",
                "passed": true,
                "first_result_commit": null,
            }),
        ]
    );
}

struct ScriptOutput {
    status: std::process::ExitStatus,
    stdout: Vec<u8>,
}

impl ScriptOutput {
    fn stdout_text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
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
