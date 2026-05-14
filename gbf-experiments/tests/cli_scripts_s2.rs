use std::collections::BTreeMap;
use std::env;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{Mutex, OnceLock};

use gbf_experiments::s2::api_drift::{ApiDriftSymbols, check_api_drift};
use gbf_experiments::s2::run::{RunInputs, RunProductS2, s2_train_run};
use gbf_experiments::s2::schema::S2BuildKind;
use serde_json::{Value, json};

#[test]
fn s2_scripts_dry_run_emit_schema_and_stable_reports() {
    let _guard = script_test_lock().lock().unwrap();
    for spec in script_specs() {
        let first = run_script(spec, &[], &[("--dry-run", "")]);
        assert!(
            first.status.success(),
            "{} dry-run failed:\n{}",
            spec.name,
            command_output(&first)
        );
        assert_single_line_stdout(&first);
        assert_ndjson_events(&first, spec.event_prefix);
        let first_report = read_report(spec);
        assert_report_schema(&first_report, spec.event_prefix, true);
        assert_top_level_evidence_mode(&first_report, true);

        let second = run_script(spec, &[], &[("--dry-run", "")]);
        assert!(
            second.status.success(),
            "{} second dry-run failed:\n{}",
            spec.name,
            command_output(&second)
        );
        let second_report = read_report(spec);
        assert_eq!(
            first_report, second_report,
            "{} dry-run report must be byte-stable",
            spec.name
        );
    }
}

#[test]
fn s2_scripts_failure_injection_marks_failed_stage() {
    let _guard = script_test_lock().lock().unwrap();
    let injections: BTreeMap<&str, &[(&str, &str)]> = BTreeMap::from([
        (
            "s2_determinism_check.sh",
            &[("S2_DETERMINISM_PERTURB_LOCK_MIDRUN", "1")] as &[(&str, &str)],
        ),
        (
            "s2_isolation_check.sh",
            &[("S2_ISOLATION_FORCE_SHARED_STATE", "1")] as &[(&str, &str)],
        ),
        (
            "s2_api_drift_check.sh",
            &[("S2_SCRIPT_INJECT_FAILURE", "api_drift_added")] as &[(&str, &str)],
        ),
        (
            "s2_distill_determinism_check.sh",
            &[("S2_SCRIPT_INJECT_FAILURE", "distill_mismatch")] as &[(&str, &str)],
        ),
    ]);

    for spec in script_specs() {
        let output = run_script(spec, injections[spec.name], &[]);
        assert!(
            !output.status.success(),
            "{} failure injection unexpectedly passed:\n{}",
            spec.name,
            command_output(&output)
        );
        assert_single_line_stdout(&output);
        assert_ndjson_events(&output, spec.event_prefix);
        let report = read_report(spec);
        assert_report_schema(&report, spec.event_prefix, false);
        let report: Value = serde_json::from_str(&report).expect("report JSON");
        assert_eq!(report["dry_run"], false);
        assert_eq!(report["evidence_mode"], "live");
        assert_eq!(report["live_evidence"], true);
        assert!(
            report["stages"]
                .as_array()
                .unwrap()
                .iter()
                .any(|stage| stage["passed"] == false),
            "{} report should mark one stage failed: {report}",
            spec.name
        );
        if spec.name == "s2_determinism_check.sh" {
            let failed = failed_stage(&report);
            assert_eq!(failed["name"], "bytewise_compare");
            assert_eq!(
                failed["detail"]["failure_injection"],
                "parsed_payload_checkpoint_mutation"
            );
        }
        if spec.name == "s2_distill_determinism_check.sh" {
            let failed = failed_stage(&report);
            assert_eq!(failed["name"], "bytewise_compare");
            assert_eq!(
                failed["detail"]["failure_injection"],
                "parsed_payload_distill_sha_mutation"
            );
            assert_eq!(failed["detail"]["run1_sha"], EXPECTED_DISTILL_LOSS_RAW_SHA);
            assert_eq!(
                failed["detail"]["run2_sha"],
                EXPECTED_FORCED_DISTILL_MISMATCH_SHA
            );
        }
    }
}

#[test]
fn s2_distill_script_wraps_live_cli_json_schema_failures() {
    let _guard = script_test_lock().lock().unwrap();
    let spec = script_specs()
        .into_iter()
        .find(|spec| spec.name == "s2_distill_determinism_check.sh")
        .unwrap();

    for injection in [
        "distill_cli_failure",
        "distill_invalid_json",
        "distill_bad_schema",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let report_path = temp.path().join("distill-report.json");
        let report_arg = report_path.to_string_lossy().to_string();
        let output = run_script(
            spec,
            &[("S2_SCRIPT_INJECT_FAILURE", injection)],
            &[("--report-path", report_arg.as_str())],
        );

        assert!(
            !output.status.success(),
            "distill {injection} unexpectedly passed:\n{}",
            command_output(&output)
        );
        assert_single_line_stdout(&output);
        assert_ndjson_events(&output, spec.event_prefix);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            !stderr.contains("Traceback"),
            "distill {injection} should be structured, not a traceback:\n{stderr}"
        );
        let report = fs::read_to_string(&report_path)
            .unwrap_or_else(|error| panic!("missing structured report for {injection}: {error}"));
        assert_report_schema(&report, spec.event_prefix, false);
        let report: Value = serde_json::from_str(&report).expect("report JSON");
        assert_eq!(report["evidence_mode"], "live");
        assert_eq!(report["live_evidence"], true);
        let failed = failed_stage(&report);
        assert_eq!(failed["name"], "distill_once_1");
        assert_eq!(failed["detail"]["failure_injection"], injection);
        assert_eq!(failed["detail"]["evidence_source"], "gbf s2 distill-once");
        if injection == "distill_bad_schema" {
            assert_eq!(
                failed["detail"]["cli_payload_schema"],
                "s2_distill_once_cli.v0"
            );
            assert_eq!(
                failed["detail"]["cli_payload_evidence_source"],
                "gbf s2 distill-once"
            );
            assert_eq!(failed["detail"]["cli_payload_fixture"], "pinned");
            assert_eq!(
                failed["detail"]["distill_loss_raw_sha"],
                EXPECTED_DISTILL_LOSS_RAW_SHA
            );
            assert_eq!(
                failed["detail"]["distill_loss_raw_bits_hex"],
                EXPECTED_DISTILL_LOSS_RAW_BITS_HEX
            );
            assert_eq!(failed["detail"]["cli_payload_class_count"], 4);
            assert_eq!(failed["detail"]["cli_payload_row_count"], 1);
            assert_eq!(
                failed["detail"]["distill_loss_weighted"],
                EXPECTED_DISTILL_LOSS_WEIGHTED
            );
            assert_eq!(
                failed["detail"]["distill_loss_weighted_bits_hex"],
                EXPECTED_DISTILL_LOSS_WEIGHTED_BITS_HEX
            );
        } else {
            assert_distill_no_payload_telemetry_is_null(failed);
        }
        assert!(
            failed["detail"]["reason"].as_str().is_some_and(|reason| {
                reason.contains("gbf s2 distill-once failed")
                    || reason.contains("invalid JSON")
                    || reason.contains("unexpected distill schema")
            }),
            "failed report should carry wrapped CLI/JSON/schema reason: {report}"
        );
    }
}

#[test]
fn s2_api_and_isolation_pass_reports_real_evidence() {
    let _guard = script_test_lock().lock().unwrap();
    // This is the pass-path live-evidence gate for isolation. The perturbation
    // tests below cover failure-path evidence mutation, not successful live
    // isolation evidence.
    for spec in script_specs()
        .into_iter()
        .filter(|spec| matches!(spec.name, "s2_api_drift_check.sh" | "s2_isolation_check.sh"))
    {
        let output = run_script(spec, &[], &[]);
        assert!(
            output.status.success(),
            "{} real-evidence pass failed:\n{}",
            spec.name,
            command_output(&output)
        );
        let report: Value = serde_json::from_str(&read_report(spec)).expect("report JSON");
        assert_eq!(report["passed"], true);
        assert_eq!(report["dry_run"], false);
        assert_eq!(report["evidence_mode"], "live");
        assert_eq!(report["live_evidence"], true);
        assert!(
            report["stages"]
                .as_array()
                .unwrap()
                .iter()
                .any(|stage| stage["detail"]["evidence_source"].is_string()),
            "{} pass report must name real evidence source: {report}",
            spec.name
        );
        assert_allowed_live_evidence_sources(&report, spec.allowed_live_evidence_sources());
    }
}

#[test]
fn s2_determinism_and_distill_pass_reports_cli_json_evidence() {
    let _guard = script_test_lock().lock().unwrap();
    let expected_sources: BTreeMap<&str, &str> = BTreeMap::from([
        ("s2_determinism_check.sh", "gbf s2 replay-full"),
        ("s2_distill_determinism_check.sh", "gbf s2 distill-once"),
    ]);
    for spec in script_specs().into_iter().filter(|spec| {
        matches!(
            spec.name,
            "s2_determinism_check.sh" | "s2_distill_determinism_check.sh"
        )
    }) {
        let output = run_script(spec, &[], &[]);
        assert!(
            output.status.success(),
            "{} real CLI evidence pass failed:\n{}",
            spec.name,
            command_output(&output)
        );
        let report: Value = serde_json::from_str(&read_report(spec)).expect("report JSON");
        assert_eq!(report["passed"], true);
        let source = expected_sources[spec.name];
        assert_top_level_evidence_mode(&report.to_string(), false);
        assert_allowed_live_evidence_sources(&report, &[source]);
        assert_cli_cargo_command_is_reported(&report, spec.name);
        match spec.name {
            "s2_determinism_check.sh" => assert_replay_cli_payload_evidence(&report),
            "s2_distill_determinism_check.sh" => assert_distill_cli_payload_evidence(&report),
            other => panic!("unexpected CLI evidence script {other}"),
        }
    }
}

#[test]
fn s2_api_drift_text_fallback_self_test_covers_rust_edge_items() {
    let _guard = script_test_lock().lock().unwrap();
    let spec = script_specs()
        .into_iter()
        .find(|spec| spec.name == "s2_api_drift_check.sh")
        .unwrap();

    let output = run_script(spec, &[("S2_API_DRIFT_TEXT_FALLBACK_SELF_TEST", "1")], &[]);

    assert!(
        output.status.success(),
        "api drift text fallback self-test failed:\n{}",
        command_output(&output)
    );
    let report: Value = serde_json::from_str(&read_report(spec)).expect("report JSON");
    assert_eq!(report["passed"], true);
    let stage = stage(&report, "text_fallback_self_test");
    let qat_symbols = stage["detail"]["qat_symbols"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap())
        .collect::<Vec<_>>();
    for expected in [
        "QAT_SENTINEL",
        "QatUnion",
        "DirectThing",
        "RenamedNested",
        "NestedThing",
    ] {
        assert!(
            qat_symbols.contains(&expected),
            "fallback self-test missed {expected}: {report}"
        );
    }
    assert!(
        !qat_symbols.contains(&"BlockCommentGhost"),
        "fallback self-test should ignore block-commented public items: {report}"
    );
}

#[test]
fn s2_api_drift_rejects_self_oracle_current_symbols() {
    let _guard = script_test_lock().lock().unwrap();
    let spec = script_specs()
        .into_iter()
        .find(|spec| spec.name == "s2_api_drift_check.sh")
        .unwrap();

    let output = run_script(spec, &[("S2_API_DRIFT_FORCE_SELF_ORACLE", "1")], &[]);

    assert!(
        !output.status.success(),
        "api drift self-oracle fixture unexpectedly passed:\n{}",
        command_output(&output)
    );
    let report: Value = serde_json::from_str(&read_report(spec)).expect("report JSON");
    assert_eq!(report["passed"], false);
    let failed_stage = failed_stage(&report);
    assert_eq!(
        failed_stage["detail"]["reason"],
        "self-oracle current symbols rejected"
    );
}

#[test]
fn s2_api_drift_text_fallback_extracts_live_workspace_symbols() {
    let _guard = script_test_lock().lock().unwrap();
    let spec = script_specs()
        .into_iter()
        .find(|spec| spec.name == "s2_api_drift_check.sh")
        .unwrap();

    let output = run_script(spec, &[("S2_API_DRIFT_FORCE_TEXT_FALLBACK", "1")], &[]);

    assert!(
        output.status.success(),
        "api drift text fallback failed:\n{}",
        command_output(&output)
    );
    let report: Value = serde_json::from_str(&read_report(spec)).expect("report JSON");
    assert_eq!(report["passed"], true);
    assert!(
        report["stages"].as_array().unwrap().iter().any(|stage| {
            stage["name"] == "extract_current_symbols"
                && stage["detail"]["evidence_source"] == "live-workspace-text-fallback"
                && stage["detail"]["qat_count"].as_u64().unwrap_or_default() > 0
                && stage["detail"]["linearstate_count"]
                    .as_u64()
                    .unwrap_or_default()
                    > 0
        }),
        "fallback report should prove live text extraction: {report}"
    );
}

#[test]
fn s2_isolation_order_perturbation_fails_with_real_evidence() {
    // Failure-path perturbation only; pass-path isolation evidence is guarded by
    // s2_api_and_isolation_pass_reports_real_evidence.
    let _guard = script_test_lock().lock().unwrap();
    let spec = script_specs()
        .into_iter()
        .find(|spec| spec.name == "s2_isolation_check.sh")
        .unwrap();

    let output = run_script(
        spec,
        &[("S2_SCRIPT_INJECT_FAILURE", "order_dependence")],
        &[],
    );

    assert!(
        !output.status.success(),
        "isolation order perturbation unexpectedly passed:\n{}",
        command_output(&output)
    );
    let report: Value = serde_json::from_str(&read_report(spec)).expect("report JSON");
    assert_eq!(report["passed"], false);
    assert!(
        report.to_string().contains("tiny_s2_run"),
        "report should carry real S2 run evidence source: {report}"
    );
}

#[test]
fn s2_isolation_stateful_seam_injection_fails_with_real_evidence() {
    // Failure-path perturbation only; pass-path isolation evidence is guarded by
    // s2_api_and_isolation_pass_reports_real_evidence.
    let _guard = script_test_lock().lock().unwrap();
    let spec = script_specs()
        .into_iter()
        .find(|spec| spec.name == "s2_isolation_check.sh")
        .unwrap();

    let output = run_script(spec, &[("S2_ISOLATION_SIMULATE_STATE_LEAK", "1")], &[]);

    assert!(
        !output.status.success(),
        "isolation stateful seam injection unexpectedly passed:\n{}",
        command_output(&output)
    );
    let report: Value = serde_json::from_str(&read_report(spec)).expect("report JSON");
    assert_eq!(report["passed"], false);
    assert!(
        report
            .to_string()
            .contains("explicit_stateful_evidence_collector"),
        "report should name the explicit stateful seam: {report}"
    );
}

#[test]
fn s2_scripts_support_report_dir_override() {
    let _guard = script_test_lock().lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    for spec in script_specs() {
        let output = run_script(
            spec,
            &[],
            &[
                ("--dry-run", ""),
                ("--report-dir", temp.path().to_str().unwrap()),
            ],
        );
        assert!(
            output.status.success(),
            "{} report-dir override failed:\n{}",
            spec.name,
            command_output(&output)
        );
        let report_path = temp.path().join(spec.report_basename());
        assert!(
            report_path.exists(),
            "{} did not write overridden report {}",
            spec.name,
            report_path.display()
        );
    }
}

#[test]
fn s2_isolation_usage_pins_tmp_collision_guidance() {
    let script = fs::read_to_string(workspace_root().join("scripts/s2_isolation_check.sh"))
        .expect("S2 isolation script should be readable");

    assert!(
        script.contains("written to /tmp/s2-isolation.json"),
        "usage should name the default /tmp report path"
    );
    assert!(
        script.contains("serial local runs but can collide under parallel jobs"),
        "usage should explain that the default /tmp report path is only a serial-local convenience"
    );
    assert!(
        script.contains(
            "use --report-path or\n--report-dir to give each job an isolated output path"
        ),
        "usage should direct parallel jobs to isolated report paths"
    );
}

#[test]
fn s2_cli_accepts_public_kebab_case_build_names() {
    let _guard = script_test_lock().lock().unwrap();

    let output = run_gbf_cli(&[
        "s2",
        "replay-full",
        "--seed-list",
        "0",
        "--builds",
        "s2-ternary-full,s2-fp-full,s2-ternary-nodistill,s2-ablation",
        "--fixture",
        "tiny",
        "--json",
    ]);

    assert!(
        output.status.success(),
        "gbf s2 replay-full kebab-case builds failed:\n{}",
        command_output(&output)
    );
    let payload: Value =
        serde_json::from_slice(&output.stdout).expect("gbf replay-full JSON payload");
    assert_eq!(payload["schema"], "s2_replay_full_cli.v1");
    assert_eq!(payload["evidence_source"], "gbf s2 replay-full");
    let builds = payload["runs"]
        .as_array()
        .expect("runs array")
        .iter()
        .map(|run| run["build_kind"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        builds,
        [
            "s2-ternary-full",
            "s2-fp-full",
            "s2-ternary-nodistill",
            "s2-ablation"
        ]
    );
}

#[cfg(unix)]
#[test]
fn s2_scripts_are_executable() {
    for spec in script_specs() {
        let mode = fs::metadata(workspace_root().join("scripts").join(spec.name))
            .unwrap()
            .permissions()
            .mode();
        assert_ne!(mode & 0o111, 0, "{} must have execute bits", spec.name);
    }
}

#[derive(Debug, Clone, Copy)]
struct ScriptSpec {
    name: &'static str,
    event_prefix: &'static str,
    report: &'static str,
}

impl ScriptSpec {
    fn report_basename(self) -> &'static str {
        Path::new(self.report)
            .file_name()
            .and_then(|name| name.to_str())
            .expect("report path has basename")
    }

    fn allowed_live_evidence_sources(self) -> &'static [&'static str] {
        match self.name {
            "s2_determinism_check.sh" => &["gbf s2 replay-full"],
            "s2_isolation_check.sh" => &["tiny_s2_run"],
            "s2_api_drift_check.sh" => &[
                "cargo-public-api",
                "live-workspace-text-fallback",
                "gbf_experiments::s2::api_drift::check_api_drift",
            ],
            "s2_distill_determinism_check.sh" => &["gbf s2 distill-once"],
            _ => &[],
        }
    }
}

fn script_specs() -> Vec<ScriptSpec> {
    vec![
        ScriptSpec {
            name: "s2_determinism_check.sh",
            event_prefix: "s2_determinism_check",
            report: "/tmp/s2-determinism.json",
        },
        ScriptSpec {
            name: "s2_isolation_check.sh",
            event_prefix: "s2_isolation_check",
            report: "/tmp/s2-isolation.json",
        },
        ScriptSpec {
            name: "s2_api_drift_check.sh",
            event_prefix: "s2_api_drift_check",
            report: "/tmp/s2-api-drift.json",
        },
        ScriptSpec {
            name: "s2_distill_determinism_check.sh",
            event_prefix: "s2_distill_determinism_check",
            report: "/tmp/s2-distill-determinism.json",
        },
    ]
}

fn run_script(
    spec: ScriptSpec,
    envs: &[(&str, &str)],
    args_as_env_compat: &[(&str, &str)],
) -> Output {
    let mut command = Command::new("bash");
    command.arg(workspace_root().join("scripts").join(spec.name));
    for (arg, value) in args_as_env_compat {
        command.arg(arg);
        if !value.is_empty() {
            command.arg(value);
        }
    }
    for (key, value) in envs {
        command.env(key, value);
    }
    command.output().expect("script command must execute")
}

fn run_gbf_cli(args: &[&str]) -> Output {
    let mut command = Command::new("cargo");
    command
        .args([
            "run",
            "--quiet",
            "-p",
            "gbf-cli",
            "--features",
            "s2-full",
            "--",
        ])
        .args(args);
    command.output().expect("gbf-cli command must execute")
}

fn read_report(spec: ScriptSpec) -> String {
    fs::read_to_string(spec.report)
        .unwrap_or_else(|error| panic!("failed to read {} for {}: {error}", spec.report, spec.name))
}

fn assert_report_schema(report: &str, script: &str, passed: bool) {
    let value: Value = serde_json::from_str(report).expect("report must be JSON");
    assert_eq!(value["script"], script);
    assert_eq!(value["passed"], passed);
    assert_eq!(value["exit_code"], if passed { 0 } else { 1 });
    let stages = value["stages"].as_array().expect("stages array");
    assert!(!stages.is_empty());
    for stage in stages {
        assert!(stage["name"].is_string());
        assert!(stage["passed"].is_boolean());
        assert!(stage["detail"].is_object());
    }
}

fn assert_top_level_evidence_mode(report: &str, dry_run: bool) {
    let value: Value = serde_json::from_str(report).expect("report must be JSON");
    assert_eq!(value["dry_run"], dry_run);
    assert_eq!(
        value["evidence_mode"],
        if dry_run { "dry_run" } else { "live" }
    );
    assert_eq!(value["live_evidence"], !dry_run);
}

fn assert_allowed_live_evidence_sources(report: &Value, allowed: &[&str]) {
    assert_eq!(report["dry_run"], false);
    assert_eq!(report["evidence_mode"], "live");
    assert_eq!(report["live_evidence"], true);
    let observed = report["stages"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|stage| {
            stage["detail"]["evidence_source"]
                .as_str()
                .or_else(|| stage["detail"]["cli_payload_evidence_source"].as_str())
        })
        .collect::<Vec<_>>();
    assert!(
        !observed.is_empty(),
        "live report must expose positive evidence sources: {report}"
    );
    assert!(
        observed.iter().all(|source| allowed.contains(source)),
        "unexpected live evidence source(s) {observed:?}; allowed={allowed:?}; report={report}"
    );
}

fn assert_cli_cargo_command_is_reported(report: &Value, script_name: &str) {
    assert!(
        report["stages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|stage| stage["detail"]["command"]
                .as_array()
                .is_some_and(|command| command.iter().any(|arg| arg == "gbf-cli"))),
        "{script_name} pass report must expose the cargo-run gbf-cli command: {report}",
    );
}

fn assert_replay_cli_payload_evidence(report: &Value) {
    let run1 = stage(report, "replay_run_1");
    let run2 = stage(report, "replay_run_2");
    for stage in [run1, run2] {
        assert_eq!(
            stage["detail"]["cli_payload_schema"],
            "s2_replay_full_cli.v1"
        );
        assert_eq!(
            stage["detail"]["cli_payload_evidence_source"],
            "gbf s2 replay-full"
        );
        assert_eq!(stage["detail"]["cli_payload_fixture"], "tiny");
        assert_eq!(stage["detail"]["cli_payload_seed"], 0);
        assert_eq!(stage["detail"]["cli_payload_build_kind"], "s2-ternary-full");
        assert_eq!(
            stage["detail"]["cli_payload_phase_boundary_steps"],
            json!(["4000", "5000", "8000", "10000"])
        );
        assert_eq!(stage["detail"]["checkpoint_count"], 4);
    }

    let compare = stage(report, "bytewise_compare");
    assert_eq!(compare["detail"]["evidence_source"], "gbf s2 replay-full");
    assert_eq!(compare["detail"]["failure_injection"], Value::Null);
    assert_eq!(
        compare["detail"]["comparison_keys"],
        json!([
            "checkpoint_4000",
            "checkpoint_5000",
            "checkpoint_8000",
            "checkpoint_10000",
            "final_checkpoint_sha",
            "phase_log_self_hash",
            "distill_log_self_hash",
            "score_self_hash"
        ])
    );
}

fn assert_distill_cli_payload_evidence(report: &Value) {
    let run1 = stage(report, "distill_once_1");
    let run2 = stage(report, "distill_once_2");
    for stage in [run1, run2] {
        assert_eq!(
            stage["detail"]["cli_payload_schema"],
            "s2_distill_once_cli.v1"
        );
        assert_eq!(
            stage["detail"]["cli_payload_evidence_source"],
            "gbf s2 distill-once"
        );
        assert_eq!(stage["detail"]["cli_payload_fixture"], "pinned");
        assert_eq!(stage["detail"]["cli_payload_class_count"], 4);
        assert_eq!(stage["detail"]["cli_payload_row_count"], 1);
        assert_eq!(
            stage["detail"]["distill_loss_raw_sha"],
            EXPECTED_DISTILL_LOSS_RAW_SHA
        );
        assert_eq!(
            stage["detail"]["distill_loss_raw_bits_hex"],
            EXPECTED_DISTILL_LOSS_RAW_BITS_HEX
        );
        assert_eq!(
            stage["detail"]["distill_loss_weighted"],
            EXPECTED_DISTILL_LOSS_WEIGHTED
        );
        assert_eq!(
            stage["detail"]["distill_loss_weighted_bits_hex"],
            EXPECTED_DISTILL_LOSS_WEIGHTED_BITS_HEX
        );
    }

    let compare = stage(report, "bytewise_compare");
    assert_eq!(compare["detail"]["evidence_source"], "gbf s2 distill-once");
    assert_eq!(compare["detail"]["failure_injection"], Value::Null);
    assert_eq!(
        compare["detail"]["comparison_keys"],
        json!([
            "distill_loss_raw_bits_hex",
            "distill_loss_raw_sha",
            "distill_loss_weighted"
        ])
    );
}

const EXPECTED_DISTILL_LOSS_RAW_BITS_HEX: &str = "3bfa6dd3";
const EXPECTED_DISTILL_LOSS_RAW_SHA: &str =
    "sha256:b1a888a6a64b00e339d45c4d45868314dcd01db6405f86337ab8f4aa5ef62ded";
const EXPECTED_DISTILL_LOSS_WEIGHTED: f64 = 0.007642486598342657_f64;
const EXPECTED_DISTILL_LOSS_WEIGHTED_BITS_HEX: &str = "3f7f4dba60000000";
const EXPECTED_FORCED_DISTILL_MISMATCH_SHA: &str = "sha256:forced-distill-mismatch";

fn stage<'a>(report: &'a Value, name: &str) -> &'a Value {
    report["stages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|stage| stage["name"] == name)
        .unwrap_or_else(|| panic!("missing stage {name}: {report}"))
}

fn failed_stage(report: &Value) -> &Value {
    report["stages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|stage| stage["passed"] == false)
        .unwrap_or_else(|| panic!("missing failed stage: {report}"))
}

fn assert_distill_no_payload_telemetry_is_null(stage: &Value) {
    for key in [
        "cli_payload_schema",
        "cli_payload_evidence_source",
        "cli_payload_fixture",
        "cli_payload_class_count",
        "cli_payload_row_count",
        "distill_loss_raw_sha",
        "distill_loss_raw_bits_hex",
        "distill_loss_weighted",
        "distill_loss_weighted_bits_hex",
    ] {
        assert_eq!(
            stage["detail"][key],
            Value::Null,
            "no-payload distill failure should not backfill {key}: {stage}"
        );
    }
}

fn assert_single_line_stdout(output: &Output) {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines = stdout.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 1, "stdout must be one line: {stdout:?}");
}

fn assert_ndjson_events(output: &Output, prefix: &str) {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut saw_start = false;
    let mut saw_done = false;
    let mut saw_exit = false;
    for line in stderr.lines() {
        let value: Value = serde_json::from_str(line).unwrap_or_else(|error| {
            panic!("stderr line should be NDJSON ({error}): {line}\nfull stderr:\n{stderr}")
        });
        let event = value["event"].as_str().expect("event field");
        assert!(
            event.starts_with(prefix),
            "unexpected event prefix {event:?}; expected {prefix:?}"
        );
        if event == format!("{prefix}_stage_start") {
            saw_start = true;
            assert!(
                value["stage"].as_u64().is_some(),
                "stage_start needs stage: {value}"
            );
            assert!(
                value["description"].as_str().is_some(),
                "stage_start needs description: {value}"
            );
        } else if event == format!("{prefix}_stage_done") {
            saw_done = true;
            assert!(
                value["stage"].as_u64().is_some(),
                "stage_done needs stage: {value}"
            );
            assert!(
                value["passed"].as_bool().is_some(),
                "stage_done needs passed: {value}"
            );
            assert!(
                value["detail"].as_object().is_some(),
                "stage_done needs detail object: {value}"
            );
        } else if event == format!("{prefix}_exit") {
            saw_exit = true;
            assert!(
                value["exit_code"].as_i64().is_some(),
                "exit needs exit_code: {value}"
            );
            assert!(
                value["passed"].as_bool().is_some(),
                "exit needs passed: {value}"
            );
            assert!(
                value["summary"].as_str().is_some(),
                "exit needs summary: {value}"
            );
        }
    }
    assert!(saw_start, "missing stage_start event in {stderr}");
    assert!(saw_done, "missing stage_done event in {stderr}");
    assert!(saw_exit, "missing exit event in {stderr}");
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("gbf-experiments lives under workspace root")
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

fn script_test_lock() -> &'static Mutex<()> {
    // Several script tests spawn nested `cargo run/test` probes. Keep them
    // serialized so the package-cache/target-dir locks are an expected cost,
    // not a source of incidental cross-test contention.
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

#[test]
#[ignore = "invoked by scripts/s2_api_drift_check.sh as a Rust API-drift wrapper"]
fn __s2_api_drift_probe() {
    let current_path = env::var("S2_API_DRIFT_CURRENT_JSON").expect("current symbols path");
    let snapshots_dir = env::var("S2_API_DRIFT_SNAPSHOTS_DIR").expect("snapshots dir");
    let result_path = env::var("S2_API_DRIFT_RESULT_JSON").expect("result path");
    let current: Value =
        serde_json::from_str(&fs::read_to_string(current_path).unwrap()).expect("current JSON");
    let symbols = ApiDriftSymbols {
        qat: current["qat"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap().to_owned())
            .collect(),
        linearstate: current["linearstate"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap().to_owned())
            .collect(),
    };

    let result = check_api_drift(snapshots_dir, symbols).expect("api drift check");
    let drifts = result
        .drifts
        .iter()
        .map(|drift| {
            json!({
                "module": drift.module,
                "symbol": drift.symbol,
                "kind": format!("{:?}", drift.kind).to_ascii_lowercase(),
                "in_allow_list": drift.in_allow_list,
            })
        })
        .collect::<Vec<_>>();
    fs::write(
        result_path,
        serde_json::to_string(&json!({
            "passed": result.passed,
            "drift_count": result.drift_count,
            "qat_public_api_snapshot_hash": result.qat_public_api_snapshot_hash.to_string(),
            "linearstate_public_api_snapshot_hash": result.linearstate_public_api_snapshot_hash.to_string(),
            "drifts": drifts,
        }))
        .unwrap(),
    )
    .unwrap();
}

#[test]
#[ignore = "invoked by scripts/s2_isolation_check.sh to produce real tiny S2 evidence"]
fn __s2_isolation_evidence_probe() {
    let result_path = env::var("S2_ISOLATION_EVIDENCE_JSON").expect("evidence path");
    let seed0 = run_hash(S2BuildKind::s2_ternary_full, 0);
    let seed1 = run_hash(S2BuildKind::s2_ternary_full, 1);
    let expected_by_key = BTreeMap::from([
        (
            run_key(S2BuildKind::s2_ternary_full, 0),
            run_hash(S2BuildKind::s2_ternary_full, 0),
        ),
        (
            run_key(S2BuildKind::s2_fp_full, 0),
            run_hash(S2BuildKind::s2_fp_full, 0),
        ),
    ]);
    let order_a = collect_order(&[
        (S2BuildKind::s2_ternary_full, 0),
        (S2BuildKind::s2_fp_full, 0),
    ]);
    let order_b = collect_order(&[
        (S2BuildKind::s2_fp_full, 0),
        (S2BuildKind::s2_ternary_full, 0),
    ]);

    fs::write(
        result_path,
        serde_json::to_string(&json!({
            "evidence_source": "tiny_s2_run",
            "stateful_seam": "explicit_stateful_evidence_collector",
            "seed_hashes": {
                "0": seed0,
                "1": seed1,
            },
            "expected_by_key": expected_by_key,
            "order_a": order_a,
            "order_b": order_b,
        }))
        .unwrap(),
    )
    .unwrap();
}

fn collect_order(order: &[(S2BuildKind, u64)]) -> BTreeMap<String, String> {
    let mut collector = EvidenceCollector::default();
    order
        .iter()
        .map(|(build_kind, seed)| {
            (
                run_key(*build_kind, *seed),
                collector.collect(*build_kind, *seed),
            )
        })
        .collect()
}

#[derive(Default)]
struct EvidenceCollector {
    previous_hash: Option<String>,
}

impl EvidenceCollector {
    fn collect(&mut self, build_kind: S2BuildKind, seed: u64) -> String {
        let current = run_hash(build_kind, seed);
        let observed = if env::var_os("S2_ISOLATION_SIMULATE_STATE_LEAK").is_some() {
            self.previous_hash
                .clone()
                .unwrap_or_else(|| current.clone())
        } else {
            current.clone()
        };
        self.previous_hash = Some(current);
        observed
    }
}

fn run_hash(build_kind: S2BuildKind, seed: u64) -> String {
    let product = s2_train_run(&RunInputs::tiny_fixture(seed, build_kind)).expect("tiny S2 run");
    let RunProductS2::Completed(product) = product else {
        panic!("tiny S2 run diverged");
    };
    product.final_checkpoint_sha.to_string()
}

fn run_key(build_kind: S2BuildKind, seed: u64) -> String {
    format!("{build_kind:?}:{seed}")
}
