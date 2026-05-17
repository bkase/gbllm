#![cfg(feature = "s3")]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

const SCRIPTS: [ScriptSpec; 8] = [
    ScriptSpec::new(
        "s3_preregistration_check.sh",
        "s3_preregistration_check",
        "s3-preregistration",
    ),
    ScriptSpec::new(
        "s3_determinism_check.sh",
        "s3_determinism_check",
        "s3-determinism",
    ),
    ScriptSpec::new(
        "s3_full_determinism_check.sh",
        "s3_full_determinism_check",
        "s3-full-determinism",
    ),
    ScriptSpec::new(
        "s3_isolation_check.sh",
        "s3_isolation_check",
        "s3-isolation",
    ),
    ScriptSpec::new(
        "s3_api_drift_check.sh",
        "s3_api_drift_check",
        "s3-api-drift",
    ),
    ScriptSpec::new(
        "s3_oracle_re_run_check.sh",
        "s3_oracle_re_run_check",
        "s3-oracle-re-run",
    ),
    ScriptSpec::new(
        "s3_no_naming_resolution_check.sh",
        "s3_no_naming_resolution_check",
        "s3-no-naming-resolution",
    ),
    ScriptSpec::new(
        "s3_feature_matrix_check.sh",
        "s3_feature_matrix_check",
        "s3-feature-matrix",
    ),
];

#[test]
fn s3_scripts_dry_run_emit_schema_and_stable_reports() {
    for spec in SCRIPTS {
        let temp = tempfile::tempdir().expect("tempdir");
        let artifact_dir = temp.path().join("artifacts");
        fs::create_dir_all(&artifact_dir).expect("artifact dir");
        write_result_artifact(&artifact_dir);
        let report_dir = temp_path(&temp);
        let artifact_dir = artifact_dir.to_str().expect("utf8 artifact path");
        let mut args = vec!["--dry-run", "--report-dir", report_dir];
        if spec.name == "s3_preregistration_check.sh" {
            args.extend(["--result-state", "post", "--artifact-dir", artifact_dir]);
        }
        let first = run_script(spec, &args);
        assert_success(spec, &first);
        assert_single_line_stdout(&first);
        let first_events = assert_ndjson_events(&first, spec.event_prefix);
        let first_report = read_report(temp.path(), spec);
        assert_report_schema(&first_report, spec.event_prefix, true);
        assert_summary_matches_report(&first_events, &first_report);

        let second = run_script(spec, &args);
        assert_success(spec, &second);
        let second_report = read_report(temp.path(), spec);
        assert_eq!(
            first_report, second_report,
            "{} dry-run report should be byte-stable",
            spec.name
        );
    }
}

#[test]
fn s3_script_report_path_overrides_default_report_location() {
    let temp = tempfile::tempdir().expect("tempdir");
    let report = temp.path().join("custom-feature-matrix.json");
    let output = run_script(
        script("s3_feature_matrix_check.sh"),
        &["--dry-run", "--report-path", report.to_str().expect("utf8")],
    );
    assert_success(script("s3_feature_matrix_check.sh"), &output);
    assert!(
        report.exists(),
        "custom --report-path should be written: {}",
        command_output(&output)
    );
    let report: Value =
        serde_json::from_slice(&fs::read(report).expect("report reads")).expect("report parses");
    assert_eq!(report["script"], "s3_feature_matrix_check");
    assert_eq!(report["dry_run"], true);
}

#[test]
fn s3_preregistration_dry_run_rejects_broken_prediction_hash() {
    let temp = tempfile::tempdir().expect("tempdir");
    let pin = temp.path().join("broken-preregistration.toml");
    fs::write(
        &pin,
        r#"schema = "s3_preregistration.v1"
predictions_commit = "b4d9f0abcbec9140506288fd8344bcd16dab7479"
predictions_section_hash = "sha256:0000000000000000000000000000000000000000000000000000000000000000"
pass_version_S3 = "s3-prereg-bd-m20r-2026-05-14"
rfc_revision = "b4d9f0abcbec9140506288fd8344bcd16dab7479"
first_result_commit = ""
"#,
    )
    .expect("broken pin writes");
    let output = run_script(
        script("s3_preregistration_check.sh"),
        &[
            "--dry-run",
            "--pin",
            pin.to_str().expect("utf8"),
            "--report-dir",
            temp_path(&temp),
        ],
    );
    assert!(
        !output.status.success(),
        "broken preregistration pin should fail:\n{}",
        command_output(&output)
    );
    assert_single_line_stdout(&output);
    assert_ndjson_events(&output, "s3_preregistration_check");
    let report = read_report(temp.path(), script("s3_preregistration_check.sh"));
    assert_report_schema(&report, "s3_preregistration_check", false);
    let report: Value = serde_json::from_str(&report).expect("report JSON");
    assert!(
        report["stages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|stage| { stage["name"] == "predictions_hash" && stage["passed"] == false })
    );
}

#[test]
fn s3_scripts_forced_failure_uses_common_structured_plumbing() {
    for spec in SCRIPTS {
        let temp = tempfile::tempdir().expect("tempdir");
        let artifact_dir = temp.path().join("artifacts");
        fs::create_dir_all(&artifact_dir).expect("artifact dir");
        write_result_artifact(&artifact_dir);
        let report_dir = temp_path(&temp);
        let artifact_dir = artifact_dir.to_str().expect("utf8 artifact path");
        let mut args = vec![
            "--dry-run",
            "--force-failure-for-test",
            "--report-dir",
            report_dir,
        ];
        if spec.name == "s3_preregistration_check.sh" {
            args.extend(["--result-state", "post", "--artifact-dir", artifact_dir]);
        }
        let output = run_script(spec, &args);
        assert!(
            !output.status.success(),
            "{} forced failure should fail:\n{}",
            spec.name,
            command_output(&output)
        );
        assert_single_line_stdout(&output);
        assert_ndjson_events(&output, spec.event_prefix);
        let report = read_report(temp.path(), spec);
        assert_report_schema(&report, spec.event_prefix, false);
        let report: Value = serde_json::from_str(&report).expect("report JSON");
        assert!(
            report["stages"]
                .as_array()
                .expect("stages")
                .iter()
                .any(|stage| stage["passed"] == false
                    && stage["detail"]["reason"]
                        == "forced failure for script plumbing regression"),
            "{} report should mark forced failed stage: {report}",
            spec.name
        );
        assert!(
            String::from_utf8_lossy(&output.stdout)
                .contains("forced failure for script plumbing regression"),
            "{} summary should carry forced failure reason",
            spec.name
        );
    }
}

#[test]
fn s3_no_naming_resolution_live_and_dry_run_reports_share_schema() {
    let temp = tempfile::tempdir().expect("tempdir");
    let dry_report_dir = temp.path().join("dry");
    let live_report_dir = temp.path().join("live");
    let target = temp.path().join("empty-artifacts");
    fs::create_dir_all(&target).expect("target dir creates");
    let spec = script("s3_no_naming_resolution_check.sh");

    let dry = run_script(
        spec,
        &[
            "--dry-run",
            "--report-dir",
            dry_report_dir.to_str().expect("utf8"),
            target.to_str().expect("utf8"),
        ],
    );
    assert_success(spec, &dry);
    let live = run_script(
        spec,
        &[
            "--report-dir",
            live_report_dir.to_str().expect("utf8"),
            target.to_str().expect("utf8"),
        ],
    );
    assert_success(spec, &live);

    let dry_report: Value =
        serde_json::from_str(&read_report(&dry_report_dir, spec)).expect("dry report JSON");
    let live_report: Value =
        serde_json::from_str(&read_report(&live_report_dir, spec)).expect("live report JSON");
    assert_eq!(top_level_keys(&dry_report), top_level_keys(&live_report));
    assert_eq!(stage_names(&dry_report), stage_names(&live_report));
    assert_eq!(dry_report["evidence_mode"], "dry_run");
    assert_eq!(live_report["evidence_mode"], "live");
    assert_eq!(live_report["live_evidence"], true);
}

#[derive(Clone, Copy)]
struct ScriptSpec {
    name: &'static str,
    event_prefix: &'static str,
    report_slug: &'static str,
}

impl ScriptSpec {
    const fn new(
        name: &'static str,
        event_prefix: &'static str,
        report_slug: &'static str,
    ) -> Self {
        Self {
            name,
            event_prefix,
            report_slug,
        }
    }
}

fn script(name: &str) -> ScriptSpec {
    SCRIPTS
        .into_iter()
        .find(|spec| spec.name == name)
        .unwrap_or_else(|| panic!("missing script spec {name}"))
}

fn run_script(spec: ScriptSpec, args: &[&str]) -> Output {
    Command::new(repo_root().join("scripts").join(spec.name))
        .current_dir(repo_root())
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("{} launches: {error}", spec.name))
}

fn assert_success(spec: ScriptSpec, output: &Output) {
    assert!(
        output.status.success(),
        "{} failed:\n{}",
        spec.name,
        command_output(output)
    );
}

fn assert_single_line_stdout(output: &Output) {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines = stdout.lines().collect::<Vec<_>>();
    assert_eq!(
        lines.len(),
        1,
        "stdout should be one summary line: {stdout:?}"
    );
    assert!(
        lines[0].contains("S3 "),
        "stdout should name S3 gate: {stdout:?}"
    );
}

fn assert_ndjson_events(output: &Output, event_prefix: &str) -> Vec<Value> {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let events = stderr
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<Value>(line).expect("stderr line is JSON"))
        .collect::<Vec<_>>();
    assert!(
        events.iter().all(|event| event["event"].as_str().is_some()),
        "each event should carry an event field: {stderr}"
    );
    let names = events
        .iter()
        .filter_map(|event| event["event"].as_str())
        .collect::<Vec<_>>();
    assert!(
        names.contains(&format!("{event_prefix}_stage_start").as_str()),
        "missing stage_start event: {names:?}"
    );
    assert!(
        names.contains(&format!("{event_prefix}_stage_done").as_str()),
        "missing stage_done event: {names:?}"
    );
    assert!(
        names.contains(&format!("{event_prefix}_summary").as_str()),
        "missing summary event: {names:?}"
    );
    events
}

fn assert_report_schema(report: &str, expected_script: &str, expected_passed: bool) {
    let report: Value = serde_json::from_str(report).expect("report JSON");
    assert_eq!(report["script"], expected_script);
    assert_eq!(report["passed"], expected_passed);
    assert_eq!(report["exit_code"], if expected_passed { 0 } else { 1 });
    assert_eq!(report["dry_run"], true);
    assert_eq!(report["evidence_mode"], "dry_run");
    assert_eq!(report["live_evidence"], false);
    assert!(
        report["stages"]
            .as_array()
            .is_some_and(|stages| !stages.is_empty())
    );
}

fn assert_summary_matches_report(events: &[Value], report: &str) {
    let report: Value = serde_json::from_str(report).expect("report JSON");
    let summary = events
        .iter()
        .find(|event| event["event"] == format!("{}_summary", report["script"].as_str().unwrap()))
        .expect("summary event exists");
    assert_eq!(summary["script"], report["script"]);
    assert_eq!(summary["passed"], report["passed"]);
    assert_eq!(summary["exit_code"], report["exit_code"]);
    assert_eq!(summary["dry_run"], report["dry_run"]);
}

fn read_report(report_dir: &Path, spec: ScriptSpec) -> String {
    fs::read_to_string(report_dir.join(format!("{}.json", spec.report_slug)))
        .unwrap_or_else(|error| panic!("{} report reads: {error}", spec.name))
}

fn top_level_keys(value: &Value) -> Vec<String> {
    let mut keys = value
        .as_object()
        .expect("object")
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    keys.sort();
    keys
}

fn stage_names(report: &Value) -> Vec<String> {
    report["stages"]
        .as_array()
        .expect("stages")
        .iter()
        .map(|stage| stage["name"].as_str().expect("stage name").to_owned())
        .collect()
}

fn temp_path(temp: &tempfile::TempDir) -> &str {
    temp.path().to_str().expect("utf8 temp path")
}

fn write_result_artifact(dir: &Path) {
    fs::write(
        dir.join("artifact-metadata.json"),
        r#"{"v0_success_self_hash":"sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"}"#,
    )
    .expect("write result artifact fixture");
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
