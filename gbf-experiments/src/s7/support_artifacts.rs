//! S7 closure support artifact materialization.
//!
//! This module is an artifact landing pad for evidence produced by the real
//! S7 closure runners. It validates canonical JSON, domain self-hashes, and
//! closure-critical invariants before copying the evidence into the packet
//! layout consumed by the final report and verifier.

use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use gbf_artifact::S7Topology;
use gbf_foundation::{CanonicalJson, CanonicalJsonError, DomainHash, Hash256};
use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde_json::Value;

const S7_N_BLOCKS: usize = 4;
const S7_N_EXPERTS: u64 = 4;
const S7_SEED_COUNT: usize = 5;
const RCS_TRAINING_EXTRA_STEPS: u64 = 1_000;
const D11_LAMBDA_SWITCH_GRID: [f64; 6] = [0.0, 0.05, 0.1, 0.5, 1.0, 5.0];
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
const S7_LAMBDA_SWITCH_RECORD_DOMAIN: DomainHash<'static> = DomainHash::new(
    "gbf-experiments",
    "LambdaSwitchSweepRecord",
    "s7_lambda_switch_sweep_step.v1",
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
const PARETO_VERDICTS: &[&str] = &[
    "MoE-dominates",
    "dense-dominates",
    "MoE-wins-under-byte-equivalence",
    "Dense-wins-under-byte-equivalence",
    "Incomparable",
    "Tied",
];

/// Leaf support artifact kinds that can be landed into an S7 closure packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum S7SupportArtifactKind {
    /// `experiments/S7/switch-stats/seed-{seed}/switch-stats.json`.
    SwitchStats,
    /// `experiments/S7/router-collapse/seed-0/sweep.json`.
    RouterCollapseSweep,
    /// `experiments/S7/frontier/frontier.json`.
    Frontier,
    /// `experiments/S7/burn-grad-smoke/expert_block_qat.json`.
    BurnGradSmoke,
    /// `experiments/S7/oracle-routed/seed-0/oracle.json`.
    OracleRouted,
    /// `experiments/S7/emulator-one-token/seed-0/{topology}/result.json`.
    EmulatorOneToken,
}

impl S7SupportArtifactKind {
    /// Stable CLI spelling for this support artifact kind.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SwitchStats => "switch-stats",
            Self::RouterCollapseSweep => "router-collapse-sweep",
            Self::Frontier => "frontier",
            Self::BurnGradSmoke => "burn-grad-smoke",
            Self::OracleRouted => "oracle-routed",
            Self::EmulatorOneToken => "emulator-one-token",
        }
    }

    const fn spec(self) -> SupportArtifactSpec {
        match self {
            Self::SwitchStats => SupportArtifactSpec {
                schema: "s7_switch_stats.v1",
                self_hash_field: "bundle_self_hash",
                domain: S7_SWITCH_STATS_DOMAIN,
            },
            Self::RouterCollapseSweep => SupportArtifactSpec {
                schema: "s7_router_collapse_sweep.v1",
                self_hash_field: "sweep_self_hash",
                domain: S7_ROUTER_COLLAPSE_SWEEP_DOMAIN,
            },
            Self::Frontier => SupportArtifactSpec {
                schema: "s7_frontier.v1",
                self_hash_field: "frontier_self_hash",
                domain: S7_FRONTIER_DOMAIN,
            },
            Self::BurnGradSmoke => SupportArtifactSpec {
                schema: "s7_burn_grad_smoke.v1",
                self_hash_field: "smoke_self_hash",
                domain: S7_BURN_GRAD_SMOKE_DOMAIN,
            },
            Self::OracleRouted => SupportArtifactSpec {
                schema: "s7_oracle_routed.v1",
                self_hash_field: "oracle_self_hash",
                domain: S7_ORACLE_ROUTED_DOMAIN,
            },
            Self::EmulatorOneToken => SupportArtifactSpec {
                schema: "s7_emulator_one_token.v1",
                self_hash_field: "emulator_self_hash",
                domain: S7_EMULATOR_ONE_TOKEN_DOMAIN,
            },
        }
    }
}

impl FromStr for S7SupportArtifactKind {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "switch-stats" => Ok(Self::SwitchStats),
            "router-collapse-sweep" => Ok(Self::RouterCollapseSweep),
            "frontier" => Ok(Self::Frontier),
            "burn-grad-smoke" => Ok(Self::BurnGradSmoke),
            "oracle-routed" => Ok(Self::OracleRouted),
            "emulator-one-token" => Ok(Self::EmulatorOneToken),
            _ => Err(
                "expected switch-stats, router-collapse-sweep, frontier, burn-grad-smoke, oracle-routed, or emulator-one-token"
                    .to_owned(),
            ),
        }
    }
}

/// Inputs for landing one support artifact into the S7 packet layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S7SupportArtifactInputs {
    /// Packet/repository root where `experiments/S7/...` should be written.
    pub root: PathBuf,
    /// Support artifact kind to validate and write.
    pub kind: S7SupportArtifactKind,
    /// Externally produced support artifact JSON.
    pub input: PathBuf,
    /// Topology for `emulator-one-token` artifacts; omitted for other kinds.
    pub topology: Option<S7Topology>,
    /// Seed for per-seed artifacts such as `switch-stats`; omitted for other kinds.
    pub seed: Option<u64>,
    /// Override output path, relative to `root` unless absolute.
    pub output: Option<PathBuf>,
}

/// Materialized support artifact output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S7MaterializedSupportArtifact {
    /// Canonical packet path written.
    pub output_path: PathBuf,
    /// Verified support artifact self-hash.
    pub self_hash: Hash256,
}

#[derive(Debug, Clone, Copy)]
struct SupportArtifactSpec {
    schema: &'static str,
    self_hash_field: &'static str,
    domain: DomainHash<'static>,
}

/// Validate and materialize one externally produced S7 support artifact.
pub fn materialize_support_artifact(
    inputs: &S7SupportArtifactInputs,
) -> Result<S7MaterializedSupportArtifact, S7SupportArtifactMaterializeError> {
    let input_path = resolve_under_root(&inputs.root, &inputs.input);
    let value = read_canonical_json(&input_path)?;
    let spec = inputs.kind.spec();
    require_string(&value, &["schema"], "schema").and_then(|schema| {
        if schema == spec.schema {
            Ok(())
        } else {
            Err(invalid(
                &input_path,
                format!("schema must be {}, observed {schema:?}", spec.schema),
            ))
        }
    })?;
    let self_hash = verified_self_hash(&input_path, &value, spec)?;
    validate_kind_specific(&input_path, &value, inputs)?;

    let output = if let Some(output) = &inputs.output {
        resolve_under_root(&inputs.root, output)
    } else {
        default_output_path(
            &inputs.root,
            inputs.kind,
            inputs.topology.clone(),
            inputs.seed,
        )?
    };
    write_canonical_json(&output, &value)?;

    Ok(S7MaterializedSupportArtifact {
        output_path: output,
        self_hash,
    })
}

fn default_output_path(
    root: &Path,
    kind: S7SupportArtifactKind,
    topology: Option<S7Topology>,
    seed: Option<u64>,
) -> Result<PathBuf, S7SupportArtifactMaterializeError> {
    let rel = match kind {
        S7SupportArtifactKind::SwitchStats => {
            let seed = require_seed(kind, seed)?;
            PathBuf::from("experiments/S7/switch-stats")
                .join(format!("seed-{seed}"))
                .join("switch-stats.json")
        }
        S7SupportArtifactKind::RouterCollapseSweep => {
            PathBuf::from("experiments/S7/router-collapse/seed-0/sweep.json")
        }
        S7SupportArtifactKind::Frontier => PathBuf::from("experiments/S7/frontier/frontier.json"),
        S7SupportArtifactKind::BurnGradSmoke => {
            PathBuf::from("experiments/S7/burn-grad-smoke/expert_block_qat.json")
        }
        S7SupportArtifactKind::OracleRouted => {
            PathBuf::from("experiments/S7/oracle-routed/seed-0/oracle.json")
        }
        S7SupportArtifactKind::EmulatorOneToken => {
            let Some(topology) = topology else {
                return Err(S7SupportArtifactMaterializeError::MissingTopology {
                    kind: kind.as_str(),
                });
            };
            PathBuf::from("experiments/S7/emulator-one-token/seed-0")
                .join(topology_path_segment(&topology))
                .join("result.json")
        }
    };
    Ok(root.join(rel))
}

fn validate_kind_specific(
    path: &Path,
    value: &Value,
    inputs: &S7SupportArtifactInputs,
) -> Result<(), S7SupportArtifactMaterializeError> {
    match inputs.kind {
        S7SupportArtifactKind::SwitchStats => {
            reject_topology_for_non_emulator(inputs.kind, inputs.topology.clone())?;
            let seed = require_seed(inputs.kind, inputs.seed)?;
            validate_switch_stats(path, value, seed)
        }
        S7SupportArtifactKind::RouterCollapseSweep => {
            reject_seed_for_non_switch(inputs.kind, inputs.seed)?;
            reject_topology_for_non_emulator(inputs.kind, inputs.topology.clone())?;
            validate_router_collapse_sweep(path, value)
        }
        S7SupportArtifactKind::Frontier => {
            reject_seed_for_non_switch(inputs.kind, inputs.seed)?;
            reject_topology_for_non_emulator(inputs.kind, inputs.topology.clone())?;
            validate_frontier(path, value)
        }
        S7SupportArtifactKind::BurnGradSmoke => {
            reject_seed_for_non_switch(inputs.kind, inputs.seed)?;
            reject_topology_for_non_emulator(inputs.kind, inputs.topology.clone())?;
            validate_burn_grad(path, value)
        }
        S7SupportArtifactKind::OracleRouted => {
            reject_seed_for_non_switch(inputs.kind, inputs.seed)?;
            reject_topology_for_non_emulator(inputs.kind, inputs.topology.clone())?;
            validate_oracle(path, value)
        }
        S7SupportArtifactKind::EmulatorOneToken => {
            reject_seed_for_non_switch(inputs.kind, inputs.seed)?;
            let Some(topology) = inputs.topology.clone() else {
                return Err(S7SupportArtifactMaterializeError::MissingTopology {
                    kind: inputs.kind.as_str(),
                });
            };
            validate_emulator(path, value, &topology)
        }
    }
}

fn require_seed(
    kind: S7SupportArtifactKind,
    seed: Option<u64>,
) -> Result<u64, S7SupportArtifactMaterializeError> {
    let Some(seed) = seed else {
        return Err(S7SupportArtifactMaterializeError::MissingSeed {
            kind: kind.as_str(),
        });
    };
    if seed >= S7_SEED_COUNT as u64 {
        return Err(S7SupportArtifactMaterializeError::InvalidSeed { seed });
    }
    Ok(seed)
}

fn reject_seed_for_non_switch(
    kind: S7SupportArtifactKind,
    seed: Option<u64>,
) -> Result<(), S7SupportArtifactMaterializeError> {
    if seed.is_some() {
        return Err(S7SupportArtifactMaterializeError::UnexpectedSeed {
            kind: kind.as_str(),
        });
    }
    Ok(())
}

fn reject_topology_for_non_emulator(
    kind: S7SupportArtifactKind,
    topology: Option<S7Topology>,
) -> Result<(), S7SupportArtifactMaterializeError> {
    if topology.is_some() {
        return Err(S7SupportArtifactMaterializeError::UnexpectedTopology {
            kind: kind.as_str(),
        });
    }
    Ok(())
}

fn validate_switch_stats(
    path: &Path,
    value: &Value,
    seed: u64,
) -> Result<(), S7SupportArtifactMaterializeError> {
    require_u64_eq(value, &["seed"], "seed", seed, path)?;
    require_non_empty_string(value, &["artifact_path"], "artifact_path", path)?;
    require_string_eq(
        value,
        &["aggregation_rule"],
        "aggregation_rule",
        "SUM",
        path,
    )?;

    for field in [
        "temporal_switch_digest",
        "clip_saturation_digest",
        "expert_payload_digest",
        "expert_slot_affinity",
    ] {
        let entries = require_array(value, &[field], field)?;
        if entries.len() != S7_N_BLOCKS {
            return Err(invalid(
                path,
                format!("{field} must contain {S7_N_BLOCKS} layer entries"),
            ));
        }
        for (layer_id, entry) in entries.iter().enumerate() {
            require_object(entry, &[], field)?;
            require_u64_eq(entry, &["layer_id"], "layer_id", layer_id as u64, path)?;
            match field {
                "temporal_switch_digest" => validate_temporal_switch_digest(path, entry, layer_id)?,
                "clip_saturation_digest" => validate_clip_saturation_digest(path, entry, layer_id)?,
                "expert_payload_digest" => validate_expert_payload_digest(path, entry, layer_id)?,
                "expert_slot_affinity" => validate_expert_slot_affinity(path, entry, layer_id)?,
                _ => unreachable!("field list is exhaustive"),
            }
        }
    }
    Ok(())
}

fn validate_router_collapse_sweep(
    path: &Path,
    value: &Value,
) -> Result<(), S7SupportArtifactMaterializeError> {
    require_u64_eq(value, &["seed"], "seed", 0, path)?;
    require_nonzero_hash(value, &["base_checkpoint_sha"], "base_checkpoint_sha", path)?;
    let grid = require_array(value, &["grid"], "grid")?;
    if grid.len() != D11_LAMBDA_SWITCH_GRID.len() {
        return Err(invalid(
            path,
            format!(
                "grid must contain {} lambda_switch values",
                D11_LAMBDA_SWITCH_GRID.len()
            ),
        ));
    }
    for (index, expected) in D11_LAMBDA_SWITCH_GRID.iter().enumerate() {
        let observed = finite_nonnegative_value(&grid[index], "grid", path)?;
        if !f64_close(observed, *expected) {
            return Err(invalid(
                path,
                format!("grid[{index}] must be {expected}, observed {observed}"),
            ));
        }
    }
    let production_lambda =
        require_finite_nonnegative(value, &["production_lambda"], "production_lambda", path)?;
    if !f64_close(production_lambda, 0.05) {
        return Err(invalid(
            path,
            format!("production_lambda must be 0.05, observed {production_lambda}"),
        ));
    }
    let collapse_threshold =
        require_finite_nonnegative(value, &["collapse_threshold"], "collapse_threshold", path)?;
    if !f64_close(collapse_threshold, 1.0) {
        return Err(invalid(
            path,
            format!("collapse_threshold must be 1.0, observed {collapse_threshold}"),
        ));
    }
    require_string_eq(
        value,
        &["guardrail_verdict"],
        "guardrail_verdict",
        "Pass",
        path,
    )?;
    let records = require_array(value, &["records"], "records")?;
    if records.len() != D11_LAMBDA_SWITCH_GRID.len() {
        return Err(invalid(
            path,
            format!(
                "records length must equal D11 grid length {}",
                D11_LAMBDA_SWITCH_GRID.len()
            ),
        ));
    }
    for (index, record) in records.iter().enumerate() {
        validate_sweep_record(path, record, index)?;
    }
    Ok(())
}

fn validate_sweep_record(
    path: &Path,
    value: &Value,
    index: usize,
) -> Result<(), S7SupportArtifactMaterializeError> {
    require_object(value, &[], "records entry")?;
    require_schema_version(value, path)?;
    require_u64_eq(value, &["seed"], "seed", 0, path)?;
    let lambda_switch =
        require_finite_nonnegative(value, &["lambda_switch"], "lambda_switch", path)?;
    let expected_lambda = D11_LAMBDA_SWITCH_GRID[index];
    if !f64_close(lambda_switch, expected_lambda) {
        return Err(invalid(
            path,
            format!(
                "records[{index}] lambda_switch must be {expected_lambda}, observed {lambda_switch}"
            ),
        ));
    }
    let base_train_step = require_u64(value, &["base_train_step"], "base_train_step", path)?;
    let train_step = require_u64(value, &["train_step"], "train_step", path)?;
    let expected_train_step = base_train_step
        .checked_add(RCS_TRAINING_EXTRA_STEPS)
        .ok_or_else(|| invalid(path, "base_train_step + 1000 overflowed".to_owned()))?;
    if train_step != expected_train_step {
        return Err(invalid(
            path,
            format!("records[{index}] train_step must equal base_train_step + 1000"),
        ));
    }
    let completion = require_value(value, &["completion"], "completion")?;
    let completion_kind = require_string(completion, &["kind"], "completion.kind")?;
    match completion_kind {
        "completed" => {
            let object = require_object(completion, &[], "completion")?;
            if object.len() != 1 {
                return Err(invalid(
                    path,
                    format!("records[{index}] completed completion must contain only kind"),
                ));
            }
            require_finite_nonnegative(value, &["bpc_eval_subset"], "bpc_eval_subset", path)?;
        }
        "diverged_at" => {
            require_positive_u64(completion, &["step"], "completion.step", path)?;
            if !json_path(value, &["bpc_eval_subset"]).is_some_and(Value::is_null) {
                return Err(invalid(
                    path,
                    format!("records[{index}] bpc_eval_subset must be null for divergent records"),
                ));
            }
        }
        _ => {
            return Err(invalid(
                path,
                format!("records[{index}] completion.kind must be completed or diverged_at"),
            ));
        }
    }
    require_finite_nonnegative(
        value,
        &["expert_usage_entropy_bits_mean"],
        "expert_usage_entropy_bits_mean",
        path,
    )?;
    if let Some(delta) = json_path(value, &["quality_delta_per_lambda_switch"]) {
        if !delta.is_null() {
            finite_value(delta, "quality_delta_per_lambda_switch", path)?;
        }
    } else {
        return Err(invalid(
            path,
            format!("records[{index}] missing quality_delta_per_lambda_switch"),
        ));
    }
    verified_self_hash(
        path,
        value,
        SupportArtifactSpec {
            schema: "s7_lambda_switch_sweep_step.v1",
            self_hash_field: "sweep_self_hash",
            domain: S7_LAMBDA_SWITCH_RECORD_DOMAIN,
        },
    )?;
    Ok(())
}

fn validate_temporal_switch_digest(
    path: &Path,
    value: &Value,
    layer_id: usize,
) -> Result<(), S7SupportArtifactMaterializeError> {
    require_u64_eq(value, &["n_experts"], "n_experts", S7_N_EXPERTS, path)?;
    require_q8_8(
        value,
        &["same_expert_rate_q8_8"],
        "same_expert_rate_q8_8",
        path,
    )?;
    verified_self_hash(
        path,
        value,
        SupportArtifactSpec {
            schema: "s7_temporal_switch_digest.v1",
            self_hash_field: "digest_self_hash",
            domain: S7_TEMPORAL_SWITCH_DIGEST_DOMAIN,
        },
    )?;
    let transitions = require_array(value, &["transition_mass"], "transition_mass")?;
    if transitions.is_empty() {
        return Err(invalid(
            path,
            format!("temporal_switch_digest[{layer_id}] transition_mass must be non-empty"),
        ));
    }
    for transition in transitions {
        require_expert_id(transition, &["from_expert"], "from_expert", path)?;
        require_expert_id(transition, &["to_expert"], "to_expert", path)?;
        require_q8_8(transition, &["mass_q8_8"], "mass_q8_8", path)?;
    }
    Ok(())
}

fn validate_clip_saturation_digest(
    path: &Path,
    value: &Value,
    _layer_id: usize,
) -> Result<(), S7SupportArtifactMaterializeError> {
    require_q8_8(
        value,
        &["saturation_rate_q8_8"],
        "saturation_rate_q8_8",
        path,
    )?;
    require_finite_positive(value, &["clip_bound_observed"], "clip_bound_observed", path)?;
    verified_self_hash(
        path,
        value,
        SupportArtifactSpec {
            schema: "s7_clip_saturation_digest.v1",
            self_hash_field: "digest_self_hash",
            domain: S7_CLIP_SATURATION_DIGEST_DOMAIN,
        },
    )?;
    Ok(())
}

fn validate_expert_payload_digest(
    path: &Path,
    value: &Value,
    layer_id: usize,
) -> Result<(), S7SupportArtifactMaterializeError> {
    require_non_empty_string(value, &["artifact_path"], "artifact_path", path)?;
    verified_self_hash(
        path,
        value,
        SupportArtifactSpec {
            schema: "s7_expert_payload_digest.v1",
            self_hash_field: "digest_self_hash",
            domain: S7_EXPERT_PAYLOAD_DIGEST_DOMAIN,
        },
    )?;
    let entries = require_array(value, &["entries"], "entries")?;
    if entries.len() != S7_N_EXPERTS as usize {
        return Err(invalid(
            path,
            format!("expert_payload_digest[{layer_id}] entries must cover {S7_N_EXPERTS} experts"),
        ));
    }
    let mut observed = BTreeSet::new();
    for entry in entries {
        let expert_id = require_expert_id(entry, &["expert_id"], "expert_id", path)?;
        observed.insert(expert_id);
        require_positive_u64(entry, &["byte_count"], "byte_count", path)?;
        require_value(entry, &["weight_quant"], "weight_quant")?;
    }
    let expected = (0..S7_N_EXPERTS).collect::<BTreeSet<_>>();
    if observed != expected {
        return Err(invalid(
            path,
            format!("expert_payload_digest[{layer_id}] entries must exhaust experts 0..3"),
        ));
    }
    Ok(())
}

fn validate_expert_slot_affinity(
    path: &Path,
    value: &Value,
    _layer_id: usize,
) -> Result<(), S7SupportArtifactMaterializeError> {
    verified_self_hash(
        path,
        value,
        SupportArtifactSpec {
            schema: "s7_expert_slot_affinity.v1",
            self_hash_field: "affinity_self_hash",
            domain: S7_EXPERT_SLOT_AFFINITY_DOMAIN,
        },
    )?;
    let affinities = require_array(value, &["affinities"], "affinities")?;
    for affinity in affinities {
        let pair = require_value(affinity, &["pair"], "pair")?;
        require_object(pair, &[], "pair")?;
        require_expert_id(pair, &["lo"], "pair.lo", path)?;
        require_expert_id(pair, &["hi"], "pair.hi", path)?;
        require_q8_8(affinity, &["affinity_score"], "affinity_score", path)?;
    }
    Ok(())
}

fn validate_frontier(path: &Path, value: &Value) -> Result<(), S7SupportArtifactMaterializeError> {
    let pareto = require_string(value, &["pareto_verdict"], "pareto_verdict")?;
    if !PARETO_VERDICTS.contains(&pareto) {
        return Err(invalid(
            path,
            format!("pareto_verdict must be a known ParetoVerdict, observed {pareto:?}"),
        ));
    }
    let points = require_array(value, &["points"], "points")?;
    if points.len() != 2 {
        return Err(invalid(
            path,
            format!(
                "points must contain one MoE and one dense point, observed {}",
                points.len()
            ),
        ));
    }
    let mut observed = BTreeSet::new();
    for point in points {
        let topology = require_string(point, &["topology"], "frontier point topology")?;
        if !matches!(topology, "MoeTiny" | "MoeTinyDenseMatched") {
            return Err(invalid(
                path,
                format!("frontier point topology must be an S7 topology, observed {topology:?}"),
            ));
        }
        observed.insert(topology);
        require_nonzero_hash(point, &["checkpoint_sha"], "checkpoint_sha", path)?;
        let quality = require_value(point, &["quality"], "frontier point quality")?;
        require_object(quality, &[], "frontier point quality")?;
        require_finite_nonnegative(quality, &["median_val_bpc"], "quality.median_val_bpc", path)?;
        let per_seed = require_array(quality, &["per_seed_val_bpc"], "quality.per_seed_val_bpc")?;
        if per_seed.len() != S7_SEED_COUNT {
            return Err(invalid(
                path,
                format!(
                    "quality.per_seed_val_bpc must contain {S7_SEED_COUNT} values, observed {}",
                    per_seed.len()
                ),
            ));
        }
        for value in per_seed {
            finite_nonnegative_value(value, "quality.per_seed_val_bpc", path)?;
        }
        let conformance = require_object(point, &["conformance"], "frontier point conformance")?;
        if conformance.is_empty() {
            return Err(invalid(
                path,
                format!("frontier point {topology} conformance must be non-empty"),
            ));
        }
        let projected_fit =
            require_value(point, &["projected_fit"], "frontier point projected_fit")?;
        require_object(projected_fit, &[], "frontier point projected_fit")?;
        require_positive_u64(
            projected_fit,
            &["deployed_bytes_total"],
            "projected_fit.deployed_bytes_total",
            path,
        )?;
        let per_block = require_array(
            projected_fit,
            &["deployed_bytes_per_block"],
            "projected_fit.deployed_bytes_per_block",
        )?;
        if per_block.len() != S7_N_BLOCKS {
            return Err(invalid(
                path,
                format!("projected_fit.deployed_bytes_per_block must contain {S7_N_BLOCKS} values"),
            ));
        }
        for value in per_block {
            positive_u64_value(value, "projected_fit.deployed_bytes_per_block", path)?;
        }
        if !point
            .as_object()
            .is_some_and(|object| object.contains_key("schedule_cost"))
        {
            return Err(invalid(
                path,
                format!("frontier point {topology} missing schedule_cost"),
            ));
        }
    }
    let expected = BTreeSet::from(["MoeTiny", "MoeTinyDenseMatched"]);
    if observed != expected {
        return Err(invalid(
            path,
            "points must cover MoeTiny and MoeTinyDenseMatched".to_owned(),
        ));
    }
    Ok(())
}

fn validate_burn_grad(path: &Path, value: &Value) -> Result<(), S7SupportArtifactMaterializeError> {
    require_nonzero_hash(value, &["fixture_input_sha"], "fixture_input_sha", path)?;
    for field in [
        "grad_up_weight_sum_abs",
        "grad_down_weight_sum_abs",
        "grad_up_bias_sum_abs",
        "grad_down_bias_sum_abs",
        "grad_activation_clip_threshold_sum_abs",
    ] {
        require_finite_positive(value, &[field], field, path)?;
    }
    require_bool(
        value,
        &["glu_construction_rejected"],
        "glu_construction_rejected",
        true,
        path,
    )?;
    require_bool(
        value,
        &["replay_byte_identical"],
        "replay_byte_identical",
        true,
        path,
    )?;
    Ok(())
}

fn validate_oracle(path: &Path, value: &Value) -> Result<(), S7SupportArtifactMaterializeError> {
    require_u64_eq(value, &["seed"], "seed", 0, path)?;
    require_string_eq(value, &["topology"], "topology", "MoeTiny", path)?;
    for field in [
        "fixture_prompt_sha",
        "train_logits_sha",
        "bundle_logits_sha",
        "artifact_logits_sha",
        "frozen_teacher_checkpoint_sha",
    ] {
        require_nonzero_hash(value, &[field], field, path)?;
    }
    require_string_eq(
        value,
        &["weight_quant_resolution"],
        "weight_quant_resolution",
        "QuantSpec::weight_quant",
        path,
    )?;
    let tolerance = require_finite_nonnegative(value, &["s3_tolerance"], "s3_tolerance", path)?;
    for field in [
        "pairwise_max_abs_diff_train_bundle",
        "pairwise_max_abs_diff_bundle_artifact",
        "pairwise_max_abs_diff_train_artifact",
    ] {
        let diff = require_finite_nonnegative(value, &[field], field, path)?;
        if diff > tolerance {
            return Err(invalid(
                path,
                format!("{field} exceeds s3_tolerance: {diff} > {tolerance}"),
            ));
        }
    }
    let coverage = require_value(value, &["route_coverage"], "route_coverage")?;
    require_object(coverage, &[], "route_coverage")?;
    for field in [
        "cross_layer_route_difference",
        "consecutive_token_route_change",
        "consecutive_token_route_same",
    ] {
        require_bool(coverage, &[field], field, true, path)?;
    }
    Ok(())
}

fn validate_emulator(
    path: &Path,
    value: &Value,
    topology: &S7Topology,
) -> Result<(), S7SupportArtifactMaterializeError> {
    require_u64_eq(value, &["seed"], "seed", 0, path)?;
    require_string_eq(
        value,
        &["topology"],
        "topology",
        topology_path_segment(topology),
        path,
    )?;
    for field in [
        "encoded_rom_sha",
        "prompt_sha",
        "artifact_oracle_logits_sha",
        "emulator_logits_sha",
    ] {
        require_nonzero_hash(value, &[field], field, path)?;
    }
    let tolerance = require_finite_nonnegative(value, &["s5_tolerance"], "s5_tolerance", path)?;
    let diff = require_finite_nonnegative(
        value,
        &["pairwise_max_abs_diff"],
        "pairwise_max_abs_diff",
        path,
    )?;
    if diff > tolerance {
        return Err(invalid(
            path,
            format!("pairwise_max_abs_diff exceeds s5_tolerance: {diff} > {tolerance}"),
        ));
    }
    let observed = require_finite_nonnegative(
        value,
        &["observed_bank_switches_per_token"],
        "observed_bank_switches_per_token",
        path,
    )?;
    let oracle = require_finite_nonnegative(
        value,
        &["oracle_recorded_bank_switches"],
        "oracle_recorded_bank_switches",
        path,
    )?;
    let bank_switch_diff =
        require_finite_nonnegative(value, &["bank_switch_diff"], "bank_switch_diff", path)?;
    let expected_diff = (observed - oracle).abs();
    if !f64_close(bank_switch_diff, expected_diff) {
        return Err(invalid(
            path,
            format!(
                "bank_switch_diff must equal |observed_bank_switches_per_token - oracle_recorded_bank_switches|: expected {expected_diff}, observed {bank_switch_diff}"
            ),
        ));
    }
    if bank_switch_diff > 1.0 {
        return Err(invalid(
            path,
            format!("bank_switch_diff must be <= 1, observed {bank_switch_diff}"),
        ));
    }
    require_bool(
        value,
        &["bank_switch_within_one"],
        "bank_switch_within_one",
        true,
        path,
    )?;
    Ok(())
}

fn verified_self_hash(
    path: &Path,
    value: &Value,
    spec: SupportArtifactSpec,
) -> Result<Hash256, S7SupportArtifactMaterializeError> {
    let observed =
        require_nonzero_hash(value, &[spec.self_hash_field], spec.self_hash_field, path)?;
    let mut payload = value.clone();
    let object = payload.as_object_mut().ok_or_else(|| {
        invalid(
            path,
            format!("{} payload must be a JSON object", spec.schema),
        )
    })?;
    object.remove(spec.self_hash_field);
    let remaining_self_hash_fields = object
        .keys()
        .filter(|key| key.ends_with("_self_hash"))
        .cloned()
        .collect::<Vec<_>>();
    if !remaining_self_hash_fields.is_empty() {
        return Err(invalid(
            path,
            format!(
                "{} self-hash input leaves top-level self-hash fields: {}",
                spec.self_hash_field,
                remaining_self_hash_fields.join(", ")
            ),
        ));
    }
    let canonical = CanonicalJson::value_to_vec(&payload)?;
    let expected = spec.domain.hash_canonical_bytes(&canonical)?;
    if observed != expected {
        return Err(S7SupportArtifactMaterializeError::SelfHashMismatch {
            path: path.display().to_string(),
            field: spec.self_hash_field,
            expected,
            observed,
        });
    }
    Ok(observed)
}

fn read_canonical_json(path: &Path) -> Result<Value, S7SupportArtifactMaterializeError> {
    let text =
        fs::read_to_string(path).map_err(|source| S7SupportArtifactMaterializeError::Io {
            path: path.display().to_string(),
            source,
        })?;
    serde_json::from_str::<JsonDuplicateKeyGuard>(&text).map_err(|source| {
        invalid(
            path,
            format!("duplicate JSON key or invalid JSON: {source}"),
        )
    })?;
    let value: Value =
        serde_json::from_str(&text).map_err(|source| S7SupportArtifactMaterializeError::Json {
            path: path.display().to_string(),
            source,
        })?;
    let canonical = String::from_utf8(CanonicalJson::value_to_vec(&value)?)
        .map_err(|error| invalid(path, format!("canonical JSON bytes are not UTF-8: {error}")))?;
    if text != canonical && text != format!("{canonical}\n") {
        return Err(invalid(path, "must use canonical JSON bytes".to_owned()));
    }
    Ok(value)
}

fn write_canonical_json(
    path: &Path,
    value: &Value,
) -> Result<(), S7SupportArtifactMaterializeError> {
    let mut bytes = CanonicalJson::value_to_vec(value)?;
    bytes.push(b'\n');
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| S7SupportArtifactMaterializeError::Io {
            path: parent.display().to_string(),
            source,
        })?;
    }
    fs::write(path, bytes).map_err(|source| S7SupportArtifactMaterializeError::Io {
        path: path.display().to_string(),
        source,
    })
}

fn require_nonzero_hash(
    value: &Value,
    path: &[&str],
    label: &str,
    artifact_path: &Path,
) -> Result<Hash256, S7SupportArtifactMaterializeError> {
    let raw = require_string(value, path, label)?;
    let parsed = Hash256::from_str(raw).map_err(|_| {
        invalid(
            artifact_path,
            format!("{label} must be a sha256 hash, observed {raw:?}"),
        )
    })?;
    if parsed == Hash256::ZERO {
        return Err(invalid(
            artifact_path,
            format!("{label} must not be sha256 zero"),
        ));
    }
    Ok(parsed)
}

fn require_string<'a>(
    value: &'a Value,
    path: &[&str],
    label: &str,
) -> Result<&'a str, S7SupportArtifactMaterializeError> {
    json_path(value, path)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid_label(label, "must be a string"))
}

fn require_non_empty_string(
    value: &Value,
    path: &[&str],
    label: &str,
    artifact_path: &Path,
) -> Result<(), S7SupportArtifactMaterializeError> {
    let observed = require_string(value, path, label)?;
    if observed.is_empty() {
        return Err(invalid(
            artifact_path,
            format!("{label} must be a non-empty string"),
        ));
    }
    Ok(())
}

fn require_value<'a>(
    value: &'a Value,
    path: &[&str],
    label: &str,
) -> Result<&'a Value, S7SupportArtifactMaterializeError> {
    json_path(value, path).ok_or_else(|| invalid_label(label, "is missing"))
}

fn require_schema_version(
    value: &Value,
    artifact_path: &Path,
) -> Result<(), S7SupportArtifactMaterializeError> {
    require_u64_eq(
        value,
        &["schema_version", "major"],
        "schema_version.major",
        1,
        artifact_path,
    )?;
    require_u64_eq(
        value,
        &["schema_version", "minor"],
        "schema_version.minor",
        0,
        artifact_path,
    )?;
    require_u64_eq(
        value,
        &["schema_version", "patch"],
        "schema_version.patch",
        0,
        artifact_path,
    )
}

fn require_string_eq(
    value: &Value,
    path: &[&str],
    label: &str,
    expected: &str,
    artifact_path: &Path,
) -> Result<(), S7SupportArtifactMaterializeError> {
    let observed = require_string(value, path, label)?;
    if observed != expected {
        return Err(invalid(
            artifact_path,
            format!("{label} must be {expected:?}, observed {observed:?}"),
        ));
    }
    Ok(())
}

fn require_array<'a>(
    value: &'a Value,
    path: &[&str],
    label: &str,
) -> Result<&'a Vec<Value>, S7SupportArtifactMaterializeError> {
    json_path(value, path)
        .and_then(Value::as_array)
        .ok_or_else(|| invalid_label(label, "must be an array"))
}

fn require_object<'a>(
    value: &'a Value,
    path: &[&str],
    label: &str,
) -> Result<&'a serde_json::Map<String, Value>, S7SupportArtifactMaterializeError> {
    json_path(value, path)
        .and_then(Value::as_object)
        .ok_or_else(|| invalid_label(label, "must be an object"))
}

fn require_bool(
    value: &Value,
    path: &[&str],
    label: &str,
    expected: bool,
    artifact_path: &Path,
) -> Result<(), S7SupportArtifactMaterializeError> {
    let observed = json_path(value, path)
        .and_then(Value::as_bool)
        .ok_or_else(|| invalid_label(label, "must be a boolean"))?;
    if observed != expected {
        return Err(invalid(
            artifact_path,
            format!("{label} must be {expected}, observed {observed}"),
        ));
    }
    Ok(())
}

fn require_u64_eq(
    value: &Value,
    path: &[&str],
    label: &str,
    expected: u64,
    artifact_path: &Path,
) -> Result<(), S7SupportArtifactMaterializeError> {
    let observed = json_path(value, path)
        .and_then(Value::as_u64)
        .ok_or_else(|| invalid_label(label, "must be a u64"))?;
    if observed != expected {
        return Err(invalid(
            artifact_path,
            format!("{label} must be {expected}, observed {observed}"),
        ));
    }
    Ok(())
}

fn require_u64(
    value: &Value,
    path: &[&str],
    label: &str,
    artifact_path: &Path,
) -> Result<u64, S7SupportArtifactMaterializeError> {
    json_path(value, path)
        .and_then(Value::as_u64)
        .ok_or_else(|| invalid(artifact_path, format!("{label} must be a u64")))
}

fn require_q8_8(
    value: &Value,
    path: &[&str],
    label: &str,
    artifact_path: &Path,
) -> Result<u64, S7SupportArtifactMaterializeError> {
    let Some(observed) = json_path(value, path).and_then(Value::as_u64) else {
        return Err(invalid(
            artifact_path,
            format!("{label} must be an integer in 0..=256"),
        ));
    };
    if observed > 256 {
        return Err(invalid(
            artifact_path,
            format!("{label} must be in 0..=256, observed {observed}"),
        ));
    }
    Ok(observed)
}

fn require_expert_id(
    value: &Value,
    path: &[&str],
    label: &str,
    artifact_path: &Path,
) -> Result<u64, S7SupportArtifactMaterializeError> {
    let Some(observed) = json_path(value, path).and_then(Value::as_u64) else {
        return Err(invalid(
            artifact_path,
            format!("{label} must be an expert id in 0..3"),
        ));
    };
    if observed >= S7_N_EXPERTS {
        return Err(invalid(
            artifact_path,
            format!("{label} must be an expert id in 0..3, observed {observed}"),
        ));
    }
    Ok(observed)
}

fn require_finite_nonnegative(
    value: &Value,
    path: &[&str],
    label: &str,
    artifact_path: &Path,
) -> Result<f64, S7SupportArtifactMaterializeError> {
    let value =
        json_path(value, path).ok_or_else(|| invalid_label(label, "must be a finite number"))?;
    finite_nonnegative_value(value, label, artifact_path)
}

fn require_finite_positive(
    value: &Value,
    path: &[&str],
    label: &str,
    artifact_path: &Path,
) -> Result<f64, S7SupportArtifactMaterializeError> {
    let number = require_finite_nonnegative(value, path, label, artifact_path)?;
    if number <= 0.0 {
        return Err(invalid(
            artifact_path,
            format!("{label} must be > 0, observed {number}"),
        ));
    }
    Ok(number)
}

fn finite_nonnegative_value(
    value: &Value,
    label: &str,
    artifact_path: &Path,
) -> Result<f64, S7SupportArtifactMaterializeError> {
    let Some(number) = value.as_f64() else {
        return Err(invalid(
            artifact_path,
            format!("{label} must be a finite number"),
        ));
    };
    if !number.is_finite() || number < 0.0 {
        return Err(invalid(
            artifact_path,
            format!("{label} must be finite and non-negative, observed {number}"),
        ));
    }
    Ok(number)
}

fn finite_value(
    value: &Value,
    label: &str,
    artifact_path: &Path,
) -> Result<f64, S7SupportArtifactMaterializeError> {
    let Some(number) = value.as_f64() else {
        return Err(invalid(
            artifact_path,
            format!("{label} must be a finite number"),
        ));
    };
    if !number.is_finite() {
        return Err(invalid(
            artifact_path,
            format!("{label} must be finite, observed {number}"),
        ));
    }
    Ok(number)
}

fn require_positive_u64(
    value: &Value,
    path: &[&str],
    label: &str,
    artifact_path: &Path,
) -> Result<u64, S7SupportArtifactMaterializeError> {
    let value = json_path(value, path).ok_or_else(|| invalid_label(label, "must be > 0"))?;
    positive_u64_value(value, label, artifact_path)
}

fn positive_u64_value(
    value: &Value,
    label: &str,
    artifact_path: &Path,
) -> Result<u64, S7SupportArtifactMaterializeError> {
    let Some(number) = value.as_u64() else {
        return Err(invalid(
            artifact_path,
            format!("{label} must be an unsigned integer"),
        ));
    };
    if number == 0 {
        return Err(invalid(artifact_path, format!("{label} must be > 0")));
    }
    Ok(number)
}

fn json_path<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    Some(current)
}

fn resolve_under_root(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

fn topology_path_segment(topology: &S7Topology) -> &'static str {
    match topology {
        S7Topology::MoeTiny => "MoeTiny",
        S7Topology::MoeTinyDenseMatched => "MoeTinyDenseMatched",
    }
}

fn f64_close(left: f64, right: f64) -> bool {
    (left - right).abs() <= 1.0e-9
}

fn invalid_label(label: &str, message: &str) -> S7SupportArtifactMaterializeError {
    S7SupportArtifactMaterializeError::InvalidArtifact {
        path: "<input>".to_owned(),
        message: format!("{label} {message}"),
    }
}

fn invalid(path: &Path, message: String) -> S7SupportArtifactMaterializeError {
    S7SupportArtifactMaterializeError::InvalidArtifact {
        path: path.display().to_string(),
        message,
    }
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

/// Error returned while materializing a support artifact.
#[derive(Debug)]
pub enum S7SupportArtifactMaterializeError {
    /// Filesystem operation failed.
    Io {
        /// Path being read or written.
        path: String,
        /// Source I/O error.
        source: io::Error,
    },
    /// JSON decoding failed.
    Json {
        /// Path being decoded.
        path: String,
        /// Source JSON error.
        source: serde_json::Error,
    },
    /// Canonical JSON encoding failed.
    CanonicalJson(CanonicalJsonError),
    /// A non-emulator artifact was passed a topology.
    UnexpectedTopology {
        /// Artifact kind.
        kind: &'static str,
    },
    /// A non-switch-stats artifact was passed a seed.
    UnexpectedSeed {
        /// Artifact kind.
        kind: &'static str,
    },
    /// An emulator artifact did not specify topology.
    MissingTopology {
        /// Artifact kind.
        kind: &'static str,
    },
    /// A per-seed artifact did not specify seed.
    MissingSeed {
        /// Artifact kind.
        kind: &'static str,
    },
    /// Seed is outside the S7 fixed seed set.
    InvalidSeed {
        /// Observed seed.
        seed: u64,
    },
    /// The artifact violates the closure-critical shape.
    InvalidArtifact {
        /// Artifact path.
        path: String,
        /// Diagnostic.
        message: String,
    },
    /// Domain self-hash did not match the payload.
    SelfHashMismatch {
        /// Artifact path.
        path: String,
        /// Self-hash field.
        field: &'static str,
        /// Expected computed hash.
        expected: Hash256,
        /// Observed hash.
        observed: Hash256,
    },
}

impl fmt::Display for S7SupportArtifactMaterializeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "{path}: {source}"),
            Self::Json { path, source } => write!(f, "{path}: {source}"),
            Self::CanonicalJson(error) => write!(f, "{error}"),
            Self::UnexpectedTopology { kind } => {
                write!(f, "S7 support artifact {kind} must not pass --topology")
            }
            Self::UnexpectedSeed { kind } => {
                write!(f, "S7 support artifact {kind} must not pass --seed")
            }
            Self::MissingTopology { kind } => {
                write!(f, "S7 support artifact {kind} requires --topology")
            }
            Self::MissingSeed { kind } => {
                write!(f, "S7 support artifact {kind} requires --seed")
            }
            Self::InvalidSeed { seed } => {
                write!(
                    f,
                    "S7 support artifact seed must be in 0..4, observed {seed}"
                )
            }
            Self::InvalidArtifact { path, message } => write!(f, "{path}: {message}"),
            Self::SelfHashMismatch {
                path,
                field,
                expected,
                observed,
            } => write!(
                f,
                "{path}: {field} mismatch: expected {expected}, observed {observed}"
            ),
        }
    }
}

impl std::error::Error for S7SupportArtifactMaterializeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Json { source, .. } => Some(source),
            Self::CanonicalJson(error) => Some(error),
            Self::UnexpectedTopology { .. }
            | Self::UnexpectedSeed { .. }
            | Self::MissingTopology { .. }
            | Self::MissingSeed { .. }
            | Self::InvalidSeed { .. }
            | Self::InvalidArtifact { .. }
            | Self::SelfHashMismatch { .. } => None,
        }
    }
}

impl From<CanonicalJsonError> for S7SupportArtifactMaterializeError {
    fn from(error: CanonicalJsonError) -> Self {
        Self::CanonicalJson(error)
    }
}
