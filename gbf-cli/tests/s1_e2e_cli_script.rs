use std::fs;
use std::process::Command;

use serde_json::Value;

#[test]
fn s1_e2e_cli_script_produces_pass_clean_report_from_cli_artifacts() {
    let temp = tempfile::tempdir().expect("tempdir");
    let out_dir = temp.path().join("e2e");
    let gbf_bin = assert_cmd::cargo::cargo_bin("gbf-cli");
    let output = Command::new(repo_root().join("scripts/s1_e2e_cli.sh"))
        .env("GBF_BIN", gbf_bin)
        .arg("--scenario")
        .arg("pass_clean")
        .arg("--fixture")
        .arg("tiny")
        .arg("--out-dir")
        .arg(&out_dir)
        .output()
        .expect("run s1_e2e_cli.sh");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(out_dir.join("s1_baseline.v1.json").exists());
    assert!(out_dir.join("checkpoints/seed-4/metadata.json").exists());
    assert!(out_dir.join("seed-0/s1_negative_test.v1.json").exists());
    assert!(out_dir.join("seed-0/s1_ablation.v1.json").exists());
    assert!(out_dir.join("s1_oracle.v1.json").exists());
    assert!(out_dir.join("S1-report.md").exists());

    let summary: Value = serde_json::from_slice(
        &fs::read(out_dir.join("report_summary.json")).expect("report summary"),
    )
    .expect("report summary json");
    assert_eq!(summary["scenario"], "pass_clean");
    assert_eq!(summary["outcome"], "Pass-with-warning");
    assert_eq!(summary["decision"], "ProceedToS2-with-T12.5-prereq");
    assert!(
        summary["report_self_hash"]
            .as_str()
            .expect("report hash")
            .starts_with("sha256:")
    );
    let negative: Value = serde_json::from_slice(
        &fs::read(out_dir.join("seed-0/s1_negative_test.v1.json")).expect("negative json"),
    )
    .expect("negative json");
    assert_eq!(negative["sensitive"], false);
    let report = fs::read_to_string(out_dir.join("S1-report.md")).expect("report markdown");
    assert!(report.contains("| H3 | Refuted |"));
    assert!(report.contains("sensitive=false"));
}

#[test]
fn s1_e2e_cli_script_produces_metric_failure_from_cli_substitutes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let out_dir = temp.path().join("e2e");
    let gbf_bin = falsify_gbf_bin(temp.path());
    let output = Command::new(repo_root().join("scripts/s1_e2e_cli.sh"))
        .env("GBF_BIN", gbf_bin)
        .arg("--scenario")
        .arg("fail_metric_modulo_shuffle")
        .arg("--fixture")
        .arg("tiny")
        .arg("--out-dir")
        .arg(&out_dir)
        .output()
        .expect("run s1_e2e_cli.sh");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let summary: Value = serde_json::from_slice(
        &fs::read(out_dir.join("report_summary.json")).expect("report summary"),
    )
    .expect("report summary json");
    assert_eq!(summary["scenario"], "fail_metric_modulo_shuffle");
    assert_eq!(summary["outcome"], "Fail-metric");
    assert_eq!(summary["decision"], "Halt(measurement-broken)");

    let oracle: Value =
        serde_json::from_slice(&fs::read(out_dir.join("s1_oracle.v1.json")).expect("oracle json"))
            .expect("oracle json");
    assert_eq!(oracle["metric_oracle_passed"], false);
    assert_eq!(
        oracle["failed_oracle_ids"],
        serde_json::json!(["O-metric-4"])
    );
}

fn repo_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

fn falsify_gbf_bin(temp: &std::path::Path) -> std::path::PathBuf {
    let status = Command::new("cargo")
        .current_dir(repo_root())
        .args(["build", "-p", "gbf-cli", "--features", "falsify"])
        .status()
        .expect("build falsify gbf-cli");
    assert!(status.success(), "falsify gbf-cli build failed");

    let source = repo_root().join("target/debug/gbf-cli");
    let dest = temp.join("gbf-cli-falsify");
    fs::copy(&source, &dest).expect("copy falsify gbf-cli");
    dest
}
