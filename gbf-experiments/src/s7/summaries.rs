//! S7 comparison-summary derivation from landed production artifacts.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::PathBuf;

use gbf_artifact::{
    ClipSaturationDigest, ExpertPayloadDigest, ExpertSlotAffinity, GuardrailVerdict, LambdaSwitch,
    S7SchemaError, SweepSummary, SwitchStatsSummary, TemporalSwitchDigest,
};
use gbf_foundation::{
    CanonicalJson, CanonicalJsonError, DomainHash, Hash256, LayerId, self_hash_omitting_fields,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

use crate::s7::schema::RouterStepTelemetry;

const S7_N_SEEDS: u64 = 5;
const S7_N_BLOCKS: usize = 4;
const S7_SWITCH_STATS_SCHEMA: &str = "s7_switch_stats.v1";
const S7_ROUTER_COLLAPSE_SWEEP_SCHEMA: &str = "s7_router_collapse_sweep.v1";
const S7_ROUTER_COLLAPSE_SWEEP_DOMAIN: DomainHash<'static> = DomainHash::new(
    "gbf-experiments",
    "RouterCollapseSweepReport",
    S7_ROUTER_COLLAPSE_SWEEP_SCHEMA,
    "1",
);
const S7_LAMBDA_SWITCH_RECORD_DOMAIN: DomainHash<'static> = DomainHash::new(
    "gbf-experiments",
    "LambdaSwitchSweepRecord",
    "s7_lambda_switch_sweep_step.v1",
    "1",
);
const S7_SWITCH_STATS_DOMAIN: DomainHash<'static> = DomainHash::new(
    "gbf-experiments",
    "S7SwitchStatsReport",
    S7_SWITCH_STATS_SCHEMA,
    "1",
);
const S7_DERIVED_SUMMARIES_DOMAIN: DomainHash<'static> = DomainHash::new(
    "gbf-experiments",
    "S7DerivedSummaries",
    "s7_derived_summaries.v1",
    "1",
);
const D11_FLOAT_TOLERANCE: f64 = 5.0e-9;

/// Inputs for deriving the comparison summary JSONs required by
/// `s7_dense_vs_moe.v1`.
#[derive(Debug, Clone)]
pub struct S7SummaryArtifactInputs {
    /// Packet/repository root containing materialized S7 artifacts.
    pub root: PathBuf,
    /// Output path for `SwitchStatsSummary`, relative to `root` unless absolute.
    pub switch_stats_output: PathBuf,
    /// Output path for `SweepSummary`, relative to `root` unless absolute.
    pub sweep_output: PathBuf,
}

/// Paths and bundle hash emitted by `materialize_s7_summaries`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S7MaterializedSummaryArtifacts {
    /// Path to the materialized switch-stat summary JSON.
    pub switch_stats_summary_path: PathBuf,
    /// Path to the materialized router-collapse sweep summary JSON.
    pub sweep_summary_path: PathBuf,
    /// Hash over the two canonical summary payloads.
    pub summary_bundle_hash: Hash256,
}

/// Derive and materialize the S7 comparison-summary inputs from landed
/// switch-stat, router-telemetry, and collapse-sweep artifacts.
pub fn materialize_s7_summaries(
    inputs: &S7SummaryArtifactInputs,
) -> Result<S7MaterializedSummaryArtifacts, S7SummaryMaterializeError> {
    let switch_reports = read_switch_stats_reports(&inputs.root)?;
    let router_telemetry = read_router_telemetry(&inputs.root)?;
    let sweep_path = inputs
        .root
        .join("experiments/S7/router-collapse/seed-0/sweep.json");
    let sweep_report = read_json_file::<Value>(&sweep_path)?;
    validate_sweep_report(&sweep_path, &sweep_report)?;

    let switch_summary = derive_switch_stats_summary(&switch_reports, &router_telemetry)?;
    let sweep_summary = derive_sweep_summary(&sweep_report)?;
    let switch_stats_summary_path = resolve_path(&inputs.root, &inputs.switch_stats_output);
    let sweep_summary_path = resolve_path(&inputs.root, &inputs.sweep_output);

    write_canonical_json(&switch_stats_summary_path, &switch_summary)?;
    write_canonical_json(&sweep_summary_path, &sweep_summary)?;
    let summary_bundle_hash = summary_bundle_hash(&switch_summary, &sweep_summary)?;

    Ok(S7MaterializedSummaryArtifacts {
        switch_stats_summary_path,
        sweep_summary_path,
        summary_bundle_hash,
    })
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SwitchStatsReport {
    schema: String,
    seed: u64,
    artifact_path: String,
    temporal_switch_digest: Vec<TemporalSwitchDigest>,
    clip_saturation_digest: Vec<ClipSaturationDigest>,
    expert_payload_digest: Vec<ExpertPayloadDigest>,
    expert_slot_affinity: Vec<ExpertSlotAffinity>,
    aggregation_rule: String,
    bundle_self_hash: Hash256,
}

impl SwitchStatsReport {
    fn validate_for_seed(&self, expected_seed: u64) -> Result<(), S7SummaryMaterializeError> {
        if self.schema != S7_SWITCH_STATS_SCHEMA {
            return Err(S7SummaryMaterializeError::InvalidSwitchStatsSchema {
                observed: self.schema.clone(),
            });
        }
        if self.seed != expected_seed {
            return Err(S7SummaryMaterializeError::SwitchStatsSeedMismatch {
                expected: expected_seed,
                observed: self.seed,
            });
        }
        if self.artifact_path.is_empty() {
            return Err(S7SummaryMaterializeError::EmptyArtifactPath {
                seed: expected_seed,
            });
        }
        if self.aggregation_rule != "SUM" {
            return Err(S7SummaryMaterializeError::InvalidAggregationRule {
                seed: expected_seed,
                observed: self.aggregation_rule.clone(),
            });
        }

        validate_temporal_digest_layers(expected_seed, &self.temporal_switch_digest)?;
        validate_clip_digest_layers(expected_seed, &self.clip_saturation_digest)?;
        validate_payload_digest_layers(expected_seed, &self.expert_payload_digest)?;
        validate_affinity_layers(expected_seed, &self.expert_slot_affinity)?;

        let expected =
            self_hash_omitting_fields(S7_SWITCH_STATS_DOMAIN, self, "bundle_self_hash", &[])?;
        if self.bundle_self_hash != expected {
            return Err(S7SummaryMaterializeError::SwitchStatsSelfHashMismatch {
                seed: expected_seed,
                expected,
                observed: self.bundle_self_hash,
            });
        }
        Ok(())
    }
}

fn derive_switch_stats_summary(
    reports: &[SwitchStatsReport],
    telemetry: &[RouterStepTelemetry],
) -> Result<SwitchStatsSummary, S7SummaryMaterializeError> {
    if reports.len() != usize::try_from(S7_N_SEEDS).expect("seed count fits usize") {
        return Err(S7SummaryMaterializeError::WrongSwitchStatsSeedCount {
            observed: reports.len(),
            expected: S7_N_SEEDS,
        });
    }
    if telemetry.is_empty() {
        return Err(S7SummaryMaterializeError::EmptyRouterTelemetry);
    }

    let mut rate_sums = [0_u64; S7_N_BLOCKS];
    for report in reports {
        for (index, digest) in report.temporal_switch_digest.iter().enumerate() {
            rate_sums[index] += u64::from(digest.same_expert_rate_q8_8);
        }
    }
    let same_expert_rate_per_layer_q8_8 = rate_sums
        .into_iter()
        .map(|sum| {
            let rounded = (sum + (S7_N_SEEDS / 2)) / S7_N_SEEDS;
            u16::try_from(rounded).map_err(|_| S7SummaryMaterializeError::LengthOverflow)
        })
        .collect::<Result<Vec<_>, _>>()?;

    let entropy_sum = telemetry
        .iter()
        .map(|record| record.expert_usage_entropy_bits)
        .map(f64::from)
        .sum::<f64>();
    let bank_switch_sum = telemetry
        .iter()
        .map(|record| record.bank_switches_per_token)
        .map(f64::from)
        .sum::<f64>();
    let telemetry_count = telemetry.len() as f64;

    Ok(SwitchStatsSummary::new(
        same_expert_rate_per_layer_q8_8,
        (entropy_sum / telemetry_count) as f32,
        (bank_switch_sum / telemetry_count) as f32,
    )?)
}

fn derive_sweep_summary(report: &Value) -> Result<SweepSummary, S7SummaryMaterializeError> {
    let mut bpc_at_lambda = BTreeMap::new();
    let mut entropy_at_lambda = BTreeMap::new();

    for record in required_array(
        report,
        "experiments/S7/router-collapse/seed-0/sweep.json",
        "records",
    )? {
        let lambda_value = required_f64(record, "lambda_switch")?;
        let lambda_switch = LambdaSwitch::new(format_lambda_switch(lambda_value as f32))?;
        let bpc = required_bpc(record)?;
        let entropy = required_f64(record, "expert_usage_entropy_bits_mean")? as f32;
        bpc_at_lambda.insert(lambda_switch.clone(), bpc);
        entropy_at_lambda.insert(lambda_switch, entropy);
    }

    Ok(SweepSummary::new(
        bpc_at_lambda,
        entropy_at_lambda,
        artifact_guardrail_verdict(report.get("guardrail_verdict"))?,
    )?)
}

fn artifact_guardrail_verdict(
    verdict: Option<&Value>,
) -> Result<GuardrailVerdict, S7SummaryMaterializeError> {
    let legacy = verdict.and_then(Value::as_str);
    let tagged = verdict
        .and_then(Value::as_object)
        .and_then(|object| object.get("kind"))
        .and_then(Value::as_str);
    Ok(match legacy.or(tagged) {
        Some("Pass" | "pass") => GuardrailVerdict::Pass,
        Some("FailA" | "fail_a") => GuardrailVerdict::FailA,
        Some("FailB" | "fail_b") => GuardrailVerdict::FailB,
        Some("FailC" | "fail_c") => GuardrailVerdict::FailC,
        Some("FailD" | "fail_d") => GuardrailVerdict::FailD,
        observed => {
            return Err(S7SummaryMaterializeError::InvalidSweep {
                path: "experiments/S7/router-collapse/seed-0/sweep.json".to_owned(),
                message: format!("unsupported guardrail_verdict {observed:?}"),
            });
        }
    })
}

fn read_switch_stats_reports(
    root: &std::path::Path,
) -> Result<Vec<SwitchStatsReport>, S7SummaryMaterializeError> {
    (0..S7_N_SEEDS)
        .map(|seed| {
            let path = root.join(format!(
                "experiments/S7/switch-stats/seed-{seed}/switch-stats.json"
            ));
            let report = read_json_file::<SwitchStatsReport>(&path)?;
            report.validate_for_seed(seed)?;
            Ok(report)
        })
        .collect()
}

fn read_router_telemetry(
    root: &std::path::Path,
) -> Result<Vec<RouterStepTelemetry>, S7SummaryMaterializeError> {
    let mut all_records = Vec::new();
    for seed in 0..S7_N_SEEDS {
        let path = root.join(format!(
            "experiments/S7/runs/MoeTiny/seed-{seed}/router-step-telemetry.jsonl"
        ));
        let content =
            std::fs::read_to_string(&path).map_err(|source| S7SummaryMaterializeError::Io {
                path: path.display().to_string(),
                source,
            })?;
        let mut seed_records = Vec::new();
        for (line_index, line) in content.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let record = serde_json::from_str::<RouterStepTelemetry>(line).map_err(|source| {
                S7SummaryMaterializeError::Json {
                    path: format!("{}:{}", path.display(), line_index + 1),
                    source,
                }
            })?;
            if record.seed != seed {
                return Err(S7SummaryMaterializeError::RouterTelemetrySeedMismatch {
                    path: path.display().to_string(),
                    expected: seed,
                    observed: record.seed,
                });
            }
            seed_records.push(record);
        }
        validate_router_telemetry_layers(seed, &path, &seed_records)?;
        all_records.extend(seed_records);
    }
    Ok(all_records)
}

fn validate_router_telemetry_layers(
    seed: u64,
    path: &std::path::Path,
    records: &[RouterStepTelemetry],
) -> Result<(), S7SummaryMaterializeError> {
    if records.is_empty() {
        return Err(S7SummaryMaterializeError::EmptyRouterTelemetrySeed { seed });
    }
    let mut seen_layers = BTreeSet::new();
    for record in records {
        let layer_in_range = usize::try_from(record.layer_id)
            .map(|layer| layer < S7_N_BLOCKS)
            .unwrap_or(false);
        if !layer_in_range {
            return Err(S7SummaryMaterializeError::RouterTelemetryLayerOutOfRange {
                path: path.display().to_string(),
                layer_id: record.layer_id,
            });
        }
        seen_layers.insert(record.layer_id);
    }
    for layer in 0..S7_N_BLOCKS {
        let layer_id = u32::try_from(layer).expect("layer index fits u32");
        if !seen_layers.contains(&layer_id) {
            return Err(S7SummaryMaterializeError::MissingRouterTelemetryLayer { seed, layer_id });
        }
    }
    Ok(())
}

fn validate_sweep_report(
    path: &std::path::Path,
    value: &Value,
) -> Result<(), S7SummaryMaterializeError> {
    if required_str(value, "schema")? != S7_ROUTER_COLLAPSE_SWEEP_SCHEMA {
        return Err(S7SummaryMaterializeError::InvalidSweep {
            path: path.display().to_string(),
            message: format!("schema must be {S7_ROUTER_COLLAPSE_SWEEP_SCHEMA:?}"),
        });
    }
    require_u64_eq(path, value, "seed", 0)?;
    require_f64_eq(path, value, "production_lambda", 0.05)?;
    require_f64_eq(path, value, "collapse_threshold", 1.0)?;
    if artifact_guardrail_verdict(value.get("guardrail_verdict"))? != GuardrailVerdict::Pass {
        return Err(S7SummaryMaterializeError::InvalidSweep {
            path: path.display().to_string(),
            message: "guardrail_verdict must be Pass".to_owned(),
        });
    }

    let grid = required_array(value, &path.display().to_string(), "grid")?;
    const EXPECTED_GRID: [f64; 6] = [0.0, 0.05, 0.1, 0.5, 1.0, 5.0];
    if grid.len() != EXPECTED_GRID.len() {
        return Err(S7SummaryMaterializeError::InvalidSweep {
            path: path.display().to_string(),
            message: format!("grid must contain {} values", EXPECTED_GRID.len()),
        });
    }
    for (index, expected) in EXPECTED_GRID.iter().copied().enumerate() {
        let observed = value_f64(&grid[index], "grid[]")?;
        if !f64_close(observed, expected) {
            return Err(S7SummaryMaterializeError::InvalidSweep {
                path: path.display().to_string(),
                message: format!("grid[{index}] must be {expected}, observed {observed}"),
            });
        }
    }

    let records = required_array(value, &path.display().to_string(), "records")?;
    if records.len() != EXPECTED_GRID.len() {
        return Err(S7SummaryMaterializeError::InvalidSweep {
            path: path.display().to_string(),
            message: format!("records must contain {} values", EXPECTED_GRID.len()),
        });
    }
    for (index, record) in records.iter().enumerate() {
        validate_sweep_record(path, record, index, EXPECTED_GRID[index])?;
    }

    verify_value_self_hash(
        path,
        value,
        "sweep_self_hash",
        S7_ROUTER_COLLAPSE_SWEEP_DOMAIN,
    )?;
    Ok(())
}

fn validate_sweep_record(
    path: &std::path::Path,
    record: &Value,
    index: usize,
    expected_lambda: f64,
) -> Result<(), S7SummaryMaterializeError> {
    require_u64_eq(path, record, "seed", 0)?;
    let lambda = required_f64(record, "lambda_switch")?;
    if !f64_close(lambda, expected_lambda) {
        return Err(S7SummaryMaterializeError::InvalidSweep {
            path: path.display().to_string(),
            message: format!(
                "records[{index}].lambda_switch must be {expected_lambda}, observed {lambda}"
            ),
        });
    }
    let bpc = required_bpc(record)?;
    if bpc < 0.0 {
        return Err(S7SummaryMaterializeError::InvalidSweep {
            path: path.display().to_string(),
            message: format!("records[{index}].bpc_eval_subset must be non-negative"),
        });
    }
    let entropy = required_f64(record, "expert_usage_entropy_bits_mean")?;
    if entropy < 0.0 {
        return Err(S7SummaryMaterializeError::InvalidSweep {
            path: path.display().to_string(),
            message: format!(
                "records[{index}].expert_usage_entropy_bits_mean must be non-negative"
            ),
        });
    }
    verify_value_self_hash(
        path,
        record,
        "sweep_self_hash",
        S7_LAMBDA_SWITCH_RECORD_DOMAIN,
    )?;
    Ok(())
}

fn verify_value_self_hash(
    path: &std::path::Path,
    value: &Value,
    field: &'static str,
    domain: DomainHash<'static>,
) -> Result<(), S7SummaryMaterializeError> {
    let observed = required_hash(value, field)?;
    let expected = self_hash_omitting_fields(domain, value, field, &[])?;
    if observed != expected {
        return Err(S7SummaryMaterializeError::SweepSelfHashMismatch {
            path: path.display().to_string(),
            field,
            expected,
            observed,
        });
    }
    Ok(())
}

fn require_u64_eq(
    path: &std::path::Path,
    value: &Value,
    field: &'static str,
    expected: u64,
) -> Result<(), S7SummaryMaterializeError> {
    let observed = value.get(field).and_then(Value::as_u64).ok_or_else(|| {
        S7SummaryMaterializeError::InvalidSweep {
            path: path.display().to_string(),
            message: format!("{field} must be u64"),
        }
    })?;
    if observed != expected {
        return Err(S7SummaryMaterializeError::InvalidSweep {
            path: path.display().to_string(),
            message: format!("{field} must be {expected}, observed {observed}"),
        });
    }
    Ok(())
}

fn require_f64_eq(
    path: &std::path::Path,
    value: &Value,
    field: &'static str,
    expected: f64,
) -> Result<(), S7SummaryMaterializeError> {
    let observed = required_f64(value, field)?;
    if !f64_close(observed, expected) {
        return Err(S7SummaryMaterializeError::InvalidSweep {
            path: path.display().to_string(),
            message: format!("{field} must be {expected}, observed {observed}"),
        });
    }
    Ok(())
}

fn required_str<'a>(
    value: &'a Value,
    field: &'static str,
) -> Result<&'a str, S7SummaryMaterializeError> {
    value.get(field).and_then(Value::as_str).ok_or_else(|| {
        S7SummaryMaterializeError::InvalidSweep {
            path: "experiments/S7/router-collapse/seed-0/sweep.json".to_owned(),
            message: format!("{field} must be a string"),
        }
    })
}

fn required_array<'a>(
    value: &'a Value,
    path: &str,
    field: &'static str,
) -> Result<&'a Vec<Value>, S7SummaryMaterializeError> {
    value.get(field).and_then(Value::as_array).ok_or_else(|| {
        S7SummaryMaterializeError::InvalidSweep {
            path: path.to_owned(),
            message: format!("{field} must be an array"),
        }
    })
}

fn required_f64(value: &Value, field: &'static str) -> Result<f64, S7SummaryMaterializeError> {
    let observed = value
        .get(field)
        .ok_or_else(|| S7SummaryMaterializeError::InvalidSweep {
            path: "experiments/S7/router-collapse/seed-0/sweep.json".to_owned(),
            message: format!("{field} is missing"),
        })?;
    value_f64(observed, field)
}

fn required_bpc(value: &Value) -> Result<f64, S7SummaryMaterializeError> {
    let observed =
        value
            .get("bpc_eval_subset")
            .ok_or(S7SummaryMaterializeError::IncompleteSweepBpc {
                lambda_switch: value
                    .get("lambda_switch")
                    .and_then(Value::as_f64)
                    .unwrap_or(f64::NAN) as f32,
            })?;
    value_f64(observed, "bpc_eval_subset")
}

fn value_f64(value: &Value, field: &'static str) -> Result<f64, S7SummaryMaterializeError> {
    let observed = value
        .as_f64()
        .ok_or_else(|| S7SummaryMaterializeError::InvalidSweep {
            path: "experiments/S7/router-collapse/seed-0/sweep.json".to_owned(),
            message: format!("{field} must be a finite number"),
        })?;
    if !observed.is_finite() {
        return Err(S7SummaryMaterializeError::InvalidSweep {
            path: "experiments/S7/router-collapse/seed-0/sweep.json".to_owned(),
            message: format!("{field} must be finite"),
        });
    }
    Ok(observed)
}

fn required_hash(value: &Value, field: &'static str) -> Result<Hash256, S7SummaryMaterializeError> {
    let hash_value =
        value
            .get(field)
            .cloned()
            .ok_or_else(|| S7SummaryMaterializeError::InvalidSweep {
                path: "experiments/S7/router-collapse/seed-0/sweep.json".to_owned(),
                message: format!("{field} is missing"),
            })?;
    serde_json::from_value(hash_value).map_err(|source| S7SummaryMaterializeError::Json {
        path: format!("experiments/S7/router-collapse/seed-0/sweep.json:{field}"),
        source,
    })
}

fn f64_close(left: f64, right: f64) -> bool {
    (left - right).abs() <= D11_FLOAT_TOLERANCE
}

fn validate_temporal_digest_layers(
    seed: u64,
    digests: &[TemporalSwitchDigest],
) -> Result<(), S7SummaryMaterializeError> {
    if digests.len() != S7_N_BLOCKS {
        return Err(S7SummaryMaterializeError::WrongSwitchStatsDigestCount {
            seed,
            field: "temporal_switch_digest",
            observed: digests.len(),
            expected: S7_N_BLOCKS,
        });
    }
    for (layer, digest) in digests.iter().enumerate() {
        let expected_layer = expected_layer_id(layer);
        if digest.layer_id != expected_layer {
            return Err(S7SummaryMaterializeError::SwitchStatsLayerMismatch {
                seed,
                field: "temporal_switch_digest",
                expected: expected_layer,
                observed: digest.layer_id,
            });
        }
        verify_nested_hash(
            seed,
            "temporal_switch_digest",
            expected_layer,
            digest.computed_self_hash()?,
            digest.digest_self_hash,
        )?;
    }
    Ok(())
}

fn validate_clip_digest_layers(
    seed: u64,
    digests: &[ClipSaturationDigest],
) -> Result<(), S7SummaryMaterializeError> {
    if digests.len() != S7_N_BLOCKS {
        return Err(S7SummaryMaterializeError::WrongSwitchStatsDigestCount {
            seed,
            field: "clip_saturation_digest",
            observed: digests.len(),
            expected: S7_N_BLOCKS,
        });
    }
    for (layer, digest) in digests.iter().enumerate() {
        let expected_layer = expected_layer_id(layer);
        if digest.layer_id != expected_layer {
            return Err(S7SummaryMaterializeError::SwitchStatsLayerMismatch {
                seed,
                field: "clip_saturation_digest",
                expected: expected_layer,
                observed: digest.layer_id,
            });
        }
        verify_nested_hash(
            seed,
            "clip_saturation_digest",
            expected_layer,
            digest.computed_self_hash()?,
            digest.digest_self_hash,
        )?;
    }
    Ok(())
}

fn validate_payload_digest_layers(
    seed: u64,
    digests: &[ExpertPayloadDigest],
) -> Result<(), S7SummaryMaterializeError> {
    if digests.len() != S7_N_BLOCKS {
        return Err(S7SummaryMaterializeError::WrongSwitchStatsDigestCount {
            seed,
            field: "expert_payload_digest",
            observed: digests.len(),
            expected: S7_N_BLOCKS,
        });
    }
    for (layer, digest) in digests.iter().enumerate() {
        let expected_layer = expected_layer_id(layer);
        if digest.layer_id != expected_layer {
            return Err(S7SummaryMaterializeError::SwitchStatsLayerMismatch {
                seed,
                field: "expert_payload_digest",
                expected: expected_layer,
                observed: digest.layer_id,
            });
        }
        verify_nested_hash(
            seed,
            "expert_payload_digest",
            expected_layer,
            digest.computed_self_hash()?,
            digest.digest_self_hash,
        )?;
    }
    Ok(())
}

fn validate_affinity_layers(
    seed: u64,
    affinities: &[ExpertSlotAffinity],
) -> Result<(), S7SummaryMaterializeError> {
    if affinities.len() != S7_N_BLOCKS {
        return Err(S7SummaryMaterializeError::WrongSwitchStatsDigestCount {
            seed,
            field: "expert_slot_affinity",
            observed: affinities.len(),
            expected: S7_N_BLOCKS,
        });
    }
    for (layer, affinity) in affinities.iter().enumerate() {
        let expected_layer = expected_layer_id(layer);
        if affinity.layer_id != expected_layer {
            return Err(S7SummaryMaterializeError::SwitchStatsLayerMismatch {
                seed,
                field: "expert_slot_affinity",
                expected: expected_layer,
                observed: affinity.layer_id,
            });
        }
        verify_nested_hash(
            seed,
            "expert_slot_affinity",
            expected_layer,
            affinity.computed_self_hash()?,
            affinity.affinity_self_hash,
        )?;
    }
    Ok(())
}

fn verify_nested_hash(
    seed: u64,
    field: &'static str,
    layer_id: LayerId,
    expected: Hash256,
    observed: Hash256,
) -> Result<(), S7SummaryMaterializeError> {
    if expected != observed {
        return Err(S7SummaryMaterializeError::NestedSelfHashMismatch {
            seed,
            field,
            layer_id,
            expected,
            observed,
        });
    }
    Ok(())
}

fn expected_layer_id(layer: usize) -> LayerId {
    LayerId::new(u16::try_from(layer).expect("S7 layer index fits u16"))
}

fn format_lambda_switch(value: f32) -> String {
    const GRID: [(f32, &str); 6] = [
        (0.0, "0.0"),
        (0.05, "0.05"),
        (0.1, "0.1"),
        (0.5, "0.5"),
        (1.0, "1.0"),
        (5.0, "5.0"),
    ];
    GRID.iter()
        .find(|(grid_value, _)| value.to_bits() == grid_value.to_bits())
        .map_or_else(|| value.to_string(), |(_, text)| (*text).to_owned())
}

fn summary_bundle_hash(
    switch_summary: &SwitchStatsSummary,
    sweep_summary: &SweepSummary,
) -> Result<Hash256, S7SummaryMaterializeError> {
    let payload = json!({
        "switch_stats_summary": serde_json::to_value(switch_summary)?,
        "sweep_summary": serde_json::to_value(sweep_summary)?,
    });
    let bytes = CanonicalJson::value_to_vec(&payload)?;
    Ok(S7_DERIVED_SUMMARIES_DOMAIN.hash_canonical_bytes(&bytes)?)
}

fn read_json_file<T: DeserializeOwned>(
    path: &std::path::Path,
) -> Result<T, S7SummaryMaterializeError> {
    let content =
        std::fs::read_to_string(path).map_err(|source| S7SummaryMaterializeError::Io {
            path: path.display().to_string(),
            source,
        })?;
    serde_json::from_str(&content).map_err(|source| S7SummaryMaterializeError::Json {
        path: path.display().to_string(),
        source,
    })
}

fn write_canonical_json<T: Serialize>(
    path: &std::path::Path,
    value: &T,
) -> Result<(), S7SummaryMaterializeError> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(|source| S7SummaryMaterializeError::Io {
            path: parent.display().to_string(),
            source,
        })?;
    }
    let mut bytes = CanonicalJson::to_vec(value)?;
    bytes.push(b'\n');
    std::fs::write(path, bytes).map_err(|source| S7SummaryMaterializeError::Io {
        path: path.display().to_string(),
        source,
    })?;
    Ok(())
}

fn resolve_path(root: &std::path::Path, path: &std::path::Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

/// Errors raised while deriving S7 comparison summaries.
#[derive(Debug)]
pub enum S7SummaryMaterializeError {
    /// Filesystem operation failed.
    Io {
        /// Path being read or written.
        path: String,
        /// Source I/O error.
        source: std::io::Error,
    },
    /// JSON decoding failed.
    Json {
        /// Path being parsed.
        path: String,
        /// Source JSON error.
        source: serde_json::Error,
    },
    /// Canonical JSON encoding failed.
    CanonicalJson(CanonicalJsonError),
    /// S7 artifact schema validation failed.
    Schema(S7SchemaError),
    /// Router-collapse sweep artifact failed summary-input validation.
    InvalidSweep {
        /// Path being validated.
        path: String,
        /// Validation failure.
        message: String,
    },
    /// Router-collapse sweep self-hash did not verify.
    SweepSelfHashMismatch {
        /// Path being validated.
        path: String,
        /// Self-hash field name.
        field: &'static str,
        /// Expected hash.
        expected: Hash256,
        /// Observed hash.
        observed: Hash256,
    },
    /// Switch-stat report used an unexpected schema literal.
    InvalidSwitchStatsSchema {
        /// Observed schema literal.
        observed: String,
    },
    /// Switch-stat report path was empty.
    EmptyArtifactPath {
        /// Expected seed.
        seed: u64,
    },
    /// Switch-stat report seed did not match the packet path.
    SwitchStatsSeedMismatch {
        /// Expected seed.
        expected: u64,
        /// Observed seed.
        observed: u64,
    },
    /// Switch-stat aggregation rule was not SUM.
    InvalidAggregationRule {
        /// Expected seed.
        seed: u64,
        /// Observed rule.
        observed: String,
    },
    /// Switch-stat digest list had the wrong S7 block count.
    WrongSwitchStatsDigestCount {
        /// Expected seed.
        seed: u64,
        /// Digest field name.
        field: &'static str,
        /// Observed digest count.
        observed: usize,
        /// Expected digest count.
        expected: usize,
    },
    /// Switch-stat report count did not cover all five S7 seeds.
    WrongSwitchStatsSeedCount {
        /// Observed report count.
        observed: usize,
        /// Expected report count.
        expected: u64,
    },
    /// Switch-stat digest layer order was not the canonical S7 order.
    SwitchStatsLayerMismatch {
        /// Expected seed.
        seed: u64,
        /// Digest field name.
        field: &'static str,
        /// Expected layer.
        expected: LayerId,
        /// Observed layer.
        observed: LayerId,
    },
    /// A nested digest self-hash did not verify.
    NestedSelfHashMismatch {
        /// Expected seed.
        seed: u64,
        /// Digest field name.
        field: &'static str,
        /// Layer id for the digest.
        layer_id: LayerId,
        /// Expected hash.
        expected: Hash256,
        /// Observed hash.
        observed: Hash256,
    },
    /// The switch-stat report bundle self-hash did not verify.
    SwitchStatsSelfHashMismatch {
        /// Expected seed.
        seed: u64,
        /// Expected hash.
        expected: Hash256,
        /// Observed hash.
        observed: Hash256,
    },
    /// No router telemetry records were found.
    EmptyRouterTelemetry,
    /// A seed-specific router telemetry JSONL was empty.
    EmptyRouterTelemetrySeed {
        /// Expected seed.
        seed: u64,
    },
    /// Router telemetry seed did not match the packet path.
    RouterTelemetrySeedMismatch {
        /// Path being parsed.
        path: String,
        /// Expected seed.
        expected: u64,
        /// Observed seed.
        observed: u64,
    },
    /// Router telemetry layer was outside the S7 layer set.
    RouterTelemetryLayerOutOfRange {
        /// Path being parsed.
        path: String,
        /// Observed layer id.
        layer_id: u32,
    },
    /// Router telemetry for one S7 layer was missing.
    MissingRouterTelemetryLayer {
        /// Expected seed.
        seed: u64,
        /// Missing layer id.
        layer_id: u32,
    },
    /// Router-collapse sweep omitted BPC for a lambda needed by the summary.
    IncompleteSweepBpc {
        /// Lambda-switch value missing BPC.
        lambda_switch: f32,
    },
    /// Integer conversion overflowed.
    LengthOverflow,
}

impl fmt::Display for S7SummaryMaterializeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "{path}: {source}"),
            Self::Json { path, source } => write!(f, "{path}: {source}"),
            Self::CanonicalJson(error) => write!(f, "{error}"),
            Self::Schema(error) => write!(f, "{error}"),
            Self::InvalidSweep { path, message } => {
                write!(f, "{path}: invalid S7 router-collapse sweep: {message}")
            }
            Self::SweepSelfHashMismatch {
                path,
                field,
                expected,
                observed,
            } => write!(
                f,
                "{path}: {field} self-hash mismatch: expected {expected}, observed {observed}"
            ),
            Self::InvalidSwitchStatsSchema { observed } => {
                write!(f, "invalid S7 switch-stats schema {observed:?}")
            }
            Self::EmptyArtifactPath { seed } => {
                write!(f, "S7 switch-stats seed {seed} has empty artifact_path")
            }
            Self::SwitchStatsSeedMismatch { expected, observed } => write!(
                f,
                "S7 switch-stats seed mismatch: expected {expected}, observed {observed}"
            ),
            Self::InvalidAggregationRule { seed, observed } => write!(
                f,
                "S7 switch-stats seed {seed} uses aggregation_rule {observed:?}; expected SUM"
            ),
            Self::WrongSwitchStatsDigestCount {
                seed,
                field,
                observed,
                expected,
            } => write!(
                f,
                "S7 switch-stats seed {seed} field {field} has {observed} digests; expected {expected}"
            ),
            Self::WrongSwitchStatsSeedCount { observed, expected } => write!(
                f,
                "S7 switch-stats summary saw {observed} seed reports; expected {expected}"
            ),
            Self::SwitchStatsLayerMismatch {
                seed,
                field,
                expected,
                observed,
            } => write!(
                f,
                "S7 switch-stats seed {seed} field {field} layer mismatch: expected {expected}, observed {observed}"
            ),
            Self::NestedSelfHashMismatch {
                seed,
                field,
                layer_id,
                expected,
                observed,
            } => write!(
                f,
                "S7 switch-stats seed {seed} field {field} layer {layer_id} self-hash mismatch: expected {expected}, observed {observed}"
            ),
            Self::SwitchStatsSelfHashMismatch {
                seed,
                expected,
                observed,
            } => write!(
                f,
                "S7 switch-stats seed {seed} bundle self-hash mismatch: expected {expected}, observed {observed}"
            ),
            Self::EmptyRouterTelemetry => f.write_str("S7 router telemetry summary input is empty"),
            Self::EmptyRouterTelemetrySeed { seed } => {
                write!(f, "S7 router telemetry seed {seed} JSONL is empty")
            }
            Self::RouterTelemetrySeedMismatch {
                path,
                expected,
                observed,
            } => write!(
                f,
                "S7 router telemetry seed mismatch in {path}: expected {expected}, observed {observed}"
            ),
            Self::RouterTelemetryLayerOutOfRange { path, layer_id } => write!(
                f,
                "S7 router telemetry layer {layer_id} in {path} is outside the 4-layer S7 topology"
            ),
            Self::MissingRouterTelemetryLayer { seed, layer_id } => write!(
                f,
                "S7 router telemetry seed {seed} is missing layer {layer_id}"
            ),
            Self::IncompleteSweepBpc { lambda_switch } => write!(
                f,
                "S7 router-collapse sweep lambda_switch={lambda_switch} has no BPC; cannot derive comparison summary"
            ),
            Self::LengthOverflow => f.write_str("S7 summary integer conversion overflowed"),
        }
    }
}

impl std::error::Error for S7SummaryMaterializeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Json { source, .. } => Some(source),
            Self::CanonicalJson(error) => Some(error),
            Self::Schema(error) => Some(error),
            Self::InvalidSweep { .. }
            | Self::SweepSelfHashMismatch { .. }
            | Self::InvalidSwitchStatsSchema { .. }
            | Self::EmptyArtifactPath { .. }
            | Self::SwitchStatsSeedMismatch { .. }
            | Self::InvalidAggregationRule { .. }
            | Self::WrongSwitchStatsDigestCount { .. }
            | Self::WrongSwitchStatsSeedCount { .. }
            | Self::SwitchStatsLayerMismatch { .. }
            | Self::NestedSelfHashMismatch { .. }
            | Self::SwitchStatsSelfHashMismatch { .. }
            | Self::EmptyRouterTelemetry
            | Self::EmptyRouterTelemetrySeed { .. }
            | Self::RouterTelemetrySeedMismatch { .. }
            | Self::RouterTelemetryLayerOutOfRange { .. }
            | Self::MissingRouterTelemetryLayer { .. }
            | Self::IncompleteSweepBpc { .. }
            | Self::LengthOverflow => None,
        }
    }
}

impl From<CanonicalJsonError> for S7SummaryMaterializeError {
    fn from(error: CanonicalJsonError) -> Self {
        Self::CanonicalJson(error)
    }
}

impl From<S7SchemaError> for S7SummaryMaterializeError {
    fn from(error: S7SchemaError) -> Self {
        Self::Schema(error)
    }
}

impl From<serde_json::Error> for S7SummaryMaterializeError {
    fn from(source: serde_json::Error) -> Self {
        Self::Json {
            path: "<generated summary bundle>".to_owned(),
            source,
        }
    }
}
