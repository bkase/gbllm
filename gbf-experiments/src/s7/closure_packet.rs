//! S7 closure-packet adapter for the Rust closure validator.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use gbf_artifact::{S7Completion, S7Topology};
use gbf_foundation::{CanonicalJson, CanonicalJsonError, DomainHash, Hash256};
use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde_json::{Value, json};

use crate::s7::outcome::{S7Decision, S7Outcome};
use crate::s7::report::{
    S7_REQUIRED_CLOSURE_ARTIFACTS, S7ArtifactHashStatus, S7ClosureArtifactKind,
    S7ClosureArtifactStatus, S7ClosureGateStatus, S7ClosureValidationError,
    S7ClosureValidationInput, S7PerSeedClosureArtifacts, validate_s7_closure,
};

const S7_SWITCH_STATS_MANIFEST_DOMAIN: DomainHash<'static> = DomainHash::new(
    "gbf-experiments",
    "S7SwitchStatsBundleManifest",
    "s7_switch_stats_bundle_manifest.v1",
    "1",
);
const S7_SWITCH_STATS_DOMAIN: DomainHash<'static> = DomainHash::new(
    "gbf-experiments",
    "S7SwitchStatsReport",
    "s7_switch_stats.v1",
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
const S7_REQUIRED_REPORT_BODY_HEADINGS: &[&str] = &[
    "## Pre-registered predictions",
    "## Observed (per-seed, per-topology table)",
    "## Hypothesis verdicts",
    "## Falsification analysis",
    "## Switch statistics summary",
    "## lambda_switch sweep summary",
    "## Pareto verdict",
    "## Surprises",
    "## Decision",
    "## Reproducibility statement",
];
const S7_REQUIRED_HYPOTHESIS_VERDICTS: &[&str] =
    &["H1", "H2", "H3", "H4", "H5", "H6", "H7", "H8", "H9", "H10"];
const S7_REQUIRED_REPORT_HASH_FIELDS: &[&str] = &[
    "matched_bytes_self_hash",
    "switch_stats_self_hash",
    "router_collapse_sweep_self_hash",
    "dense_vs_moe_self_hash",
    "frontier_self_hash",
    "burn_grad_smoke_self_hash",
    "oracle_routed_self_hash",
    "emulator_one_token_moe_self_hash",
    "predictions_section_hash",
    "report_self_hash",
];

pub(crate) struct ValidateClosurePacketArgs {
    pub(crate) root: PathBuf,
    pub(crate) report: PathBuf,
    pub(crate) predictions_verified: bool,
}

pub(crate) fn validate_closure_packet(
    args: ValidateClosurePacketArgs,
) -> Result<Hash256, S7ClosurePacketError> {
    let report_path = resolve_path(&args.root, &args.report);
    let report_text =
        std::fs::read_to_string(&report_path).map_err(|source| S7ClosurePacketError::Io {
            path: report_path.display().to_string(),
            source,
        })?;
    let parsed = ParsedReport::parse(&report_text)?;
    let comparison = read_json(&args.root, "experiments/S7/dense-vs-moe/comparison.json")?;

    let report_self_hash = report_self_hash_status(&parsed, &report_text)?;
    let outcome = parsed.outcome()?;
    let per_seed_bpc_parity_failed = per_seed_bpc_parity_failed(&comparison)?;
    verified_self_hash(
        &comparison,
        "comparison_self_hash",
        S7_DENSE_VS_MOE_DOMAIN,
        "s7_dense_vs_moe",
        "s7_dense_vs_moe.v1",
    )?;
    validate_matched_bytes_hash(&parsed, &comparison)?;
    validate_outcome_comparison_alignment(outcome, &comparison, per_seed_bpc_parity_failed)?;
    let input = S7ClosureValidationInput {
        outcome,
        decision: parsed.decision()?,
        bytes_within_tolerance: bool_field(
            &comparison,
            &["bytes_within_tolerance"],
            "dense-vs-MoE bytes_within_tolerance",
        )?,
        per_seed_bpc_parity_failed,
        predictions_verified: args.predictions_verified,
        gates: parsed.gates(),
        per_seed_artifacts: per_seed_artifacts(&args.root, &parsed)?,
        required_artifacts: required_artifacts(&args.root, &parsed)?,
    };
    validate_s7_closure(&input).map_err(S7ClosurePacketError::Closure)?;
    Ok(report_self_hash)
}

fn per_seed_artifacts(
    root: &Path,
    parsed: &ParsedReport,
) -> Result<Vec<S7PerSeedClosureArtifacts>, S7ClosurePacketError> {
    parsed
        .rows
        .iter()
        .map(|row| {
            let seed = parse_seed(row)?;
            let topology = parse_topology(row_scalar(row, "topology")?)?;
            let topology_path = topology_path_segment(&topology);
            let actual_run = read_json(
                root,
                &format!("experiments/S7/runs/{topology_path}/seed-{seed}/run-log.json"),
            )?;
            validate_actual_artifact_identity(&actual_run, "run-log", &topology, seed)?;
            validate_actual_run_completed(&actual_run, topology_path, seed)?;
            let actual_run_hash = verified_self_hash(
                &actual_run,
                "run_log_self_hash",
                S7_RUN_LOG_DOMAIN,
                "s7_run_log",
                "s7_run_log.v1",
            )?;
            let actual_score = read_json(
                root,
                &format!("experiments/S7/scores/{topology_path}/seed-{seed}/score.json"),
            )?;
            validate_actual_artifact_identity(&actual_score, "score", &topology, seed)?;
            let actual_score_hash = verified_self_hash(
                &actual_score,
                "score_self_hash",
                S7_SCORE_DOMAIN,
                "s7_score",
                "s7_score.v1",
            )?;
            let actual_checkpoint_hash =
                hash_field(&actual_score, &["checkpoint_sha"], "checkpoint_sha")?;
            Ok(S7PerSeedClosureArtifacts {
                seed,
                topology,
                completion: parse_completion(row_scalar(row, "completion")?)?,
                checkpoint_self_hash: status_from_report_and_actual(
                    row.get("checkpoint_self_hash").and_then(Option::as_deref),
                    Some(actual_checkpoint_hash),
                    "checkpoint_self_hash",
                )?,
                run_log_self_hash: status_from_report_and_actual(
                    row.get("run_log_self_hash").and_then(Option::as_deref),
                    Some(actual_run_hash),
                    "run_log_self_hash",
                )?,
                score_self_hash: status_from_report_and_actual(
                    row.get("score_self_hash").and_then(Option::as_deref),
                    Some(actual_score_hash),
                    "score_self_hash",
                )?,
            })
        })
        .collect()
}

fn required_artifacts(
    root: &Path,
    parsed: &ParsedReport,
) -> Result<Vec<S7ClosureArtifactStatus>, S7ClosurePacketError> {
    let per_seed_rows = per_seed_artifacts(root, parsed)?;
    let mut artifacts = Vec::new();
    for kind in S7_REQUIRED_CLOSURE_ARTIFACTS {
        let status = match kind {
            S7ClosureArtifactKind::RunLog => aggregate_status(
                per_seed_rows
                    .iter()
                    .map(|row| row.run_log_self_hash)
                    .collect::<Vec<_>>()
                    .as_slice(),
            ),
            S7ClosureArtifactKind::Score => aggregate_status(
                per_seed_rows
                    .iter()
                    .map(|row| row.score_self_hash)
                    .collect::<Vec<_>>()
                    .as_slice(),
            ),
            S7ClosureArtifactKind::SwitchStats => switch_stats_status(root, parsed)?,
            S7ClosureArtifactKind::RouterCollapseSweep => top_level_verified_status(
                root,
                parsed,
                "router_collapse_sweep_self_hash",
                "experiments/S7/router-collapse/seed-0/sweep.json",
                "sweep_self_hash",
                S7_ROUTER_COLLAPSE_SWEEP_DOMAIN,
                "s7_router_collapse_sweep",
                "s7_router_collapse_sweep.v1",
            )?,
            S7ClosureArtifactKind::DenseVsMoe => top_level_status(
                root,
                parsed,
                "dense_vs_moe_self_hash",
                "experiments/S7/dense-vs-moe/comparison.json",
                &["comparison_self_hash"],
            )?,
            S7ClosureArtifactKind::Frontier => top_level_verified_status(
                root,
                parsed,
                "frontier_self_hash",
                "experiments/S7/frontier/frontier.json",
                "frontier_self_hash",
                S7_FRONTIER_DOMAIN,
                "s7_frontier",
                "s7_frontier.v1",
            )?,
            S7ClosureArtifactKind::BurnGradSmoke => top_level_verified_status(
                root,
                parsed,
                "burn_grad_smoke_self_hash",
                "experiments/S7/burn-grad-smoke/expert_block_qat.json",
                "smoke_self_hash",
                S7_BURN_GRAD_SMOKE_DOMAIN,
                "s7_burn_grad_smoke",
                "s7_burn_grad_smoke.v1",
            )?,
            S7ClosureArtifactKind::OracleRouted => top_level_verified_status(
                root,
                parsed,
                "oracle_routed_self_hash",
                "experiments/S7/oracle-routed/seed-0/oracle.json",
                "oracle_self_hash",
                S7_ORACLE_ROUTED_DOMAIN,
                "s7_oracle_routed",
                "s7_oracle_routed.v1",
            )?,
            S7ClosureArtifactKind::EmulatorOneTokenMoe => top_level_verified_status(
                root,
                parsed,
                "emulator_one_token_moe_self_hash",
                "experiments/S7/emulator-one-token/seed-0/MoeTiny/result.json",
                "emulator_self_hash",
                S7_EMULATOR_ONE_TOKEN_DOMAIN,
                "s7_emulator_one_token",
                "s7_emulator_one_token.v1",
            )?,
            S7ClosureArtifactKind::Report => S7ArtifactHashStatus::present_valid(
                report_self_hash_status(parsed, &parsed.raw_text)?,
            ),
            S7ClosureArtifactKind::EmulatorOneTokenDense => continue,
        };
        artifacts.push(S7ClosureArtifactStatus { kind, status });
    }

    if parsed.decision()? == S7Decision::ProceedToS8DenseOnly
        || parsed
            .scalar("emulator_one_token_dense_self_hash")
            .is_some()
    {
        artifacts.push(S7ClosureArtifactStatus {
            kind: S7ClosureArtifactKind::EmulatorOneTokenDense,
            status: top_level_verified_status(
                root,
                parsed,
                "emulator_one_token_dense_self_hash",
                "experiments/S7/emulator-one-token/seed-0/MoeTinyDenseMatched/result.json",
                "emulator_self_hash",
                S7_EMULATOR_ONE_TOKEN_DOMAIN,
                "s7_emulator_one_token",
                "s7_emulator_one_token.v1",
            )?,
        });
    }
    Ok(artifacts)
}

fn report_self_hash_status(
    parsed: &ParsedReport,
    text: &str,
) -> Result<Hash256, S7ClosurePacketError> {
    let observed = parsed.hash_scalar("report_self_hash")?;
    let normalized = normalize_report_for_hash(text)?;
    let expected = S7_REPORT_MARKDOWN_DOMAIN.hash_canonical_bytes(normalized.as_bytes())?;
    if observed != expected {
        return Err(S7ClosurePacketError::InvalidPacket(
            "invalid s7_report self-hash".to_owned(),
        ));
    }
    Ok(observed)
}

fn normalize_report_for_hash(text: &str) -> Result<String, S7ClosurePacketError> {
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
    if !report_hash_seen {
        return Err(S7ClosurePacketError::InvalidPacket(
            "report_self_hash line must be a top-level scalar".to_owned(),
        ));
    }
    Ok(normalized)
}

fn top_level_status(
    root: &Path,
    parsed: &ParsedReport,
    report_field: &'static str,
    rel_path: &str,
    json_path: &[&str],
) -> Result<S7ArtifactHashStatus, S7ClosurePacketError> {
    let actual = json_hash(root, rel_path, json_path)?;
    status_from_report_and_actual(parsed.scalar(report_field), Some(actual), report_field)
}

fn top_level_verified_status(
    root: &Path,
    parsed: &ParsedReport,
    report_field: &'static str,
    rel_path: &str,
    self_hash_field: &'static str,
    domain: DomainHash<'static>,
    label: &'static str,
    expected_schema: &'static str,
) -> Result<S7ArtifactHashStatus, S7ClosurePacketError> {
    let value = read_json(root, rel_path)?;
    let actual = verified_self_hash(&value, self_hash_field, domain, label, expected_schema)?;
    status_from_report_and_actual(parsed.scalar(report_field), Some(actual), report_field)
}

fn verified_self_hash(
    value: &Value,
    field: &'static str,
    domain: DomainHash<'static>,
    label: &'static str,
    expected_schema: &'static str,
) -> Result<Hash256, S7ClosurePacketError> {
    let schema = string_field(value, &["schema"], &format!("{label}.schema"))?;
    if schema != expected_schema {
        return Err(S7ClosurePacketError::InvalidPacket(format!(
            "{label} schema must be {expected_schema}, observed {schema:?}"
        )));
    }
    let observed = hash_field(value, &[field], field)?;
    let mut payload = value.clone();
    let Some(object) = payload.as_object_mut() else {
        return Err(S7ClosurePacketError::InvalidPacket(format!(
            "{label} must be a JSON object"
        )));
    };
    object.remove(field);
    let remaining = object
        .keys()
        .filter(|key| key.ends_with("_self_hash"))
        .cloned()
        .collect::<Vec<_>>();
    if !remaining.is_empty() {
        return Err(S7ClosurePacketError::InvalidPacket(format!(
            "{label} self-hash input leaves top-level self-hash fields: {}",
            remaining.join(", ")
        )));
    }
    let canonical = CanonicalJson::value_to_vec(&payload)?;
    let expected = domain.hash_canonical_bytes(&canonical)?;
    if observed != expected {
        return Err(S7ClosurePacketError::InvalidPacket(format!(
            "invalid {label} self-hash"
        )));
    }
    Ok(observed)
}

fn validate_matched_bytes_hash(
    parsed: &ParsedReport,
    comparison: &Value,
) -> Result<(), S7ClosurePacketError> {
    let actual = hash_field(
        comparison,
        &["matched_bytes_pin", "matched_bytes_self_hash"],
        "matched_bytes_self_hash",
    )?;
    let status = status_from_report_and_actual(
        parsed.scalar("matched_bytes_self_hash"),
        Some(actual),
        "matched_bytes_self_hash",
    )?;
    if !status.is_present() {
        return Err(S7ClosurePacketError::InvalidPacket(
            "missing matched_bytes_self_hash".to_owned(),
        ));
    }
    if !status.self_hash_valid {
        return Err(S7ClosurePacketError::InvalidPacket(
            "invalid matched_bytes_self_hash".to_owned(),
        ));
    }
    Ok(())
}

fn validate_outcome_comparison_alignment(
    outcome: S7Outcome,
    comparison: &Value,
    per_seed_bpc_parity_failed: bool,
) -> Result<(), S7ClosurePacketError> {
    let aggregate = string_field(
        comparison,
        &["aggregate_parity_verdict"],
        "aggregate_parity_verdict",
    )?;
    match outcome {
        S7Outcome::PassClean if aggregate != "Pass-clean" || per_seed_bpc_parity_failed => {
            Err(S7ClosurePacketError::InvalidPacket(
                "PassClean outcome conflicts with dense-vs-MoE parity verdict".to_owned(),
            ))
        }
        S7Outcome::FailParity if aggregate != "Fail-parity" || !per_seed_bpc_parity_failed => {
            Err(S7ClosurePacketError::InvalidPacket(
                "FailParity outcome requires dense-vs-MoE parity failure".to_owned(),
            ))
        }
        _ => Ok(()),
    }
}

fn switch_stats_status(
    root: &Path,
    parsed: &ParsedReport,
) -> Result<S7ArtifactHashStatus, S7ClosurePacketError> {
    let mut seed_bundle_self_hashes = Vec::new();
    for seed in 0..5 {
        let value = read_json(
            root,
            &format!("experiments/S7/switch-stats/seed-{seed}/switch-stats.json"),
        )?;
        let bundle_self_hash = verified_self_hash(
            &value,
            "bundle_self_hash",
            S7_SWITCH_STATS_DOMAIN,
            "s7_switch_stats_bundle",
            "s7_switch_stats.v1",
        )?;
        seed_bundle_self_hashes.push(json!({
            "seed": seed,
            "bundle_self_hash": bundle_self_hash,
        }));
    }
    let manifest = json!({
        "schema": "s7_switch_stats_bundle_manifest.v1",
        "seed_bundle_self_hashes": seed_bundle_self_hashes,
    });
    let canonical = CanonicalJson::value_to_vec(&manifest)?;
    let actual = S7_SWITCH_STATS_MANIFEST_DOMAIN.hash_canonical_bytes(&canonical)?;
    status_from_report_and_actual(
        parsed.scalar("switch_stats_self_hash"),
        Some(actual),
        "switch_stats_self_hash",
    )
}

fn aggregate_status(statuses: &[S7ArtifactHashStatus]) -> S7ArtifactHashStatus {
    let first = statuses.iter().find_map(|status| status.self_hash);
    if statuses
        .iter()
        .any(|status| !status.is_present() || first.is_none())
    {
        return S7ArtifactHashStatus::missing();
    }
    if statuses.iter().any(|status| !status.self_hash_valid) {
        return S7ArtifactHashStatus::present_invalid(first.expect("first hash exists"));
    }
    S7ArtifactHashStatus::present_valid(first.expect("first hash exists"))
}

fn per_seed_bpc_parity_failed(comparison: &Value) -> Result<bool, S7ClosurePacketError> {
    if string_field(
        comparison,
        &["aggregate_parity_verdict"],
        "aggregate_parity_verdict",
    )? == "Fail-parity"
    {
        return Ok(true);
    }
    let per_seed = comparison
        .get("per_seed")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            S7ClosurePacketError::InvalidPacket(
                "dense-vs-MoE comparison missing per_seed array".to_owned(),
            )
        })?;
    Ok(per_seed.iter().any(|entry| {
        entry
            .get("parity_verdict")
            .and_then(Value::as_str)
            .is_some_and(|verdict| verdict != "Pass")
    }))
}

fn validate_actual_artifact_identity(
    artifact: &Value,
    artifact_name: &str,
    expected_topology: &S7Topology,
    expected_seed: u64,
) -> Result<(), S7ClosurePacketError> {
    let expected_topology = topology_path_segment(expected_topology);
    let observed_seed = artifact.get("seed").and_then(Value::as_u64);
    if observed_seed != Some(expected_seed) {
        return Err(S7ClosurePacketError::InvalidPacket(format!(
            "{artifact_name} seed mismatch for {expected_topology} seed {expected_seed}"
        )));
    }
    let observed_topology = artifact.get("topology").and_then(Value::as_str);
    if observed_topology != Some(expected_topology) {
        return Err(S7ClosurePacketError::InvalidPacket(format!(
            "{artifact_name} topology mismatch for {expected_topology} seed {expected_seed}"
        )));
    }
    Ok(())
}

fn validate_actual_run_completed(
    run: &Value,
    topology_path: &str,
    seed: u64,
) -> Result<(), S7ClosurePacketError> {
    let Some(completion) = run.get("completion").and_then(Value::as_object) else {
        return Err(S7ClosurePacketError::InvalidPacket(format!(
            "{topology_path} seed {seed} run-log completion must be an object"
        )));
    };
    if completion.get("kind").and_then(Value::as_str) != Some("completed") || completion.len() != 1
    {
        return Err(S7ClosurePacketError::InvalidPacket(format!(
            "{topology_path} seed {seed} actual run-log completion is not completed"
        )));
    }
    Ok(())
}

fn parse_seed(row: &BTreeMap<String, Option<String>>) -> Result<u64, S7ClosurePacketError> {
    row_scalar(row, "seed")?
        .parse::<u64>()
        .map_err(|_| S7ClosurePacketError::InvalidPacket("invalid per-seed seed".to_owned()))
}

fn parse_topology(value: &str) -> Result<S7Topology, S7ClosurePacketError> {
    match value {
        "MoeTiny" => Ok(S7Topology::MoeTiny),
        "MoeTinyDenseMatched" => Ok(S7Topology::MoeTinyDenseMatched),
        _ => Err(S7ClosurePacketError::InvalidPacket(format!(
            "unknown S7 topology {value:?}"
        ))),
    }
}

fn topology_path_segment(topology: &S7Topology) -> &'static str {
    match topology {
        S7Topology::MoeTiny => "MoeTiny",
        S7Topology::MoeTinyDenseMatched => "MoeTinyDenseMatched",
    }
}

fn parse_completion(value: &str) -> Result<S7Completion, S7ClosurePacketError> {
    match value {
        "Completed" => Ok(S7Completion::Completed),
        _ => Err(S7ClosurePacketError::InvalidPacket(format!(
            "closure report completion must be Completed, observed {value:?}"
        ))),
    }
}

fn status_from_report_and_actual(
    report_hash: Option<&str>,
    actual_hash: Option<Hash256>,
    field: &'static str,
) -> Result<S7ArtifactHashStatus, S7ClosurePacketError> {
    let Some(report_hash) = report_hash else {
        return Ok(S7ArtifactHashStatus::missing());
    };
    let parsed = parse_hash(report_hash, field)?;
    Ok(if Some(parsed) == actual_hash {
        S7ArtifactHashStatus::present_valid(parsed)
    } else {
        S7ArtifactHashStatus::present_invalid(parsed)
    })
}

fn read_json(root: &Path, rel_path: &str) -> Result<Value, S7ClosurePacketError> {
    let path = root.join(rel_path);
    let text = std::fs::read_to_string(&path).map_err(|source| S7ClosurePacketError::Io {
        path: path.display().to_string(),
        source,
    })?;
    serde_json::from_str::<JsonDuplicateKeyGuard>(&text).map_err(|source| {
        S7ClosurePacketError::InvalidPacket(format!(
            "{} has duplicate JSON key or invalid JSON: {source}",
            path.display()
        ))
    })?;
    let value: Value =
        serde_json::from_str(&text).map_err(|source| S7ClosurePacketError::Json {
            path: path.display().to_string(),
            source,
        })?;
    let canonical = String::from_utf8(CanonicalJson::value_to_vec(&value)?).map_err(|error| {
        S7ClosurePacketError::InvalidPacket(format!(
            "{} canonical JSON bytes are not UTF-8: {error}",
            path.display()
        ))
    })?;
    if text != canonical && text != format!("{canonical}\n") {
        return Err(S7ClosurePacketError::InvalidPacket(format!(
            "{} must use canonical JSON bytes",
            path.display()
        )));
    }
    Ok(value)
}

struct JsonDuplicateKeyGuard;

impl<'de> serde::Deserialize<'de> for JsonDuplicateKeyGuard {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(JsonDuplicateKeyVisitor)
    }
}

struct JsonDuplicateKeyVisitor;

impl<'de> Visitor<'de> for JsonDuplicateKeyVisitor {
    type Value = JsonDuplicateKeyGuard;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("JSON without duplicate object keys")
    }

    fn visit_bool<E>(self, _value: bool) -> Result<Self::Value, E> {
        Ok(JsonDuplicateKeyGuard)
    }

    fn visit_i64<E>(self, _value: i64) -> Result<Self::Value, E> {
        Ok(JsonDuplicateKeyGuard)
    }

    fn visit_u64<E>(self, _value: u64) -> Result<Self::Value, E> {
        Ok(JsonDuplicateKeyGuard)
    }

    fn visit_f64<E>(self, _value: f64) -> Result<Self::Value, E> {
        Ok(JsonDuplicateKeyGuard)
    }

    fn visit_str<E>(self, _value: &str) -> Result<Self::Value, E> {
        Ok(JsonDuplicateKeyGuard)
    }

    fn visit_string<E>(self, _value: String) -> Result<Self::Value, E> {
        Ok(JsonDuplicateKeyGuard)
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(JsonDuplicateKeyGuard)
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(JsonDuplicateKeyGuard)
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(JsonDuplicateKeyVisitor)
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        while seq.next_element::<JsonDuplicateKeyGuard>()?.is_some() {}
        Ok(JsonDuplicateKeyGuard)
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut seen = BTreeSet::new();
        while let Some(key) = map.next_key::<String>()? {
            if !seen.insert(key.clone()) {
                return Err(de::Error::custom(format!("duplicate JSON key: {key}")));
            }
            map.next_value::<JsonDuplicateKeyGuard>()?;
        }
        Ok(JsonDuplicateKeyGuard)
    }
}

fn json_hash(root: &Path, rel_path: &str, path: &[&str]) -> Result<Hash256, S7ClosurePacketError> {
    let value = read_json(root, rel_path)?;
    hash_field(&value, path, &path.join("."))
}

fn hash_field(value: &Value, path: &[&str], label: &str) -> Result<Hash256, S7ClosurePacketError> {
    parse_hash(string_field(value, path, label)?, label)
}

fn bool_field(value: &Value, path: &[&str], label: &str) -> Result<bool, S7ClosurePacketError> {
    let Some(field) = json_path(value, path).and_then(Value::as_bool) else {
        return Err(S7ClosurePacketError::InvalidPacket(format!(
            "{label} must be a boolean"
        )));
    };
    Ok(field)
}

fn string_field<'a>(
    value: &'a Value,
    path: &[&str],
    label: &str,
) -> Result<&'a str, S7ClosurePacketError> {
    json_path(value, path)
        .and_then(Value::as_str)
        .ok_or_else(|| S7ClosurePacketError::InvalidPacket(format!("{label} must be a string")))
}

fn json_path<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    Some(current)
}

fn row_scalar<'a>(
    row: &'a BTreeMap<String, Option<String>>,
    field: &'static str,
) -> Result<&'a str, S7ClosurePacketError> {
    row.get(field)
        .and_then(Option::as_deref)
        .ok_or_else(|| S7ClosurePacketError::InvalidPacket(format!("missing per-seed {field}")))
}

fn parse_hash(value: &str, field: &str) -> Result<Hash256, S7ClosurePacketError> {
    Hash256::from_str(value)
        .map_err(|_| S7ClosurePacketError::InvalidPacket(format!("{field} must be a sha256 hash")))
}

fn resolve_path(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

#[derive(Debug)]
struct ParsedReport {
    raw_text: String,
    scalars: BTreeMap<String, Option<String>>,
    rows: Vec<BTreeMap<String, Option<String>>>,
    body: String,
}

impl ParsedReport {
    fn parse(text: &str) -> Result<Self, S7ClosurePacketError> {
        let Some(rest) = text.strip_prefix("---\n") else {
            return Err(S7ClosurePacketError::InvalidPacket(
                "report must start with front matter".to_owned(),
            ));
        };
        let Some((front_matter, body)) = rest.split_once("\n---\n") else {
            return Err(S7ClosurePacketError::InvalidPacket(
                "report front matter closing delimiter not found".to_owned(),
            ));
        };
        let (scalars, rows) = parse_front_matter(front_matter)?;
        validate_report_scalars(&scalars)?;
        validate_report_body(body)?;
        validate_report_rows(&rows)?;
        Ok(Self {
            raw_text: text.to_owned(),
            scalars,
            rows,
            body: body.to_owned(),
        })
    }

    fn scalar(&self, field: &'static str) -> Option<&str> {
        self.scalars.get(field).and_then(Option::as_deref)
    }

    fn required_scalar(&self, field: &'static str) -> Result<&str, S7ClosurePacketError> {
        self.scalar(field)
            .ok_or_else(|| S7ClosurePacketError::InvalidPacket(format!("missing {field}")))
    }

    fn hash_scalar(&self, field: &'static str) -> Result<Hash256, S7ClosurePacketError> {
        parse_hash(self.required_scalar(field)?, field)
    }

    fn outcome(&self) -> Result<S7Outcome, S7ClosurePacketError> {
        match self.required_scalar("s7_outcome")? {
            "PassClean" => Ok(S7Outcome::PassClean),
            "FailMoeTrain" => Ok(S7Outcome::FailMoeTrain),
            "FailRouterCollapse" => Ok(S7Outcome::FailRouterCollapse),
            "FailDenseBaseline" => Ok(S7Outcome::FailDenseBaseline),
            "FailParity" => Ok(S7Outcome::FailParity),
            "FailBytes" => Ok(S7Outcome::FailBytes),
            "FailPareto" => Ok(S7Outcome::FailPareto),
            "FailSwitchStats" => Ok(S7Outcome::FailSwitchStats),
            "FailRouterCollapseGuardrail" => Ok(S7Outcome::FailRouterCollapseGuardrail),
            "FailGradProvenance" => Ok(S7Outcome::FailGradProvenance),
            "FailBurnGrad" => Ok(S7Outcome::FailBurnGrad),
            "FailOracleRouted" => Ok(S7Outcome::FailOracleRouted),
            "FailEmulatorRouted" => Ok(S7Outcome::FailEmulatorRouted),
            "FailSuspicious" => Ok(S7Outcome::FailSuspicious),
            value => Err(S7ClosurePacketError::InvalidPacket(format!(
                "unknown s7_outcome {value:?}"
            ))),
        }
    }

    fn decision(&self) -> Result<S7Decision, S7ClosurePacketError> {
        match self.required_scalar("decision")? {
            "ProceedToS8" => Ok(S7Decision::ProceedToS8),
            "ProceedToS8DenseOnly" => Ok(S7Decision::ProceedToS8DenseOnly),
            "Halt" => Ok(S7Decision::Halt {
                reason: "report-halt",
            }),
            "Investigate" => Ok(S7Decision::Investigate {
                reason: "report-investigate",
            }),
            value => Err(S7ClosurePacketError::InvalidPacket(format!(
                "unknown S7 decision {value:?}"
            ))),
        }
    }

    fn gates(&self) -> S7ClosureGateStatus {
        S7ClosureGateStatus {
            h5_switch_stats_confirmed: self.gate_confirmed("H5"),
            h6_router_collapse_guardrail_confirmed: self.gate_confirmed("H6"),
            h7_loss_gradient_provenance_confirmed: self.gate_confirmed("H7"),
            h8_burn_gradient_confirmed: self.gate_confirmed("H8"),
            h9_oracle_routed_confirmed: self.gate_confirmed("H9"),
            h10_emulator_routed_confirmed: self.gate_confirmed("H10"),
        }
    }

    fn gate_confirmed(&self, gate: &str) -> bool {
        self.body.lines().any(|line| {
            let trimmed = line.trim_start();
            trimmed.starts_with(gate) && trimmed.contains("Confirmed")
        })
    }
}

fn validate_report_scalars(
    scalars: &BTreeMap<String, Option<String>>,
) -> Result<(), S7ClosurePacketError> {
    let schema = scalar_from_map(scalars, "schema");
    if schema != Some("s7_report.v1") {
        return Err(S7ClosurePacketError::InvalidPacket(format!(
            "schema must be \"s7_report.v1\", observed {schema:?}"
        )));
    }

    let decision = scalar_from_map(scalars, "decision");
    if !matches!(decision, Some("ProceedToS8" | "ProceedToS8DenseOnly")) {
        return Err(S7ClosurePacketError::InvalidPacket(
            "decision must be ProceedToS8 or ProceedToS8DenseOnly for bd-2v9r closure".to_owned(),
        ));
    }
    let outcome = scalar_from_map(scalars, "s7_outcome");
    if decision == Some("ProceedToS8DenseOnly") && outcome != Some("FailParity") {
        return Err(S7ClosurePacketError::InvalidPacket(
            "ProceedToS8DenseOnly is permitted only when s7_outcome is FailParity".to_owned(),
        ));
    }

    for field in S7_REQUIRED_REPORT_HASH_FIELDS {
        require_report_hash(scalars, field)?;
    }
    if decision == Some("ProceedToS8DenseOnly") {
        require_report_hash(scalars, "emulator_one_token_dense_self_hash")?;
    } else if let Some(value) = scalar_from_map(scalars, "emulator_one_token_dense_self_hash") {
        validate_report_hash_literal(value, "emulator_one_token_dense_self_hash")?;
    }

    for field in ["predictions_commit", "first_result_commit"] {
        let value = required_scalar_from_map(scalars, field)?;
        if !is_git_commit_id(value) {
            return Err(S7ClosurePacketError::InvalidPacket(format!(
                "{field} must be a 40-hex git commit id"
            )));
        }
    }
    let rfc_revision = required_scalar_from_map(scalars, "rfc_revision")?;
    if !is_git_commit_id(rfc_revision) && !is_report_hash_literal(rfc_revision) {
        return Err(S7ClosurePacketError::InvalidPacket(
            "rfc_revision must be a git commit id or sha256 hash".to_owned(),
        ));
    }
    Ok(())
}

fn require_report_hash(
    scalars: &BTreeMap<String, Option<String>>,
    field: &'static str,
) -> Result<(), S7ClosurePacketError> {
    validate_report_hash_literal(required_scalar_from_map(scalars, field)?, field)
}

fn validate_report_hash_literal(
    value: &str,
    field: &'static str,
) -> Result<(), S7ClosurePacketError> {
    if !is_report_hash_literal(value) {
        return Err(S7ClosurePacketError::InvalidPacket(format!(
            "{field} must be a non-null sha256 hash"
        )));
    }
    Ok(())
}

fn is_report_hash_literal(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(|hex| hex.len() == 64 && hex.chars().all(is_lower_hex_digit))
}

fn scalar_from_map<'a>(
    scalars: &'a BTreeMap<String, Option<String>>,
    field: &str,
) -> Option<&'a str> {
    scalars.get(field).and_then(Option::as_deref)
}

fn required_scalar_from_map<'a>(
    scalars: &'a BTreeMap<String, Option<String>>,
    field: &'static str,
) -> Result<&'a str, S7ClosurePacketError> {
    scalar_from_map(scalars, field)
        .ok_or_else(|| S7ClosurePacketError::InvalidPacket(format!("missing {field}")))
}

fn is_git_commit_id(value: &str) -> bool {
    value.len() == 40 && value.chars().all(is_lower_hex_digit)
}

fn is_lower_hex_digit(ch: char) -> bool {
    ch.is_ascii_digit() || matches!(ch, 'a'..='f')
}

fn validate_report_body(body: &str) -> Result<(), S7ClosurePacketError> {
    for heading in S7_REQUIRED_REPORT_BODY_HEADINGS {
        if !body.contains(heading) {
            return Err(S7ClosurePacketError::InvalidPacket(format!(
                "missing body heading: {heading}"
            )));
        }
    }
    for hypothesis in S7_REQUIRED_HYPOTHESIS_VERDICTS {
        if !contains_body_token(body, hypothesis) {
            return Err(S7ClosurePacketError::InvalidPacket(format!(
                "missing explicit {hypothesis} hypothesis verdict"
            )));
        }
    }
    if body.contains("NotEvaluatedDueToPriorGate") {
        return Err(S7ClosurePacketError::InvalidPacket(
            "closure-candidate reports must not use NotEvaluatedDueToPriorGate".to_owned(),
        ));
    }
    Ok(())
}

fn validate_report_rows(
    rows: &[BTreeMap<String, Option<String>>],
) -> Result<(), S7ClosurePacketError> {
    if rows.len() != 10 {
        return Err(S7ClosurePacketError::InvalidPacket(format!(
            "per_seed_artifacts must contain 10 rows, observed {}",
            rows.len()
        )));
    }

    let mut observed = BTreeSet::new();
    for row in rows {
        let seed = parse_seed(row)?;
        let topology = parse_topology(row_scalar(row, "topology")?)?;
        let topology_name = topology_path_segment(&topology);
        if !observed.insert((topology_name, seed)) {
            return Err(S7ClosurePacketError::InvalidPacket(format!(
                "duplicate per_seed_artifacts row for {topology_name} seed {seed}"
            )));
        }
        parse_completion(row_scalar(row, "completion")?)?;
        for field in [
            "checkpoint_self_hash",
            "run_log_self_hash",
            "score_self_hash",
        ] {
            parse_hash(row_scalar(row, field)?, field)?;
        }
    }

    for seed in 0..5 {
        for topology_name in ["MoeTiny", "MoeTinyDenseMatched"] {
            if !observed.contains(&(topology_name, seed)) {
                return Err(S7ClosurePacketError::InvalidPacket(format!(
                    "per_seed_artifacts missing row for {topology_name} seed {seed}"
                )));
            }
        }
    }
    Ok(())
}

fn contains_body_token(body: &str, needle: &str) -> bool {
    body.match_indices(needle).any(|(index, _)| {
        let left = body[..index].chars().next_back();
        let right = body[index + needle.len()..].chars().next();
        left.is_none_or(|ch| !ch.is_ascii_alphanumeric())
            && right.is_none_or(|ch| !ch.is_ascii_alphanumeric())
    })
}

fn parse_front_matter(
    front_matter: &str,
) -> Result<
    (
        BTreeMap<String, Option<String>>,
        Vec<BTreeMap<String, Option<String>>>,
    ),
    S7ClosurePacketError,
> {
    let mut scalars = BTreeMap::new();
    let mut rows = Vec::new();
    let mut current: Option<BTreeMap<String, Option<String>>> = None;
    let mut in_per_seed = false;

    for (line_no, raw_line) in front_matter.lines().enumerate() {
        let line_no = line_no + 1;
        if raw_line.trim().is_empty() || raw_line.trim_start().starts_with('#') {
            continue;
        }
        if raw_line.contains('\t') {
            return Err(S7ClosurePacketError::InvalidPacket(format!(
                "unsupported tab indentation in report front matter line {line_no}"
            )));
        }
        if has_yaml_anchor_or_alias(raw_line) {
            return Err(S7ClosurePacketError::InvalidPacket(format!(
                "unsupported YAML anchor/alias in report front matter line {line_no}"
            )));
        }
        if !raw_line.starts_with(' ') {
            if let Some(row) = current.take() {
                rows.push(row);
            }
            in_per_seed = false;
            let (key, value) = parse_key_value(raw_line, line_no)?;
            if key == "per_seed_artifacts" {
                if value.is_some() {
                    return Err(S7ClosurePacketError::InvalidPacket(
                        "per_seed_artifacts must use block list form".to_owned(),
                    ));
                }
                in_per_seed = true;
            } else if scalars.insert(key.clone(), value).is_some() {
                return Err(S7ClosurePacketError::InvalidPacket(format!(
                    "duplicate front matter field {key:?}"
                )));
            }
            continue;
        }
        if !in_per_seed {
            return Err(S7ClosurePacketError::InvalidPacket(format!(
                "unexpected nested front matter line {line_no}"
            )));
        }
        let trimmed = raw_line.trim();
        if let Some(inline) = trimmed.strip_prefix("- ") {
            if let Some(row) = current.take() {
                rows.push(row);
            }
            let mut row = BTreeMap::new();
            if !inline.is_empty() {
                let (key, value) = parse_key_value(inline, line_no)?;
                row.insert(key, value);
            }
            current = Some(row);
            continue;
        }
        let Some(row) = current.as_mut() else {
            return Err(S7ClosurePacketError::InvalidPacket(format!(
                "per_seed_artifacts field before first row in line {line_no}"
            )));
        };
        let (key, value) = parse_key_value(trimmed, line_no)?;
        if row.insert(key.clone(), value).is_some() {
            return Err(S7ClosurePacketError::InvalidPacket(format!(
                "duplicate per_seed_artifacts field {key:?}"
            )));
        }
    }
    if let Some(row) = current.take() {
        rows.push(row);
    }
    Ok((scalars, rows))
}

fn parse_key_value(
    text: &str,
    line_no: usize,
) -> Result<(String, Option<String>), S7ClosurePacketError> {
    let Some((key, raw_value)) = text.split_once(':') else {
        return Err(S7ClosurePacketError::InvalidPacket(format!(
            "invalid front matter key/value syntax in line {line_no}"
        )));
    };
    let key = key.trim();
    if key.is_empty()
        || !key
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        return Err(S7ClosurePacketError::InvalidPacket(format!(
            "invalid front matter key in line {line_no}"
        )));
    }
    let raw_value = raw_value.trim();
    if matches!(raw_value, "|" | ">") {
        return Err(S7ClosurePacketError::InvalidPacket(format!(
            "unsupported YAML block scalar for {key:?} in line {line_no}"
        )));
    }
    if raw_value.starts_with('[') || raw_value.starts_with('{') {
        return Err(S7ClosurePacketError::InvalidPacket(format!(
            "unsupported YAML flow collection for {key:?} in line {line_no}"
        )));
    }
    let value = clean_scalar(raw_value);
    Ok((key.to_owned(), value))
}

fn has_yaml_anchor_or_alias(line: &str) -> bool {
    let mut previous: Option<char> = None;
    let mut chars = line.char_indices().peekable();
    while let Some((_, ch)) = chars.next() {
        let yaml_token_boundary = match previous {
            Some(prev) => prev == ':' || prev.is_whitespace(),
            None => true,
        };
        if (ch == '*' || ch == '&') && yaml_token_boundary {
            if chars.peek().is_some_and(|(_, next)| {
                next.is_ascii_alphanumeric() || *next == '_' || *next == '-'
            }) {
                return true;
            }
        }
        previous = Some(ch);
    }
    false
}

fn clean_scalar(value: &str) -> Option<String> {
    if matches!(value, "" | "null" | "Null" | "NULL" | "~") {
        return None;
    }
    if let Some(stripped) = value
        .strip_prefix('"')
        .and_then(|raw| raw.strip_suffix('"'))
    {
        return Some(stripped.to_owned());
    }
    if let Some(stripped) = value
        .strip_prefix('\'')
        .and_then(|raw| raw.strip_suffix('\''))
    {
        return Some(stripped.to_owned());
    }
    Some(value.to_owned())
}

/// Errors emitted while adapting an F-S7 report packet into Rust closure input.
#[derive(Debug)]
pub enum S7ClosurePacketError {
    /// Filesystem operation failed.
    Io {
        /// Path being read.
        path: String,
        /// I/O source.
        source: std::io::Error,
    },
    /// JSON parsing failed.
    Json {
        /// Path being parsed.
        path: String,
        /// JSON source.
        source: serde_json::Error,
    },
    /// Packet shape was invalid.
    InvalidPacket(String),
    /// Canonical JSON or domain hash computation failed.
    CanonicalJson(CanonicalJsonError),
    /// Rust closure contract rejected the packet.
    Closure(S7ClosureValidationError),
}

impl fmt::Display for S7ClosurePacketError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "{path}: {source}"),
            Self::Json { path, source } => write!(f, "{path}: {source}"),
            Self::InvalidPacket(error) => f.write_str(error),
            Self::CanonicalJson(error) => write!(f, "{error}"),
            Self::Closure(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for S7ClosurePacketError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Json { source, .. } => Some(source),
            Self::InvalidPacket(_) => None,
            Self::CanonicalJson(error) => Some(error),
            Self::Closure(error) => Some(error),
        }
    }
}

impl From<CanonicalJsonError> for S7ClosurePacketError {
    fn from(error: CanonicalJsonError) -> Self {
        Self::CanonicalJson(error)
    }
}
