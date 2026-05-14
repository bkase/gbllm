use std::path::{Path, PathBuf};
use std::process::Command;

use gbf_experiments::s2::report::predictions_section_hash;
use serde_json::Value;

#[test]
fn clean_history_passes_and_schema_template_is_not_a_result() {
    let fixture = PreregFixture::new();
    let result = fixture.run_check();

    assert!(result.status.success(), "stderr:\n{}", result.stderr);
    assert!(result.stderr.contains("\"event\":\"prereg_done\""));
    assert!(
        result.stderr.contains(&format!(
            "\"commit\":\"{}\",\"event\":\"prereg_commit_scanned\",\"introduces_non_null_result\":false",
            fixture.template_commit
        )),
        "template commit should be scanned as non-result stderr:\n{}",
        result.stderr
    );
    insta::assert_snapshot!(
        "preregistration__clean_history_output",
        fixture.redact_known_commits(&result.stderr)
    );
}

#[test]
fn editing_predictions_after_preregistration_fails_stage_one() {
    let fixture = PreregFixture::new();
    fixture.write_current_report(
        "H2 ternary-full gap remains <= 0.4 bpc.",
        &fixture.predictions_hash,
        &fixture.predictions_commit,
        &fixture.result_commit,
    );
    fixture.commit_all("edit predictions after preregistration");

    let result = fixture.run_check();

    assert!(!result.status.success());
    assert!(result.stderr.contains("\"event\":\"prereg_violation\""));
    assert!(
        result
            .stderr
            .contains("predictions_section_hash mismatch in current report"),
        "{}",
        result.stderr
    );
    let output = fixture.read_output();
    assert_prereg_live_evidence_fields(&output);
    assert_eq!(output["passed"], false);
}

#[test]
fn earlier_result_commit_must_match_recorded_first_result_commit() {
    let fixture = PreregFixture::new();
    fixture.write_current_report(
        PREDICTIONS,
        &fixture.predictions_hash,
        &fixture.predictions_commit,
        &fixture.template_commit,
    );
    fixture.commit_all("record wrong first result commit");

    let result = fixture.run_check();

    assert!(!result.status.success());
    assert!(result.stderr.contains("\"stage\":3"));
    assert!(
        result
            .stderr
            .contains("first_result_commit is not the earliest non-null S2 result commit"),
        "{}",
        result.stderr
    );
}

#[test]
fn step_three_result_scan_is_limited_to_report_and_artifact_paths() {
    let fixture = PreregFixture::new();
    write_file(
        fixture.root.path().join("outside/S2/result.json"),
        "{\"score_self_hash\":\"sha256:3434343434343434343434343434343434343434343434343434343434343434\"}\n",
    );
    let outside_result_commit = fixture.commit_all("add out-of-scope S2-shaped result");

    let result = fixture.run_check();

    assert!(result.status.success(), "stderr:\n{}", result.stderr);
    assert!(
        !result.stderr.contains(&outside_result_commit),
        "out-of-scope result commit should not be scanned stderr:\n{}",
        result.stderr
    );
    let output = fixture.read_output();
    assert_prereg_live_evidence_fields(&output);
    assert_eq!(
        output["earliest_result_commit"].as_str(),
        Some(fixture.result_commit.as_str()),
        "Step 3 is intentionally path-filtered to --report and --artifact-dir"
    );
}

#[test]
fn non_ascii_predictions_section_hash_matches_python_checker() {
    let predictions = "H2 café Δ stays ≤ 0.5 bpc; 東京 seed stays deterministic.";
    let fixture = PreregFixture::new_with_predictions(predictions);

    let result = fixture.run_check();

    assert!(result.status.success(), "stderr:\n{}", result.stderr);
    assert!(
        result
            .stderr
            .contains("\"event\":\"prereg_section_hash_compare\"")
    );
    let output = fixture.read_output();
    assert_prereg_live_evidence_fields(&output);
    assert_eq!(
        output["predictions_section_hash"].as_str(),
        Some(fixture.predictions_hash.as_str())
    );
}

#[test]
fn prereg_script_documents_live_only_no_dry_run_contract() {
    let script = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("workspace root")
            .join("scripts/s2_preregistration_check.sh"),
    )
    .expect("read prereg script");

    assert!(script.contains("intentionally has no --dry-run mode"));
    assert!(script.contains("git-history/preregistration scan"));
    assert!(script.contains("\"dry_run\": False"));
    assert!(script.contains("\"evidence_mode\": \"live\""));
    assert!(script.contains("\"live_evidence\": True"));
    assert!(
        !script.contains("parser.add_argument(\"--dry-run\""),
        "S2 prereg must remain live-only because dry-run would not exercise git-history evidence"
    );
}

const PREDICTIONS: &str = "H2 ternary-full gap remains <= 0.5 bpc.";

struct PreregFixture {
    root: tempfile::TempDir,
    predictions_hash: String,
    predictions_commit: String,
    template_commit: String,
    result_commit: String,
    final_commit: String,
}

struct ScriptResult {
    status: std::process::ExitStatus,
    stderr: String,
}

impl PreregFixture {
    fn new() -> Self {
        Self::new_with_predictions(PREDICTIONS)
    }

    fn new_with_predictions(predictions: &str) -> Self {
        let root = tempfile::tempdir().expect("tempdir");
        git(root.path(), &["init"]);
        git(root.path(), &["config", "user.email", "s2@example.invalid"]);
        git(root.path(), &["config", "user.name", "S2 Tester"]);

        let predictions_hash = predictions_section_hash(predictions)
            .expect("predictions hash")
            .to_string();
        write_report(root.path(), predictions, &predictions_hash, None, None);
        let predictions_commit = commit_all(root.path(), "pre-register S2 predictions");

        write_file(
            root.path().join("experiments/S2/schema-template.json"),
            "{\"score_self_hash\":null,\"completion\":{\"kind\":\"NotReached\"}}\n",
        );
        let template_commit = commit_all(root.path(), "add S2 schema template");

        write_file(
            root.path().join("experiments/S2/result.json"),
            "{\"score_self_hash\":\"sha256:1212121212121212121212121212121212121212121212121212121212121212\"}\n",
        );
        let result_commit = commit_all(root.path(), "add first S2 result");

        write_report(
            root.path(),
            predictions,
            &predictions_hash,
            Some(&predictions_commit),
            Some(&result_commit),
        );
        let final_commit = commit_all(root.path(), "finalize S2 report");

        Self {
            root,
            predictions_hash,
            predictions_commit,
            template_commit,
            result_commit,
            final_commit,
        }
    }

    fn write_current_report(
        &self,
        predictions: &str,
        predictions_hash: &str,
        predictions_commit: &str,
        first_result_commit: &str,
    ) {
        write_report(
            self.root.path(),
            predictions,
            predictions_hash,
            Some(predictions_commit),
            Some(first_result_commit),
        );
    }

    fn commit_all(&self, message: &str) -> String {
        commit_all(self.root.path(), message)
    }

    fn run_check(&self) -> ScriptResult {
        let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("workspace root")
            .join("scripts/s2_preregistration_check.sh");
        let output = Command::new(script)
            .current_dir(self.root.path())
            .arg("--report")
            .arg("docs/experiments/S2-report.md")
            .arg("--artifact-dir")
            .arg("experiments/S2")
            .arg("--output")
            .arg(self.output_path())
            .output()
            .expect("run prereg script");
        ScriptResult {
            status: output.status,
            stderr: String::from_utf8(output.stderr).expect("stderr utf8"),
        }
    }

    fn read_output(&self) -> Value {
        serde_json::from_str(
            &std::fs::read_to_string(self.output_path()).expect("prereg output JSON"),
        )
        .expect("prereg output parses")
    }

    fn output_path(&self) -> PathBuf {
        self.root.path().join("s2-prereg.json")
    }

    fn redact_known_commits(&self, input: &str) -> String {
        input
            .replace(&self.predictions_commit, "<predictions_commit>")
            .replace(&self.template_commit, "<template_commit>")
            .replace(&self.result_commit, "<first_result_commit>")
            .replace(&self.final_commit, "<final_report_commit>")
    }
}

fn write_report(
    root: &Path,
    predictions: &str,
    predictions_hash: &str,
    predictions_commit: Option<&str>,
    first_result_commit: Option<&str>,
) {
    let front_matter = serde_json::json!({
        "schema": "s2_report.v1",
        "predictions_section_hash": predictions_hash,
        "predictions_commit": predictions_commit,
        "first_result_commit": first_result_commit,
        "report_self_hash": null,
    });
    let front_matter = serde_json::to_string(&front_matter).expect("front matter serializes");
    write_file(
        root.join("docs/experiments/S2-report.md"),
        &format!(
            "---\n{front_matter}\n---\n# S2 Report\n\n## Pre-registered predictions\n\n{predictions}\n\n## Observed\n\nPending.\n"
        ),
    );
}

fn write_file(path: PathBuf, contents: &str) {
    std::fs::create_dir_all(path.parent().expect("parent")).expect("create parent");
    std::fs::write(path, contents).expect("write file");
}

fn commit_all(root: &Path, message: &str) -> String {
    git(root, &["add", "."]);
    git(root, &["commit", "-m", message]);
    git(root, &["rev-parse", "HEAD"]).trim().to_owned()
}

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {:?} failed\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("git stdout utf8")
}

fn assert_prereg_live_evidence_fields(output: &Value) {
    assert_eq!(output["script"], "s2_preregistration_check");
    assert_eq!(output["dry_run"], false);
    assert_eq!(output["evidence_mode"], "live");
    assert_eq!(output["live_evidence"], true);
    assert_eq!(
        output["evidence_source"],
        "git-history-report-and-artifact-scan"
    );
}
