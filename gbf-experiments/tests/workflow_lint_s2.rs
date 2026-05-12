use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const WORKFLOWS: [&str; 3] = ["s2-pr.yml", "s2-nightly.yml", "s2-on-demand.yml"];
const S2_SCRIPTS: [&str; 5] = [
    "scripts/s2_preregistration_check.sh",
    "scripts/s2_determinism_check.sh",
    "scripts/s2_isolation_check.sh",
    "scripts/s2_api_drift_check.sh",
    "scripts/s2_distill_determinism_check.sh",
];
const RAW_WORKSPACE_ALL_FEATURES_CLIPPY: &str =
    "cargo clippy --workspace --all-features -- -D warnings";

#[test]
fn s2_workflows_are_yaml_shaped_and_upload_forensic_artifacts() {
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
                "/tmp/s2-*.json",
                "experiments/S2/**",
                "bd-30fu",
            ],
        );
        assert_capture_modes_are_honest(&source, workflow);
    }
}

#[test]
fn s2_pr_workflow_pins_rfc_pr_gates() {
    let source = read_workflow("s2-pr.yml");
    assert_contains_all(
        &source,
        "s2-pr.yml",
        &[
            "cargo fmt --check --all",
            "cargo clippy --workspace --exclude gbf-cli --exclude gbf-train --exclude gbf-experiments --all-features -- -D warnings",
            "cargo clippy -p gbf-train --features qat,burn-adapter -- -D warnings",
            "cargo clippy -p gbf-train --features qat-ablation,burn-adapter -- -D warnings",
            "cargo clippy -p gbf-experiments --no-default-features --features s2-full -- -D warnings",
            "cargo clippy -p gbf-experiments --no-default-features --features s2-ablation -- -D warnings",
            "cargo clippy -p gbf-cli --no-default-features --features s2-full -- -D warnings",
            "cargo test -p gbf-experiments --features s2-full",
            "cargo test -p gbf-experiments --features falsify --test falsification",
            "cargo test -p gbf-experiments --test loss_grad_flow_s2",
            "cargo test -p gbf-experiments --test linearstate_smoke_s2",
            "cargo test -p gbf-experiments --test phase_transition_integ_s2",
            "cargo test -p gbf-experiments --test canonical_json_s2",
            "cargo test -p gbf-experiments --test integration_s2",
            "cargo test -p gbf-experiments --test oracle_re_run_s2",
            "cargo test -p gbf-experiments --test outcome_totality_s2",
            "cargo test -p gbf-train --features burn-adapter -- linear_state::gradient phase::linear_state_hardness distillation loss::config teacher::freeze",
            "cargo build -p gbf-experiments --features s2-full",
            "cargo build -p gbf-experiments --no-default-features --features s2-ablation",
        ],
    );
    assert_script_references_are_known(&source, "s2-pr.yml");
    assert_required_cargo_invocations(&source, "s2-pr.yml", s2_pr_cargo_invocations());
}

#[test]
fn s2_nightly_and_on_demand_are_parameterized_and_call_script_gates() {
    let nightly = read_workflow("s2-nightly.yml");
    assert_contains_all(
        &nightly,
        "s2-nightly.yml",
        &[
            "schedule:",
            "workflow_dispatch:",
            "seed_list:",
            "pass_version:",
            "S2_SEED_LIST:",
            "S2_BUILDS:",
            "S2_PASS_VERSION:",
            "--seed-list \"$S2_SEED_LIST\"",
            "--builds \"$S2_BUILDS\"",
            "replay-full-${pass_slug}.json",
            "distill-once-${pass_slug}.json",
            "report-${pass_slug}.json",
            "commit-comment-${pass_slug}.json",
            "cargo test -p gbf-experiments --test integration_s2",
            "Comment S2 outcome on commit",
            "gh api --method POST \"repos/${GITHUB_REPOSITORY}/commits/${commit_sha}/comments\"",
        ],
    );
    assert_report_generation_and_commit_comment(&nightly, "s2-nightly.yml");
    assert_script_references_are_known(&nightly, "s2-nightly.yml");
    assert_required_cargo_invocations(&nightly, "s2-nightly.yml", s2_nightly_cargo_invocations());

    let on_demand = read_workflow("s2-on-demand.yml");
    assert_contains_all(
        &on_demand,
        "s2-on-demand.yml",
        &[
            "workflow_dispatch:",
            "ref:",
            "seed_list:",
            "builds:",
            "pass_version:",
            "S2_BUILDS:",
            "S2_PASS_VERSION:",
            "--seed-list \"$S2_SEED_LIST\"",
            "--builds \"$S2_BUILDS\"",
            "replay-full-${pass_slug}.json",
            "distill-once-${pass_slug}.json",
            "report-${pass_slug}.json",
            "commit-comment-${pass_slug}.json",
            "cargo test -p gbf-experiments --test integration_s2",
            "Comment S2 outcome on commit",
            "gh api --method POST \"repos/${GITHUB_REPOSITORY}/commits/${commit_sha}/comments\"",
        ],
    );
    assert_report_generation_and_commit_comment(&on_demand, "s2-on-demand.yml");
    assert_script_references_are_known(&on_demand, "s2-on-demand.yml");
    assert_required_cargo_invocations(
        &on_demand,
        "s2-on-demand.yml",
        s2_on_demand_cargo_invocations(),
    );
}

#[test]
fn s2_script_ndjson_content_shape_is_bound_to_cli_scripts_gate() {
    let script_tests =
        fs::read_to_string(repo_root().join("gbf-experiments/tests/cli_scripts_s2.rs"))
            .expect("cli_scripts_s2 test source must be readable");
    assert_contains_all(
        &script_tests,
        "cli_scripts_s2.rs",
        &[
            "fn s2_scripts_dry_run_emit_schema_and_stable_reports()",
            "fn assert_ndjson_events(",
            "fn assert_report_schema(",
            "stage_start",
            "stage_done",
            "exit_code",
            "event field",
        ],
    );
}

#[test]
fn s2_workflow_lint_rejects_raw_cargo_outside_capture_wrappers() {
    let raw_block_cargo = r#"
name: bad
on:
  pull_request:
jobs:
  s2:
    runs-on: ubuntu-latest
    steps:
      - run: |
          cargo test -p gbf-experiments --test integration_s2
"#;
    let panic = std::panic::catch_unwind(|| {
        assert_capture_modes_are_honest(raw_block_cargo, "fixture.yml");
    })
    .expect_err("raw cargo invocation should be rejected");
    let message = panic_message(&panic);

    assert!(
        message.contains("raw cargo invocation outside run_capture_log wrapper"),
        "unexpected panic message: {message}"
    );

    let raw_inline_cargo = r#"
name: bad-inline
on:
  pull_request:
jobs:
  s2:
    runs-on: ubuntu-latest
    steps:
      - run: cargo test -p gbf-experiments --test integration_s2
"#;
    let panic = std::panic::catch_unwind(|| {
        assert_capture_modes_are_honest(raw_inline_cargo, "fixture.yml");
    })
    .expect_err("inline raw cargo invocation should be rejected");
    let message = panic_message(&panic);

    assert!(
        message.contains("raw cargo invocation outside run_capture_log wrapper"),
        "unexpected panic message: {message}"
    );
}

fn assert_script_references_are_known(source: &str, workflow: &str) {
    let expected = S2_SCRIPTS.into_iter().collect::<BTreeSet<_>>();
    let observed = source
        .split_whitespace()
        .filter(|token| token.starts_with("scripts/s2_"))
        .map(|token| token.trim_matches(|ch: char| ch == '"' || ch == '\''))
        .collect::<BTreeSet<_>>();

    assert_eq!(
        observed, expected,
        "{workflow} must reference exactly the S2 script gate set"
    );
    for script in observed {
        assert!(
            repo_root().join(script).exists(),
            "{workflow} references missing script {script}"
        );
    }
    assert_cargo_test_targets_exist(source, workflow);
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
            panic!("{workflow} has cargo test --test without a target: {line}");
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
        if line.contains("scripts/s2_") {
            assert!(
                line.starts_with("run_capture_ndjson "),
                "{workflow} S2 script command must capture structured stderr as .ndjson: {line}"
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
        || line.starts_with("if ! \"$@\"")
    {
        return;
    }

    let has_raw_cargo = line.starts_with("cargo ")
        || line.starts_with("- cargo ")
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
    assert_cargo_build_targets_are_pinned(source, workflow);
    if source.contains("cargo clippy ") {
        assert_supported_clippy_matrix(source, workflow);
    }
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
        let Some((name, command)) = captured_command(line) else {
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

fn captured_command(line: &str) -> Option<(&str, &str)> {
    let rest = line.strip_prefix("run_capture_log ")?;
    rest.split_once(' ')
}

fn assert_cargo_build_targets_are_pinned(source: &str, workflow: &str) {
    for line in source.lines().map(str::trim) {
        if !line.contains("cargo build -p gbf-experiments") {
            continue;
        }
        let allowed = line.contains("cargo build -p gbf-experiments --features s2-full")
            || line.contains(
                "cargo build -p gbf-experiments --no-default-features --features s2-ablation",
            );
        assert!(
            allowed,
            "{workflow} has unpinned S2 build invocation: {line}"
        );
    }
}

fn assert_report_generation_and_commit_comment(source: &str, workflow: &str) {
    assert_contains_all(
        source,
        workflow,
        &[
            "contents: write",
            "docs/experiments/S2-report.md",
            "--replay-full-json \"$S2_ARTIFACT_DIR/replay-full-${pass_slug}.json\"",
            "--distill-json \"$S2_ARTIFACT_DIR/distill-once-${pass_slug}.json\"",
            "replay-evidence report",
            "fixture/default verifier fields remain present",
            "not full live closure and not deployable artifact acceptance",
            "provenance-only; distill JSON bytes are not threaded into report_self_hash",
            "s2_outcome",
            "report_self_hash",
            "s2 replay-full",
            "s2 distill-once",
            "s2 report",
            "body = \"\\n\".join([",
            "comment_md.write_text(body, encoding=\"utf-8\")",
            "comment_payload.write_text(json.dumps({\"body\": body}, sort_keys=True)",
            "--input \"$S2_ARTIFACT_DIR/commit-comment-${pass_slug}.json\"",
            "commit-comment.stdout",
            "commit-comment.stderr.log",
        ],
    );
}

fn assert_supported_clippy_matrix(source: &str, workflow: &str) {
    assert!(
        !source.contains(RAW_WORKSPACE_ALL_FEATURES_CLIPPY),
        "{workflow} must not use raw workspace all-features clippy because S2/QAT features are mutually exclusive"
    );
    for (name, command) in supported_clippy_invocations() {
        let expected_line = format!("run_capture_log {name} {command}");
        assert!(
            source.contains(&expected_line),
            "{workflow} is missing supported clippy matrix entry {expected_line:?}"
        );
    }
}

fn supported_clippy_invocations() -> BTreeMap<&'static str, &'static str> {
    BTreeMap::from([
        (
            "cargo-clippy-workspace-supported",
            "cargo clippy --workspace --exclude gbf-cli --exclude gbf-train --exclude gbf-experiments --all-features -- -D warnings",
        ),
        (
            "cargo-clippy-gbf-train-qat",
            "cargo clippy -p gbf-train --features qat,burn-adapter -- -D warnings",
        ),
        (
            "cargo-clippy-gbf-train-qat-ablation",
            "cargo clippy -p gbf-train --features qat-ablation,burn-adapter -- -D warnings",
        ),
        (
            "cargo-clippy-gbf-experiments-s2-full",
            "cargo clippy -p gbf-experiments --no-default-features --features s2-full -- -D warnings",
        ),
        (
            "cargo-clippy-gbf-experiments-s2-ablation",
            "cargo clippy -p gbf-experiments --no-default-features --features s2-ablation -- -D warnings",
        ),
        (
            "cargo-clippy-gbf-cli-s2-full",
            "cargo clippy -p gbf-cli --no-default-features --features s2-full -- -D warnings",
        ),
    ])
}

fn s2_pr_cargo_invocations() -> BTreeMap<&'static str, &'static str> {
    let mut expected = BTreeMap::from([
        ("cargo-fmt", "cargo fmt --check --all"),
        (
            "gbf-experiments-s2-full",
            "cargo test -p gbf-experiments --features s2-full",
        ),
        (
            "falsification-s2",
            "cargo test -p gbf-experiments --features falsify --test falsification",
        ),
        (
            "loss-grad-flow-s2",
            "cargo test -p gbf-experiments --test loss_grad_flow_s2",
        ),
        (
            "linearstate-smoke-s2",
            "cargo test -p gbf-experiments --test linearstate_smoke_s2",
        ),
        (
            "phase-transition-integ-s2",
            "cargo test -p gbf-experiments --test phase_transition_integ_s2",
        ),
        (
            "canonical-json-s2",
            "cargo test -p gbf-experiments --test canonical_json_s2",
        ),
        (
            "integration-s2",
            "cargo test -p gbf-experiments --test integration_s2",
        ),
        (
            "oracle-re-run-s2",
            "cargo test -p gbf-experiments --test oracle_re_run_s2",
        ),
        (
            "outcome-totality-s2",
            "cargo test -p gbf-experiments --test outcome_totality_s2",
        ),
        (
            "gbf-train-burn-s2",
            "cargo test -p gbf-train --features burn-adapter -- linear_state::gradient phase::linear_state_hardness distillation loss::config teacher::freeze",
        ),
        (
            "gbf-experiments-build-s2-full",
            "cargo build -p gbf-experiments --features s2-full",
        ),
        (
            "gbf-experiments-build-s2-ablation",
            "cargo build -p gbf-experiments --no-default-features --features s2-ablation",
        ),
    ]);
    expected.extend(supported_clippy_invocations());
    expected
}

fn s2_nightly_cargo_invocations() -> BTreeMap<&'static str, &'static str> {
    let mut expected = BTreeMap::from([
        ("cargo-fmt", "cargo fmt --check --all"),
        (
            "oracle-re-run-s2",
            "cargo test -p gbf-experiments --test oracle_re_run_s2",
        ),
    ]);
    expected.extend(supported_clippy_invocations());
    expected.extend(replay_workflow_cargo_invocations());
    expected
}

fn s2_on_demand_cargo_invocations() -> BTreeMap<&'static str, &'static str> {
    replay_workflow_cargo_invocations()
}

fn replay_workflow_cargo_invocations() -> BTreeMap<&'static str, &'static str> {
    BTreeMap::from([
        (
            "gbf-experiments-build-s2-full",
            "cargo build -p gbf-experiments --features s2-full",
        ),
        (
            "gbf-experiments-build-s2-ablation",
            "cargo build -p gbf-experiments --no-default-features --features s2-ablation",
        ),
        (
            "integration-s2",
            "cargo test -p gbf-experiments --test integration_s2",
        ),
        (
            "replay-full-s2",
            "bash -c 'set -euo pipefail; cargo run --quiet -p gbf-cli --features s2-full -- s2 replay-full --seed-list \"$S2_SEED_LIST\" --builds \"$S2_BUILDS\" --fixture tiny --json | tee \"$S2_ARTIFACT_DIR/replay-full-${pass_slug}.json\"'",
        ),
        (
            "distill-once-s2",
            "bash -c 'set -euo pipefail; cargo run --quiet -p gbf-cli --features s2-full -- s2 distill-once --json | tee \"$S2_ARTIFACT_DIR/distill-once-${pass_slug}.json\"'",
        ),
        (
            "report-s2",
            "bash -c 'set -euo pipefail; cargo run --quiet -p gbf-cli --features s2-full -- s2 report --replay-full-json \"$S2_ARTIFACT_DIR/replay-full-${pass_slug}.json\" --distill-json \"$S2_ARTIFACT_DIR/distill-once-${pass_slug}.json\" --output docs/experiments/S2-report.md --json | tee \"$S2_ARTIFACT_DIR/report-${pass_slug}.json\"'",
        ),
    ])
}

fn assert_contains_all(source: &str, workflow: &str, needles: &[&str]) {
    for needle in needles {
        assert!(
            source.contains(needle),
            "{workflow} is missing expected reference {needle:?}"
        );
    }
}

fn assert_yaml_shape(source: &str, workflow: &str) {
    let mut top_level = BTreeSet::new();
    let mut block_scalar_indent = None;
    for (line_no, line) in source.lines().enumerate() {
        let line_no = line_no + 1;
        assert!(
            !line.contains('\t'),
            "{workflow}:{line_no} contains a tab, which GitHub Actions YAML rejects"
        );
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
                .unwrap_or_else(|| panic!("{workflow}:{line_no} top-level YAML key lacks ':'"));
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

fn read_workflow(name: &str) -> String {
    fs::read_to_string(workflow_path(name)).expect("workflow file must be readable")
}

fn workflow_path(name: &str) -> PathBuf {
    repo_root().join(".github/workflows").join(name)
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("gbf-experiments has a workspace parent")
        .to_path_buf()
}
