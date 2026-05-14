use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::{Value, json};
use std::path::Path;
use std::process::{Command as ProcessCommand, Output};

fn gbf() -> Command {
    Command::cargo_bin("gbf-cli").expect("gbf-cli binary")
}

#[test]
fn s2_help_lists_replay_and_diagnostic_subcommands() {
    let mut command = gbf();
    command.args(["s2", "--help"]);

    command.assert().success().stdout(
        predicate::str::contains("replay-full")
            .and(predicate::str::contains("replay-ablation"))
            .and(predicate::str::contains("verify-determinism"))
            .and(predicate::str::contains("grad-flow"))
            .and(predicate::str::contains("linearstate-smoke"))
            .and(predicate::str::contains("phase-integ"))
            .and(predicate::str::contains("oracle-re-run"))
            .and(predicate::str::contains("report")),
    );
}

#[test]
fn replay_full_seed0_tiny_exits_zero() {
    let temp = tempfile::tempdir().expect("tempdir");
    let events = temp.path().join("events.ndjson");
    let mut command = gbf();
    command.args([
        "--log-format",
        "json",
        "--capture-events",
        events.to_str().expect("utf8 path"),
        "s2",
        "replay-full",
        "--manifest",
        "stub",
        "--pass-version",
        "0.0.0",
        "--seed-list",
        "0",
        "--builds",
        "s2_ternary_full",
        "--device-profile",
        "S1CpuDeterministic",
        "--json",
    ]);

    let output = command.assert().success().get_output().clone();
    let replay = json_stdout_payload(&output, "s2_replay_full_cli.v1");
    // Inline json! golden keeps the full runs payload pinned without adding an
    // insta snapshot file for this tiny CLI contract test.
    assert_eq!(
        replay,
        json!({
            "evidence_source": "gbf s2 replay-full",
            "fixture": "tiny",
            "manifest": "stub",
            "pass_version": "0.0.0",
            "runs": [{
                "build_kind": "s2-ternary-full",
                "checkpoints": {
                    "4000": "sha256:22e27e5c2b63163fed0bd19f6197242030b9708a760c83734a2f3d26ba01daaa",
                    "5000": "sha256:a153b6863b7048fb0e7744c3f6ec6a5b1a74d4e65de93597bb8a85d5a36c8ea6",
                    "8000": "sha256:6c38dcd57c2fd5509d03474b2acb80c41a29e9b1f14a23bafe08e8b33c8727c9",
                    "10000": "sha256:ea0948e99c1ddcc3856f2ceeccadd3c4807b131b9de98d32aa9dada69e78bd80"
                },
                "distill_log_self_hash": "sha256:1175cd5b7bcad00faebd78b54d6e3ee77eac7941b6001fc089ab14dbc2ce8d5c",
                "final_checkpoint_sha": "sha256:ea0948e99c1ddcc3856f2ceeccadd3c4807b131b9de98d32aa9dada69e78bd80",
                "phase_boundary_steps": ["4000", "5000", "8000", "10000"],
                "phase_log_self_hash": "sha256:0071113cdd2d3f640b2ca20c45fdd39c3fc5c04ef02ce4e306ba568aa04ddc15",
                "score_self_hash": "sha256:2a22b245542a24260f6e88a627484ad035e1eee7519a2328fbe574444061641b",
                "seed": 0
            }],
            "schema": "s2_replay_full_cli.v1"
        })
    );

    let captured = std::fs::read_to_string(events).expect("capture file");
    let start = captured
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("event JSON"))
        .find(|event| event["event"] == "cli_subcommand_start")
        .expect("start event");
    assert_eq!(start["command"], "replay-full");
    assert_eq!(start["args"]["manifest"], "stub");
    assert_eq!(start["args"]["pass_version"], "0.0.0");
    assert_eq!(start["args"]["fixture"], "tiny");
    assert_eq!(start["args"]["seed_list"], "0");
    assert_eq!(start["args"]["builds"], "s2_ternary_full");
    assert_eq!(start["args"]["device_profile"], "S1CpuDeterministic");
    assert_eq!(start["args"]["json"], true);
}

#[test]
fn s2_feature_forwarding_rejects_full_plus_ablation_mutex() {
    assert_gbf_cli_forwards_s2_features();
    let output = cargo_check_gbf_cli_with_features(&["s2-full", "s2-ablation"]);

    assert!(
        !output.status.success(),
        "gbf-cli s2-full+s2-ablation unexpectedly compiled"
    );
    let combined = command_output(&output);
    assert!(
        !combined.contains("does not have these features")
            && !combined.contains("does not contain this feature")
            && !combined.contains("unknown feature"),
        "gbf-cli feature probe failed before forwarding features to gbf-experiments:\n{combined}"
    );
    assert!(
        combined.contains("S2 feature mutex violated"),
        "gbf-cli feature forwarding probe failed without the gbf-experiments mutex diagnostic:\n{combined}"
    );
}

#[test]
fn replay_ablation_and_verify_determinism_exit_zero() {
    let temp = tempfile::tempdir().expect("tempdir");
    let events = temp.path().join("events.ndjson");
    let mut ablation = gbf();
    ablation.args([
        "--log-format",
        "json",
        "--capture-events",
        events.to_str().expect("utf8 path"),
        "s2",
        "replay-ablation",
        "--manifest",
        "stub",
        "--pass-version",
        "0.0.0",
        "--seed-list",
        "0",
        "--device-profile",
        "S1CpuDeterministic",
        "--json",
    ]);
    ablation.assert().success().stdout(
        predicate::str::contains("\"schema\":\"s2_replay_ablation_cli.v1\"")
            .and(predicate::str::contains("final_checkpoint_sha")),
    );
    let captured = std::fs::read_to_string(events).expect("capture file");
    let start = captured
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("event JSON"))
        .find(|event| event["event"] == "cli_subcommand_start")
        .expect("start event");
    assert_eq!(start["command"], "replay-ablation");
    assert_eq!(start["args"]["manifest"], "stub");
    assert_eq!(start["args"]["pass_version"], "0.0.0");
    assert_eq!(start["args"]["fixture"], "tiny");
    assert_eq!(start["args"]["seed_list"], "0");
    assert_eq!(start["args"]["device_profile"], "S1CpuDeterministic");
    assert_eq!(start["args"]["json"], true);

    let mut determinism = gbf();
    determinism.args([
        "s2",
        "verify-determinism",
        "--seed",
        "0",
        "--build",
        "s2_ternary_full",
        "--json",
    ]);
    determinism
        .assert()
        .success()
        .stdout(predicate::str::contains("\"passed\":true"));
}

#[test]
fn report_consumes_replay_full_json_and_exits_zero() {
    let temp = tempfile::tempdir().expect("tempdir");
    let replay_path = temp.path().join("replay-full.json");
    let distill_path = temp.path().join("distill-once.json");
    let report_path = temp.path().join("S2-report.md");
    write_json_file(&replay_path, &replay_full_fixture_json());
    write_json_file(&distill_path, &distill_once_fixture_json());

    let mut command = gbf();
    command.args([
        "s2",
        "report",
        "--replay-full-json",
        replay_path.to_str().expect("utf8 replay path"),
        "--distill-json",
        distill_path.to_str().expect("utf8 distill path"),
        "--output",
        report_path.to_str().expect("utf8 report path"),
        "--json",
    ]);

    let output = command.assert().success().get_output().clone();
    let payload = json_stdout_payload(&output, "s2_report_cli.v1");
    assert_eq!(payload["evidence_source"], "gbf s2 report");
    assert_eq!(payload["replay_evidence_source"], "gbf s2 replay-full");
    assert_eq!(payload["distill_evidence_source"], "gbf s2 distill-once");
    assert!(report_path.exists());

    let markdown = std::fs::read_to_string(&report_path).expect("report markdown");
    let front_matter = markdown
        .split("---")
        .nth(1)
        .expect("front matter delimiter");
    let front_matter: Value = serde_json::from_str(front_matter.trim()).expect("front matter JSON");
    assert_eq!(front_matter["per_seed_artifacts"][0]["seed"], 0);
    assert_eq!(
        front_matter["per_seed_artifacts"][0]["checkpoint_self_hashes"]["phase_a"],
        hash_hex(0x10)
    );
    assert_eq!(
        front_matter["per_seed_artifacts"][0]["checkpoint_self_hashes"]["final"],
        hash_hex(0x13)
    );
    assert!(
        markdown.contains("Live replay evidence consumed from gbf s2 replay-full"),
        "report body should name live replay evidence:\n{markdown}"
    );
}

#[test]
fn report_rejects_replay_schema_and_evidence_source_mismatch() {
    for (field, value, expected) in [
        (
            "schema",
            "wrong_schema.v1",
            "expected schema s2_replay_full_cli.v1",
        ),
        (
            "evidence_source",
            "gbf s2 replay-ablation",
            "expected evidence_source gbf s2 replay-full",
        ),
    ] {
        let temp = tempfile::tempdir().expect("tempdir");
        let replay_path = temp.path().join("replay-full.json");
        let distill_path = temp.path().join("distill-once.json");
        let report_path = temp.path().join("S2-report.md");
        let mut replay = replay_full_fixture_json();
        replay[field] = Value::String(value.to_owned());
        write_json_file(&replay_path, &replay);
        write_json_file(&distill_path, &distill_once_fixture_json());

        let mut command = gbf();
        command.args(report_args(&replay_path, &distill_path, &report_path));
        command
            .assert()
            .failure()
            .stderr(predicate::str::contains(expected));
    }
}

#[test]
fn report_rejects_final_checkpoint_mismatch() {
    let temp = tempfile::tempdir().expect("tempdir");
    let replay_path = temp.path().join("replay-full.json");
    let distill_path = temp.path().join("distill-once.json");
    let report_path = temp.path().join("S2-report.md");
    let mut replay = replay_full_fixture_json();
    replay["runs"][0]["checkpoints"]["10000"] = Value::String(hash_hex(0xaa));
    write_json_file(&replay_path, &replay);
    write_json_file(&distill_path, &distill_once_fixture_json());

    let mut command = gbf();
    command.args(report_args(&replay_path, &distill_path, &report_path));
    command.assert().failure().stderr(predicate::str::contains(
        "final_checkpoint_sha does not match checkpoint 10000",
    ));
}

#[test]
fn distill_once_json_exits_zero() {
    let mut command = gbf();
    command.args(["s2", "distill-once", "--json"]);

    let output = command.assert().success().get_output().clone();
    let payload = json_stdout_payload(&output, "s2_distill_once_cli.v1");
    assert_eq!(payload["evidence_source"], "gbf s2 distill-once");
    assert!(payload["distill"]["distill_loss_raw_bits_hex"].is_string());
}

#[test]
fn diagnostic_subcommands_exit_zero() {
    for subcommand in [
        "grad-flow",
        "linearstate-smoke",
        "phase-integ",
        "oracle-re-run",
    ] {
        let mut command = gbf();
        command.args(["s2", subcommand, "--json"]);
        command
            .assert()
            .success()
            .stdout(predicate::str::contains("\"schema\""));
    }
}

#[test]
fn json_logging_capture_writes_start_and_done_events() {
    let temp = tempfile::tempdir().expect("tempdir");
    let events = temp.path().join("events.ndjson");
    let mut command = gbf();
    command.args([
        "--log-format",
        "json",
        "--capture-events",
        events.to_str().expect("utf8 path"),
        "s2",
        "grad-flow",
        "--json",
    ]);

    command.assert().success().stderr(
        predicate::str::contains("\"event\":\"cli_subcommand_start\"").and(
            predicate::str::contains("\"event\":\"cli_subcommand_done\""),
        ),
    );
    let captured = std::fs::read_to_string(events).expect("capture file");
    assert!(captured.contains("\"event\":\"cli_subcommand_start\""));
    assert!(captured.contains("\"event\":\"cli_subcommand_done\""));
}

fn json_stdout_payload(output: &Output, schema: &str) -> Value {
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|value| value["schema"] == schema)
        .unwrap_or_else(|| panic!("missing {schema} JSON payload in stdout:\n{stdout}"))
}

fn cargo_check_gbf_cli_with_features(features: &[&str]) -> Output {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let target_dir = std::env::var_os("CARGO_TARGET_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| workspace_root().join("target"));
    ProcessCommand::new(cargo)
        .arg("check")
        .arg("--quiet")
        .arg("-p")
        .arg("gbf-cli")
        .arg("--no-default-features")
        .arg("--features")
        .arg(features.join(","))
        .env("CARGO_TARGET_DIR", target_dir)
        .current_dir(workspace_root())
        .output()
        .expect("gbf-cli feature forwarding cargo check must run")
}

fn assert_gbf_cli_forwards_s2_features() {
    let manifest = std::fs::read_to_string(workspace_root().join("gbf-cli/Cargo.toml"))
        .expect("gbf-cli Cargo.toml must be readable");
    assert!(
        manifest.contains(r#"s2-full = ["gbf-experiments/s2-full"]"#),
        "gbf-cli must define and forward s2-full to gbf-experiments"
    );
    assert!(
        manifest.contains(r#"s2-ablation = ["gbf-experiments/s2-ablation"]"#),
        "gbf-cli must define and forward s2-ablation to gbf-experiments"
    );
}

fn replay_full_fixture_json() -> Value {
    let mut runs = Vec::new();
    for (build_index, build_kind) in ["s2-ternary-full", "s2-fp-full", "s2-ternary-nodistill"]
        .into_iter()
        .enumerate()
    {
        for seed in 0..5 {
            let fill = 0x10 + build_index as u8 * 0x20 + seed as u8;
            runs.push(json!({
                "seed": seed,
                "build_kind": build_kind,
                "final_checkpoint_sha": hash_hex(fill + 3),
                "phase_boundary_steps": ["4000", "5000", "8000", "10000"],
                "checkpoints": {
                    "4000": hash_hex(fill),
                    "5000": hash_hex(fill + 1),
                    "8000": hash_hex(fill + 2),
                    "10000": hash_hex(fill + 3),
                },
                "phase_log_self_hash": hash_hex(fill + 4),
                "score_self_hash": hash_hex(fill + 5),
                "distill_log_self_hash": hash_hex(fill + 6),
            }));
        }
    }
    json!({
        "schema": "s2_replay_full_cli.v1",
        "evidence_source": "gbf s2 replay-full",
        "fixture": "tiny",
        "manifest": "stub",
        "pass_version": "0.1.0",
        "runs": runs,
    })
}

fn distill_once_fixture_json() -> Value {
    json!({
        "schema": "s2_distill_once_cli.v1",
        "evidence_source": "gbf s2 distill-once",
        "fixture": "pinned",
        "distill": {
            "distill_loss_raw": 0.125,
            "distill_loss_raw_bits_hex": "3e000000",
            "distill_loss_raw_sha": hash_hex(0xee),
            "pre_clamp_kl_loss": 0.125,
            "distill_loss_weighted": 0.0625,
            "temperature": 2.0,
            "class_count": 3,
            "row_count": 1
        }
    })
}

fn report_args(replay_path: &Path, distill_path: &Path, report_path: &Path) -> Vec<String> {
    vec![
        "s2".to_owned(),
        "report".to_owned(),
        "--replay-full-json".to_owned(),
        replay_path.to_str().expect("utf8 replay path").to_owned(),
        "--distill-json".to_owned(),
        distill_path.to_str().expect("utf8 distill path").to_owned(),
        "--output".to_owned(),
        report_path.to_str().expect("utf8 report path").to_owned(),
        "--json".to_owned(),
    ]
}

fn write_json_file(path: &Path, value: &Value) {
    std::fs::write(path, serde_json::to_vec(value).expect("fixture JSON"))
        .expect("write fixture JSON");
}

fn hash_hex(byte: u8) -> String {
    format!("sha256:{}", format!("{byte:02x}").repeat(32))
}

fn workspace_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("gbf-cli must live under workspace root")
        .to_path_buf()
}

fn command_output(output: &Output) -> String {
    format!(
        "status: {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}
