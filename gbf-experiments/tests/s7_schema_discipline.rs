#![cfg(feature = "s7")]

use gbf_artifact::S7Topology;
use gbf_experiments::s7::schema::{
    charset_v1_normalized_token_count, s7_score_report_from_val_bytes,
};
use std::io::Write as _;
use std::process::{Command, Stdio};

use gbf_foundation::{CanonicalJson, DomainHash, Hash256};
use serde_json::json;

#[test]
fn score_report_token_count_uses_charset_v1_normalized_length() {
    let val_bytes = "“hi”".as_bytes();
    let token_count = charset_v1_normalized_token_count(val_bytes).expect("normalizes");

    assert_eq!(token_count, 4);
    assert_ne!(token_count, val_bytes.len() as u64);

    let report = s7_score_report_from_val_bytes(
        9,
        S7Topology::MoeTinyDenseMatched,
        Hash256::ZERO,
        Hash256::ZERO,
        val_bytes,
        8.0,
    )
    .expect("score report");

    assert_eq!(report.token_count, token_count);
    assert_eq!(report.bpc, 2.0);
    assert_eq!(
        serde_json::to_value(&report).expect("json"),
        json!({
            "schema": "s7_score.v1",
            "seed": 9,
            "topology": "MoeTinyDenseMatched",
            "checkpoint_sha": Hash256::ZERO,
            "corpus_val_sha": Hash256::ZERO,
            "chunk_size": 256,
            "token_count": 4,
            "log2_sum": 8.0,
            "bpc": 2.0,
            "score_self_hash": report.score_self_hash,
        })
    );
}

#[test]
fn s7_isolation_script_pins_split_replay_order() {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root");
    let script_path = workspace_root.join("scripts/s7_isolation_check.sh");
    let script = std::fs::read_to_string(script_path).expect("script exists");

    let dense_replay = script
        .find("run_replay s7-dense-matched MoeTinyDenseMatched")
        .expect("script names dense matched replay");
    let moe_replay = script
        .find("run_replay s7-moe MoeTiny")
        .expect("script names MoE replay");

    assert!(moe_replay < dense_replay);
    assert!(script.contains("cargo run --release -p gbf-cli --features \"$feature\" -- s7 replay"));
    assert!(
        script.contains("Live execution depends on the gbf-cli S7 feature gates owned by bd-1ryn.")
    );
    assert!(script.contains("--topology \"$topology\""));
    assert!(script.contains("run_replay s7-moe MoeTiny"));
    assert!(script.contains("run_replay s7-dense-matched MoeTinyDenseMatched"));
    assert!(!script.contains("--topology MoeTiny,MoeTinyDenseMatched"));
}

#[test]
fn s7_cli_surface_names_replay_materialization_and_report_emission() {
    let cli_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/s7/cli.rs");
    let cli = std::fs::read_to_string(cli_path).expect("S7 CLI source exists");

    for required in [
        "Replay(S7ReplayArgs)",
        "MaterializeRun(S7MaterializeRunArgs)",
        "DeriveComparison(S7DeriveComparisonArgs)",
        "MaterializeSupportArtifact(S7MaterializeSupportArtifactArgs)",
        "EmitReport(S7EmitReportArgs)",
        "ValidateClosure(S7ValidateClosureArgs)",
        "gbf s7 materialize-run",
        "gbf s7 derive-comparison",
        "gbf s7 materialize-support-artifact",
        "gbf s7 emit-report",
        "gbf s7 validate-closure",
        "materialize_completed_run_artifacts",
        "materialize_dense_vs_moe_comparison",
        "materialize_support_artifact",
        "scripts/review/f-s7/emit-report.py",
        "docs/experiments/S7-report.md",
        "validate_closure_packet",
        "UnsupportedStdoutReport",
        "ReportEmitterFailed",
        "ClosurePacket",
    ] {
        assert!(cli.contains(required), "S7 CLI source missing {required}");
    }

    let adapter_path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/s7/closure_packet.rs");
    let adapter = std::fs::read_to_string(adapter_path).expect("S7 closure adapter exists");
    for required in [
        "validate_s7_closure",
        "S7ClosureValidationInput",
        "predictions_verified",
        "S7SwitchStatsBundleManifest",
        "S7SwitchStatsReport",
        "seed_bundle_self_hashes",
        "S7ReportMarkdown",
        "S7_REQUIRED_REPORT_HASH_FIELDS",
        "validate_report_scalars",
        "schema must be \\\"s7_report.v1\\\"",
        "decision must be ProceedToS8 or ProceedToS8DenseOnly",
        "rfc_revision must be a git commit id or sha256 hash",
        ".scalar(\"emulator_one_token_dense_self_hash\")",
        "S7_REQUIRED_REPORT_BODY_HEADINGS",
        "S7_REQUIRED_HYPOTHESIS_VERDICTS",
        "missing body heading",
        "missing explicit {hypothesis} hypothesis verdict",
        "closure-candidate reports must not use NotEvaluatedDueToPriorGate",
        "validate_report_rows",
        "per_seed_artifacts must contain 10 rows",
        "duplicate per_seed_artifacts row",
        "## Reproducibility statement",
        "normalize_report_for_hash",
        "validate_matched_bytes_hash",
        "matched_bytes_self_hash",
        "validate_actual_run_completed",
        "actual run-log completion is not completed",
        "validate_actual_artifact_identity",
        "topology mismatch",
        "seed mismatch",
        "validate_outcome_comparison_alignment",
        "PassClean outcome conflicts",
        "FailParity outcome requires",
        "verified_self_hash",
        "expected_schema",
        "schema must be",
        "must use canonical JSON bytes",
        "duplicate JSON key",
        "unsupported YAML anchor/alias",
        "unsupported YAML block scalar",
        "unsupported YAML flow collection",
        "S7RunLog",
        "S7ScoreReport",
        "S7DenseVsMoeComparisonReport",
        "RouterCollapseSweepReport",
        "S7FrontierReport",
        "S7BurnGradSmokeReport",
        "S7OracleRoutedReport",
        "EmulatorOneTokenReport",
        "invalid {label} self-hash",
        "\"s7_run_log\"",
        "\"s7_score\"",
        "\"s7_switch_stats_bundle\"",
        "\"s7_dense_vs_moe\"",
        "\"s7_router_collapse_sweep\"",
        "\"s7_frontier\"",
        "\"s7_burn_grad_smoke\"",
        "\"s7_oracle_routed\"",
        "\"s7_emulator_one_token\"",
    ] {
        assert!(
            adapter.contains(required),
            "S7 closure adapter missing {required}"
        );
    }
}

#[test]
fn s7_preregistration_script_names_all_result_hash_families() {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root");
    let script_path = workspace_root.join("scripts/s7_preregistration_check.sh");
    let script = std::fs::read_to_string(script_path).expect("script exists");
    let pin_script_path = workspace_root.join("scripts/s7_preregistration_pin.sh");
    let pin_script =
        std::fs::read_to_string(pin_script_path).expect("preregistration pin script exists");

    for field in [
        "checkpoint_self_hash",
        "run_log_self_hash",
        "score_self_hash",
        "switch_stats_self_hash",
        "router_collapse_sweep_self_hash",
        "dense_vs_moe_self_hash",
        "frontier_self_hash",
        "burn_grad_smoke_self_hash",
        "oracle_routed_self_hash",
        "emulator_one_token_moe_self_hash",
        "emulator_one_token_dense_self_hash",
        "report_self_hash",
        "comparison_self_hash",
        "sweep_self_hash",
    ] {
        assert!(
            script.contains(field),
            "missing S7 result hash family {field}"
        );
    }

    assert!(script.contains("fixtures/preregistration/s7.toml"));
    assert!(script.contains("validate_pass_version"));
    assert!(script.contains("pass_version_S7 must be finalized"));
    assert!(script.contains("pass_version_S7 must be semver or an s7-* final pin id"));
    assert!(script.contains("matched_bytes_self_hash"));
    assert!(script.contains("experiments/S7/smoke"));
    assert!(script.contains("predictions_commit must be an ancestor of HEAD/current checkout"));
    assert!(script.contains("rfc_revision must be an ancestor of HEAD/current checkout"));
    assert!(script.contains("predictions_commit must be a strict ancestor"));
    assert!(script.contains("first_result_commit is not the earliest S7 result artifact commit"));
    assert!(pin_script.contains("Emit fixtures/preregistration/s7.toml"));
    assert!(pin_script.contains("current RFC predictions section differs from predictions_commit"));
    assert!(
        pin_script
            .contains("current RFC prediction heading line range differs from predictions_commit")
    );
    assert!(pin_script.contains("--check-ready"));
    assert!(pin_script.contains("predictions_commit must be an ancestor of HEAD/current checkout"));
}

#[test]
fn s7_verify_packet_names_production_closure_anchors() {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root");
    let script_path = workspace_root.join("scripts/review/f-s7/verify-packet.sh");
    let script = std::fs::read_to_string(script_path).expect("verify-packet script exists");

    for required in [
        "S7 verify-packet: NEEDS_CHANGES",
        "scripts/s7_preregistration_check.sh",
        "scripts/s7_preregistration_pin.sh --check-ready",
        "scripts/review/f-s7/validate-report.py",
        "scripts/review/f-s7/emit-report.py",
        "scripts/review/f-s7/validate-artifacts.py",
        "scripts/review/f-s7/validate-reviews.py",
        "s7 validate-closure",
        "S7 Rust closure validation failed",
        "S7 preregistration pin readiness failed",
        "synthetic S7 CLI feature preflight self-test",
        "synthetic Rust closure gate self-test",
        "fixtures/preregistration/s7.toml",
        "docs/experiments/S7-report.md",
        "docs/review/f-s7/reviews/bd-2v9r-gemini.json",
        "docs/review/f-s7/reviews/bd-2v9r-claude.json",
        "history/rfcs/F-S7-moe-beats-dense.md",
        "experiments/S7/runs/$topology/seed-$seed/run-log.json",
        "experiments/S7/scores/$topology/seed-$seed/score.json",
        "experiments/S7/switch-stats/seed-$seed/switch-stats.json",
        "experiments/S7/router-collapse/seed-0/sweep.json",
        "experiments/S7/dense-vs-moe/comparison.json",
        "experiments/S7/frontier/frontier.json",
        "experiments/S7/burn-grad-smoke/expert_block_qat.json",
        "experiments/S7/oracle-routed/seed-0/oracle.json",
        "experiments/S7/emulator-one-token/seed-0/MoeTiny/result.json",
        "F-S7 RFC still has DRAFT/pre-implementation/[ESTIMATE] closure blockers",
        "S7 verify-packet: substrate-only mode completed",
    ] {
        assert!(
            script.contains(required),
            "verify packet missing {required}"
        );
    }
    for stale in [
        "missing required section/pattern",
        "dense emulator one-token result required by DenseOnly decision",
    ] {
        assert!(
            !script.contains(stale),
            "verify packet kept stale duplicate report check {stale}"
        );
    }
}

#[test]
fn s7_pr_workflow_runs_closure_substrate_without_production_artifacts() {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root");
    let workflow_path = workspace_root.join(".github/workflows/s7-pr.yml");
    let workflow = std::fs::read_to_string(workflow_path).expect("S7 PR workflow exists");

    for required in [
        "S7 PR Gates",
        "scripts/review/f-s7/verify-packet.sh --substrate-only",
        "scripts/tests/s7_verify_packet_test.sh",
        "scripts/tests/s7_preregistration_check_test.sh",
        "scripts/tests/s7_validate_artifacts_test.sh",
        "scripts/tests/s7_validate_report_test.sh",
        "scripts/tests/s7_validate_reviews_test.sh",
        "docs/review/f-s7/**",
        "actions/upload-artifact@v4",
    ] {
        assert!(
            workflow.contains(required),
            "S7 PR workflow missing {required}"
        );
    }
}

#[test]
fn s7_report_validator_guards_closure_front_matter() {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root");
    let script_path = workspace_root.join("scripts/review/f-s7/validate-report.py");
    let script = std::fs::read_to_string(script_path).expect("report validator exists");
    let emitter_path = workspace_root.join("scripts/review/f-s7/emit-report.py");
    let emitter = std::fs::read_to_string(emitter_path).expect("report emitter exists");

    for required in [
        "S7 report closure shape: NEEDS_CHANGES",
        "per_seed_artifacts must contain 10 rows",
        "ProceedToS8DenseOnly is permitted only when s7_outcome is FailParity",
        "Decision body must match front matter decision",
        "body_section",
        "must match artifact self-hash",
        "checkpoint_self_hash",
        "switch_stats_self_hash",
        "report_self_hash must match report bytes",
        "generated_at and report_self_hash nulled",
        "closure-candidate reports must not use NotEvaluatedDueToPriorGate",
        "unsupported YAML flow collection",
        "duplicate front matter field",
        "report artifact reference has duplicate JSON key",
        "report artifact reference must use canonical JSON bytes",
        "report artifact reference has non-canonical JSON value",
        "emulator_one_token_dense_self_hash",
        "## Pre-registered predictions",
        "## Reproducibility statement",
    ] {
        assert!(
            script.contains(required),
            "report validator missing {required}"
        );
    }
    assert!(emitter.contains("Build a fail-closed F-S7 s7_report.v1 from production artifacts"));
    assert!(emitter.contains("missing artifact:"));
    assert!(emitter.contains("completion is not completed"));
    assert!(emitter.contains("report_self_hash: null"));
}

#[test]
fn s7_artifact_validator_guards_closure_artifact_invariants() {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root");
    let script_path = workspace_root.join("scripts/review/f-s7/validate-artifacts.py");
    let script = std::fs::read_to_string(script_path).expect("artifact validator exists");

    for required in [
        "S7 artifact closure shape: NEEDS_CHANGES",
        "must use canonical JSON bytes",
        "duplicate JSON key",
        "has non-canonical JSON value",
        "self-hash mismatch",
        "bpc must equal log2_sum / token_count",
        "records length must equal D11 grid length",
        "route_coverage must prove all routed fixture axes",
        "bank_switch_within_one must be true",
        "bytes_within_tolerance must be true for bd-2v9r closure",
        "aggregate_parity_verdict must match derived",
        "pareto_verdict must match derived",
        "transition_mass must be non-empty",
        "quality.per_seed_val_bpc must contain 5 finite values",
        "points must contain one MoE and one dense point",
        "dense router_config_hash must be null",
        "guardrail_verdict",
        "producer_kind",
        "production_closure_retrain_score",
        "QuantSpec::weight_quant",
        "grad log record",
        "grad log must contain",
        "schema must be s7_grad_log.v1",
        "grad_norms must match run-log grad_norms",
        "GradNormSummary fields must be global_l2, max_l2, mean_l2",
        "router-step telemetry must cover layers 0..3",
        "dense router-step telemetry must be empty",
        "telemetry_self_hash",
        "SWITCH_STATS_DOMAIN",
        "FRONTIER_DOMAIN",
        "BURN_GRAD_SMOKE_DOMAIN",
        "supported_clipped_activation_count",
        "learned_activation_range_unsupported",
        "projection_biases_unsupported",
        "ExpertBlockQat bias and learned activation-range parameters are rejected",
        "ORACLE_ROUTED_DOMAIN",
        "frontier_self_hash",
        "smoke_self_hash",
        "oracle_self_hash",
        "completion must be Rust tagged S7Completion",
        "losses length must be",
        "RawLossDiagnostics fields must be lm_loss_raw",
        "diagnostics_self_hash",
        "train_step must match loss step",
    ] {
        assert!(
            script.contains(required),
            "artifact validator missing {required}"
        );
    }
}

#[test]
fn s7_artifact_validator_python_canonical_hash_matches_rust_foundation() {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root");
    let script_path = workspace_root.join("scripts/review/f-s7/validate-artifacts.py");
    let mut child = Command::new("python3")
        .arg("-")
        .arg(&script_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("python3 starts");
    child
        .stdin
        .as_mut()
        .expect("python stdin")
        .write_all(
            br#"
from pathlib import Path
import importlib.util
import json
import sys

module_path = Path(sys.argv[1])
spec = importlib.util.spec_from_file_location("s7_validate_artifacts", module_path)
module = importlib.util.module_from_spec(spec)
assert spec.loader is not None
spec.loader.exec_module(module)

payload = {
    "lm_loss_raw": 1.0,
    "distill_loss_raw": {
        "kind": "not_available",
        "reason": "no_frozen_teacher_\u00b5",
        "phase": "phase_a",
    },
    "balance_loss_raw": 0.1,
    "zrouter_loss_raw": 0.2,
    "switch_loss_raw": 0.3,
}
print(json.dumps({
    "canonical": module.canonical_json_text(payload),
    "hash": module.domain_self_hash(module.RAW_LOSS_DIAGNOSTICS_DOMAIN, payload),
}, sort_keys=True))
"#,
        )
        .expect("write python probe");
    let output = child.wait_with_output().expect("python probe output");
    assert!(
        output.status.success(),
        "python canonical/hash probe failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let python: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("python probe emits JSON");
    let raw_loss = json!({
        "lm_loss_raw": 1.0,
        "distill_loss_raw": {
            "kind": "not_available",
            "reason": "no_frozen_teacher_\u{00b5}",
            "phase": "phase_a",
        },
        "balance_loss_raw": 0.1,
        "zrouter_loss_raw": 0.2,
        "switch_loss_raw": 0.3,
    });
    let rust_canonical =
        String::from_utf8(CanonicalJson::value_to_vec(&raw_loss).expect("Rust canonical JSON"))
            .expect("canonical JSON is UTF-8");
    let rust_hash = DomainHash::new(
        "gbf-artifact",
        "RawLossDiagnostics",
        "s7_raw_loss_diagnostics.v1",
        "1",
    )
    .hash_canonical_bytes(rust_canonical.as_bytes())
    .expect("Rust domain hash")
    .to_string();

    assert_eq!(
        python
            .get("canonical")
            .and_then(serde_json::Value::as_str)
            .expect("python canonical string"),
        rust_canonical
    );
    assert_eq!(
        python
            .get("hash")
            .and_then(serde_json::Value::as_str)
            .expect("python hash string"),
        rust_hash
    );
}

#[test]
fn s7_review_validator_requires_gemini_claude_acpx_passes() {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root");
    let script_path = workspace_root.join("scripts/review/f-s7/validate-reviews.py");
    let script = std::fs::read_to_string(script_path).expect("review validator exists");

    for required in [
        "S7 ACPX review evidence: NEEDS_CHANGES",
        "s7_acpx_review.v1",
        "\"gemini\": {\"P3\", \"P4\", \"P5\", \"P6\", \"P7\", \"P8\"}",
        "\"claude\": {\"P3\", \"P5\", \"P6\", \"P8\"}",
        "ALWAYS_ON_PERSONAS",
        "transport",
        "acpx",
        "verdict",
        "PASS",
        "missing required personas for",
        "missing always-on persona",
        "reviewed_head must match expected_head",
        "PASS review has unresolved blocking finding",
        "allowed_severities",
        "severity must be one of",
        "status must be one of",
        "must be an object",
        "command must record an ACPX invocation prefix",
    ] {
        assert!(
            script.contains(required),
            "review validator missing {required}"
        );
    }
}
