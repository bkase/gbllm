#![cfg(feature = "s3")]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

const SCRIPTS: [&str; 8] = [
    "s3_preregistration_check.sh",
    "s3_determinism_check.sh",
    "s3_full_determinism_check.sh",
    "s3_isolation_check.sh",
    "s3_api_drift_check.sh",
    "s3_oracle_re_run_check.sh",
    "s3_no_naming_resolution_check.sh",
    "s3_feature_matrix_check.sh",
];

#[test]
fn s3_script_event_shape_carries_stage_and_summary_events() {
    for script in SCRIPTS {
        let temp = tempfile::tempdir().expect("tempdir");
        let artifact_dir = temp.path().join("artifacts");
        fs::create_dir_all(&artifact_dir).expect("artifact dir");
        write_result_artifact(&artifact_dir);
        let mut args = vec![
            "--dry-run",
            "--report-dir",
            temp.path().to_str().expect("utf8"),
        ];
        if script == "s3_preregistration_check.sh" {
            args.extend([
                "--result-state",
                "post",
                "--artifact-dir",
                artifact_dir.to_str().expect("utf8 artifact path"),
            ]);
        }
        let output = Command::new(repo_root().join("scripts").join(script))
            .current_dir(repo_root())
            .args(args)
            .output()
            .unwrap_or_else(|error| panic!("{script} launches: {error}"));
        assert!(
            output.status.success(),
            "{script} dry-run failed:\n{}",
            command_output(&output)
        );

        let prefix = script.trim_end_matches(".sh");
        let events = parse_stderr_events(&output);
        assert_event(&events, prefix, "stage_start", |event| {
            event["stage"].as_u64().is_some() && event["description"].as_str().is_some()
        });
        assert_event(&events, prefix, "stage_done", |event| {
            event["stage"].as_u64().is_some()
                && event["passed"].as_bool().is_some()
                && event["detail"].as_object().is_some()
        });
        assert_event(&events, prefix, "summary", |event| {
            event["script"] == prefix
                && event["passed"] == true
                && event["exit_code"] == 0
                && event["dry_run"] == true
                && event["stages"].as_array().is_some()
        });
    }
}

fn assert_event(events: &[Value], prefix: &str, suffix: &str, predicate: impl Fn(&Value) -> bool) {
    let name = format!("{prefix}_{suffix}");
    let event = events
        .iter()
        .find(|event| event["event"] == name)
        .unwrap_or_else(|| panic!("missing {name}: {events:#?}"));
    assert!(predicate(event), "malformed {name}: {event:#?}");
}

fn parse_stderr_events(output: &Output) -> Vec<Value> {
    String::from_utf8_lossy(&output.stderr)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<Value>(line).expect("stderr line is JSON"))
        .collect()
}

fn command_output(output: &Output) -> String {
    format!(
        "status={}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn write_result_artifact(dir: &Path) {
    fs::write(
        dir.join("artifact-metadata.json"),
        r#"{"v0_success_self_hash":"sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"}"#,
    )
    .expect("write result artifact fixture");
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("gbf-experiments has workspace parent")
        .to_path_buf()
}
