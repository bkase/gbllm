#![cfg(feature = "s7")]

use assert_cmd::Command;
use gbf_artifact::{
    DistillRawDiagnostic, GradNormSummary, GuardrailVerdict, LambdaSwitch, QuantSpec,
    RawLossDiagnostics, S7_EVAL_EVERY_STEPS, S7_N_BLOCKS, S7_OPTIMIZER_STEPS, S7Completion,
    S7RunLog, S7ScoreReport, S7Topology, SweepSummary, SwitchStatsSummary,
};
use gbf_experiments::s7::baseline_match::canonical_s7_matched_bytes_pin;
use gbf_experiments::s7::schema::{ConfidenceDist, RouterStepTelemetry};
use gbf_foundation::Hash256;
use gbf_foundation::{CanonicalJson, DomainHash};
use predicates::prelude::*;
use serde_json::Value;
use std::path::Path;
use std::process::{Command as ProcessCommand, Output};

const S7_SWITCH_STATS_DOMAIN: DomainHash<'static> = DomainHash::new(
    "gbf-experiments",
    "S7SwitchStatsReport",
    "s7_switch_stats.v1",
    "1",
);
const S7_TEMPORAL_SWITCH_DIGEST_DOMAIN: DomainHash<'static> = DomainHash::new(
    "gbf-artifact",
    "TemporalSwitchDigest",
    "s7_temporal_switch_digest.v1",
    "1",
);
const S7_EXPERT_SLOT_AFFINITY_DOMAIN: DomainHash<'static> = DomainHash::new(
    "gbf-artifact",
    "ExpertSlotAffinity",
    "s7_expert_slot_affinity.v1",
    "1",
);
const S7_CLIP_SATURATION_DIGEST_DOMAIN: DomainHash<'static> = DomainHash::new(
    "gbf-artifact",
    "ClipSaturationDigest",
    "s7_clip_saturation_digest.v1",
    "1",
);
const S7_EXPERT_PAYLOAD_DIGEST_DOMAIN: DomainHash<'static> = DomainHash::new(
    "gbf-artifact",
    "ExpertPayloadDigest",
    "s7_expert_payload_digest.v1",
    "1",
);
const S7_RUN_LOG_DOMAIN: DomainHash<'static> =
    DomainHash::new("gbf-artifact", "S7RunLog", "s7_run_log.v1", "1");
const S7_SCORE_DOMAIN: DomainHash<'static> =
    DomainHash::new("gbf-artifact", "S7ScoreReport", "s7_score.v1", "1");
const S7_DENSE_VS_MOE_DOMAIN: DomainHash<'static> = DomainHash::new(
    "gbf-artifact",
    "S7DenseVsMoeComparisonReport",
    "s7_dense_vs_moe.v1",
    "1",
);
const S7_ROUTER_COLLAPSE_SWEEP_DOMAIN: DomainHash<'static> = DomainHash::new(
    "gbf-experiments",
    "RouterCollapseSweepReport",
    "s7_router_collapse_sweep.v1",
    "1",
);
const S7_LAMBDA_SWITCH_RECORD_DOMAIN: DomainHash<'static> = DomainHash::new(
    "gbf-experiments",
    "LambdaSwitchSweepRecord",
    "s7_lambda_switch_sweep_step.v1",
    "1",
);
const S7_FRONTIER_DOMAIN: DomainHash<'static> =
    DomainHash::new("gbf-experiments", "S7FrontierReport", "s7_frontier.v1", "1");
const S7_BURN_GRAD_SMOKE_DOMAIN: DomainHash<'static> = DomainHash::new(
    "gbf-experiments",
    "S7BurnGradSmokeReport",
    "s7_burn_grad_smoke.v1",
    "1",
);
const S7_ORACLE_ROUTED_DOMAIN: DomainHash<'static> = DomainHash::new(
    "gbf-experiments",
    "S7OracleRoutedReport",
    "s7_oracle_routed.v1",
    "1",
);
const S7_EMULATOR_ONE_TOKEN_DOMAIN: DomainHash<'static> = DomainHash::new(
    "gbf-experiments",
    "EmulatorOneTokenReport",
    "s7_emulator_one_token.v1",
    "1",
);
const S7_REPORT_MARKDOWN_DOMAIN: DomainHash<'static> =
    DomainHash::new("gbf-experiments", "S7ReportMarkdown", "s7_report.v1", "1");

fn gbf() -> Command {
    Command::cargo_bin("gbf-cli").expect("gbf-cli binary")
}

#[test]
fn s7_help_lists_dispatch_verbs() {
    let mut command = gbf();
    command.args(["s7", "--help"]);

    command.assert().success().stdout(
        predicate::str::contains("replay")
            .and(predicate::str::contains("materialize-run"))
            .and(predicate::str::contains("derive-comparison"))
            .and(predicate::str::contains("materialize-support-artifact"))
            .and(predicate::str::contains("emulator-one-token"))
            .and(predicate::str::contains("emit-report"))
            .and(predicate::str::contains("validate-closure")),
    );
}

#[cfg(feature = "s7-burn-grad-smoke")]
#[test]
fn s7_help_lists_burn_grad_smoke_when_feature_enabled() {
    let mut command = gbf();
    command.args(["s7", "--help"]);

    command
        .assert()
        .success()
        .stdout(predicate::str::contains("burn-grad-smoke"));
}

#[test]
fn s7_replay_fixture_writes_split_feature_evidence() {
    let temp = tempfile::tempdir().expect("tempdir");
    let output = temp.path().join("s7-replay.json");

    let mut command = gbf();
    command.args([
        "--log-level",
        "off",
        "s7",
        "replay",
        "--pass-version",
        "fixture-pass",
        "--topology",
        expected_topology(),
        "--seed-list",
        "0,1",
        "--output",
        output.to_str().expect("utf8 output path"),
    ]);

    let output_result = command.output().expect("s7 replay runs");
    assert!(
        output_result.status.success(),
        "s7 replay failed:\n{}",
        command_output(&output_result)
    );
    let artifact_self_hash = single_stdout_hash(&output_result);
    let evidence: Value =
        serde_json::from_slice(&std::fs::read(&output).expect("s7 replay evidence reads"))
            .expect("s7 replay evidence parses");

    assert_eq!(evidence["schema"], "s7_replay_cli.v1");
    assert_eq!(evidence["artifact_self_hash"], artifact_self_hash);
    assert_eq!(evidence["status"], "fixture_replayed");
    assert_eq!(
        evidence["support_scope"],
        "s7_fixture_contract_no_full_cli_replay"
    );
    assert_eq!(evidence["moved_full_cli_replay_to"], "bd-1ryn");
    assert_eq!(evidence["moved_full_closure_to"], "bd-2v9r");
    assert_eq!(evidence["feature_gate"], expected_feature_gate());
    assert_eq!(evidence["topology"], expected_topology());
    assert_eq!(evidence["runs"].as_array().expect("runs array").len(), 2);
    assert_sha256(&evidence["runs"][0]["checkpoint_sha"]);
    assert_sha256(&evidence["runs"][0]["run_log_self_hash"]);
    assert_sha256(&evidence["runs"][0]["score_self_hash"]);
}

#[test]
fn s7_replay_rejects_wrong_topology_for_split_feature() {
    let wrong_topology = if cfg!(feature = "s7-moe") {
        Some("MoeTinyDenseMatched")
    } else if cfg!(feature = "s7-dense-matched") {
        Some("MoeTiny")
    } else {
        None
    };
    let Some(wrong_topology) = wrong_topology else {
        return;
    };

    let mut command = gbf();
    command.args([
        "--log-level",
        "off",
        "s7",
        "replay",
        "--pass-version",
        "fixture-pass",
        "--topology",
        wrong_topology,
    ]);

    command
        .assert()
        .failure()
        .stderr(predicate::str::contains("S7 feature/topology mismatch"));
}

#[test]
fn s7_materialize_run_writes_completed_artifact_paths() {
    let temp = tempfile::tempdir().expect("tempdir");
    let input = temp.path().join("input");
    let packet = temp.path().join("packet");
    let paths =
        write_materialize_run_inputs(&input, S7Topology::MoeTiny, 0, S7Completion::Completed);

    let mut command = gbf();
    command.args([
        "--log-level",
        "off",
        "s7",
        "materialize-run",
        "--root",
        packet.to_str().expect("utf8 packet root"),
        "--topology",
        "MoeTiny",
        "--seed",
        "0",
        "--run-log",
        paths.run_log.to_str().expect("utf8 run log"),
        "--score",
        paths.score.to_str().expect("utf8 score"),
        "--grad-log",
        paths.grad_log.to_str().expect("utf8 grad log"),
        "--router-step-telemetry",
        paths
            .router_step_telemetry
            .to_str()
            .expect("utf8 router telemetry"),
    ]);

    let output_result = command.output().expect("s7 materialize-run runs");
    assert!(
        output_result.status.success(),
        "s7 materialize-run failed:\n{}",
        command_output(&output_result)
    );
    let run_log_self_hash = single_stdout_hash(&output_result);
    let out_run_log = packet.join("experiments/S7/runs/MoeTiny/seed-0/run-log.json");
    let out_score = packet.join("experiments/S7/scores/MoeTiny/seed-0/score.json");
    let out_grad_log = packet.join("experiments/S7/runs/MoeTiny/seed-0/grad-log.jsonl");
    let out_telemetry =
        packet.join("experiments/S7/runs/MoeTiny/seed-0/router-step-telemetry.jsonl");

    let materialized_run: S7RunLog =
        serde_json::from_slice(&std::fs::read(&out_run_log).expect("run-log reads"))
            .expect("run-log parses");
    assert_eq!(
        materialized_run.run_log_self_hash.to_string(),
        run_log_self_hash
    );
    assert_eq!(materialized_run.completion, S7Completion::Completed);
    assert_eq!(materialized_run.losses.len(), S7_OPTIMIZER_STEPS as usize);
    let materialized_score: S7ScoreReport =
        serde_json::from_slice(&std::fs::read(&out_score).expect("score reads"))
            .expect("score parses");
    assert_eq!(materialized_score.seed, 0);
    assert_eq!(materialized_score.topology, S7Topology::MoeTiny);
    assert_eq!(
        std::fs::read_to_string(&out_grad_log)
            .expect("grad log reads")
            .lines()
            .count(),
        S7_OPTIMIZER_STEPS as usize
    );
    assert_eq!(
        std::fs::read_to_string(&out_telemetry)
            .expect("telemetry reads")
            .lines()
            .count(),
        S7_N_BLOCKS as usize
    );
}

#[test]
fn s7_materialize_run_rejects_incomplete_fixture_shaped_run_log() {
    let temp = tempfile::tempdir().expect("tempdir");
    let input = temp.path().join("input");
    let packet = temp.path().join("packet");
    let paths = write_materialize_run_inputs(
        &input,
        S7Topology::MoeTiny,
        0,
        S7Completion::DivergedAt { step: 2 },
    );

    let mut command = gbf();
    command.args([
        "--log-level",
        "off",
        "s7",
        "materialize-run",
        "--root",
        packet.to_str().expect("utf8 packet root"),
        "--topology",
        "MoeTiny",
        "--seed",
        "0",
        "--run-log",
        paths.run_log.to_str().expect("utf8 run log"),
        "--score",
        paths.score.to_str().expect("utf8 score"),
        "--grad-log",
        paths.grad_log.to_str().expect("utf8 grad log"),
        "--router-step-telemetry",
        paths
            .router_step_telemetry
            .to_str()
            .expect("utf8 router telemetry"),
    ]);

    let output_result = command.output().expect("s7 materialize-run runs");
    assert!(
        !output_result.status.success(),
        "s7 materialize-run unexpectedly succeeded:\n{}",
        command_output(&output_result)
    );
    assert!(output_result.stdout.is_empty());
    assert!(
        command_output(&output_result).contains("requires completed run-log"),
        "{}",
        command_output(&output_result)
    );
}

#[test]
fn s7_derive_comparison_writes_dense_vs_moe_artifact() {
    let temp = tempfile::tempdir().expect("tempdir");
    let packet = temp.path().join("packet");
    write_comparison_inputs(&packet, true);

    let mut command = gbf();
    command.args([
        "--log-level",
        "off",
        "s7",
        "derive-comparison",
        "--root",
        packet.to_str().expect("utf8 packet root"),
        "--moe-topology-hash",
        &test_hash(91).to_string(),
        "--dense-matched-topology-hash",
        &test_hash(92).to_string(),
    ]);

    let output_result = command.output().expect("s7 derive-comparison runs");
    assert!(
        output_result.status.success(),
        "s7 derive-comparison failed:\n{}",
        command_output(&output_result)
    );
    let comparison_self_hash = single_stdout_hash(&output_result);
    let out_comparison = packet.join("experiments/S7/dense-vs-moe/comparison.json");
    let comparison: Value =
        serde_json::from_slice(&std::fs::read(&out_comparison).expect("comparison reads"))
            .expect("comparison parses");

    assert_eq!(comparison["schema"], "s7_dense_vs_moe.v1");
    assert_eq!(
        comparison["comparison_self_hash"]
            .as_str()
            .expect("comparison hash string"),
        comparison_self_hash
    );
    assert_eq!(
        comparison["per_seed"].as_array().expect("per seed").len(),
        5
    );
    assert_eq!(comparison["median_val_bpc_moe"], 1.25);
    assert_eq!(comparison["median_val_bpc_dense"], 1.75);
    assert_eq!(comparison["aggregate_parity_verdict"], "Pass-clean");
    assert_eq!(comparison["pareto_verdict"], "MoE-dominates");
}

#[test]
fn s7_derive_comparison_rejects_missing_materialized_score() {
    let temp = tempfile::tempdir().expect("tempdir");
    let packet = temp.path().join("packet");
    write_comparison_inputs(&packet, false);

    let mut command = gbf();
    command.args([
        "--log-level",
        "off",
        "s7",
        "derive-comparison",
        "--root",
        packet.to_str().expect("utf8 packet root"),
        "--moe-topology-hash",
        &test_hash(91).to_string(),
        "--dense-matched-topology-hash",
        &test_hash(92).to_string(),
    ]);

    let output_result = command.output().expect("s7 derive-comparison runs");
    assert!(
        !output_result.status.success(),
        "s7 derive-comparison unexpectedly succeeded:\n{}",
        command_output(&output_result)
    );
    assert!(output_result.stdout.is_empty());
    assert!(
        command_output(&output_result)
            .contains("experiments/S7/scores/MoeTinyDenseMatched/seed-4/score.json"),
        "{}",
        command_output(&output_result)
    );
}

#[test]
fn s7_materialize_support_artifact_writes_frontier_packet_path() {
    let temp = tempfile::tempdir().expect("tempdir");
    let input_root = temp.path().join("input");
    let packet = temp.path().join("packet");
    write_json(&input_root, "frontier.json", &frontier_support_artifact());

    let mut command = gbf();
    command.args([
        "--log-level",
        "off",
        "s7",
        "materialize-support-artifact",
        "--root",
        packet.to_str().expect("utf8 packet root"),
        "--kind",
        "frontier",
        "--input",
        input_root
            .join("frontier.json")
            .to_str()
            .expect("utf8 frontier input"),
    ]);

    let output_result = command
        .output()
        .expect("s7 materialize-support-artifact runs");
    assert!(
        output_result.status.success(),
        "s7 materialize-support-artifact failed:\n{}",
        command_output(&output_result)
    );
    let frontier_self_hash = single_stdout_hash(&output_result);
    let out_frontier = packet.join("experiments/S7/frontier/frontier.json");
    let frontier: Value =
        serde_json::from_slice(&std::fs::read(&out_frontier).expect("frontier reads"))
            .expect("frontier parses");

    assert_eq!(frontier["schema"], "s7_frontier.v1");
    assert_eq!(
        frontier["frontier_self_hash"]
            .as_str()
            .expect("frontier hash string"),
        frontier_self_hash
    );
    assert_eq!(frontier["points"].as_array().expect("points").len(), 2);
}

#[test]
fn s7_materialize_support_artifact_rejects_self_hash_mismatch() {
    let temp = tempfile::tempdir().expect("tempdir");
    let input_root = temp.path().join("input");
    let packet = temp.path().join("packet");
    let mut frontier = frontier_support_artifact();
    frontier.as_object_mut().expect("frontier object").insert(
        "frontier_self_hash".to_owned(),
        Value::String(test_hash(199).to_string()),
    );
    write_json(&input_root, "frontier.json", &frontier);

    let mut command = gbf();
    command.args([
        "--log-level",
        "off",
        "s7",
        "materialize-support-artifact",
        "--root",
        packet.to_str().expect("utf8 packet root"),
        "--kind",
        "frontier",
        "--input",
        input_root
            .join("frontier.json")
            .to_str()
            .expect("utf8 frontier input"),
    ]);

    let output_result = command
        .output()
        .expect("s7 materialize-support-artifact runs");
    assert!(
        !output_result.status.success(),
        "s7 materialize-support-artifact unexpectedly succeeded:\n{}",
        command_output(&output_result)
    );
    assert!(output_result.stdout.is_empty());
    assert!(
        command_output(&output_result).contains("frontier_self_hash mismatch"),
        "{}",
        command_output(&output_result)
    );
}

#[test]
fn s7_materialize_support_artifact_writes_burn_grad_packet_path() {
    let temp = tempfile::tempdir().expect("tempdir");
    let input_root = temp.path().join("input");
    let packet = temp.path().join("packet");
    write_json(&input_root, "burn-grad.json", &burn_grad_support_artifact());

    let mut command = gbf();
    command.args([
        "--log-level",
        "off",
        "s7",
        "materialize-support-artifact",
        "--root",
        packet.to_str().expect("utf8 packet root"),
        "--kind",
        "burn-grad-smoke",
        "--input",
        input_root
            .join("burn-grad.json")
            .to_str()
            .expect("utf8 burn-grad input"),
    ]);

    let output_result = command
        .output()
        .expect("s7 materialize-support-artifact runs");
    assert!(
        output_result.status.success(),
        "s7 materialize-support-artifact failed:\n{}",
        command_output(&output_result)
    );
    let smoke_self_hash = single_stdout_hash(&output_result);
    let out_burn_grad = packet.join("experiments/S7/burn-grad-smoke/expert_block_qat.json");
    let burn_grad: Value =
        serde_json::from_slice(&std::fs::read(&out_burn_grad).expect("burn grad reads"))
            .expect("burn grad parses");

    assert_eq!(burn_grad["schema"], "s7_burn_grad_smoke.v1");
    assert_eq!(burn_grad["projection_biases_unsupported"], true);
    assert_eq!(
        burn_grad["smoke_self_hash"]
            .as_str()
            .expect("smoke hash string"),
        smoke_self_hash
    );
}

#[cfg(feature = "s7-burn-grad-smoke")]
#[test]
fn s7_burn_grad_smoke_writes_h8_fixture_report() {
    let temp = tempfile::tempdir().expect("tempdir");
    let packet = temp.path().join("packet");

    let mut command = gbf();
    command.args([
        "--log-level",
        "off",
        "s7",
        "burn-grad-smoke",
        "--root",
        packet.to_str().expect("utf8 packet root"),
    ]);

    let output_result = command.output().expect("s7 burn-grad-smoke runs");
    assert!(
        output_result.status.success(),
        "s7 burn-grad-smoke failed:\n{}",
        command_output(&output_result)
    );
    let smoke_self_hash = single_stdout_hash(&output_result);
    let out_burn_grad = packet.join("experiments/S7/burn-grad-smoke/expert_block_qat.json");
    let burn_grad: Value =
        serde_json::from_slice(&std::fs::read(&out_burn_grad).expect("burn grad reads"))
            .expect("burn grad parses");

    assert_eq!(burn_grad["schema"], "s7_burn_grad_smoke.v1");
    assert_eq!(burn_grad["fixture_seed"], 65261);
    assert!(
        burn_grad["burn_adapter_version"]
            .as_str()
            .expect("adapter version")
            .contains("burn-adapter")
    );
    assert_sha256(&burn_grad["fixture_input_sha"]);
    assert!(burn_grad["grad_up_weight_sum_abs"].as_f64().unwrap() > 0.0);
    assert!(burn_grad["grad_down_weight_sum_abs"].as_f64().unwrap() > 0.0);
    assert_eq!(burn_grad["supported_clipped_activation_count"], 3);
    assert_eq!(burn_grad["learned_activation_range_unsupported"], true);
    assert_eq!(burn_grad["projection_biases_unsupported"], true);
    assert_eq!(burn_grad["glu_construction_rejected"], true);
    assert_eq!(burn_grad["replay_byte_identical"], true);
    assert!(burn_grad.get("grad_up_bias_sum_abs").is_none());
    assert!(burn_grad.get("grad_down_bias_sum_abs").is_none());
    assert!(
        burn_grad
            .get("grad_activation_clip_threshold_sum_abs")
            .is_none()
    );
    assert_eq!(
        burn_grad["smoke_self_hash"]
            .as_str()
            .expect("smoke hash string"),
        smoke_self_hash
    );
    assert_eq!(
        burn_grad,
        with_domain_self_hash(
            burn_grad.clone(),
            "smoke_self_hash",
            S7_BURN_GRAD_SMOKE_DOMAIN
        )
    );
}

#[test]
fn s7_materialize_support_artifact_rejects_burn_grad_bias_gradient_fields() {
    for rejected_field in [
        "grad_up_bias_sum_abs",
        "grad_down_bias_sum_abs",
        "grad_activation_clip_threshold_sum_abs",
    ] {
        let temp = tempfile::tempdir().expect("tempdir");
        let input_root = temp.path().join("input");
        let packet = temp.path().join("packet");
        let mut burn_grad = burn_grad_support_artifact();
        burn_grad
            .as_object_mut()
            .expect("burn grad object")
            .insert(rejected_field.to_owned(), serde_json::json!(1.0));
        let burn_grad =
            with_domain_self_hash(burn_grad, "smoke_self_hash", S7_BURN_GRAD_SMOKE_DOMAIN);
        write_json(&input_root, "burn-grad.json", &burn_grad);

        let mut command = gbf();
        command.args([
            "--log-level",
            "off",
            "s7",
            "materialize-support-artifact",
            "--root",
            packet.to_str().expect("utf8 packet root"),
            "--kind",
            "burn-grad-smoke",
            "--input",
            input_root
                .join("burn-grad.json")
                .to_str()
                .expect("utf8 burn-grad input"),
        ]);

        let output_result = command
            .output()
            .expect("s7 materialize-support-artifact runs");
        assert!(
            !output_result.status.success(),
            "s7 materialize-support-artifact unexpectedly succeeded for {rejected_field}:\n{}",
            command_output(&output_result)
        );
        assert!(output_result.stdout.is_empty());
        let expected = format!(
            "{rejected_field} is unsupported because ExpertBlockQat bias and learned activation-range parameters are rejected"
        );
        assert!(
            command_output(&output_result).contains(&expected),
            "{}",
            command_output(&output_result)
        );
    }
}

#[test]
fn s7_materialize_support_artifact_writes_switch_stats_packet_path() {
    let temp = tempfile::tempdir().expect("tempdir");
    let input_root = temp.path().join("input");
    let packet = temp.path().join("packet");
    write_json(
        &input_root,
        "switch-stats.json",
        &switch_stats_support_artifact(3),
    );

    let mut command = gbf();
    command.args([
        "--log-level",
        "off",
        "s7",
        "materialize-support-artifact",
        "--root",
        packet.to_str().expect("utf8 packet root"),
        "--kind",
        "switch-stats",
        "--seed",
        "3",
        "--input",
        input_root
            .join("switch-stats.json")
            .to_str()
            .expect("utf8 switch stats input"),
    ]);

    let output_result = command
        .output()
        .expect("s7 materialize-support-artifact runs");
    assert!(
        output_result.status.success(),
        "s7 materialize-support-artifact failed:\n{}",
        command_output(&output_result)
    );
    let bundle_self_hash = single_stdout_hash(&output_result);
    let out_stats = packet.join("experiments/S7/switch-stats/seed-3/switch-stats.json");
    let stats: Value =
        serde_json::from_slice(&std::fs::read(&out_stats).expect("switch stats reads"))
            .expect("switch stats parses");

    assert_eq!(stats["schema"], "s7_switch_stats.v1");
    assert_eq!(stats["seed"], 3);
    assert_eq!(
        stats["bundle_self_hash"]
            .as_str()
            .expect("bundle hash string"),
        bundle_self_hash
    );
    assert_eq!(
        stats["temporal_switch_digest"]
            .as_array()
            .expect("temporal digest array")
            .len(),
        S7_N_BLOCKS as usize
    );
}

#[test]
fn s7_materialize_support_artifact_rejects_switch_stats_seed_mismatch() {
    let temp = tempfile::tempdir().expect("tempdir");
    let input_root = temp.path().join("input");
    let packet = temp.path().join("packet");
    write_json(
        &input_root,
        "switch-stats.json",
        &switch_stats_support_artifact(2),
    );

    let mut command = gbf();
    command.args([
        "--log-level",
        "off",
        "s7",
        "materialize-support-artifact",
        "--root",
        packet.to_str().expect("utf8 packet root"),
        "--kind",
        "switch-stats",
        "--seed",
        "3",
        "--input",
        input_root
            .join("switch-stats.json")
            .to_str()
            .expect("utf8 switch stats input"),
    ]);

    let output_result = command
        .output()
        .expect("s7 materialize-support-artifact runs");
    assert!(
        !output_result.status.success(),
        "s7 materialize-support-artifact unexpectedly succeeded:\n{}",
        command_output(&output_result)
    );
    assert!(output_result.stdout.is_empty());
    assert!(
        command_output(&output_result).contains("seed must be 3"),
        "{}",
        command_output(&output_result)
    );
}

#[test]
fn s7_materialize_support_artifact_writes_router_collapse_sweep_packet_path() {
    let temp = tempfile::tempdir().expect("tempdir");
    let input_root = temp.path().join("input");
    let packet = temp.path().join("packet");
    write_json(
        &input_root,
        "sweep.json",
        &router_collapse_sweep_support_artifact(),
    );

    let mut command = gbf();
    command.args([
        "--log-level",
        "off",
        "s7",
        "materialize-support-artifact",
        "--root",
        packet.to_str().expect("utf8 packet root"),
        "--kind",
        "router-collapse-sweep",
        "--input",
        input_root
            .join("sweep.json")
            .to_str()
            .expect("utf8 sweep input"),
    ]);

    let output_result = command
        .output()
        .expect("s7 materialize-support-artifact runs");
    assert!(
        output_result.status.success(),
        "s7 materialize-support-artifact failed:\n{}",
        command_output(&output_result)
    );
    let sweep_self_hash = single_stdout_hash(&output_result);
    let out_sweep = packet.join("experiments/S7/router-collapse/seed-0/sweep.json");
    let sweep: Value = serde_json::from_slice(&std::fs::read(&out_sweep).expect("sweep reads"))
        .expect("sweep parses");

    assert_eq!(sweep["schema"], "s7_router_collapse_sweep.v1");
    assert_eq!(sweep["seed"], 0);
    assert_eq!(
        sweep["sweep_self_hash"]
            .as_str()
            .expect("sweep hash string"),
        sweep_self_hash
    );
    assert_eq!(sweep["records"].as_array().expect("records").len(), 6);
}

#[test]
fn s7_materialize_support_artifact_rejects_router_collapse_sweep_record_hash_mismatch() {
    let temp = tempfile::tempdir().expect("tempdir");
    let input_root = temp.path().join("input");
    let packet = temp.path().join("packet");
    let mut sweep = router_collapse_sweep_support_artifact();
    sweep["records"][2]
        .as_object_mut()
        .expect("record object")
        .insert(
            "sweep_self_hash".to_owned(),
            Value::String(test_hash(200).to_string()),
        );
    let sweep = with_domain_self_hash(sweep, "sweep_self_hash", S7_ROUTER_COLLAPSE_SWEEP_DOMAIN);
    write_json(&input_root, "sweep.json", &sweep);

    let mut command = gbf();
    command.args([
        "--log-level",
        "off",
        "s7",
        "materialize-support-artifact",
        "--root",
        packet.to_str().expect("utf8 packet root"),
        "--kind",
        "router-collapse-sweep",
        "--input",
        input_root
            .join("sweep.json")
            .to_str()
            .expect("utf8 sweep input"),
    ]);

    let output_result = command
        .output()
        .expect("s7 materialize-support-artifact runs");
    assert!(
        !output_result.status.success(),
        "s7 materialize-support-artifact unexpectedly succeeded:\n{}",
        command_output(&output_result)
    );
    assert!(output_result.stdout.is_empty());
    assert!(
        command_output(&output_result).contains("sweep_self_hash mismatch"),
        "{}",
        command_output(&output_result)
    );
}

#[test]
fn s7_materialize_support_artifact_requires_emulator_topology() {
    let temp = tempfile::tempdir().expect("tempdir");
    let input_root = temp.path().join("input");
    let packet = temp.path().join("packet");
    write_json(
        &input_root,
        "emulator.json",
        &emulator_support_artifact("MoeTiny"),
    );

    let mut command = gbf();
    command.args([
        "--log-level",
        "off",
        "s7",
        "materialize-support-artifact",
        "--root",
        packet.to_str().expect("utf8 packet root"),
        "--kind",
        "emulator-one-token",
        "--input",
        input_root
            .join("emulator.json")
            .to_str()
            .expect("utf8 emulator input"),
    ]);

    let output_result = command
        .output()
        .expect("s7 materialize-support-artifact runs");
    assert!(
        !output_result.status.success(),
        "s7 materialize-support-artifact unexpectedly succeeded:\n{}",
        command_output(&output_result)
    );
    assert!(output_result.stdout.is_empty());
    assert!(
        command_output(&output_result).contains("requires --topology"),
        "{}",
        command_output(&output_result)
    );
}

#[test]
fn s7_emulator_one_token_writes_h10_report() {
    let temp = tempfile::tempdir().expect("tempdir");
    let packet = temp.path().join("packet");

    let mut command = gbf();
    command.args([
        "--log-level",
        "off",
        "s7",
        "emulator-one-token",
        "--root",
        packet.to_str().expect("utf8 packet root"),
        "--topology",
        "MoeTiny",
        "--encoded-rom-sha",
        &test_hash(200).to_string(),
        "--prompt-sha",
        &test_hash(201).to_string(),
        "--artifact-oracle-logits-sha",
        &test_hash(202).to_string(),
        "--emulator-logits-sha",
        &test_hash(203).to_string(),
        "--pairwise-max-abs-diff",
        "0",
        "--s5-tolerance",
        "0.125",
        "--observed-bank-switches-per-token",
        "0.25",
        "--oracle-recorded-bank-switches",
        "0.75",
    ]);

    let output_result = command.output().expect("s7 emulator-one-token runs");
    assert!(
        output_result.status.success(),
        "s7 emulator-one-token failed:\n{}",
        command_output(&output_result)
    );
    let emulator_self_hash = single_stdout_hash(&output_result);
    let out_emulator = packet.join("experiments/S7/emulator-one-token/seed-0/MoeTiny/result.json");
    let emulator: Value =
        serde_json::from_slice(&std::fs::read(&out_emulator).expect("emulator report reads"))
            .expect("emulator report parses");

    assert_eq!(emulator["schema"], "s7_emulator_one_token.v1");
    assert_eq!(emulator["seed"], 0);
    assert_eq!(emulator["topology"], "MoeTiny");
    assert_eq!(emulator["bank_switch_diff"], 0.5);
    assert_eq!(emulator["bank_switch_within_one"], true);
    assert_eq!(
        emulator["emulator_self_hash"]
            .as_str()
            .expect("emulator hash string"),
        emulator_self_hash
    );
    assert_eq!(
        emulator,
        with_domain_self_hash(
            emulator.clone(),
            "emulator_self_hash",
            S7_EMULATOR_ONE_TOKEN_DOMAIN
        )
    );

    let landed = temp.path().join("landed");
    let mut materialize = gbf();
    materialize.args([
        "--log-level",
        "off",
        "s7",
        "materialize-support-artifact",
        "--root",
        landed.to_str().expect("utf8 landed root"),
        "--kind",
        "emulator-one-token",
        "--topology",
        "MoeTiny",
        "--input",
        out_emulator.to_str().expect("utf8 emulator input"),
    ]);
    let materialize_result = materialize
        .output()
        .expect("s7 materialize-support-artifact runs");
    assert!(
        materialize_result.status.success(),
        "generated emulator report failed materialization:\n{}",
        command_output(&materialize_result)
    );
    assert_eq!(single_stdout_hash(&materialize_result), emulator_self_hash);
}

#[test]
fn s7_emulator_one_token_rejects_over_tolerance_logits() {
    let temp = tempfile::tempdir().expect("tempdir");
    let packet = temp.path().join("packet");

    let mut command = gbf();
    command.args([
        "--log-level",
        "off",
        "s7",
        "emulator-one-token",
        "--root",
        packet.to_str().expect("utf8 packet root"),
        "--topology",
        "MoeTiny",
        "--encoded-rom-sha",
        &test_hash(210).to_string(),
        "--prompt-sha",
        &test_hash(211).to_string(),
        "--artifact-oracle-logits-sha",
        &test_hash(212).to_string(),
        "--emulator-logits-sha",
        &test_hash(213).to_string(),
        "--pairwise-max-abs-diff",
        "0.25",
        "--s5-tolerance",
        "0.125",
        "--observed-bank-switches-per-token",
        "0.25",
        "--oracle-recorded-bank-switches",
        "0.75",
    ]);

    let output_result = command.output().expect("s7 emulator-one-token runs");
    assert!(
        !output_result.status.success(),
        "s7 emulator-one-token unexpectedly succeeded:\n{}",
        command_output(&output_result)
    );
    assert!(output_result.stdout.is_empty());
    assert!(
        command_output(&output_result).contains("pairwise_max_abs_diff"),
        "{}",
        command_output(&output_result)
    );
}

#[test]
fn s7_emit_report_wraps_fail_closed_report_emitter() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_minimal_report_packet(temp.path());
    let report_path = temp.path().join("docs/experiments/S7-report.md");
    let rfc_revision = "a".repeat(40);
    let predictions_section_hash = format!("sha256:{}", "2".repeat(64));
    let predictions_commit = "b".repeat(40);
    let first_result_commit = "c".repeat(40);

    let mut command = gbf();
    command.current_dir(workspace_root()).args([
        "--log-level",
        "off",
        "s7",
        "emit-report",
        "--root",
        temp.path().to_str().expect("utf8 temp path"),
        "--s7-outcome",
        "PassClean",
        "--rfc-revision",
        rfc_revision.as_str(),
        "--predictions-section-hash",
        predictions_section_hash.as_str(),
        "--predictions-commit",
        predictions_commit.as_str(),
        "--first-result-commit",
        first_result_commit.as_str(),
        "--generated-at",
        "2026-06-25T00:00:00Z",
    ]);

    let output_result = command.output().expect("s7 emit-report runs");
    assert!(
        output_result.status.success(),
        "s7 emit-report failed:\n{}",
        command_output(&output_result)
    );
    let report_self_hash = single_stdout_hash(&output_result);
    let report = std::fs::read_to_string(&report_path).expect("report emitted");

    assert!(report.contains(&format!("report_self_hash: \"{report_self_hash}\"")));
    assert!(report.contains("schema: \"s7_report.v1\""));
    assert!(report.contains("s7_outcome: PassClean"));
    assert!(report.contains("decision: ProceedToS8"));
    assert!(report.contains("generated_at: \"2026-06-25T00:00:00Z\""));
    assert!(report.contains("H10 Confirmed"));
}

#[test]
fn s7_validate_closure_invokes_rust_closure_contract_on_emitted_report() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_minimal_report_packet(temp.path());
    let emitted_hash = emit_report_for_test(temp.path());

    let output_result = validate_closure_for_test(temp.path());
    assert!(
        output_result.status.success(),
        "s7 validate-closure failed:\n{}",
        command_output(&output_result)
    );
    assert_eq!(single_stdout_hash(&output_result), emitted_hash);
}

#[test]
fn s7_validate_closure_rejects_unverified_preregistration() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_minimal_report_packet(temp.path());
    emit_report_for_test(temp.path());

    let mut command = gbf();
    command.current_dir(workspace_root()).args([
        "--log-level",
        "off",
        "s7",
        "validate-closure",
        "--root",
        temp.path().to_str().expect("utf8 temp path"),
    ]);

    let output_result = command.output().expect("s7 validate-closure runs");
    assert!(
        !output_result.status.success(),
        "s7 validate-closure unexpectedly succeeded:\n{}",
        command_output(&output_result)
    );
    assert!(output_result.stdout.is_empty());
    assert!(command_output(&output_result).contains("S7 pre-registration was not verified"));
}

#[test]
fn s7_validate_closure_rejects_switch_stats_manifest_mismatch() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_minimal_report_packet(temp.path());
    emit_report_for_test(temp.path());
    let mutated = with_domain_self_hash(
        serde_json::json!({
            "schema": "s7_switch_stats.v1",
            "seed": 3,
            "artifact_path": "seed-3-mutated",
            "aggregation_rule": "SUM",
            "bundle_self_hash": format!("sha256:{}", "4".repeat(64)),
        }),
        "bundle_self_hash",
        S7_SWITCH_STATS_DOMAIN,
    );
    write_json(
        temp.path(),
        "experiments/S7/switch-stats/seed-3/switch-stats.json",
        &mutated,
    );

    let output_result = validate_closure_for_test(temp.path());
    assert!(
        !output_result.status.success(),
        "s7 validate-closure unexpectedly succeeded:\n{}",
        command_output(&output_result)
    );
    assert!(output_result.stdout.is_empty());
    assert!(command_output(&output_result).contains("invalid s7_switch_stats self-hash"));
}

#[test]
fn s7_validate_closure_rejects_switch_stats_bundle_self_hash_mismatch() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_minimal_report_packet(temp.path());
    emit_report_for_test(temp.path());
    let hash = format!("sha256:{}", "1".repeat(64));
    write_json(
        temp.path(),
        "experiments/S7/switch-stats/seed-2/switch-stats.json",
        &serde_json::json!({
            "schema": "s7_switch_stats.v1",
            "seed": 2,
            "artifact_path": "seed-2",
            "aggregation_rule": "SUM",
            "bundle_self_hash": hash.as_str(),
        }),
    );

    let output_result = validate_closure_for_test(temp.path());
    assert!(
        !output_result.status.success(),
        "s7 validate-closure unexpectedly succeeded:\n{}",
        command_output(&output_result)
    );
    assert!(output_result.stdout.is_empty());
    assert!(command_output(&output_result).contains("invalid s7_switch_stats_bundle self-hash"));
}

#[test]
fn s7_validate_closure_rejects_matched_bytes_hash_mismatch() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_minimal_report_packet(temp.path());
    emit_report_for_test(temp.path());
    let hash = format!("sha256:{}", "1".repeat(64));
    let mismatched_hash = format!("sha256:{}", "5".repeat(64));
    write_json(
        temp.path(),
        "experiments/S7/dense-vs-moe/comparison.json",
        &with_domain_self_hash(
            serde_json::json!({
                "schema": "s7_dense_vs_moe.v1",
                "matched_bytes_pin": {"matched_bytes_self_hash": mismatched_hash.as_str()},
                "comparison_self_hash": hash.as_str(),
                "bytes_within_tolerance": true,
                "aggregate_parity_verdict": "Pass-clean",
                "per_seed": [
                    {"seed": 0, "parity_verdict": "Pass"},
                    {"seed": 1, "parity_verdict": "Pass"},
                    {"seed": 2, "parity_verdict": "Pass"},
                    {"seed": 3, "parity_verdict": "Pass"},
                    {"seed": 4, "parity_verdict": "Pass"}
                ],
            }),
            "comparison_self_hash",
            S7_DENSE_VS_MOE_DOMAIN,
        ),
    );

    let output_result = validate_closure_for_test(temp.path());
    assert!(
        !output_result.status.success(),
        "s7 validate-closure unexpectedly succeeded:\n{}",
        command_output(&output_result)
    );
    assert!(output_result.stdout.is_empty());
    assert!(command_output(&output_result).contains("invalid matched_bytes_self_hash"));
}

#[test]
fn s7_validate_closure_rejects_dense_vs_moe_self_hash_mismatch() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_minimal_report_packet(temp.path());
    emit_report_for_test(temp.path());
    let hash = format!("sha256:{}", "1".repeat(64));
    write_json(
        temp.path(),
        "experiments/S7/dense-vs-moe/comparison.json",
        &serde_json::json!({
            "schema": "s7_dense_vs_moe.v1",
            "matched_bytes_pin": {"matched_bytes_self_hash": hash.as_str()},
            "comparison_self_hash": hash.as_str(),
            "bytes_within_tolerance": true,
            "aggregate_parity_verdict": "Pass-clean",
            "per_seed": [
                {"seed": 0, "parity_verdict": "Pass"},
                {"seed": 1, "parity_verdict": "Pass"},
                {"seed": 2, "parity_verdict": "Pass"},
                {"seed": 3, "parity_verdict": "Pass"},
                {"seed": 4, "parity_verdict": "Pass"}
            ],
        }),
    );

    let output_result = validate_closure_for_test(temp.path());
    assert!(
        !output_result.status.success(),
        "s7 validate-closure unexpectedly succeeded:\n{}",
        command_output(&output_result)
    );
    assert!(output_result.stdout.is_empty());
    assert!(command_output(&output_result).contains("invalid s7_dense_vs_moe self-hash"));
}

#[test]
fn s7_validate_closure_rejects_mutated_run_log_completion() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_minimal_report_packet(temp.path());
    emit_report_for_test(temp.path());
    let hash = format!("sha256:{}", "1".repeat(64));
    write_json(
        temp.path(),
        "experiments/S7/runs/MoeTiny/seed-2/run-log.json",
        &serde_json::json!({
            "schema": "s7_run_log.v1",
            "seed": 2,
            "topology": "MoeTiny",
            "completion": {"kind": "collapsed_at", "step": 9000},
            "run_log_self_hash": hash.as_str(),
        }),
    );

    let output_result = validate_closure_for_test(temp.path());
    assert!(
        !output_result.status.success(),
        "s7 validate-closure unexpectedly succeeded:\n{}",
        command_output(&output_result)
    );
    assert!(output_result.stdout.is_empty());
    assert!(command_output(&output_result).contains("actual run-log completion is not completed"));
}

#[test]
fn s7_validate_closure_rejects_score_identity_mismatch() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_minimal_report_packet(temp.path());
    emit_report_for_test(temp.path());
    let hash = format!("sha256:{}", "1".repeat(64));
    write_json(
        temp.path(),
        "experiments/S7/scores/MoeTiny/seed-1/score.json",
        &serde_json::json!({
            "schema": "s7_score.v1",
            "seed": 1,
            "topology": "MoeTinyDenseMatched",
            "checkpoint_sha": hash.as_str(),
            "corpus_val_sha": hash.as_str(),
            "chunk_size": 256,
            "token_count": 100,
            "log2_sum": 101.0,
            "bpc": 1.01,
            "score_self_hash": hash.as_str(),
        }),
    );

    let output_result = validate_closure_for_test(temp.path());
    assert!(
        !output_result.status.success(),
        "s7 validate-closure unexpectedly succeeded:\n{}",
        command_output(&output_result)
    );
    assert!(output_result.stdout.is_empty());
    assert!(command_output(&output_result).contains("score topology mismatch"));
}

#[test]
fn s7_validate_closure_rejects_self_consistent_wrong_schema() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_minimal_report_packet(temp.path());
    let hash = format!("sha256:{}", "1".repeat(64));
    write_json(
        temp.path(),
        "experiments/S7/scores/MoeTiny/seed-0/score.json",
        &with_domain_self_hash(
            serde_json::json!({
                "schema": "s7_score_future.v9",
                "seed": 0,
                "topology": "MoeTiny",
                "checkpoint_sha": hash.as_str(),
                "corpus_val_sha": hash.as_str(),
                "chunk_size": 256,
                "token_count": 100,
                "log2_sum": 100.0,
                "bpc": 1.0,
                "score_self_hash": hash.as_str(),
            }),
            "score_self_hash",
            S7_SCORE_DOMAIN,
        ),
    );
    emit_report_for_test(temp.path());

    let output_result = validate_closure_for_test(temp.path());
    assert!(
        !output_result.status.success(),
        "s7 validate-closure unexpectedly succeeded:\n{}",
        command_output(&output_result)
    );
    assert!(output_result.stdout.is_empty());
    assert!(command_output(&output_result).contains("s7_score schema must be s7_score.v1"));
}

#[test]
fn s7_validate_closure_rejects_noncanonical_artifact_json() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_minimal_report_packet(temp.path());
    emit_report_for_test(temp.path());
    let score_path = temp
        .path()
        .join("experiments/S7/scores/MoeTiny/seed-0/score.json");
    let score: Value = serde_json::from_slice(&std::fs::read(&score_path).expect("score reads"))
        .expect("score parses");
    std::fs::write(
        &score_path,
        serde_json::to_string_pretty(&score).expect("pretty score JSON"),
    )
    .expect("pretty score writes");

    let output_result = validate_closure_for_test(temp.path());
    assert!(
        !output_result.status.success(),
        "s7 validate-closure unexpectedly succeeded:\n{}",
        command_output(&output_result)
    );
    assert!(output_result.stdout.is_empty());
    assert!(command_output(&output_result).contains("must use canonical JSON bytes"));
}

#[test]
fn s7_validate_closure_rejects_duplicate_artifact_json_key() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_minimal_report_packet(temp.path());
    emit_report_for_test(temp.path());
    let score_path = temp
        .path()
        .join("experiments/S7/scores/MoeTiny/seed-0/score.json");
    let score = std::fs::read_to_string(&score_path).expect("score reads");
    let duplicate_schema = score.replacen(
        "\"schema\":\"s7_score.v1\"",
        "\"schema\":\"s7_score.v1\",\"schema\":\"s7_score.v1\"",
        1,
    );
    assert_ne!(
        score, duplicate_schema,
        "score fixture must contain schema field"
    );
    std::fs::write(&score_path, duplicate_schema).expect("duplicate-key score writes");

    let output_result = validate_closure_for_test(temp.path());
    assert!(
        !output_result.status.success(),
        "s7 validate-closure unexpectedly succeeded:\n{}",
        command_output(&output_result)
    );
    assert!(output_result.stdout.is_empty());
    assert!(command_output(&output_result).contains("duplicate JSON key"));
}

#[test]
fn s7_validate_closure_rejects_score_self_hash_mismatch() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_minimal_report_packet(temp.path());
    emit_report_for_test(temp.path());
    let hash = format!("sha256:{}", "1".repeat(64));
    write_json(
        temp.path(),
        "experiments/S7/scores/MoeTiny/seed-4/score.json",
        &serde_json::json!({
            "schema": "s7_score.v1",
            "seed": 4,
            "topology": "MoeTiny",
            "checkpoint_sha": hash.as_str(),
            "corpus_val_sha": hash.as_str(),
            "chunk_size": 256,
            "token_count": 100,
            "log2_sum": 104.0,
            "bpc": 1.04,
            "score_self_hash": hash.as_str(),
        }),
    );

    let output_result = validate_closure_for_test(temp.path());
    assert!(
        !output_result.status.success(),
        "s7 validate-closure unexpectedly succeeded:\n{}",
        command_output(&output_result)
    );
    assert!(output_result.stdout.is_empty());
    assert!(command_output(&output_result).contains("invalid s7_score self-hash"));
}

#[test]
fn s7_validate_closure_rejects_run_log_self_hash_mismatch() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_minimal_report_packet(temp.path());
    emit_report_for_test(temp.path());
    let hash = format!("sha256:{}", "1".repeat(64));
    write_json(
        temp.path(),
        "experiments/S7/runs/MoeTinyDenseMatched/seed-0/run-log.json",
        &serde_json::json!({
            "schema": "s7_run_log.v1",
            "seed": 0,
            "topology": "MoeTinyDenseMatched",
            "completion": {"kind": "completed"},
            "run_log_self_hash": hash.as_str(),
        }),
    );

    let output_result = validate_closure_for_test(temp.path());
    assert!(
        !output_result.status.success(),
        "s7 validate-closure unexpectedly succeeded:\n{}",
        command_output(&output_result)
    );
    assert!(output_result.stdout.is_empty());
    assert!(command_output(&output_result).contains("invalid s7_run_log self-hash"));
}

#[test]
fn s7_validate_closure_rejects_sweep_self_hash_mismatch() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_minimal_report_packet(temp.path());
    emit_report_for_test(temp.path());
    let hash = format!("sha256:{}", "1".repeat(64));
    write_json(
        temp.path(),
        "experiments/S7/router-collapse/seed-0/sweep.json",
        &serde_json::json!({
            "schema": "s7_router_collapse_sweep.v1",
            "seed": 0,
            "sweep_self_hash": hash.as_str(),
        }),
    );

    let output_result = validate_closure_for_test(temp.path());
    assert!(
        !output_result.status.success(),
        "s7 validate-closure unexpectedly succeeded:\n{}",
        command_output(&output_result)
    );
    assert!(output_result.stdout.is_empty());
    assert!(command_output(&output_result).contains("invalid s7_router_collapse_sweep self-hash"));
}

#[test]
fn s7_validate_closure_rejects_emulator_self_hash_mismatch() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_minimal_report_packet(temp.path());
    emit_report_for_test(temp.path());
    let hash = format!("sha256:{}", "1".repeat(64));
    write_json(
        temp.path(),
        "experiments/S7/emulator-one-token/seed-0/MoeTiny/result.json",
        &serde_json::json!({
            "schema": "s7_emulator_one_token.v1",
            "seed": 0,
            "topology": "MoeTiny",
            "emulator_self_hash": hash.as_str(),
        }),
    );

    let output_result = validate_closure_for_test(temp.path());
    assert!(
        !output_result.status.success(),
        "s7 validate-closure unexpectedly succeeded:\n{}",
        command_output(&output_result)
    );
    assert!(output_result.stdout.is_empty());
    assert!(command_output(&output_result).contains("invalid s7_emulator_one_token self-hash"));
}

#[test]
fn s7_validate_closure_rejects_optional_dense_emulator_self_hash_mismatch() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_minimal_report_packet(temp.path());
    write_dense_emulator_result(temp.path());
    emit_report_for_test(temp.path());
    let hash = format!("sha256:{}", "1".repeat(64));
    write_json(
        temp.path(),
        "experiments/S7/emulator-one-token/seed-0/MoeTinyDenseMatched/result.json",
        &serde_json::json!({
            "schema": "s7_emulator_one_token.v1",
            "seed": 0,
            "topology": "MoeTinyDenseMatched",
            "emulator_self_hash": hash.as_str(),
        }),
    );

    let output_result = validate_closure_for_test(temp.path());
    assert!(
        !output_result.status.success(),
        "s7 validate-closure unexpectedly succeeded:\n{}",
        command_output(&output_result)
    );
    assert!(output_result.stdout.is_empty());
    assert!(
        command_output(&output_result).contains("invalid s7_emulator_one_token self-hash"),
        "{}",
        command_output(&output_result)
    );
}

#[test]
fn s7_validate_closure_rejects_frontier_self_hash_mismatch() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_minimal_report_packet(temp.path());
    emit_report_for_test(temp.path());
    let hash = format!("sha256:{}", "1".repeat(64));
    write_json(
        temp.path(),
        "experiments/S7/frontier/frontier.json",
        &serde_json::json!({
            "schema": "s7_frontier.v1",
            "pareto_verdict": "MoE-dominates",
            "frontier_self_hash": hash.as_str(),
        }),
    );

    let output_result = validate_closure_for_test(temp.path());
    assert!(
        !output_result.status.success(),
        "s7 validate-closure unexpectedly succeeded:\n{}",
        command_output(&output_result)
    );
    assert!(output_result.stdout.is_empty());
    assert!(command_output(&output_result).contains("invalid s7_frontier self-hash"));
}

#[test]
fn s7_validate_closure_rejects_burn_grad_self_hash_mismatch() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_minimal_report_packet(temp.path());
    emit_report_for_test(temp.path());
    let hash = format!("sha256:{}", "1".repeat(64));
    write_json(
        temp.path(),
        "experiments/S7/burn-grad-smoke/expert_block_qat.json",
        &serde_json::json!({
            "schema": "s7_burn_grad_smoke.v1",
            "fixture_input_sha": hash.as_str(),
            "smoke_self_hash": hash.as_str(),
        }),
    );

    let output_result = validate_closure_for_test(temp.path());
    assert!(
        !output_result.status.success(),
        "s7 validate-closure unexpectedly succeeded:\n{}",
        command_output(&output_result)
    );
    assert!(output_result.stdout.is_empty());
    assert!(command_output(&output_result).contains("invalid s7_burn_grad_smoke self-hash"));
}

#[test]
fn s7_validate_closure_rejects_oracle_routed_self_hash_mismatch() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_minimal_report_packet(temp.path());
    emit_report_for_test(temp.path());
    let hash = format!("sha256:{}", "1".repeat(64));
    write_json(
        temp.path(),
        "experiments/S7/oracle-routed/seed-0/oracle.json",
        &serde_json::json!({
            "schema": "s7_oracle_routed.v1",
            "seed": 0,
            "topology": "MoeTiny",
            "oracle_self_hash": hash.as_str(),
        }),
    );

    let output_result = validate_closure_for_test(temp.path());
    assert!(
        !output_result.status.success(),
        "s7 validate-closure unexpectedly succeeded:\n{}",
        command_output(&output_result)
    );
    assert!(output_result.stdout.is_empty());
    assert!(command_output(&output_result).contains("invalid s7_oracle_routed self-hash"));
}

#[test]
fn s7_validate_closure_rejects_passclean_parity_mismatch() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_minimal_report_packet(temp.path());
    emit_report_for_test(temp.path());
    let hash = format!("sha256:{}", "1".repeat(64));
    write_json(
        temp.path(),
        "experiments/S7/dense-vs-moe/comparison.json",
        &with_domain_self_hash(
            serde_json::json!({
                "schema": "s7_dense_vs_moe.v1",
                "matched_bytes_pin": {"matched_bytes_self_hash": hash.as_str()},
                "comparison_self_hash": hash.as_str(),
                "bytes_within_tolerance": true,
                "aggregate_parity_verdict": "Fail-parity",
                "per_seed": [
                    {"seed": 0, "parity_verdict": "Pass"},
                    {"seed": 1, "parity_verdict": "Pass"},
                    {"seed": 2, "parity_verdict": "Fail"},
                    {"seed": 3, "parity_verdict": "Pass"},
                    {"seed": 4, "parity_verdict": "Pass"}
                ],
            }),
            "comparison_self_hash",
            S7_DENSE_VS_MOE_DOMAIN,
        ),
    );

    let output_result = validate_closure_for_test(temp.path());
    assert!(
        !output_result.status.success(),
        "s7 validate-closure unexpectedly succeeded:\n{}",
        command_output(&output_result)
    );
    assert!(output_result.stdout.is_empty());
    assert!(
        command_output(&output_result)
            .contains("PassClean outcome conflicts with dense-vs-MoE parity verdict")
    );
}

#[test]
fn s7_validate_closure_allows_generated_at_hash_exclusion() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_minimal_report_packet(temp.path());
    let emitted_hash = emit_report_for_test(temp.path());
    rewrite_report(temp.path(), |text| {
        text.replace("2026-06-25T00:00:00Z", "2099-01-01T00:00:00Z")
    });

    let output_result = validate_closure_for_test(temp.path());
    assert!(
        output_result.status.success(),
        "s7 validate-closure failed after generated_at mutation:\n{}",
        command_output(&output_result)
    );
    assert_eq!(single_stdout_hash(&output_result), emitted_hash);
}

#[test]
fn s7_validate_closure_rejects_report_body_self_hash_mismatch() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_minimal_report_packet(temp.path());
    emit_report_for_test(temp.path());
    rewrite_report(temp.path(), |text| {
        text.replace("No falsification rule fired", "A falsification rule fired")
    });

    let output_result = validate_closure_for_test(temp.path());
    assert!(
        !output_result.status.success(),
        "s7 validate-closure unexpectedly succeeded:\n{}",
        command_output(&output_result)
    );
    assert!(output_result.stdout.is_empty());
    assert!(command_output(&output_result).contains("invalid s7_report self-hash"));
}

#[test]
fn s7_validate_closure_rejects_report_yaml_anchor() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_minimal_report_packet(temp.path());
    emit_report_for_test(temp.path());
    rewrite_report(temp.path(), |text| {
        text.replace("decision: ProceedToS8", "decision: &decision ProceedToS8")
    });

    let output_result = validate_closure_for_test(temp.path());
    assert!(
        !output_result.status.success(),
        "s7 validate-closure unexpectedly succeeded:\n{}",
        command_output(&output_result)
    );
    assert!(output_result.stdout.is_empty());
    assert!(command_output(&output_result).contains("unsupported YAML anchor/alias"));
}

#[test]
fn s7_validate_closure_rejects_report_flow_collection() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_minimal_report_packet(temp.path());
    emit_report_for_test(temp.path());
    rewrite_report(temp.path(), |text| {
        text.replace("decision: ProceedToS8", "decision: [ProceedToS8]")
    });

    let output_result = validate_closure_for_test(temp.path());
    assert!(
        !output_result.status.success(),
        "s7 validate-closure unexpectedly succeeded:\n{}",
        command_output(&output_result)
    );
    assert!(output_result.stdout.is_empty());
    assert!(command_output(&output_result).contains("unsupported YAML flow collection"));
}

#[test]
fn s7_validate_closure_rejects_missing_report_body_heading() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_minimal_report_packet(temp.path());
    emit_report_for_test(temp.path());
    rewrite_report_and_refresh_hash(temp.path(), |text| {
        text.replace("## Reproducibility statement", "## Reproducibility notes")
    });

    let output_result = validate_closure_for_test(temp.path());
    assert!(
        !output_result.status.success(),
        "s7 validate-closure unexpectedly succeeded:\n{}",
        command_output(&output_result)
    );
    assert!(output_result.stdout.is_empty());
    assert!(
        command_output(&output_result)
            .contains("missing body heading: ## Reproducibility statement")
    );
}

#[test]
fn s7_validate_closure_rejects_missing_explicit_hypothesis_verdict() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_minimal_report_packet(temp.path());
    emit_report_for_test(temp.path());
    rewrite_report_and_refresh_hash(temp.path(), |text| {
        text.replace("H10 Confirmed", "Emulator route Confirmed")
    });

    let output_result = validate_closure_for_test(temp.path());
    assert!(
        !output_result.status.success(),
        "s7 validate-closure unexpectedly succeeded:\n{}",
        command_output(&output_result)
    );
    assert!(output_result.stdout.is_empty());
    assert!(
        command_output(&output_result).contains("missing explicit H10 hypothesis verdict"),
        "{}",
        command_output(&output_result)
    );
}

#[test]
fn s7_validate_closure_rejects_prior_gate_placeholder() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_minimal_report_packet(temp.path());
    emit_report_for_test(temp.path());
    rewrite_report_and_refresh_hash(temp.path(), |text| {
        text.replace("H6 Confirmed", "H6 NotEvaluatedDueToPriorGate")
    });

    let output_result = validate_closure_for_test(temp.path());
    assert!(
        !output_result.status.success(),
        "s7 validate-closure unexpectedly succeeded:\n{}",
        command_output(&output_result)
    );
    assert!(output_result.stdout.is_empty());
    assert!(
        command_output(&output_result)
            .contains("closure-candidate reports must not use NotEvaluatedDueToPriorGate"),
        "{}",
        command_output(&output_result)
    );
}

#[test]
fn s7_validate_closure_rejects_extra_per_seed_report_row() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_minimal_report_packet(temp.path());
    emit_report_for_test(temp.path());
    rewrite_report_and_refresh_hash(temp.path(), duplicate_first_report_row);

    let output_result = validate_closure_for_test(temp.path());
    assert!(
        !output_result.status.success(),
        "s7 validate-closure unexpectedly succeeded:\n{}",
        command_output(&output_result)
    );
    assert!(output_result.stdout.is_empty());
    assert!(
        command_output(&output_result)
            .contains("per_seed_artifacts must contain 10 rows, observed 11"),
        "{}",
        command_output(&output_result)
    );
}

#[test]
fn s7_validate_closure_rejects_wrong_report_schema() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_minimal_report_packet(temp.path());
    emit_report_for_test(temp.path());
    rewrite_report_and_refresh_hash(temp.path(), |text| {
        text.replace(
            "schema: \"s7_report.v1\"",
            "schema: \"s7_report_future.v9\"",
        )
    });

    let output_result = validate_closure_for_test(temp.path());
    assert!(
        !output_result.status.success(),
        "s7 validate-closure unexpectedly succeeded:\n{}",
        command_output(&output_result)
    );
    assert!(output_result.stdout.is_empty());
    assert!(
        command_output(&output_result).contains("schema must be \"s7_report.v1\""),
        "{}",
        command_output(&output_result)
    );
}

#[test]
fn s7_validate_closure_rejects_uppercase_prediction_commit() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_minimal_report_packet(temp.path());
    emit_report_for_test(temp.path());
    let old = format!("predictions_commit: \"{}\"", "b".repeat(40));
    let new = format!("predictions_commit: \"{}\"", "B".repeat(40));
    rewrite_report_and_refresh_hash(temp.path(), |text| text.replace(&old, &new));

    let output_result = validate_closure_for_test(temp.path());
    assert!(
        !output_result.status.success(),
        "s7 validate-closure unexpectedly succeeded:\n{}",
        command_output(&output_result)
    );
    assert!(output_result.stdout.is_empty());
    assert!(
        command_output(&output_result)
            .contains("predictions_commit must be a 40-hex git commit id"),
        "{}",
        command_output(&output_result)
    );
}

#[test]
fn s7_validate_closure_rejects_uppercase_prediction_section_hash() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_minimal_report_packet(temp.path());
    emit_report_for_test(temp.path());
    let old = format!("predictions_section_hash: \"sha256:{}\"", "2".repeat(64));
    let new = format!("predictions_section_hash: \"sha256:{}\"", "A".repeat(64));
    rewrite_report_and_refresh_hash(temp.path(), |text| text.replace(&old, &new));

    let output_result = validate_closure_for_test(temp.path());
    assert!(
        !output_result.status.success(),
        "s7 validate-closure unexpectedly succeeded:\n{}",
        command_output(&output_result)
    );
    assert!(output_result.stdout.is_empty());
    assert!(
        command_output(&output_result)
            .contains("predictions_section_hash must be a non-null sha256 hash"),
        "{}",
        command_output(&output_result)
    );
}

#[test]
fn s7_emit_report_fails_closed_when_required_artifact_is_missing() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_minimal_report_packet(temp.path());
    std::fs::remove_file(temp.path().join("experiments/S7/frontier/frontier.json"))
        .expect("frontier artifact removed");
    let rfc_revision = "a".repeat(40);
    let predictions_section_hash = format!("sha256:{}", "2".repeat(64));
    let predictions_commit = "b".repeat(40);
    let first_result_commit = "c".repeat(40);

    let mut command = gbf();
    command.current_dir(workspace_root()).args([
        "--log-level",
        "off",
        "s7",
        "emit-report",
        "--root",
        temp.path().to_str().expect("utf8 temp path"),
        "--s7-outcome",
        "PassClean",
        "--rfc-revision",
        rfc_revision.as_str(),
        "--predictions-section-hash",
        predictions_section_hash.as_str(),
        "--predictions-commit",
        predictions_commit.as_str(),
        "--first-result-commit",
        first_result_commit.as_str(),
    ]);

    let output_result = command.output().expect("s7 emit-report runs");
    assert!(
        !output_result.status.success(),
        "s7 emit-report unexpectedly succeeded:\n{}",
        command_output(&output_result)
    );
    assert!(
        output_result.stdout.is_empty(),
        "failed emit-report must not print a hash:\n{}",
        command_output(&output_result)
    );
    let combined = command_output(&output_result);
    assert!(combined.contains("S7 report emitter failed"));
    assert!(combined.contains("missing artifact:"));
    assert!(combined.contains("experiments/S7/frontier/frontier.json"));
}

#[test]
fn s7_cli_feature_forwarding_is_registered() {
    let manifest = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
        .expect("gbf-cli Cargo.toml reads");

    assert!(
        manifest.contains("s7 = [\"gbf-experiments/s7\"]"),
        "gbf-cli must forward the base s7 feature"
    );
    assert!(
        manifest.contains("s7-moe = [\"s7\", \"gbf-experiments/s7-moe\"]"),
        "gbf-cli must forward s7-moe through gbf-experiments"
    );
    assert!(
        manifest.contains("s7-dense-matched = [\"s7\", \"gbf-experiments/s7-dense-matched\"]"),
        "gbf-cli must forward s7-dense-matched through gbf-experiments"
    );
}

#[test]
fn s7_split_features_are_mutually_exclusive() {
    let output = ProcessCommand::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
        .args([
            "check",
            "-p",
            "gbf-cli",
            "--no-default-features",
            "--features",
            "s7-moe,s7-dense-matched",
        ])
        .env("CARGO_TARGET_DIR", cargo_target_dir())
        .output()
        .expect("cargo check must run");

    assert!(
        !output.status.success(),
        "gbf-cli s7-moe+s7-dense-matched unexpectedly compiled"
    );
    let combined = command_output(&output);
    assert!(
        combined.contains(
            "S7 feature mutex violated: s7-moe and s7-dense-matched must build in separate replay passes"
        ),
        "S7 mutex probe failed without stable diagnostic:\n{combined}"
    );
}

fn expected_feature_gate() -> &'static str {
    if cfg!(feature = "s7-moe") {
        "s7-moe"
    } else if cfg!(feature = "s7-dense-matched") {
        "s7-dense-matched"
    } else {
        "s7"
    }
}

fn expected_topology() -> &'static str {
    if cfg!(feature = "s7-dense-matched") {
        "MoeTinyDenseMatched"
    } else {
        "MoeTiny"
    }
}

fn assert_sha256(value: &Value) {
    let text = value.as_str().expect("sha string");
    assert!(
        text.strip_prefix("sha256:")
            .is_some_and(|hex| hex.len() == 64 && hex.chars().all(|ch| ch.is_ascii_hexdigit())),
        "expected sha256 hash, got {text:?}"
    );
}

fn with_domain_self_hash(
    mut payload: Value,
    field: &'static str,
    domain: DomainHash<'static>,
) -> Value {
    {
        let object = payload.as_object_mut().expect("payload object");
        object.remove(field);
    }
    let canonical = CanonicalJson::value_to_vec(&payload).expect("canonical JSON");
    let hash = domain
        .hash_canonical_bytes(&canonical)
        .expect("domain self-hash");
    payload
        .as_object_mut()
        .expect("payload object")
        .insert(field.to_owned(), Value::String(hash.to_string()));
    payload
}

fn emit_report_for_test(root: &Path) -> String {
    let rfc_revision = "a".repeat(40);
    let predictions_section_hash = format!("sha256:{}", "2".repeat(64));
    let predictions_commit = "b".repeat(40);
    let first_result_commit = "c".repeat(40);

    let mut command = gbf();
    command.current_dir(workspace_root()).args([
        "--log-level",
        "off",
        "s7",
        "emit-report",
        "--root",
        root.to_str().expect("utf8 root path"),
        "--s7-outcome",
        "PassClean",
        "--rfc-revision",
        rfc_revision.as_str(),
        "--predictions-section-hash",
        predictions_section_hash.as_str(),
        "--predictions-commit",
        predictions_commit.as_str(),
        "--first-result-commit",
        first_result_commit.as_str(),
        "--generated-at",
        "2026-06-25T00:00:00Z",
    ]);
    let output_result = command.output().expect("s7 emit-report runs");
    assert!(
        output_result.status.success(),
        "s7 emit-report failed:\n{}",
        command_output(&output_result)
    );
    single_stdout_hash(&output_result)
}

fn validate_closure_for_test(root: &Path) -> Output {
    let mut command = gbf();
    command.current_dir(workspace_root()).args([
        "--log-level",
        "off",
        "s7",
        "validate-closure",
        "--root",
        root.to_str().expect("utf8 root path"),
        "--predictions-verified",
    ]);
    command.output().expect("s7 validate-closure runs")
}

fn rewrite_report(root: &Path, rewrite: impl FnOnce(String) -> String) {
    let path = root.join("docs/experiments/S7-report.md");
    let text = std::fs::read_to_string(&path).expect("report reads");
    std::fs::write(&path, rewrite(text)).expect("report writes");
}

fn rewrite_report_and_refresh_hash(root: &Path, rewrite: impl FnOnce(String) -> String) {
    let path = root.join("docs/experiments/S7-report.md");
    let text = std::fs::read_to_string(&path).expect("report reads");
    let rewritten = rewrite(text);
    let normalized = normalize_report_for_test_hash(&rewritten);
    let hash = S7_REPORT_MARKDOWN_DOMAIN
        .hash_canonical_bytes(normalized.as_bytes())
        .expect("domain report self-hash");
    let mut refreshed = String::with_capacity(rewritten.len());
    let mut report_hash_seen = false;
    for line in rewritten.split_inclusive('\n') {
        let (body, newline) = line
            .strip_suffix('\n')
            .map_or((line, ""), |body| (body, "\n"));
        if !report_hash_seen && body.starts_with("report_self_hash:") {
            refreshed.push_str(&format!("report_self_hash: \"{hash}\""));
            refreshed.push_str(newline);
            report_hash_seen = true;
        } else {
            refreshed.push_str(body);
            refreshed.push_str(newline);
        }
    }
    assert!(report_hash_seen, "report_self_hash line missing");
    std::fs::write(&path, refreshed).expect("report writes");
}

fn normalize_report_for_test_hash(text: &str) -> String {
    let mut normalized = String::with_capacity(text.len());
    let mut report_hash_seen = false;
    let mut generated_at_seen = false;
    for line in text.split_inclusive('\n') {
        let (body, newline) = line
            .strip_suffix('\n')
            .map_or((line, ""), |body| (body, "\n"));
        if !report_hash_seen && body.starts_with("report_self_hash:") {
            normalized.push_str("report_self_hash: null");
            normalized.push_str(newline);
            report_hash_seen = true;
        } else if !generated_at_seen && body.starts_with("generated_at:") {
            normalized.push_str("generated_at: null");
            normalized.push_str(newline);
            generated_at_seen = true;
        } else {
            normalized.push_str(body);
            normalized.push_str(newline);
        }
    }
    assert!(report_hash_seen, "report_self_hash line missing");
    normalized
}

fn duplicate_first_report_row(text: String) -> String {
    let row_start = text
        .find("  - seed: 0\n    topology: \"MoeTiny\"\n")
        .expect("first report row");
    let next_row = text[row_start..]
        .find("  - seed: 1\n")
        .expect("second report row");
    let insert_at = row_start + next_row;
    let row = &text[row_start..insert_at];
    let mut duplicated = String::with_capacity(text.len() + row.len());
    duplicated.push_str(&text[..insert_at]);
    duplicated.push_str(row);
    duplicated.push_str(&text[insert_at..]);
    duplicated
}

fn write_dense_emulator_result(root: &Path) {
    let hash = format!("sha256:{}", "1".repeat(64));
    write_json(
        root,
        "experiments/S7/emulator-one-token/seed-0/MoeTinyDenseMatched/result.json",
        &with_domain_self_hash(
            serde_json::json!({
                "schema": "s7_emulator_one_token.v1",
                "seed": 0,
                "topology": "MoeTinyDenseMatched",
                "emulator_self_hash": hash.as_str(),
            }),
            "emulator_self_hash",
            S7_EMULATOR_ONE_TOKEN_DOMAIN,
        ),
    );
}

struct MaterializeRunInputPaths {
    run_log: std::path::PathBuf,
    score: std::path::PathBuf,
    grad_log: std::path::PathBuf,
    router_step_telemetry: std::path::PathBuf,
}

fn write_materialize_run_inputs(
    root: &Path,
    topology: S7Topology,
    seed: u64,
    completion: S7Completion,
) -> MaterializeRunInputPaths {
    let run_log = materialize_run_log(topology.clone(), seed, completion);
    let checkpoint_sha = test_hash(11);
    let score = S7ScoreReport::new(
        seed,
        topology.clone(),
        checkpoint_sha,
        test_hash(12),
        4096,
        4096.0 * 1.25,
    )
    .expect("score constructs")
    .with_computed_self_hash()
    .expect("score hashes");

    let run_log_path = root.join("run-log.json");
    let score_path = root.join("score.json");
    let grad_log_path = root.join("grad-log.jsonl");
    let telemetry_path = root.join("router-step-telemetry.jsonl");
    write_bytes(
        &run_log_path,
        &with_trailing_newline(run_log.canonical_json_bytes().unwrap()),
    );
    write_bytes(
        &score_path,
        &with_trailing_newline(score.canonical_json_bytes().unwrap()),
    );
    write_bytes(
        &grad_log_path,
        &materialize_grad_log_jsonl(seed, &run_log.grad_norms),
    );
    write_bytes(
        &telemetry_path,
        &materialize_router_step_telemetry_jsonl(topology, seed),
    );

    MaterializeRunInputPaths {
        run_log: run_log_path,
        score: score_path,
        grad_log: grad_log_path,
        router_step_telemetry: telemetry_path,
    }
}

fn materialize_run_log(topology: S7Topology, seed: u64, completion: S7Completion) -> S7RunLog {
    let last_step = match completion {
        S7Completion::Completed => S7_OPTIMIZER_STEPS,
        S7Completion::DivergedAt { step } | S7Completion::CollapsedAt { step } => step,
    };
    let eval_points = (0..=((last_step / S7_EVAL_EVERY_STEPS) as usize))
        .map(|index| {
            (
                u64::try_from(index).unwrap() * S7_EVAL_EVERY_STEPS,
                1.25 + f64::from(index as u32) * 0.001,
            )
        })
        .collect::<Vec<_>>();
    let (router_config_hash, expert_block_config_hash) = match topology {
        S7Topology::MoeTiny => (Some(test_hash(5)), Some(test_hash(6))),
        S7Topology::MoeTinyDenseMatched => (None, None),
    };

    S7RunLog::new(
        seed,
        topology,
        test_hash(1),
        test_hash(2),
        router_config_hash,
        expert_block_config_hash,
        test_hash(3),
        test_hash(4),
        Some(test_hash(7)),
        (1..=last_step)
            .map(|step| (step, materialize_loss(step)))
            .collect(),
        (1..=last_step)
            .map(|step| (step, materialize_grad_norm(step)))
            .collect(),
        eval_points,
        materialize_grad_norm(last_step.max(1)),
        completion,
    )
    .expect("run log constructs")
    .with_computed_self_hash()
    .expect("run log hashes")
}

fn materialize_loss(step: u64) -> RawLossDiagnostics {
    RawLossDiagnostics::new(
        1.0 + step as f32 * 0.000001,
        DistillRawDiagnostic::Value { loss: 0.25 },
        0.125,
        0.0625,
        0.5,
    )
    .expect("loss constructs")
    .with_computed_self_hash()
    .expect("loss hashes")
}

fn materialize_grad_norm(step: u64) -> GradNormSummary {
    GradNormSummary::new(
        0.5 + step as f32 * 0.000001,
        0.25 + step as f32 * 0.0000005,
        0.125,
    )
    .expect("grad norm constructs")
}

fn materialize_grad_log_jsonl(seed: u64, grad_norms: &[(u64, GradNormSummary)]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for (step, grad_norms) in grad_norms {
        let record = serde_json::json!({
            "schema": "s7_grad_log.v1",
            "seed": seed,
            "train_step": step,
            "grad_norms": grad_norms,
        });
        bytes.extend(CanonicalJson::value_to_vec(&record).expect("grad log canonicalizes"));
        bytes.push(b'\n');
    }
    bytes
}

fn materialize_router_step_telemetry_jsonl(topology: S7Topology, seed: u64) -> Vec<u8> {
    if topology == S7Topology::MoeTinyDenseMatched {
        return Vec::new();
    }
    let mut bytes = Vec::new();
    for layer in 0..u32::from(S7_N_BLOCKS) {
        let telemetry = RouterStepTelemetry::new(
            seed,
            1,
            layer,
            0.5,
            ConfidenceDist::new(0.8, 0.7, 0.8, 0.9).expect("confidence dist"),
            vec![2, 2, 2, 2],
            1.0,
            u32::from(S7_N_BLOCKS),
        )
        .expect("telemetry constructs");
        bytes.extend(
            telemetry
                .canonical_json_bytes()
                .expect("telemetry canonicalizes"),
        );
        bytes.push(b'\n');
    }
    bytes
}

fn write_comparison_inputs(root: &Path, include_dense_seed_four: bool) {
    let matched_bytes = canonical_s7_matched_bytes_pin()
        .expect("matched bytes pin")
        .canonical_json_bytes()
        .expect("matched bytes canonicalizes");
    write_bytes(
        &root.join("experiments/S7/profile/matched_bytes.json"),
        &with_trailing_newline(matched_bytes),
    );

    for seed in 0..5 {
        write_score_packet(root, S7Topology::MoeTiny, seed, 1.0 + seed as f64 * 0.125);
        if include_dense_seed_four || seed != 4 {
            write_score_packet(
                root,
                S7Topology::MoeTinyDenseMatched,
                seed,
                1.5 + seed as f64 * 0.125,
            );
        }
    }

    let switch_stats =
        SwitchStatsSummary::new(vec![128, 128, 128, 128], 1.0, 0.5).expect("switch stats summary");
    write_bytes(
        &root.join("experiments/S7/summaries/switch-stats-summary.json"),
        &with_trailing_newline(CanonicalJson::to_vec(&switch_stats).unwrap()),
    );

    let mut bpc_at_lambda = std::collections::BTreeMap::new();
    bpc_at_lambda.insert(LambdaSwitch::new("0.0").expect("lambda"), 1.0);
    bpc_at_lambda.insert(LambdaSwitch::new("0.05").expect("lambda"), 1.0);
    let mut entropy_at_lambda = std::collections::BTreeMap::new();
    entropy_at_lambda.insert(LambdaSwitch::new("0.0").expect("lambda"), 1.0);
    entropy_at_lambda.insert(LambdaSwitch::new("0.05").expect("lambda"), 1.0);
    let sweep = SweepSummary::new(bpc_at_lambda, entropy_at_lambda, GuardrailVerdict::Pass)
        .expect("sweep summary");
    write_bytes(
        &root.join("experiments/S7/summaries/router-collapse-sweep-summary.json"),
        &with_trailing_newline(CanonicalJson::to_vec(&sweep).unwrap()),
    );
}

fn write_score_packet(root: &Path, topology: S7Topology, seed: u64, bpc: f64) {
    let topology_path = match topology {
        S7Topology::MoeTiny => "MoeTiny",
        S7Topology::MoeTinyDenseMatched => "MoeTinyDenseMatched",
    };
    let score = S7ScoreReport::new(
        seed,
        topology,
        test_hash(40 + u8::try_from(seed).expect("seed fits")),
        test_hash(55),
        4096,
        4096.0 * bpc,
    )
    .expect("score constructs")
    .with_computed_self_hash()
    .expect("score hashes");
    write_bytes(
        &root
            .join("experiments/S7/scores")
            .join(topology_path)
            .join(format!("seed-{seed}"))
            .join("score.json"),
        &with_trailing_newline(score.canonical_json_bytes().unwrap()),
    );
}

fn with_trailing_newline(mut bytes: Vec<u8>) -> Vec<u8> {
    bytes.push(b'\n');
    bytes
}

fn write_bytes(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().expect("path parent")).expect("input dir");
    std::fs::write(path, bytes).expect("input bytes write");
}

fn test_hash(salt: u8) -> Hash256 {
    Hash256::from_bytes([salt; 32])
}

fn switch_stats_support_artifact(seed: u64) -> Value {
    let temporal_switch_digest = (0..S7_N_BLOCKS)
        .map(|layer| {
            let salt = u8::try_from(120 + layer).expect("salt fits u8");
            with_domain_self_hash(
                serde_json::json!({
                    "schema_version": {"major": 1, "minor": 0, "patch": 0},
                    "layer_id": layer,
                    "n_experts": 4,
                    "same_expert_rate_q8_8": 128,
                    "transition_mass": [
                        {"from_expert": 0, "to_expert": 1, "mass_q8_8": 64},
                        {"from_expert": 1, "to_expert": 0, "mass_q8_8": 32},
                    ],
                    "digest_self_hash": test_hash(salt),
                }),
                "digest_self_hash",
                S7_TEMPORAL_SWITCH_DIGEST_DOMAIN,
            )
        })
        .collect::<Vec<_>>();
    let clip_saturation_digest = (0..S7_N_BLOCKS)
        .map(|layer| {
            let salt = u8::try_from(130 + layer).expect("salt fits u8");
            with_domain_self_hash(
                serde_json::json!({
                    "schema_version": {"major": 1, "minor": 0, "patch": 0},
                    "layer_id": layer,
                    "saturation_rate_q8_8": 16,
                    "clip_bound_observed": 6.0,
                    "digest_self_hash": test_hash(salt),
                }),
                "digest_self_hash",
                S7_CLIP_SATURATION_DIGEST_DOMAIN,
            )
        })
        .collect::<Vec<_>>();
    let expert_payload_digest = (0..S7_N_BLOCKS)
        .map(|layer| {
            let salt = u8::try_from(140 + layer).expect("salt fits u8");
            with_domain_self_hash(
                serde_json::json!({
                    "schema_version": {"major": 1, "minor": 0, "patch": 0},
                    "layer_id": layer,
                    "artifact_path": format!("model.layers.{layer}.experts"),
                    "entries": [
                        {"expert_id": 0, "byte_count": 128, "weight_quant": QuantSpec::default()},
                        {"expert_id": 1, "byte_count": 128, "weight_quant": QuantSpec::default()},
                        {"expert_id": 2, "byte_count": 128, "weight_quant": QuantSpec::default()},
                        {"expert_id": 3, "byte_count": 128, "weight_quant": QuantSpec::default()},
                    ],
                    "digest_self_hash": test_hash(salt),
                }),
                "digest_self_hash",
                S7_EXPERT_PAYLOAD_DIGEST_DOMAIN,
            )
        })
        .collect::<Vec<_>>();
    let expert_slot_affinity = (0..S7_N_BLOCKS)
        .map(|layer| {
            let salt = u8::try_from(150 + layer).expect("salt fits u8");
            with_domain_self_hash(
                serde_json::json!({
                    "schema_version": {"major": 1, "minor": 0, "patch": 0},
                    "layer_id": layer,
                    "affinities": [
                        {
                            "pair": {"lo": 0, "hi": 1},
                            "affinity_score": 96,
                        },
                    ],
                    "affinity_self_hash": test_hash(salt),
                }),
                "affinity_self_hash",
                S7_EXPERT_SLOT_AFFINITY_DOMAIN,
            )
        })
        .collect::<Vec<_>>();

    with_domain_self_hash(
        serde_json::json!({
            "schema": "s7_switch_stats.v1",
            "seed": seed,
            "artifact_path": format!("experiments/S7/switch-stats/seed-{seed}/switch-stats.json"),
            "aggregation_rule": "SUM",
            "temporal_switch_digest": temporal_switch_digest,
            "clip_saturation_digest": clip_saturation_digest,
            "expert_payload_digest": expert_payload_digest,
            "expert_slot_affinity": expert_slot_affinity,
            "bundle_self_hash": test_hash(160),
        }),
        "bundle_self_hash",
        S7_SWITCH_STATS_DOMAIN,
    )
}

fn router_collapse_sweep_support_artifact() -> Value {
    let lambdas = [0.0, 0.05, 0.1, 0.5, 1.0, 5.0];
    let bpc_eval_subset = [1.0, 1.01, 1.02, 1.1, 1.2, 1.5];
    let entropy_bits = [2.0, 1.9, 1.85, 1.7, 1.55, 1.4];
    let quality_delta = [-0.01, 0.0, 0.01, 0.09, 0.19, 0.49];
    let records = lambdas
        .iter()
        .enumerate()
        .map(|(index, lambda_switch)| {
            with_domain_self_hash(
                serde_json::json!({
                    "schema_version": {"major": 1, "minor": 0, "patch": 0},
                    "seed": 0,
                    "lambda_switch": lambda_switch,
                    "base_train_step": 20_000,
                    "train_step": 21_000,
                    "completion": {"kind": "completed"},
                    "bpc_eval_subset": bpc_eval_subset[index],
                    "expert_usage_entropy_bits_mean": entropy_bits[index],
                    "quality_delta_per_lambda_switch": quality_delta[index],
                    "sweep_self_hash": test_hash(170 + u8::try_from(index).expect("index fits u8")),
                }),
                "sweep_self_hash",
                S7_LAMBDA_SWITCH_RECORD_DOMAIN,
            )
        })
        .collect::<Vec<_>>();

    with_domain_self_hash(
        serde_json::json!({
            "schema": "s7_router_collapse_sweep.v1",
            "seed": 0,
            "base_checkpoint_sha": test_hash(176),
            "grid": lambdas,
            "records": records,
            "production_lambda": 0.05,
            "collapse_threshold": 1.0,
            "guardrail_verdict": "Pass",
            "sweep_self_hash": test_hash(177),
        }),
        "sweep_self_hash",
        S7_ROUTER_COLLAPSE_SWEEP_DOMAIN,
    )
}

fn frontier_support_artifact() -> Value {
    with_domain_self_hash(
        serde_json::json!({
            "schema": "s7_frontier.v1",
            "points": [
                {
                    "topology": "MoeTiny",
                    "checkpoint_sha": test_hash(101),
                    "quality": {
                        "median_val_bpc": 1.0,
                        "per_seed_val_bpc": [1.0, 1.125, 1.25, 1.375, 1.5],
                    },
                    "conformance": {"status": "ok"},
                    "projected_fit": {
                        "deployed_bytes_total": 100,
                        "deployed_bytes_per_block": [25, 25, 25, 25],
                    },
                    "schedule_cost": null,
                },
                {
                    "topology": "MoeTinyDenseMatched",
                    "checkpoint_sha": test_hash(102),
                    "quality": {
                        "median_val_bpc": 1.75,
                        "per_seed_val_bpc": [1.5, 1.625, 1.75, 1.875, 2.0],
                    },
                    "conformance": {"status": "ok"},
                    "projected_fit": {
                        "deployed_bytes_total": 100,
                        "deployed_bytes_per_block": [25, 25, 25, 25],
                    },
                    "schedule_cost": null,
                },
            ],
            "pareto_verdict": "MoE-dominates",
            "frontier_self_hash": test_hash(103),
        }),
        "frontier_self_hash",
        S7_FRONTIER_DOMAIN,
    )
}

fn burn_grad_support_artifact() -> Value {
    with_domain_self_hash(
        serde_json::json!({
            "schema": "s7_burn_grad_smoke.v1",
            "fixture_seed": 65261,
            "burn_adapter_version": "test",
            "fixture_input_sha": test_hash(104),
            "grad_up_weight_sum_abs": 1.0,
            "grad_down_weight_sum_abs": 1.25,
            "supported_clipped_activation_count": 3,
            "learned_activation_range_unsupported": true,
            "projection_biases_unsupported": true,
            "glu_construction_rejected": true,
            "replay_byte_identical": true,
            "smoke_self_hash": test_hash(105),
        }),
        "smoke_self_hash",
        S7_BURN_GRAD_SMOKE_DOMAIN,
    )
}

fn emulator_support_artifact(topology: &str) -> Value {
    with_domain_self_hash(
        serde_json::json!({
            "schema": "s7_emulator_one_token.v1",
            "seed": 0,
            "topology": topology,
            "encoded_rom_sha": test_hash(111),
            "prompt_sha": test_hash(112),
            "artifact_oracle_logits_sha": test_hash(113),
            "emulator_logits_sha": test_hash(114),
            "pairwise_max_abs_diff": 0.0,
            "s5_tolerance": 0.1,
            "observed_bank_switches_per_token": 0.25,
            "oracle_recorded_bank_switches": 0.75,
            "bank_switch_diff": 0.5,
            "bank_switch_within_one": true,
            "emulator_self_hash": test_hash(115),
        }),
        "emulator_self_hash",
        S7_EMULATOR_ONE_TOKEN_DOMAIN,
    )
}

fn write_minimal_report_packet(root: &Path) {
    let hash = format!("sha256:{}", "1".repeat(64));
    for topology in ["MoeTiny", "MoeTinyDenseMatched"] {
        for seed in 0..5 {
            write_json(
                root,
                &format!("experiments/S7/runs/{topology}/seed-{seed}/run-log.json"),
                &with_domain_self_hash(
                    serde_json::json!({
                        "schema": "s7_run_log.v1",
                        "seed": seed,
                        "topology": topology,
                        "completion": {"kind": "completed"},
                        "run_log_self_hash": hash.as_str(),
                    }),
                    "run_log_self_hash",
                    S7_RUN_LOG_DOMAIN,
                ),
            );
            let bpc = 1.0 + f64::from(seed) / 100.0;
            write_json(
                root,
                &format!("experiments/S7/scores/{topology}/seed-{seed}/score.json"),
                &with_domain_self_hash(
                    serde_json::json!({
                        "schema": "s7_score.v1",
                        "seed": seed,
                        "topology": topology,
                        "checkpoint_sha": hash.as_str(),
                        "corpus_val_sha": hash.as_str(),
                        "chunk_size": 256,
                        "token_count": 100,
                        "log2_sum": bpc * 100.0,
                        "bpc": bpc,
                        "score_self_hash": hash.as_str(),
                    }),
                    "score_self_hash",
                    S7_SCORE_DOMAIN,
                ),
            );
        }
    }
    for seed in 0..5 {
        write_json(
            root,
            &format!("experiments/S7/switch-stats/seed-{seed}/switch-stats.json"),
            &with_domain_self_hash(
                serde_json::json!({
                    "schema": "s7_switch_stats.v1",
                    "seed": seed,
                    "artifact_path": format!("seed-{seed}"),
                    "aggregation_rule": "SUM",
                    "bundle_self_hash": hash.as_str(),
                }),
                "bundle_self_hash",
                S7_SWITCH_STATS_DOMAIN,
            ),
        );
    }
    write_json(
        root,
        "experiments/S7/dense-vs-moe/comparison.json",
        &with_domain_self_hash(
            serde_json::json!({
                "schema": "s7_dense_vs_moe.v1",
                "matched_bytes_pin": {"matched_bytes_self_hash": hash.as_str()},
                "comparison_self_hash": hash.as_str(),
                "bytes_within_tolerance": true,
                "aggregate_parity_verdict": "Pass-clean",
                "per_seed": [
                    {"seed": 0, "parity_verdict": "Pass"},
                    {"seed": 1, "parity_verdict": "Pass"},
                    {"seed": 2, "parity_verdict": "Pass"},
                    {"seed": 3, "parity_verdict": "Pass"},
                    {"seed": 4, "parity_verdict": "Pass"}
                ],
            }),
            "comparison_self_hash",
            S7_DENSE_VS_MOE_DOMAIN,
        ),
    );
    write_json(
        root,
        "experiments/S7/router-collapse/seed-0/sweep.json",
        &with_domain_self_hash(
            serde_json::json!({
                "schema": "s7_router_collapse_sweep.v1",
                "seed": 0,
                "sweep_self_hash": hash.as_str(),
            }),
            "sweep_self_hash",
            S7_ROUTER_COLLAPSE_SWEEP_DOMAIN,
        ),
    );
    write_json(
        root,
        "experiments/S7/frontier/frontier.json",
        &with_domain_self_hash(
            serde_json::json!({
                "schema": "s7_frontier.v1",
                "frontier_self_hash": hash.as_str(),
            }),
            "frontier_self_hash",
            S7_FRONTIER_DOMAIN,
        ),
    );
    write_json(
        root,
        "experiments/S7/burn-grad-smoke/expert_block_qat.json",
        &with_domain_self_hash(
            serde_json::json!({
                "schema": "s7_burn_grad_smoke.v1",
                "smoke_self_hash": hash.as_str(),
            }),
            "smoke_self_hash",
            S7_BURN_GRAD_SMOKE_DOMAIN,
        ),
    );
    write_json(
        root,
        "experiments/S7/oracle-routed/seed-0/oracle.json",
        &with_domain_self_hash(
            serde_json::json!({
                "schema": "s7_oracle_routed.v1",
                "seed": 0,
                "topology": "MoeTiny",
                "oracle_self_hash": hash.as_str(),
            }),
            "oracle_self_hash",
            S7_ORACLE_ROUTED_DOMAIN,
        ),
    );
    write_json(
        root,
        "experiments/S7/emulator-one-token/seed-0/MoeTiny/result.json",
        &with_domain_self_hash(
            serde_json::json!({
                "schema": "s7_emulator_one_token.v1",
                "seed": 0,
                "topology": "MoeTiny",
                "emulator_self_hash": hash.as_str(),
            }),
            "emulator_self_hash",
            S7_EMULATOR_ONE_TOKEN_DOMAIN,
        ),
    );
}

fn write_json(root: &Path, rel_path: &str, payload: &Value) {
    let path = root.join(rel_path);
    std::fs::create_dir_all(path.parent().expect("artifact parent")).expect("artifact dir");
    let mut canonical = CanonicalJson::value_to_vec(payload).expect("canonical json payload");
    canonical.push(b'\n');
    std::fs::write(&path, canonical).expect("artifact writes");
}

fn single_stdout_hash(output: &Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines = stdout.lines().collect::<Vec<_>>();
    assert_eq!(
        lines.len(),
        1,
        "stdout must be one pipeable line:\n{stdout}"
    );
    let line = lines[0];
    assert!(
        line.strip_prefix("sha256:")
            .is_some_and(|hex| hex.len() == 64 && hex.chars().all(|ch| ch.is_ascii_hexdigit())),
        "stdout must be a sha256 self-hash line, got {line:?}"
    );
    line.to_owned()
}

fn cargo_target_dir() -> std::path::PathBuf {
    std::env::var_os("CARGO_TARGET_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| workspace_root().join("target"))
}

fn workspace_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
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
