#![cfg(feature = "s3")]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const WORKFLOWS: [&str; 3] = ["s3-pr.yml", "s3-nightly.yml", "s3-on-demand.yml"];
const S3_SCRIPTS: [&str; 8] = [
    "scripts/s3_preregistration_check.sh",
    "scripts/s3_determinism_check.sh",
    "scripts/s3_full_determinism_check.sh",
    "scripts/s3_isolation_check.sh",
    "scripts/s3_api_drift_check.sh",
    "scripts/s3_oracle_re_run_check.sh",
    "scripts/s3_no_naming_resolution_check.sh",
    "scripts/s3_feature_matrix_check.sh",
];

#[test]
fn s3_workflows_are_yaml_shaped_and_upload_forensic_artifacts() {
    for workflow in WORKFLOWS {
        let source = read_workflow(workflow);
        assert_yaml_shape(&source, workflow);
        assert_contains_all(
            &source,
            workflow,
            &[
                "actions/checkout@v4",
                "dtolnay/rust-toolchain@stable",
                "actions/cache@v4",
                "actions/upload-artifact@v4",
                "run_capture_log()",
                "run_capture_ndjson()",
                ".stderr.log",
                ".ndjson",
                "/tmp/s3-*.json",
                "experiments/S3/**",
            ],
        );
        assert_capture_modes_are_honest(&source, workflow);
        assert_script_references_are_known(&source, workflow);
        assert_checkout_fetches_full_history(&source, workflow);
    }
}

#[test]
fn s3_pr_workflow_pins_b24_trigger_paths_and_gates() {
    let source = read_workflow("s3-pr.yml");
    assert_contains_all(
        &source,
        "s3-pr.yml",
        &[
            "gbf-experiments/**",
            "gbf-train/**",
            "gbf-policy/**",
            "gbf-data/**",
            "gbf-artifact/**",
            "gbf-foundation/**",
            "gbf-model/**",
            "gbf-test/**",
            "gbf-cli/**",
            "gbf-workload/**",
            "gbf-oracle/**",
            "scripts/s3_*.sh",
            "scripts/s3_*.py",
            "history/rfcs/F-S3-v0-success-tinystories.md",
            ".github/workflows/s3-*.yml",
            "s3-pr-artifacts",
        ],
    );
    assert_required_cargo_invocations(&source, "s3-pr.yml", s3_pr_cargo_invocations());
    assert_required_script_invocations(&source, "s3-pr.yml");
}

#[test]
fn s3_nightly_and_on_demand_are_parameterized_and_call_script_gates() {
    let nightly = read_workflow("s3-nightly.yml");
    assert_contains_all(
        &nightly,
        "s3-nightly.yml",
        &[
            "schedule:",
            "workflow_dispatch:",
            "seed_list:",
            "pass_version:",
            "S3_SEED_LIST:",
            "S3_PASS_VERSION:",
            "S3_ARTIFACT_DIR:",
            "--seed-list \"$S3_SEED_LIST\"",
            "replay-full-${pass_slug}.json",
            "artifact-metadata-${pass_slug}.json",
            "scripts/s3_no_naming_resolution_check.sh \"$S3_ARTIFACT_DIR/artifacts/seed-0/artifact-metadata-${pass_slug}.json\"",
            "oracle-re-run-cli-${pass_slug}.json",
            "report-cli-${pass_slug}.json",
            "Comment S3 outcome on commit",
            "gh api --method POST \"repos/${GITHUB_REPOSITORY}/commits/${commit_sha}/comments\"",
            "s3-nightly-artifacts",
        ],
    );
    assert_required_cargo_invocations(
        &nightly,
        "s3-nightly.yml",
        s3_replay_workflow_cargo_invocations(true),
    );
    assert_required_script_invocations(&nightly, "s3-nightly.yml");
    assert_order(
        &nightly,
        &[
            "run_capture_log export-artifact-s3",
            "run_capture_ndjson s3-no-naming-resolution scripts/s3_no_naming_resolution_check.sh \"$S3_ARTIFACT_DIR/artifacts/seed-0/artifact-metadata-${pass_slug}.json\"",
        ],
        "s3-nightly.yml",
    );

    let on_demand = read_workflow("s3-on-demand.yml");
    assert_contains_all(
        &on_demand,
        "s3-on-demand.yml",
        &[
            "workflow_dispatch:",
            "ref:",
            "seed_list:",
            "pass_version:",
            "S3_SEED_LIST:",
            "S3_PASS_VERSION:",
            "S3_ARTIFACT_DIR:",
            "ref: ${{ github.event.inputs.ref }}",
            "--seed-list \"$S3_SEED_LIST\"",
            "replay-full-${pass_slug}.json",
            "report-cli-${pass_slug}.json",
            "Comment S3 outcome on commit",
            "gh api --method POST \"repos/${GITHUB_REPOSITORY}/commits/${commit_sha}/comments\"",
            "s3-on-demand-artifacts",
        ],
    );
    assert_required_cargo_invocations(
        &on_demand,
        "s3-on-demand.yml",
        s3_replay_workflow_cargo_invocations(false),
    );
    assert_required_script_invocations(&on_demand, "s3-on-demand.yml");
}

#[test]
fn s3_workflow_lint_rejects_raw_cargo_outside_capture_wrappers() {
    let raw_cargo = r#"
name: bad
on:
  pull_request:
jobs:
  s3:
    runs-on: ubuntu-latest
    steps:
      - run: cargo test -p gbf-experiments --test cli_scripts_s3
"#;
    let panic = std::panic::catch_unwind(|| {
        assert_capture_modes_are_honest(raw_cargo, "fixture.yml");
    })
    .expect_err("raw cargo invocation should be rejected");
    assert!(
        panic_message(&panic).contains("raw cargo invocation outside run_capture_log wrapper"),
        "unexpected panic: {}",
        panic_message(&panic)
    );
}

fn assert_required_script_invocations(source: &str, workflow: &str) {
    for script in S3_SCRIPTS {
        assert!(
            source.contains(script),
            "{workflow} is missing script gate {script}"
        );
        assert!(
            source.contains(&format!("run_capture_ndjson {}", script_slug(script))),
            "{workflow} should capture {script} as NDJSON"
        );
    }
}

fn script_slug(script: &str) -> &'static str {
    match script {
        "scripts/s3_preregistration_check.sh" => "s3-preregistration",
        "scripts/s3_determinism_check.sh" => "s3-determinism",
        "scripts/s3_full_determinism_check.sh" => "s3-full-determinism",
        "scripts/s3_isolation_check.sh" => "s3-isolation",
        "scripts/s3_api_drift_check.sh" => "s3-api-drift",
        "scripts/s3_oracle_re_run_check.sh" => "s3-oracle-re-run",
        "scripts/s3_no_naming_resolution_check.sh" => "s3-no-naming-resolution",
        "scripts/s3_feature_matrix_check.sh" => "s3-feature-matrix",
        _ => panic!("unknown S3 script {script}"),
    }
}

fn assert_script_references_are_known(source: &str, workflow: &str) {
    let expected = S3_SCRIPTS.into_iter().collect::<BTreeSet<_>>();
    let observed = source
        .split_whitespace()
        .filter(|token| token.starts_with("scripts/s3_") && token.contains("_check.sh"))
        .map(|token| token.trim_matches(|ch: char| ch == '"' || ch == '\''))
        .collect::<BTreeSet<_>>();

    assert_eq!(
        observed, expected,
        "{workflow} must reference exactly the S3 script gate set"
    );
    for script in observed {
        assert!(
            repo_root().join(script).exists(),
            "{workflow} references missing script {script}"
        );
    }
    assert_cargo_test_targets_exist(source, workflow);
}

fn assert_checkout_fetches_full_history(source: &str, workflow: &str) {
    let checkout = "uses: actions/checkout@v4";
    let Some(start) = source.find(checkout) else {
        panic!("{workflow} must use actions/checkout@v4");
    };
    let checkout_block = &source[start..];
    let checkout_block = checkout_block
        .split("\n\n")
        .next()
        .expect("checkout block exists");
    assert!(
        checkout_block.contains("fetch-depth: 0"),
        "{workflow} must fetch full history for strict S3 prediction ancestry"
    );
}

fn assert_capture_modes_are_honest(source: &str, workflow: &str) {
    for line in source.lines().map(str::trim) {
        assert_no_raw_cargo_invocation(line, workflow);
        if !line.starts_with("run_capture_") {
            continue;
        }
        if line.contains(" cargo ") || line.contains(" bash -c 'set -euo pipefail; cargo ") {
            assert!(
                line.starts_with("run_capture_log "),
                "{workflow} cargo command must capture stderr as .stderr.log, not NDJSON: {line}"
            );
        }
        if line.contains("scripts/s3_") {
            assert!(
                line.starts_with("run_capture_ndjson "),
                "{workflow} S3 script command must capture structured stderr as .ndjson: {line}"
            );
        }
    }
}

fn assert_no_raw_cargo_invocation(line: &str, workflow: &str) {
    let inline_run = line
        .strip_prefix("- run: ")
        .or_else(|| line.strip_prefix("run: "));
    if line.is_empty()
        || line.starts_with('#')
        || line.starts_with("run_capture_log ")
        || line.starts_with("run_capture_ndjson ")
        || line.starts_with("run_capture_log()")
        || line.starts_with("run_capture_ndjson()")
    {
        return;
    }

    let has_raw_cargo = line.starts_with("cargo ")
        || inline_run.is_some_and(|command| {
            command.starts_with("cargo ")
                || command.starts_with("bash -c 'set -euo pipefail; cargo ")
        })
        || line.starts_with("bash -c 'set -euo pipefail; cargo ")
        || line.contains("; cargo ")
        || line.contains("| cargo ")
        || line.contains("&& cargo ");
    assert!(
        !has_raw_cargo,
        "{workflow} has raw cargo invocation outside run_capture_log wrapper: {line}"
    );
}

fn assert_required_cargo_invocations(
    source: &str,
    workflow: &str,
    expected: BTreeMap<&'static str, &'static str>,
) {
    for (name, command) in &expected {
        let expected_line = format!("run_capture_log {name} {command}");
        assert!(
            source.contains(&expected_line),
            "{workflow} is missing pinned cargo invocation {expected_line:?}"
        );
    }
    assert_only_expected_cargo_invocations(source, workflow, expected);
}

fn assert_only_expected_cargo_invocations(
    source: &str,
    workflow: &str,
    expected: BTreeMap<&'static str, &'static str>,
) {
    for line in source.lines().map(str::trim) {
        if !line.starts_with("run_capture_log ") {
            continue;
        }
        let Some((name, command)) = line
            .strip_prefix("run_capture_log ")
            .and_then(|rest| rest.split_once(' '))
        else {
            continue;
        };
        let is_cargo = command.starts_with("cargo ")
            || command.starts_with("bash -c 'set -euo pipefail; cargo ");
        if !is_cargo {
            continue;
        }
        let Some(expected_command) = expected.get(name) else {
            panic!("{workflow} has unpinned cargo invocation {name:?}: {command}");
        };
        assert_eq!(
            command, *expected_command,
            "{workflow} cargo invocation {name:?} drifted"
        );
    }
}

fn s3_pr_cargo_invocations() -> BTreeMap<&'static str, &'static str> {
    BTreeMap::from([
        ("cargo-fmt", "cargo fmt --check --all"),
        (
            "cargo-clippy-workspace-supported",
            "cargo clippy --workspace --exclude gbf-cli --exclude gbf-train --exclude gbf-experiments --exclude gbf-test --all-features -- -D warnings",
        ),
        (
            "cargo-clippy-gbf-experiments-s3",
            "cargo clippy -p gbf-experiments --no-default-features --features s3,s3-phase-d,s3-oracle-real -- -D warnings",
        ),
        (
            "cargo-clippy-gbf-cli-s3",
            "cargo clippy -p gbf-cli --no-default-features --features s3-full -- -D warnings",
        ),
        (
            "gbf-experiments-s3",
            "cargo test -p gbf-experiments --features s3,s3-phase-d,s3-oracle-real",
        ),
        (
            "gbf-cli-s3",
            "cargo test -p gbf-cli --features s3-full --test s3_cli_s3",
        ),
        (
            "falsification-s3",
            "cargo test -p gbf-experiments --features s3,s3-phase-d,s3-oracle-real,falsify --test falsification_s3",
        ),
        (
            "conformance-s3",
            "cargo test -p gbf-experiments --features s3,s3-phase-d,s3-oracle-real --test conformance_round_trip_s3",
        ),
        (
            "v0-success-s3",
            "cargo test -p gbf-experiments --features s3,s3-phase-d,s3-oracle-real --test v0_success_canonical_s3",
        ),
        (
            "cli-scripts-s3",
            "cargo test -p gbf-experiments --features s3,s3-phase-d,s3-oracle-real --test cli_scripts_s3",
        ),
        (
            "cli-scripts-event-shape-s3",
            "cargo test -p gbf-experiments --features s3,s3-phase-d,s3-oracle-real --test cli_scripts_event_shape_s3",
        ),
        (
            "workflow-lint-s3",
            "cargo test -p gbf-experiments --features s3,s3-phase-d,s3-oracle-real --test workflow_lint_s3",
        ),
        (
            "workflow-regression-s3",
            "cargo test -p gbf-experiments --features s3,s3-phase-d,s3-oracle-real --test workflow_regression_s3",
        ),
    ])
}

fn s3_replay_workflow_cargo_invocations(nightly: bool) -> BTreeMap<&'static str, &'static str> {
    let mut expected = BTreeMap::from([
        (
            "gbf-experiments-build-s3",
            "cargo build -p gbf-experiments --no-default-features --features s3,s3-phase-d,s3-oracle-real",
        ),
        (
            "gbf-cli-build-s3",
            "cargo build -p gbf-cli --no-default-features --features s3-full",
        ),
        (
            "replay-full-s3",
            "bash -c 'set -euo pipefail; cargo run --quiet -p gbf-cli --features s3-full -- s3 replay-full --seed-list \"$S3_SEED_LIST\" --pass-version \"$S3_PASS_VERSION\" --output \"$S3_ARTIFACT_DIR/replay-full-${pass_slug}.json\"'",
        ),
        (
            "report-s3",
            "bash -c 'set -euo pipefail; cargo run --quiet -p gbf-cli --features s3-full -- s3 report --replay-full \"$S3_ARTIFACT_DIR/replay-full-${pass_slug}.json\" --output docs/experiments/S3-report.md --evidence-output \"$S3_ARTIFACT_DIR/report-cli-${pass_slug}.json\"'",
        ),
    ]);
    if nightly {
        expected.insert("cargo-fmt", "cargo fmt --check --all");
        expected.insert(
            "oracle-re-run-s3",
            "bash -c 'set -euo pipefail; cargo run --quiet -p gbf-cli --features s3-full -- s3 oracle-re-run --output \"$S3_ARTIFACT_DIR/oracle-re-run-${pass_slug}.json\" --evidence-output \"$S3_ARTIFACT_DIR/oracle-re-run-cli-${pass_slug}.json\"'",
        );
        expected.insert(
            "export-artifact-s3",
            "bash -c 'set -euo pipefail; cargo run --quiet -p gbf-cli --features s3-full -- s3 export-artifact --artifact-output \"$S3_ARTIFACT_DIR/artifacts/seed-0/artifact-${pass_slug}.bin\" --metadata-output \"$S3_ARTIFACT_DIR/artifacts/seed-0/artifact-metadata-${pass_slug}.json\" --evidence-output \"$S3_ARTIFACT_DIR/artifacts/seed-0/artifact-cli-${pass_slug}.json\"'",
        );
        expected.insert(
            "report-s3",
            "bash -c 'set -euo pipefail; cargo run --quiet -p gbf-cli --features s3-full -- s3 report --replay-full \"$S3_ARTIFACT_DIR/replay-full-${pass_slug}.json\" --oracle-re-run \"$S3_ARTIFACT_DIR/oracle-re-run-cli-${pass_slug}.json\" --output docs/experiments/S3-report.md --evidence-output \"$S3_ARTIFACT_DIR/report-cli-${pass_slug}.json\"'",
        );
    }
    expected
}

fn assert_cargo_test_targets_exist(source: &str, workflow: &str) {
    for line in source.lines() {
        let line = line.trim();
        if !line.contains("cargo test -p gbf-experiments") || !line.contains("--test") {
            continue;
        }
        let Some(test_name) = line
            .split_whitespace()
            .skip_while(|token| *token != "--test")
            .nth(1)
        else {
            panic!("{workflow} has cargo test --test without target: {line}");
        };
        let test_name = test_name.trim_matches(|ch: char| ch == '"' || ch == '\'');
        assert!(
            repo_root()
                .join("gbf-experiments/tests")
                .join(format!("{test_name}.rs"))
                .exists(),
            "{workflow} references missing cargo test target {test_name}"
        );
    }
}

fn assert_contains_all(source: &str, workflow: &str, needles: &[&str]) {
    for needle in needles {
        assert!(
            source.contains(needle),
            "{workflow} is missing expected reference {needle:?}"
        );
    }
}

fn assert_order(source: &str, needles: &[&str], workflow: &str) {
    let mut offset = 0;
    for needle in needles {
        let haystack = &source[offset..];
        let Some(index) = haystack.find(needle) else {
            panic!("{workflow} is missing ordered reference {needle:?}");
        };
        offset += index + needle.len();
    }
}

fn assert_yaml_shape(source: &str, workflow: &str) {
    let mut top_level = BTreeSet::new();
    let mut block_scalar_indent = None;
    for (line_no, line) in source.lines().enumerate() {
        let line_no = line_no + 1;
        assert!(!line.contains('\t'), "{workflow}:{line_no} contains a tab");
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indent = line.len() - line.trim_start_matches(' ').len();
        if let Some(block_indent) = block_scalar_indent {
            if indent > block_indent {
                continue;
            }
            block_scalar_indent = None;
        }
        if indent == 0 {
            let key = trimmed
                .split_once(':')
                .map(|(key, _)| key)
                .unwrap_or_else(|| panic!("{workflow}:{line_no} top-level key lacks ':'"));
            top_level.insert(key.to_owned());
        }
        if trimmed.ends_with('|') {
            block_scalar_indent = Some(indent);
            continue;
        }
        assert!(
            trimmed.starts_with("- ") || trimmed.contains(':'),
            "{workflow}:{line_no} is not a YAML mapping or sequence item: {trimmed}"
        );
    }
    for key in ["name", "on", "concurrency", "permissions", "jobs"] {
        assert!(
            top_level.contains(key),
            "{workflow} is missing top-level YAML key {key:?}"
        );
    }
}

fn panic_message(panic: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = panic.downcast_ref::<String>() {
        message.clone()
    } else if let Some(message) = panic.downcast_ref::<&'static str>() {
        (*message).to_owned()
    } else {
        "<non-string panic>".to_owned()
    }
}

fn read_workflow(name: &str) -> String {
    fs::read_to_string(repo_root().join(".github/workflows").join(name))
        .expect("workflow file reads")
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("gbf-experiments has workspace parent")
        .to_path_buf()
}
