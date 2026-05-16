#![cfg(feature = "s3")]

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;

fn gbf() -> Command {
    Command::cargo_bin("gbf-cli").expect("gbf-cli binary")
}

#[test]
fn s3_help_lists_export_bundle() {
    let mut command = gbf();
    command.args(["s3", "--help"]);

    command.assert().success().stdout(
        predicate::str::contains("fit-baseline")
            .and(predicate::str::contains("oracle-re-run"))
            .and(predicate::str::contains("export-bundle"))
            .and(predicate::str::contains("export-artifact")),
    );
}

#[test]
fn s3_export_artifact_help_names_fixture_scope() {
    let mut command = gbf();
    command.args(["s3", "export-artifact", "--help"]);

    command.assert().success().stdout(
        predicate::str::contains("fixture model artifact")
            .and(predicate::str::contains("does not run training"))
            .and(predicate::str::contains("Phase-D runner")),
    );
}

#[test]
fn s3_export_bundle_cli_writes_bundle_metadata_and_events() {
    let temp = tempfile::tempdir().expect("tempdir");
    let events = temp.path().join("events.ndjson");
    let bundle = temp.path().join("bundle.json");
    let metadata = temp.path().join("bundle-metadata.json");

    let mut command = gbf();
    command.args([
        "--capture-events",
        events.to_str().expect("utf8 events path"),
        "s3",
        "export-bundle",
        "--seed",
        "0",
        "--bundle-output",
        bundle.to_str().expect("utf8 bundle path"),
        "--metadata-output",
        metadata.to_str().expect("utf8 metadata path"),
    ]);

    command.assert().success().stdout(predicate::str::contains(
        metadata.to_str().expect("utf8 metadata path"),
    ));

    assert!(bundle.exists(), "bundle file should be written");
    let metadata_value: Value =
        serde_json::from_slice(&std::fs::read(&metadata).expect("metadata reads"))
            .expect("metadata parses");
    assert_eq!(metadata_value["schema"], "s3_bundle.v1");
    assert_eq!(metadata_value["seed"], 0);
    assert_eq!(
        metadata_value["program_validation"]["structural_valid"],
        true
    );
    assert_eq!(
        metadata_value["program_validation"]["argmax_token_all_match"],
        true
    );

    let captured = std::fs::read_to_string(events).expect("events read");
    assert!(captured.contains("\"event_name\":\"s3::bundle_export::started\""));
    assert!(captured.contains("\"event_name\":\"s3::bundle_export::complete\""));
}

#[test]
fn s3_cli_s3_end_to_end_invokes_every_b23_verb() {
    let temp = tempfile::tempdir().expect("tempdir");
    let replay = temp.path().join("replay.json");
    let fallback = temp.path().join("fallback.json");
    let determinism = temp.path().join("determinism.json");
    let charset = temp.path().join("charset.json");
    let baseline = temp.path().join("baseline.json");
    let bundle = temp.path().join("bundle.json");
    let bundle_metadata = temp.path().join("bundle-metadata.json");
    let artifact = temp.path().join("artifact.bin");
    let artifact_metadata = temp.path().join("artifact-metadata.json");
    let agreement = temp.path().join("agreement.json");
    let oracle_re_run = temp.path().join("oracle-re-run.json");
    let report = temp.path().join("S3-report.md");

    assert_cli_success([
        "s3",
        "replay-full",
        "--output",
        replay.to_str().expect("utf8"),
    ]);
    assert_cli_failure_contains(
        [
            "s3",
            "replay-fallback",
            "--output",
            fallback.to_str().expect("utf8 temp path"),
        ],
        "s3-oracle-fallback",
    );
    assert_cli_success([
        "s3",
        "verify-determinism",
        "--seed-list",
        "0",
        "--output",
        determinism.to_str().expect("utf8 temp path"),
    ]);
    assert_cli_success([
        "s3",
        "normalize-corpus",
        "--output",
        charset.to_str().expect("utf8 temp path"),
    ]);
    assert_cli_success([
        "s3",
        "fit-baseline",
        "--output",
        baseline.to_str().expect("utf8 temp path"),
    ]);
    assert_cli_success([
        "s3",
        "export-bundle",
        "--bundle-output",
        bundle.to_str().expect("utf8 temp path"),
        "--metadata-output",
        bundle_metadata.to_str().expect("utf8 temp path"),
    ]);
    assert_cli_success([
        "s3",
        "export-artifact",
        "--artifact-output",
        artifact.to_str().expect("utf8 temp path"),
        "--metadata-output",
        artifact_metadata.to_str().expect("utf8 temp path"),
    ]);
    assert_cli_success([
        "s3",
        "oracle-agreement",
        "--output",
        agreement.to_str().expect("utf8 temp path"),
    ]);
    assert_cli_success([
        "s3",
        "oracle-re-run",
        "--output",
        oracle_re_run.to_str().expect("utf8 temp path"),
    ]);
    assert_cli_success([
        "s3",
        "report",
        "--replay-full",
        replay.to_str().expect("utf8"),
        "--output",
        report.to_str().expect("utf8 temp path"),
    ]);
}

fn assert_cli_success<const N: usize>(args: [&str; N]) {
    let mut command = gbf();
    command.args(args);
    command.assert().success();
}

fn assert_cli_failure_contains<const N: usize>(args: [&str; N], expected: &str) {
    let mut command = gbf();
    command.args(args);
    command
        .assert()
        .failure()
        .stderr(predicate::str::contains(expected));
}
