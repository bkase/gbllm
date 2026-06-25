#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

tmp="$(mktemp -d /tmp/s7-validate-artifacts-test.XXXXXX)"
trap 'rm -rf "$tmp"' EXIT

python3 - "$tmp" <<'PY'
from __future__ import annotations

import hashlib
import json
import sys
from pathlib import Path

root = Path(sys.argv[1])
h = "sha256:" + "2" * 64
grid = [0.0, 0.05, 0.1, 0.5, 1.0, 5.0]
S7_OPTIMIZER_STEPS = 20_000
S7_EVAL_EVERY_STEPS = 1_000
RUN_LOG_DOMAIN = ("gbf-artifact", "S7RunLog", "s7_run_log.v1", "1")
RAW_LOSS_DIAGNOSTICS_DOMAIN = ("gbf-artifact", "RawLossDiagnostics", "s7_raw_loss_diagnostics.v1", "1")
SCORE_DOMAIN = ("gbf-artifact", "S7ScoreReport", "s7_score.v1", "1")
SWITCH_STATS_DOMAIN = ("gbf-experiments", "S7SwitchStatsReport", "s7_switch_stats.v1", "1")
TEMPORAL_SWITCH_DIGEST_DOMAIN = ("gbf-artifact", "TemporalSwitchDigest", "s7_temporal_switch_digest.v1", "1")
CLIP_SATURATION_DIGEST_DOMAIN = ("gbf-artifact", "ClipSaturationDigest", "s7_clip_saturation_digest.v1", "1")
EXPERT_PAYLOAD_DIGEST_DOMAIN = ("gbf-artifact", "ExpertPayloadDigest", "s7_expert_payload_digest.v1", "1")
EXPERT_SLOT_AFFINITY_DOMAIN = ("gbf-artifact", "ExpertSlotAffinity", "s7_expert_slot_affinity.v1", "1")
MATCHED_BYTES_PIN_DOMAIN = ("gbf-artifact", "MatchedBytesPin", "s7_matched_bytes_pin.v1", "1")
DENSE_VS_MOE_DOMAIN = ("gbf-artifact", "S7DenseVsMoeComparisonReport", "s7_dense_vs_moe.v1", "1")
LAMBDA_SWITCH_RECORD_DOMAIN = ("gbf-experiments", "LambdaSwitchSweepRecord", "s7_lambda_switch_sweep_step.v1", "1")
ROUTER_COLLAPSE_SWEEP_DOMAIN = ("gbf-experiments", "RouterCollapseSweepReport", "s7_router_collapse_sweep.v1", "1")
FRONTIER_DOMAIN = ("gbf-experiments", "S7FrontierReport", "s7_frontier.v1", "1")
BURN_GRAD_SMOKE_DOMAIN = ("gbf-experiments", "S7BurnGradSmokeReport", "s7_burn_grad_smoke.v1", "1")
ORACLE_ROUTED_DOMAIN = ("gbf-experiments", "S7OracleRoutedReport", "s7_oracle_routed.v1", "1")
ROUTER_STEP_TELEMETRY_DOMAIN = ("gbf-experiments", "RouterStepTelemetry", "s7_router_step_telemetry.v1", "1")
EMULATOR_DOMAIN = ("gbf-experiments", "EmulatorOneTokenReport", "s7_emulator_one_token.v1", "1")


def write_text(rel: str, text: str) -> None:
    path = root / rel
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


def write(rel: str, payload) -> None:
    write_text(rel, json.dumps(payload, sort_keys=True, separators=(",", ":")) + "\n")


def write_jsonl(rel: str, records) -> None:
    write_text(
        rel,
        "".join(json.dumps(record, sort_keys=True, separators=(",", ":")) + "\n" for record in records),
    )


def canonical(payload) -> str:
    return json.dumps(payload, sort_keys=True, separators=(",", ":"), ensure_ascii=False, allow_nan=False)


def with_self_hash(payload, field: str, domain):
    payload = dict(payload)
    payload.pop(field, None)
    crate_name, type_name, schema_id, schema_version = domain
    material = (
        f"gbf:{crate_name}:{type_name}:{schema_id}:{schema_version}".encode("utf-8")
        + b"\0"
        + canonical(payload).encode("utf-8")
    )
    payload[field] = f"sha256:{hashlib.sha256(material).hexdigest()}"
    return payload


def version():
    return {"major": 1, "minor": 0, "patch": 0}


def grad_norms():
    return {"global_l2": 1.0, "max_l2": 0.5, "mean_l2": 0.25}


def grad_log_record(seed: int, step: int):
    return {
        "schema": "s7_grad_log.v1",
        "seed": seed,
        "train_step": step,
        "grad_norms": grad_norms(),
    }


def raw_loss():
    return with_self_hash({
        "lm_loss_raw": 1.0,
        "distill_loss_raw": {
            "kind": "not_available",
            "reason": "no_frozen_teacher",
            "phase": "phase_a",
        },
        "balance_loss_raw": 0.1,
        "zrouter_loss_raw": 0.2,
        "switch_loss_raw": 0.3,
    }, "diagnostics_self_hash", RAW_LOSS_DIAGNOSTICS_DOMAIN)


def router_telemetry(seed: int, layer: int):
    return with_self_hash({
        "schema_version": version(),
        "seed": seed,
        "train_step": 1,
        "layer_id": layer,
        "expert_usage_entropy_bits": 1.0,
        "same_expert_rate": 0.5,
        "router_confidence_distribution": {"mean": 0.6, "p10": 0.4, "p50": 0.6, "p90": 0.8},
        "tokens_per_expert": [2, 2, 0, 0],
        "bank_switches_per_token": 0.5,
    }, "telemetry_self_hash", ROUTER_STEP_TELEMETRY_DOMAIN)


def temporal(layer: int):
    return with_self_hash({
        "schema_version": version(),
        "layer_id": layer,
        "n_experts": 4,
        "same_expert_rate_q8_8": 128,
        "transition_mass": [{"from_expert": 0, "to_expert": 1, "mass_q8_8": 64}],
    }, "digest_self_hash", TEMPORAL_SWITCH_DIGEST_DOMAIN)


def clip(layer: int):
    return with_self_hash({
        "schema_version": version(),
        "layer_id": layer,
        "saturation_rate_q8_8": 1,
        "clip_bound_observed": 1.0,
    }, "digest_self_hash", CLIP_SATURATION_DIGEST_DOMAIN)


def payload(layer: int):
    return with_self_hash({
        "schema_version": version(),
        "layer_id": layer,
        "artifact_path": f"layer-{layer}",
        "entries": [
            {"expert_id": expert, "byte_count": 128 + expert, "weight_quant": {"kind": "ternary2"}}
            for expert in range(4)
        ],
    }, "digest_self_hash", EXPERT_PAYLOAD_DIGEST_DOMAIN)


def affinity(layer: int):
    return with_self_hash({
        "schema_version": version(),
        "layer_id": layer,
        "affinities": [{"pair": {"lo": 0, "hi": 1}, "affinity_score": 64}],
    }, "affinity_self_hash", EXPERT_SLOT_AFFINITY_DOMAIN)


def sweep_record(index: int):
    return with_self_hash({
        "schema_version": version(),
        "seed": 0,
        "lambda_switch": grid[index],
        "base_train_step": 1000,
        "train_step": 2000,
        "completion": {"kind": "completed"},
        "bpc_eval_subset": 1.0 + (index * 0.1),
        "expert_usage_entropy_bits_mean": 1.0,
        "quality_delta_per_lambda_switch": 0.0,
    }, "sweep_self_hash", LAMBDA_SWITCH_RECORD_DOMAIN)


loss_diagnostics = raw_loss()
for topology in ["MoeTiny", "MoeTinyDenseMatched"]:
    for seed in range(5):
        run = {
            "schema": "s7_run_log.v1",
            "seed": seed,
            "topology": topology,
            "train_config_hash": h,
            "model_topology_hash": h,
            "router_config_hash": h if topology == "MoeTiny" else None,
            "expert_block_config_hash": h if topology == "MoeTiny" else None,
            "loss_config_hash": h,
            "phase_schedule_hash": h,
            "frozen_teacher_checkpoint_sha": h,
            "losses": [[step, loss_diagnostics] for step in range(1, S7_OPTIMIZER_STEPS + 1)],
            "grad_norms": [[step, grad_norms()] for step in range(1, S7_OPTIMIZER_STEPS + 1)],
            "eval_points": [
                [step, 1.0]
                for step in range(0, S7_OPTIMIZER_STEPS + 1, S7_EVAL_EVERY_STEPS)
            ],
            "final_grad_norms": grad_norms(),
            "completion": {"kind": "completed"},
        }
        write(f"experiments/S7/runs/{topology}/seed-{seed}/run-log.json", with_self_hash(run, "run_log_self_hash", RUN_LOG_DOMAIN))
        write_jsonl(
            f"experiments/S7/runs/{topology}/seed-{seed}/grad-log.jsonl",
            [grad_log_record(seed, step) for step in range(1, S7_OPTIMIZER_STEPS + 1)],
        )
        write_jsonl(
            f"experiments/S7/runs/{topology}/seed-{seed}/router-step-telemetry.jsonl",
            [router_telemetry(seed, layer) for layer in range(4)] if topology == "MoeTiny" else [],
        )
        score = {
            "schema": "s7_score.v1",
            "seed": seed,
            "topology": topology,
            "checkpoint_sha": h,
            "corpus_val_sha": h,
            "chunk_size": 256,
            "token_count": 4,
            "log2_sum": 8.0,
            "bpc": 2.0,
        }
        write(f"experiments/S7/scores/{topology}/seed-{seed}/score.json", with_self_hash(score, "score_self_hash", SCORE_DOMAIN))

for seed in range(5):
    write(
        f"experiments/S7/switch-stats/seed-{seed}/switch-stats.json",
        with_self_hash({
            "schema": "s7_switch_stats.v1",
            "seed": seed,
            "artifact_path": f"seed-{seed}",
            "temporal_switch_digest": [temporal(layer) for layer in range(4)],
            "clip_saturation_digest": [clip(layer) for layer in range(4)],
            "expert_payload_digest": [payload(layer) for layer in range(4)],
            "expert_slot_affinity": [affinity(layer) for layer in range(4)],
            "aggregation_rule": "SUM",
        }, "bundle_self_hash", SWITCH_STATS_DOMAIN),
    )

write(
    "experiments/S7/router-collapse/seed-0/sweep.json",
    with_self_hash({
        "schema": "s7_router_collapse_sweep.v1",
        "seed": 0,
        "base_checkpoint_sha": h,
        "grid": grid,
        "records": [sweep_record(index) for index in range(len(grid))],
        "production_lambda": 0.05,
        "collapse_threshold": 1.0,
        "guardrail_verdict": "Pass",
    }, "sweep_self_hash", ROUTER_COLLAPSE_SWEEP_DOMAIN),
)
matched_bytes_pin = with_self_hash({
    "formula_version": {"major": 0, "minor": 2, "patch": 0},
    "d_ff_dense_resolved": 128,
    "bias_policy": "test",
    "b_experts_total": 100,
    "b_router_overhead_total": 0,
    "b_dense_ffn_total": 100,
    "b_deployed_total_moe": 100,
    "b_deployed_total_dense": 100,
    "tolerance_bytes": 0,
}, "matched_bytes_self_hash", MATCHED_BYTES_PIN_DOMAIN)
write(
    "experiments/S7/dense-vs-moe/comparison.json",
    with_self_hash({
        "schema": "s7_dense_vs_moe.v1",
        "moe_topology_hash": h,
        "dense_matched_topology_hash": h,
        "matched_bytes_pin": matched_bytes_pin,
        "per_seed": [
            {
                "seed": seed,
                "val_bpc_moe": 1.0,
                "val_bpc_dense": 1.1,
                "delta": 0.1,
                "parity_verdict": "Pass",
            }
            for seed in range(5)
        ],
        "median_val_bpc_moe": 1.0,
        "median_val_bpc_dense": 1.1,
        "deployed_bytes_total_moe": 100,
        "deployed_bytes_total_dense": 100,
        "bytes_diff": 0,
        "bytes_within_tolerance": True,
        "aggregate_parity_verdict": "Pass-clean",
        "pareto_verdict": "MoE-dominates",
        "switch_stats_summary": {
            "same_expert_rate_per_layer_q8_8": [128, 128, 128, 128],
            "expert_usage_entropy_bits_mean": 1.0,
            "bank_switches_per_token_mean": 0.5,
        },
        "sweep_summary": {
            "bpc_at_lambda": {"0.0": 1.0, "0.05": 1.0},
            "entropy_at_lambda": {"0.0": 1.0, "0.05": 1.0},
            "guardrail_verdict": {"kind": "pass"},
        },
    }, "comparison_self_hash", DENSE_VS_MOE_DOMAIN),
)
write(
    "experiments/S7/frontier/frontier.json",
    with_self_hash({
        "schema": "s7_frontier.v1",
        "points": [
            {
                "topology": "MoeTiny",
                "checkpoint_sha": h,
                "quality": {"median_val_bpc": 1.0, "per_seed_val_bpc": [1.0] * 5},
                "conformance": {"status": "ok"},
                "projected_fit": {"deployed_bytes_total": 100, "deployed_bytes_per_block": [25] * 4},
                "schedule_cost": None,
            },
            {
                "topology": "MoeTinyDenseMatched",
                "checkpoint_sha": h,
                "quality": {"median_val_bpc": 1.1, "per_seed_val_bpc": [1.1] * 5},
                "conformance": {"status": "ok"},
                "projected_fit": {"deployed_bytes_total": 100, "deployed_bytes_per_block": [25] * 4},
                "schedule_cost": None,
            },
        ],
        "pareto_verdict": "MoE-dominates",
    }, "frontier_self_hash", FRONTIER_DOMAIN),
)
write(
    "experiments/S7/burn-grad-smoke/expert_block_qat.json",
    with_self_hash({
        "schema": "s7_burn_grad_smoke.v1",
        "fixture_seed": 65261,
        "burn_adapter_version": "test",
        "fixture_input_sha": h,
        "grad_up_weight_sum_abs": 1.0,
        "grad_down_weight_sum_abs": 1.0,
        "supported_clipped_activation_count": 3,
        "learned_activation_range_unsupported": True,
        "projection_biases_unsupported": True,
        "glu_construction_rejected": True,
        "replay_byte_identical": True,
    }, "smoke_self_hash", BURN_GRAD_SMOKE_DOMAIN),
)
write(
    "experiments/S7/oracle-routed/seed-0/oracle.json",
    with_self_hash({
        "schema": "s7_oracle_routed.v1",
        "seed": 0,
        "topology": "MoeTiny",
        "fixture_prompt_sha": h,
        "train_logits_sha": h,
        "bundle_logits_sha": h,
        "artifact_logits_sha": h,
        "frozen_teacher_checkpoint_sha": h,
        "pairwise_max_abs_diff_train_bundle": 0.0,
        "pairwise_max_abs_diff_bundle_artifact": 0.0,
        "pairwise_max_abs_diff_train_artifact": 0.0,
        "s3_tolerance": 0.1,
        "route_coverage": {
            "cross_layer_route_difference": True,
            "consecutive_token_route_change": True,
            "consecutive_token_route_same": True,
        },
        "weight_quant_resolution": "QuantSpec::weight_quant",
    }, "oracle_self_hash", ORACLE_ROUTED_DOMAIN),
)
for topology in ["MoeTiny", "MoeTinyDenseMatched"]:
    write(
        f"experiments/S7/emulator-one-token/seed-0/{topology}/result.json",
        with_self_hash({
            "schema": "s7_emulator_one_token.v1",
            "seed": 0,
            "topology": topology,
            "encoded_rom_sha": h,
            "prompt_sha": h,
            "artifact_oracle_logits_sha": h,
            "emulator_logits_sha": h,
            "pairwise_max_abs_diff": 0.0,
            "s5_tolerance": 0.1,
            "observed_bank_switches_per_token": 0.0,
            "oracle_recorded_bank_switches": 0.0,
            "bank_switch_diff": 0.0,
            "bank_switch_within_one": True,
        }, "emulator_self_hash", EMULATOR_DOMAIN),
    )
PY

scripts/review/f-s7/validate-artifacts.py --root "$tmp" >/tmp/s7-validate-artifacts-ok.out

frontier_path="$tmp/experiments/S7/frontier/frontier.json"
cp "$frontier_path" "$tmp/frontier.json.valid"
python3 - "$frontier_path" <<'PY'
from pathlib import Path
import json
import sys

path = Path(sys.argv[1])
data = json.loads(path.read_text(encoding="utf-8"))
data["frontier_self_hash"] = "sha256:" + "9" * 64
path.write_text(json.dumps(data, sort_keys=True, separators=(",", ":")) + "\n", encoding="utf-8")
PY

if scripts/review/f-s7/validate-artifacts.py --root "$tmp" --fast-fail >/tmp/s7-validate-artifacts-bad.out 2>&1; then
  echo "expected frontier self-hash validation failure" >&2
  exit 1
fi
rg -n "frontier_self_hash self-hash mismatch" /tmp/s7-validate-artifacts-bad.out >/dev/null
mv "$tmp/frontier.json.valid" "$frontier_path"

switch_stats_path="$tmp/experiments/S7/switch-stats/seed-1/switch-stats.json"
cp "$switch_stats_path" "$tmp/switch-stats.json.valid"
python3 - "$switch_stats_path" <<'PY'
from pathlib import Path
import json
import sys

path = Path(sys.argv[1])
data = json.loads(path.read_text(encoding="utf-8"))
data["bundle_self_hash"] = "sha256:" + "6" * 64
path.write_text(json.dumps(data, sort_keys=True, separators=(",", ":")) + "\n", encoding="utf-8")
PY

if scripts/review/f-s7/validate-artifacts.py --root "$tmp" --fast-fail >/tmp/s7-validate-artifacts-bad.out 2>&1; then
  echo "expected switch-stats bundle self-hash validation failure" >&2
  exit 1
fi
rg -n "bundle_self_hash self-hash mismatch" /tmp/s7-validate-artifacts-bad.out >/dev/null
mv "$tmp/switch-stats.json.valid" "$switch_stats_path"

python3 - "$ROOT/scripts/review/f-s7/validate-artifacts.py" <<'PY'
from pathlib import Path
import importlib.util
import json
import sys
import tempfile

module_path = Path(sys.argv[1])
spec = importlib.util.spec_from_file_location("s7_validate_artifacts", module_path)
module = importlib.util.module_from_spec(spec)
assert spec.loader is not None
spec.loader.exec_module(module)
assert module.canonical_json_text({"x": 1.0e20}) == '{"x":1e20}'

def with_self_hash(payload, field, domain):
    payload = dict(payload)
    payload.pop(field, None)
    payload[field] = module.domain_self_hash(domain, payload)
    return payload

grad_norms = {"global_l2": 1.0, "max_l2": 0.5, "mean_l2": 0.25}
raw_loss = with_self_hash({
    "lm_loss_raw": 1.0,
    "distill_loss_raw": {"kind": "not_available", "reason": "no_frozen_teacher", "phase": "phase_a"},
    "balance_loss_raw": 0.1,
    "zrouter_loss_raw": 0.2,
    "switch_loss_raw": 0.3,
}, "diagnostics_self_hash", module.RAW_LOSS_DIAGNOSTICS_DOMAIN)

errors = []
bad_loss_hash = dict(raw_loss)
bad_loss_hash["diagnostics_self_hash"] = "sha256:" + "7" * 64
module.validate_raw_loss_diagnostics(errors, "raw-loss", bad_loss_hash)
assert any("diagnostics_self_hash self-hash mismatch" in error for error in errors), errors

errors = []
bad_loss_shape = dict(raw_loss)
bad_loss_shape.pop("zrouter_loss_raw")
module.validate_raw_loss_diagnostics(errors, "raw-loss", bad_loss_shape)
assert any("RawLossDiagnostics fields must be lm_loss_raw" in error for error in errors), errors

errors = []
module.validate_run_log_series(errors, Path("run-log.json"), {
    "losses": [[1, raw_loss]],
    "grad_norms": [[1, grad_norms]],
    "eval_points": [[1, 1.0]],
})
assert any("eval_points length must be 21" in error for error in errors), errors
assert any("eval_points[0] eval_step must be 0" in error for error in errors), errors

errors = []
module.validate_run_log_grad_norms(errors, Path("run-log.json"), {
    "grad_norms": [[1, grad_norms]],
})
assert any("missing final_grad_norms" in error for error in errors), errors

telemetry = with_self_hash({
    "schema_version": module.version_dict(),
    "seed": 0,
    "train_step": 1,
    "layer_id": 0,
    "expert_usage_entropy_bits": 1.0,
    "same_expert_rate": 0.5,
    "router_confidence_distribution": {"mean": 0.6, "p10": 0.4, "p50": 0.6, "p90": 0.8},
    "tokens_per_expert": [2, 2, 0, 0],
    "bank_switches_per_token": 0.5,
}, "telemetry_self_hash", module.ROUTER_STEP_TELEMETRY_DOMAIN)
errors = []
envelope = {
    "event_name": "s7.router.step",
    "telemetry_self_hash": "sha256:" + "8" * 64,
    "telemetry_canonical_json": module.canonical_json_text(telemetry),
}
module.unwrap_router_step_telemetry_record(errors, "router-envelope", envelope)
assert any("flat telemetry_self_hash must match telemetry_canonical_json" in error for error in errors), errors

with tempfile.TemporaryDirectory() as tmpdir:
    tmpdir_path = Path(tmpdir)
    telemetry_path = tmpdir_path / "router-step-telemetry.jsonl"
    telemetry_path.write_text("", encoding="utf-8")
    errors = []
    module.validate_router_step_telemetry_log(errors, telemetry_path, "MoeTiny", 0)
    assert any("router-step telemetry must contain at least one JSONL record" in error for error in errors), errors

    partial_layers = []
    for layer in range(3):
        record = dict(telemetry)
        record["layer_id"] = layer
        record = with_self_hash(record, "telemetry_self_hash", module.ROUTER_STEP_TELEMETRY_DOMAIN)
        partial_layers.append(module.canonical_json_text(record))
    telemetry_path.write_text("\n".join(partial_layers) + "\n", encoding="utf-8")
    errors = []
    module.validate_router_step_telemetry_log(errors, telemetry_path, "MoeTiny", 0)
    assert any("router-step telemetry must cover layers 0..3" in error for error in errors), errors

    grad_path = tmpdir_path / "grad-log.jsonl"
    grad_path.write_text(json.dumps({"seed": 0, "train_step": 1, "grad_norms": grad_norms}, sort_keys=True, separators=(",", ":")) + "\n", encoding="utf-8")
    errors = []
    module.validate_grad_log(errors, grad_path, 0, [[1, grad_norms]])
    assert any("schema must be s7_grad_log.v1" in error for error in errors), errors
PY

score_path="$tmp/experiments/S7/scores/MoeTiny/seed-0/score.json"
cp "$score_path" "$tmp/score.json.valid"
python3 - "$score_path" <<'PY'
from pathlib import Path
import json
import sys

path = Path(sys.argv[1])
data = json.loads(path.read_text(encoding="utf-8"))
data["score_self_hash"] = "sha256:" + "4" * 64
path.write_text(json.dumps(data, sort_keys=True, separators=(",", ":")) + "\n", encoding="utf-8")
PY

if scripts/review/f-s7/validate-artifacts.py --root "$tmp" --fast-fail >/tmp/s7-validate-artifacts-bad.out 2>&1; then
  echo "expected self-hash validation failure" >&2
  exit 1
fi
rg -n "score_self_hash self-hash mismatch" /tmp/s7-validate-artifacts-bad.out >/dev/null
mv "$tmp/score.json.valid" "$score_path"

python3 - "$tmp" <<'PY'
from pathlib import Path
import json
import sys

root = Path(sys.argv[1])
score = root / "experiments/S7/scores/MoeTiny/seed-0/score.json"
data = json.loads(score.read_text(encoding="utf-8"))
score.write_text(json.dumps(data, sort_keys=True, indent=2) + "\n", encoding="utf-8")
PY

if scripts/review/f-s7/validate-artifacts.py --root "$tmp" --fast-fail >/tmp/s7-validate-artifacts-bad.out 2>&1; then
  echo "expected canonical JSON validation failure" >&2
  exit 1
fi
rg -n "must use canonical JSON bytes" /tmp/s7-validate-artifacts-bad.out >/dev/null

python3 - "$tmp" <<'PY'
from pathlib import Path
import json
import sys

root = Path(sys.argv[1])
score = root / "experiments/S7/scores/MoeTiny/seed-0/score.json"
data = json.loads(score.read_text(encoding="utf-8"))
score.write_text(json.dumps(data, sort_keys=True, separators=(",", ":")) + "\n", encoding="utf-8")
PY

python3 - "$tmp" <<'PY'
from pathlib import Path
import json
import sys

root = Path(sys.argv[1])
score = root / "experiments/S7/scores/MoeTiny/seed-0/score.json"
data = json.loads(score.read_text(encoding="utf-8"))
data["bpc"] = float("nan")
score.write_text(json.dumps(data, sort_keys=True, separators=(",", ":")) + "\n", encoding="utf-8")
PY

if scripts/review/f-s7/validate-artifacts.py --root "$tmp" --fast-fail >/tmp/s7-validate-artifacts-bad.out 2>&1; then
  echo "expected non-finite canonical JSON value validation failure" >&2
  exit 1
fi
rg -n "has non-canonical JSON value" /tmp/s7-validate-artifacts-bad.out >/dev/null

python3 - "$tmp" <<'PY'
from pathlib import Path
import json
import sys

root = Path(sys.argv[1])
score = root / "experiments/S7/scores/MoeTiny/seed-0/score.json"
data = json.loads(score.read_text(encoding="utf-8"))
data["bpc"] = 2.0
score.write_text(json.dumps(data, sort_keys=True, separators=(",", ":")) + "\n", encoding="utf-8")
PY

python3 - "$tmp" <<'PY'
from pathlib import Path
import json
import sys

root = Path(sys.argv[1])

def mutate(rel, fn):
    path = root / rel
    data = json.loads(path.read_text(encoding="utf-8"))
    fn(data)
    path.write_text(json.dumps(data, sort_keys=True, separators=(",", ":")) + "\n", encoding="utf-8")

mutate("experiments/S7/scores/MoeTiny/seed-0/score.json", lambda data: data.__setitem__("bpc", 3.0))
PY

if scripts/review/f-s7/validate-artifacts.py --root "$tmp" --fast-fail >/tmp/s7-validate-artifacts-bad.out 2>&1; then
  echo "expected score-math validation failure" >&2
  exit 1
fi
rg -n "bpc must equal log2_sum / token_count" /tmp/s7-validate-artifacts-bad.out >/dev/null

python3 - "$tmp" <<'PY'
from pathlib import Path
import json
import sys

root = Path(sys.argv[1])
score = root / "experiments/S7/scores/MoeTiny/seed-0/score.json"
data = json.loads(score.read_text(encoding="utf-8"))
data["bpc"] = 2.0
score.write_text(json.dumps(data, sort_keys=True, separators=(",", ":")) + "\n", encoding="utf-8")
PY

run_log_path="$tmp/experiments/S7/runs/MoeTiny/seed-0/run-log.json"
cp "$run_log_path" "$tmp/run-log.json.valid"
python3 - "$run_log_path" <<'PY'
from pathlib import Path
import json
import sys

path = Path(sys.argv[1])
data = json.loads(path.read_text(encoding="utf-8"))
data["completion"] = "Completed"
path.write_text(json.dumps(data, sort_keys=True, separators=(",", ":")) + "\n", encoding="utf-8")
PY

if scripts/review/f-s7/validate-artifacts.py --root "$tmp" --fast-fail >/tmp/s7-validate-artifacts-bad.out 2>&1; then
  echo "expected run-log completion shape validation failure" >&2
  exit 1
fi
rg -n 'completion must be Rust tagged S7Completion' /tmp/s7-validate-artifacts-bad.out >/dev/null
mv "$tmp/run-log.json.valid" "$run_log_path"

cp "$run_log_path" "$tmp/run-log.json.valid"
python3 - "$run_log_path" <<'PY'
from pathlib import Path
import json
import sys

path = Path(sys.argv[1])
data = json.loads(path.read_text(encoding="utf-8"))
data["losses"].pop()
path.write_text(json.dumps(data, sort_keys=True, separators=(",", ":")) + "\n", encoding="utf-8")
PY

if scripts/review/f-s7/validate-artifacts.py --root "$tmp" --fast-fail >/tmp/s7-validate-artifacts-bad.out 2>&1; then
  echo "expected completed run-log length validation failure" >&2
  exit 1
fi
rg -n 'losses length must be 20000 for completed run' /tmp/s7-validate-artifacts-bad.out >/dev/null
mv "$tmp/run-log.json.valid" "$run_log_path"

cp "$run_log_path" "$tmp/run-log.json.valid"
python3 - "$run_log_path" <<'PY'
from pathlib import Path
import json
import sys

path = Path(sys.argv[1])
data = json.loads(path.read_text(encoding="utf-8"))
data["grad_norms"][0][0] = 2
path.write_text(json.dumps(data, sort_keys=True, separators=(",", ":")) + "\n", encoding="utf-8")
PY

if scripts/review/f-s7/validate-artifacts.py --root "$tmp" --fast-fail >/tmp/s7-validate-artifacts-bad.out 2>&1; then
  echo "expected grad_norm step validation failure" >&2
  exit 1
fi
rg -n -F 'grad_norms[0] train_step must match loss step 1' /tmp/s7-validate-artifacts-bad.out >/dev/null
mv "$tmp/run-log.json.valid" "$run_log_path"

telemetry_path="$tmp/experiments/S7/runs/MoeTiny/seed-0/router-step-telemetry.jsonl"
cp "$telemetry_path" "$tmp/router-step-telemetry.jsonl.valid"
python3 - "$telemetry_path" <<'PY'
from pathlib import Path
import json
import sys

path = Path(sys.argv[1])
lines = path.read_text(encoding="utf-8").splitlines()
record = json.loads(lines[0])
record["telemetry_self_hash"] = "sha256:" + "5" * 64
lines[0] = json.dumps(record, sort_keys=True, separators=(",", ":"))
path.write_text("\n".join(lines) + "\n", encoding="utf-8")
PY

if scripts/review/f-s7/validate-artifacts.py --root "$tmp" --fast-fail >/tmp/s7-validate-artifacts-bad.out 2>&1; then
  echo "expected router telemetry self-hash validation failure" >&2
  exit 1
fi
rg -n "telemetry_self_hash self-hash mismatch" /tmp/s7-validate-artifacts-bad.out >/dev/null
mv "$tmp/router-step-telemetry.jsonl.valid" "$telemetry_path"

grad_path="$tmp/experiments/S7/runs/MoeTiny/seed-0/grad-log.jsonl"
cp "$grad_path" "$tmp/grad-log.jsonl.valid"
python3 - "$grad_path" <<'PY'
from pathlib import Path
import json
import sys

path = Path(sys.argv[1])
lines = path.read_text(encoding="utf-8").splitlines()
record = json.loads(lines[0])
record["grad_norms"]["global_l2"] = 9.0
lines[0] = json.dumps(record, sort_keys=True, separators=(",", ":"))
path.write_text("\n".join(lines) + "\n", encoding="utf-8")
PY

if scripts/review/f-s7/validate-artifacts.py --root "$tmp" --fast-fail >/tmp/s7-validate-artifacts-bad.out 2>&1; then
  echo "expected grad log/run log mismatch validation failure" >&2
  exit 1
fi
rg -n "grad_norms must match run-log grad_norms" /tmp/s7-validate-artifacts-bad.out >/dev/null
mv "$tmp/grad-log.jsonl.valid" "$grad_path"

cp "$grad_path" "$tmp/grad-log.jsonl.valid"
: >"$grad_path"

if scripts/review/f-s7/validate-artifacts.py --root "$tmp" --fast-fail >/tmp/s7-validate-artifacts-bad.out 2>&1; then
  echo "expected empty grad log validation failure" >&2
  exit 1
fi
rg -n "grad log must contain 20000 completed-run JSONL records" /tmp/s7-validate-artifacts-bad.out >/dev/null
mv "$tmp/grad-log.jsonl.valid" "$grad_path"

telemetry_path="$tmp/experiments/S7/runs/MoeTinyDenseMatched/seed-0/router-step-telemetry.jsonl"
cp "$telemetry_path" "$tmp/dense-router-step-telemetry.jsonl.valid"
python3 - "$telemetry_path" <<'PY'
from pathlib import Path
import json
import sys

path = Path(sys.argv[1])
path.write_text(json.dumps({"unexpected": True}, sort_keys=True, separators=(",", ":")) + "\n", encoding="utf-8")
PY

if scripts/review/f-s7/validate-artifacts.py --root "$tmp" --fast-fail >/tmp/s7-validate-artifacts-bad.out 2>&1; then
  echo "expected dense router telemetry validation failure" >&2
  exit 1
fi
rg -n "dense router-step telemetry must be empty" /tmp/s7-validate-artifacts-bad.out >/dev/null
mv "$tmp/dense-router-step-telemetry.jsonl.valid" "$telemetry_path"

python3 - "$tmp" <<'PY'
from pathlib import Path
import json
import sys

root = Path(sys.argv[1])
oracle = root / "experiments/S7/oracle-routed/seed-0/oracle.json"
data = json.loads(oracle.read_text(encoding="utf-8"))
data["route_coverage"]["consecutive_token_route_same"] = False
oracle.write_text(json.dumps(data, sort_keys=True, separators=(",", ":")) + "\n", encoding="utf-8")
PY

if scripts/review/f-s7/validate-artifacts.py --root "$tmp" --fast-fail >/tmp/s7-validate-artifacts-bad.out 2>&1; then
  echo "expected route coverage validation failure" >&2
  exit 1
fi
rg -n "route_coverage must prove all routed fixture axes" /tmp/s7-validate-artifacts-bad.out >/dev/null

echo "s7_validate_artifacts_test: ok"
